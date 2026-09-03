use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::client_protocol::connection::{self, SessionRequest, emit, ok_response};
use crate::client_protocol::controller::SessionController;
use crate::client_protocol::permission_bridge::PendingPermission;
use crate::client_protocol::project;
use crate::client_protocol::protocol::{JsonRpcRequestEnvelope, TransportRequest, methods};
use crate::client_protocol::workspace;
use crate::engines::WorkspaceEngines;
use crate::serve::state::ServeState;
use crate::telemetry;

#[derive(Debug, Deserialize)]
pub struct WsQuery {
    token: Option<String>,
    /// Optional `?session=<id>` query param used as an initial subscribe hint.
    /// When present, the connection auto-subscribes to that session after
    /// handshake. Not a single-binding parameter.
    #[serde(default)]
    session: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<ServeState>,
    Query(query): Query<WsQuery>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    // G1: reject cross-origin / fabricated Origin on the WS handshake. Native
    // clients omit the header (allowed); browsers always send Origin.
    if let Some(origin) = headers.get(axum::http::header::ORIGIN)
        && let Ok(origin_str) = origin.to_str()
    {
        // G1: only localhost origins may open the WS handshake (bracketed IPv6
        // like `http://[::1]:3000` is normalized before the port split).
        let host = crate::serve::auth::origin_host(origin_str);
        if host != "localhost" && host != "127.0.0.1" && host != "::1" {
            return axum::http::StatusCode::FORBIDDEN.into_response();
        }
    }

    let session_hint = query.session.clone();
    if let Some(expected) = &state.auth_token
        && query.token.as_deref() != Some(expected.as_str())
    {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state, session_hint))
}

fn spawn_stats_ticker(
    response_tx: mpsc::UnboundedSender<serde_json::Value>,
    engines: Arc<WorkspaceEngines>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let emit_stats = || {
            let sample = engines.memory_sample();
            let _ = response_tx.send(project::server_stats(sample));
        };
        emit_stats();
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_stats();
            if response_tx.is_closed() {
                break;
            }
        }
    })
}

