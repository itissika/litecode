use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use serde_json::json;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::client_protocol::ws;
use crate::serve::auth;
use crate::serve::settings;
use crate::serve::shutdown::{self, ShutdownWatch};
use crate::serve::state::ServeState;
use crate::session::manager::EMPTY_SESSION_TTL;
use crate::workspace::{restart_watcher, workspace_router};

pub fn router(state: ServeState, web_dist: PathBuf) -> Router {
    let cors = if state.auth_token.is_some() {
        CorsLayer::new()
    } else {
        // G1: without a token the CORS policy is tightened to localhost-only
        // origins (the desktop/dev case); previously it was wide open.
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(
                |origin: &axum::http::HeaderValue, _parts: &axum::http::request::Parts| {
                    origin
                        .to_str()
                        .ok()
                        .map(crate::serve::auth::origin_host)
                        .map(|host| host == "localhost" || host == "127.0.0.1" || host == "::1")
                        .unwrap_or(false)
                },
            ))
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let auth_layer = middleware::from_fn_with_state(state.clone(), auth::middleware);

    // G3: never record the full URI (which may carry the /ws token query) in
    // access logs — span carries method + path only.
    let trace_layer = TraceLayer::new_for_http().make_span_with(
        |request: &axum::http::Request<axum::body::Body>| {
            let path = request.uri().path().to_string();
            tracing::info_span!("http", method = %request.method(), path)
        },
    );

    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws::ws_handler))
        .nest("/api/workspace", workspace_router())
        .nest("/api/settings", settings::router())
        .fallback_service(ServeDir::new(web_dist))
        .layer(auth_layer)
        .layer(cors)
        .layer(trace_layer)
        .with_state(state)
}

async fn health(State(state): State<ServeState>) -> impl IntoResponse {
    let (workspace_root, workspace_id) = {
        let runtime = state.runtime.read().expect("runtime lock");
        (
            runtime
                .workspace
                .workspace_root
                .to_string_lossy()
                .to_string(),
            runtime.workspace.workspace_id.clone(),
        )
    };
    Json(json!({
        "ok": true,
        "workspace_root": workspace_root,
        "workspace_id": workspace_id,
    }))
}

pub async fn listen(
    state: ServeState,
    addr: SocketAddr,
    web_dist: PathBuf,
    shutdown_watch: ShutdownWatch,
) -> anyhow::Result<()> {
    let engine_manager = state.engine_manager.clone();
    let workspace_engines = state.workspace_engines.clone();
    let sessions = state.sessions.clone();
    let mcp_pool = state.runtime.read().expect("runtime lock").mcp_pool.clone();
    let session_gc = sessions.clone();
    restart_watcher(&state.watcher, state.workspace.clone()).await?;
    // Sole OS watcher → code_search worker Index dirty queue (DESIGN §2.9).
    {
        let workspace = state.workspace.clone();
        let engines = state.workspace_engines.clone();
        let runtime = state.runtime.clone();
        let turn_guard = state.turn_guard.clone();
        tokio::spawn(async move {
            let mut rx = workspace.subscribe_changes();
            loop {
                match rx.recv().await {
                    Ok(change) => {
                        // Sole ServeState reload of mcp.json / custom_tools.json.
                        // Skip during a turn (same as Settings 409); start_turn reads disk.
                        if !turn_guard.is_turn_in_progress()
                            && change.paths.iter().any(|p| {
                                crate::config::workspace::is_workspace_tool_defs_rel(p)
                            })
                        {
                            runtime
                                .write()
                                .expect("runtime lock")
                                .sync_workspace_tool_readiness();
                        }
                        let deleted = change.kind == "deleted";
                        engines
                            .code_search()
                            .notify_fs_changes(&change.paths, deleted);
                        engines
                            .text_index()
                            .notify_fs_changes(&change.paths, deleted);
                        if change.paths.iter().any(|p| {
                            crate::workspace::filter::path_triggers_code_index_sync(p)
                        }) {
                            engines.request_index_sync(workspace.sandbox().root());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // Missed discrete events — force a full disk↔index reconcile
                        // so the semantic index cannot stay silently stale.
                        tracing::warn!(
                            skipped,
                            "workspace change subscriber lagged; requesting index reconcile"
                        );
                        engines.code_search().request_reconcile();
                        engines.text_index().request_reconcile();
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    tokio::spawn(async move {
        session_gc.gc_stale_empty_sessions(EMPTY_SESSION_TTL).await;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
            session_gc.gc_stale_empty_sessions(EMPTY_SESSION_TTL).await;
        }
    });
    let app = router(state, web_dist);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let ready_line = format!("LITECODE_READY http://{local_addr}/");
    writeln!(std::io::stdout(), "{ready_line}")?;
    std::io::stdout().flush()?;
    tracing::info!("{ready_line}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown::wait_for_shutdown(shutdown_watch).await;
            sessions.shutdown_cleanup().await;
            mcp_pool.stop_all().await;
            engine_manager.stop_all();
            workspace_engines.stop_all();
        })
        .await?;
    Ok(())
}