fn spawn_log_forwarder(response_tx: mpsc::UnboundedSender<serde_json::Value>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut rx = telemetry::subscribe_logs();
        loop {
            match rx.recv().await {
                Ok(line) => {
                    if response_tx.send(project::log_line(line)).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn spawn_terminal_forwarder(
    response_tx: mpsc::UnboundedSender<serde_json::Value>,
    mut rx: mpsc::UnboundedReceiver<crate::terminal::TerminalEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let msg = match ev.kind {
                crate::terminal::TerminalEventKind::Data(data) => {
                    project::terminal_data(&ev.id, &data)
                }
                crate::terminal::TerminalEventKind::Exit { code } => {
                    project::terminal_exit(&ev.id, code)
                }
            };
            if response_tx.send(msg).is_err() {
                break;
            }
        }
    })
}

async fn handle_socket(socket: WebSocket, state: ServeState, session_hint: Option<String>) {
    let conn_id = crate::terminal::ConnectionId::new();
    let terminal_hub = state.terminal_hub.clone();
    let mut runtime = state.runtime_snapshot();
    if let Err(e) = runtime.apply(crate::config::DocId::ALL) {
        tracing::error!("runtime settings reload failed: {}", e);
        return;
    }
    let workspace_runtime = runtime.clone();
    let sessions = state.sessions.clone();
    let mut session = match SessionController::with_turn_guard(runtime, None, sessions.clone()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("session init failed: {}", e);
            return;
        }
    };
    // If a session hint is provided, auto-subscribe to that session.
    if let Some(ref sid) = session_hint {
        session.subscribe(sid).await;
    }

    tracing::info!(
        hint = ?session_hint,
        projections = session.projections.len(),
        "websocket connected"
    );

    let (perm_tx, perm_rx) = mpsc::unbounded_channel::<PendingPermission>();
    let (request_tx, request_rx) = mpsc::unbounded_channel::<SessionRequest>();
    let (response_tx, mut response_rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let mut settings_rx = state.subscribe_settings();

    let stats_handle = spawn_stats_ticker(response_tx.clone(), state.workspace_engines.clone());
    let terminal_rx = terminal_hub.attach_connection(conn_id.clone());
    let terminal_handle = spawn_terminal_forwarder(response_tx.clone(), terminal_rx);
    let mut log_forwarder: Option<JoinHandle<()>> = None;

    let workspace = state.workspace.clone();
    let workspace_tx = response_tx.clone();
    tokio::spawn(async move {
        let mut broadcast_rx = workspace.subscribe_changes();
        loop {
            match broadcast_rx.recv().await {
                Ok(change) => {
                    let Some(change) = crate::workspace::filter_change_for_ui(change) else {
                        continue;
                    };
                    if workspace_tx
                        .send(project::notification(
                            "workspace/changed",
                            serde_json::json!({
                                "paths": change.paths,
                                "kind": change.kind,
                            }),
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let settings_tx = response_tx.clone();
    tokio::spawn(async move {
        loop {
            match settings_rx.recv().await {
                Ok(event) => {
                    let msg = project::settings_changed(event.revision, &event.docs, event.summary);
                    if settings_tx.send(msg).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let lsp_hub = state.workspace_engines.lsp_hub();
    let mut lsp_diag_rx = lsp_hub.subscribe_diagnostics();
    let lsp_diag_tx = response_tx.clone();
    tokio::spawn(async move {
        loop {
            match lsp_diag_rx.recv().await {
                Ok(ev) => {
                    if lsp_diag_tx
                        .send(project::notification(
                            methods::LSP_DIAGNOSTICS,
                            serde_json::json!({
                                "uri": ev.uri,
                                "version": ev.version,
                                "diagnostics": ev.diagnostics,
                            }),
                        ))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // G0: forward session lifecycle events to this connection.
    let lifecycle_tx = response_tx.clone();
    let mut lifecycle_rx = state.sessions.subscribe_lifecycle();
    tokio::spawn(async move {
        loop {
            match lifecycle_rx.recv().await {
                Ok(msg) => {
                    if lifecycle_tx
                        .send(project::lifecycle_event_to_wire(&msg))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    for msg in session.handshake_frames() {
        emit(&response_tx, msg);
    }

    let loop_session = session;
    let loop_tx = response_tx.clone();
    let loop_perm_tx = perm_tx.clone();
    let loop_hub = terminal_hub.clone();
    let loop_handle = tokio::spawn(async move {
        let mut session = loop_session;
        connection::run_session_loop(
            &mut session,
            request_rx,
            loop_tx,
            loop_perm_tx,
            perm_rx,
            loop_hub,
        )
        .await;
    });

    let (mut ws_tx, mut ws_rx) = socket.split();

    let writer = tokio::spawn(async move {
        while let Some(resp) = response_rx.recv().await {
            let line = match serde_json::to_string(&resp) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("response serialize failed: {}", e);
                    continue;
                }
            };
            if ws_tx.send(Message::Text(line.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = ws_rx.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }

        // Try JSON-RPC 2.0 first
        if let Ok(rpc) = serde_json::from_str::<JsonRpcRequestEnvelope>(&text)
            && rpc.jsonrpc == "2.0"
        {
            if workspace::is_workspace_method(&rpc.method) {
                workspace::handle_jsonrpc(
                    &workspace_runtime,
                    &response_tx,
                    &terminal_hub,
                    &conn_id,
                    rpc,
                )
                .await;
                continue;
            }
            // Intercept agent/permission and convert to PermissionGrant.
            // Enqueue acceptance is the JSON-RPC receipt (same pattern as
            // subscribe_logs); grant application still happens asynchronously.
            if rpc.method == methods::AGENT_PERMISSION {
                #[derive(serde::Deserialize)]
                struct PermissionParams {
                    request_id: String,
                    tool: String,
                    approved: bool,
                    #[serde(default)]
                    always: bool,
                }
                match serde_json::from_value::<PermissionParams>(rpc.params.clone()) {
                    Ok(params) => {
                        let request_id = params.request_id.clone();
                        if request_tx
                            .send(SessionRequest::PermissionGrant {
                                request_id: params.request_id,
                                tool: params.tool,
                                approved: params.approved,
                                always: params.always,
                            })
                            .is_err()
                        {
                            break;
                        }
                        emit(
                            &response_tx,
                            serde_json::to_value(ok_response(
                                rpc.id.clone(),
                                serde_json::json!({
                                    "accepted": true,
                                    "request_id": request_id,
                                }),
                            ))
                            .unwrap(),
                        );
                        continue;
                    }
                    Err(error) => {
                        emit(
                            &response_tx,
                            serde_json::to_value(connection::err_response(
                                rpc.id.clone(),
                                -32602,
                                format!("Invalid params: {error}"),
                            ))
                            .unwrap(),
                        );
                        continue;
                    }
                }
            }
            // Intercept log subscription and convert to the log forwarder
            // task, mirroring the AGENT_PERMISSION intercept above. This
            // converges the log stream onto the unified JSON-RPC protocol
            // (the frontend sends `subscribe_logs` as a JSON-RPC method).
            if rpc.method == methods::SUBSCRIBE_LOGS {
                if log_forwarder.is_none() {
                    log_forwarder = Some(spawn_log_forwarder(response_tx.clone()));
                }
                emit(
                    &response_tx,
                    serde_json::to_value(ok_response(rpc.id.clone(), serde_json::json!({})))
                        .unwrap(),
                );
                continue;
            }
            if rpc.method == methods::UNSUBSCRIBE_LOGS {
                if let Some(handle) = log_forwarder.take() {
                    handle.abort();
                }
                emit(
                    &response_tx,
                    serde_json::to_value(ok_response(rpc.id.clone(), serde_json::json!({})))
                        .unwrap(),
                );
                continue;
            }
            if request_tx.send(SessionRequest::JsonRpc(rpc)).is_err() {
                break;
            }
            continue;
        }

        // Fallback: try TransportRequest (only Quit remains a transport-level
        // control with no JSON-RPC method equivalent).
        match serde_json::from_str::<TransportRequest>(&text) {
            Ok(TransportRequest::Quit) => {
                let _ = request_tx.send(SessionRequest::Quit);
                break;
            }
            Err(e) => {
                tracing::warn!("invalid request: {} — line: {}", e, text);
            }
        }
    }

    if let Some(handle) = log_forwarder.take() {
        handle.abort();
    }
    stats_handle.abort();
    terminal_handle.abort();
    let cleanup_hub = state.terminal_hub.clone();
    let cleanup_conn = conn_id.clone();
    let _ = tokio::task::spawn_blocking(move || cleanup_hub.disconnect(&cleanup_conn)).await;
    // Disconnecting a WebSocket no longer cancels the turn: turn lifecycle is
    // owned by the process-level `SessionManager`. The connection's broadcast
    // subscription is dropped when `session` is dropped (task exit), which
    // detaches it from the session's event fan-out.
    drop(request_tx);
    let _ = loop_handle.await;
    writer.abort();
}
