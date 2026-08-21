use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::task::JoinHandle;

use crate::client_protocol::controller::SessionController;
use crate::client_protocol::permission_bridge::PendingPermission;
use crate::client_protocol::protocol::{
    ErrorCode, JsonRpcErrorBody, JsonRpcRequestEnvelope, JsonRpcResponse, OperationKind,
    StructuredError,
};
use crate::permission::{self, AskOutcome, PermissionAction};

/// Upper bound for a single `agent/run` input payload (defensive cap).
const MAX_AGENT_RUN_INPUT_BYTES: usize = 256 * 1024;

pub fn emit(sink: &UnboundedSender<serde_json::Value>, msg: serde_json::Value) {
    tracing::debug!("wire notification sent");
    let _ = sink.send(msg);
}

pub fn ok_response(id: serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

pub fn err_response(id: serde_json::Value, code: i64, message: String) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcErrorBody { code, message }),
    }
}

fn operation_error(
    session: &SessionController,
    session_id: &str,
    ok: bool,
    message: &str,
    op: OperationKind,
    code: ErrorCode,
) -> serde_json::Value {
    use crate::client_protocol::project;
    let snapshot = session.snapshot_for(session_id).unwrap_or_else(|| {
        let binding = session.session_binding(session_id);
        project::buffer_snapshot(
            session_id,
            &session.project,
            &binding,
            -1,
            0,
            0,
            None,
            None,
            None,
            0,
            false,
        )
    });
    project::operation_result(
        op,
        ok,
        Some(StructuredError {
            code,
            message: message.into(),
        }),
        snapshot,
    )
}

/// Resolve session_id from params, falling back to the primary projection id.
fn resolve_sid(session: &SessionController, params_sid: &str) -> String {
    if !params_sid.is_empty() {
        params_sid.to_string()
    } else {
        session.first_session_id().unwrap_or_default()
    }
}

pub async fn handle_jsonrpc(
    session: &mut SessionController,
    sink: &UnboundedSender<serde_json::Value>,
    perm_tx: &UnboundedSender<PendingPermission>,
    rpc: &JsonRpcRequestEnvelope,
    terminal_hub: &std::sync::Arc<crate::terminal::TerminalHub>,
) -> bool {
    use crate::client_protocol::protocol::methods;
    let id = rpc.id.clone();

    match rpc.method.as_str() {
        methods::AGENT_RUN => {
            #[derive(serde::Deserialize)]
            struct Params {
                input: String,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value::<Params>(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            if params.input.len() > MAX_AGENT_RUN_INPUT_BYTES {
                emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32602,
                        format!(
                            "agent/run input exceeds {} bytes",
                            MAX_AGENT_RUN_INPUT_BYTES
                        ),
                    ))
                    .unwrap(),
                );
                return false;
            }
            let sid = resolve_sid(session, &params.session_id);
            if session.sessions.is_turn_running(&sid).await {
                emit(
                    sink,
                    operation_error(
                        session,
                        &sid,
                        false,
                        "agent already running",
                        OperationKind::Start,
                        ErrorCode::AgentAlreadyRunning,
                    ),
                );
                emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32000,
                        "agent already running".to_string(),
                    ))
                    .unwrap(),
                );
                return false;
            }
            // 2.14: generate the turn_id before creating the permission sink so
            // the wire carries a real turn_id (the sink previously saw "no-turn"
            // because it was created before the turn started).
            let turn_id = uuid::Uuid::new_v4().to_string();
            let permission_sink = session.permission_sink_for(&sid, perm_tx, &turn_id);
            match session
                .start_turn(&sid, &params.input, permission_sink, &turn_id)
                .await
            {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({"started": true})))
                            .unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::Start,
                            e.error_code(),
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::AGENT_CANCEL => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                session_id: String,
            }
            let params: Params = serde_json::from_value(rpc.params.clone()).unwrap_or(Params {
                session_id: String::new(),
            });
            let sid = resolve_sid(session, &params.session_id);
            {
                let sessions = session.sessions.clone();
                sessions.cancel_turn(&sid).await;
            }
            emit(
                sink,
                serde_json::to_value(ok_response(id, serde_json::json!({"cancelled": true})))
                    .unwrap(),
            );
        }

        methods::AGENT_PERMISSION => {
            emit(
                sink,
                serde_json::to_value(err_response(
                    id,
                    -32601,
                    "agent/permission not supported on this transport".into(),
                ))
                .unwrap(),
            );
        }

        methods::SESSION_NEW => match session.new_session().await {
            Ok(session_id) => {
                for msg in session.take_all_outgoing() {
                    emit(sink, msg);
                }
                emit(
                    sink,
                    serde_json::to_value(ok_response(
                        id,
                        serde_json::json!({"session_id": session_id}),
                    ))
                    .unwrap(),
                );
            }
            Err(e) => {
                let msg = e.to_string();
                let sid = session.first_session_id().unwrap_or_default();
                emit(
                    sink,
                    operation_error(
                        session,
                        &sid,
                        false,
                        &msg,
                        OperationKind::NewSession,
                        ErrorCode::Internal,
                    ),
                );
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                );
            }
        },

        methods::SESSION_SUBSCRIBE => {
            #[derive(serde::Deserialize)]
            struct Params {
                session_id: String,
            }
            let params: Params = match serde_json::from_value::<Params>(rpc.params.clone()) {
                Ok(params) if !params.session_id.is_empty() => params,
                Ok(_) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            "session_id must not be empty".into(),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
                Err(error) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {error}"),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };

            match session.subscribe_checked(&params.session_id).await {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&params.session_id) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(error) => {
                    let code = if matches!(
                        error.downcast_ref::<crate::types::LitecodeError>(),
                        Some(crate::types::LitecodeError::SessionNotFound(_))
                    ) {
                        ErrorCode::SessionNotFound
                    } else {
                        ErrorCode::Internal
                    };
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                    );
                    tracing::warn!(session_id = %params.session_id, ?code, "session subscribe failed");
                }
            }
        }

        methods::SESSION_UNSUBSCRIBE => {
            #[derive(serde::Deserialize)]
            struct Params {
                session_id: String,
            }
            match serde_json::from_value::<Params>(rpc.params.clone()) {
                Ok(params) if !params.session_id.is_empty() => {
                    session.unsubscribe(&params.session_id);
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Ok(_) => emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32602,
                        "session_id must not be empty".into(),
                    ))
                    .unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32602,
                        format!("Invalid params: {error}"),
                    ))
                    .unwrap(),
                ),
            }
        }

        methods::SESSION_DELETE => {
            #[derive(serde::Deserialize)]
            struct Params {
                id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            match session.delete_session(&params.id).await {
                Ok(()) => {
                    for msg in session.take_all_outgoing() {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    let code = if msg.contains("session is running") {
                        ErrorCode::AgentAlreadyRunning
                    } else if matches!(
                        e.downcast_ref::<crate::types::LitecodeError>(),
                        Some(crate::types::LitecodeError::SessionNotFound(_))
                    ) {
                        ErrorCode::SessionNotFound
                    } else {
                        ErrorCode::Internal
                    };
                    emit(
                        sink,
                        operation_error(
                            session,
                            &params.id,
                            false,
                            &msg,
                            OperationKind::DeleteSession,
                            code,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::SESSION_LIST => match session.list_sessions().await {
            Ok(sessions) => {
                for msg in session.take_all_outgoing() {
                    emit(sink, msg);
                }
                emit(
                    sink,
                    serde_json::to_value(ok_response(
                        id,
                        serde_json::json!({"sessions": sessions}),
                    ))
                    .unwrap(),
                );
            }
            Err(e) => {
                let msg = e.to_string();
                let sid = session.first_session_id().unwrap_or_default();
                emit(
                    sink,
                    operation_error(
                        session,
                        &sid,
                        false,
                        &msg,
                        OperationKind::ListSessions,
                        ErrorCode::Internal,
                    ),
                );
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                );
            }
        },

        methods::SESSION_SNAPSHOT => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                session_id: String,
            }
            let params: Params = serde_json::from_value(rpc.params.clone()).unwrap_or(Params {
                session_id: String::new(),
            });
            let sid = resolve_sid(session, &params.session_id);
            let snapshot = session.snapshot_for(&sid).or_else(|| session.snapshot());
            let Some(mut snapshot) = snapshot else {
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, "no session bound".to_string()))
                        .unwrap(),
                );
                return false;
            };
            snapshot.bash = Some(terminal_hub.jobs.wire_snapshot(&sid));
            for msg in session.take_all_outgoing() {
                emit(sink, msg);
            }
            emit(
                sink,
                serde_json::to_value(ok_response(id, serde_json::to_value(&snapshot).unwrap()))
                    .unwrap(),
            );
        }

        methods::SESSION_COMPACT => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {e}"),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.start_manual_compact(&sid).await {
                Ok(operation_id) => {
                    emit(
                        sink,
                        serde_json::to_value(ok_response(
                            id,
                            serde_json::json!({
                                "accepted": true,
                                "operation_id": operation_id,
                            }),
                        ))
                        .unwrap(),
                    );
                }
                Err(error) => {
                    let message = error.to_string();
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, message)).unwrap(),
                    );
                }
            }
        }

        methods::SESSION_REVERT_TO_USER_ANCHOR => {
            #[derive(serde::Deserialize)]
            struct Params {
                k: u32,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.revert_to_user_anchor(&sid, params.k) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::RevertToUserAnchor,
                            ErrorCode::Internal,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::SESSION_REVERT_FILES => {
            #[derive(serde::Deserialize)]
            struct Params {
                k: u32,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.revert_files(&sid, params.k) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::RevertFiles,
                            ErrorCode::Internal,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::AGENT_SET_PRIMARY => {
            #[derive(serde::Deserialize)]
            struct Params {
                agent_id: String,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.set_active_primary(&sid, &params.agent_id) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::SetActivePrimary,
                            ErrorCode::InvalidRequest,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::AGENT_SET_MODEL => {
            #[derive(serde::Deserialize)]
            struct Params {
                model_id: String,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.set_session_model(&sid, &params.model_id) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::SetModel,
                            ErrorCode::InvalidRequest,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::AGENT_SET_THINKING_TIER => {
            #[derive(serde::Deserialize)]
            struct Params {
                thinking_tier: String,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.set_thinking_tier(&sid, &params.thinking_tier) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::SetThinkingTier,
                            ErrorCode::InvalidRequest,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::AGENT_SET_CONTEXT_MODE => {
            #[derive(serde::Deserialize)]
            struct Params {
                context_mode: String,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.set_context_mode(&sid, &params.context_mode) {
                Ok(()) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(id, serde_json::json!({}))).unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        operation_error(
                            session,
                            &sid,
                            false,
                            &msg,
                            OperationKind::SetContextMode,
                            ErrorCode::InvalidRequest,
                        ),
                    );
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        methods::BUFFER_LOAD => {
            #[derive(serde::Deserialize)]
            struct Params {
                from_seq: crate::session::event::Seq,
                to_seq: crate::session::event::Seq,
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(p) => p,
                Err(e) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(
                            id,
                            -32602,
                            format!("Invalid params: {}", e),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            match session.materialize_range(&sid, params.from_seq, params.to_seq) {
                Ok(range) => {
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    let subagent_bindings = session.child_bindings_for_parent(&sid);
                    let result = crate::client_protocol::protocol::BufferLoadResult {
                        session_id: sid.clone(),
                        from_seq: params.from_seq,
                        to_seq: params.to_seq,
                        events: range.events,
                        subagent_bindings,
                        user_detail_before: range.user_detail_before,
                    };
                    emit(
                        sink,
                        serde_json::to_value(ok_response(
                            id,
                            serde_json::to_value(result).unwrap_or_default(),
                        ))
                        .unwrap(),
                    );
                }
                Err(e) => {
                    let msg = e.to_string();
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                    );
                }
            }
        }

        _ => {
            emit(
                sink,
                serde_json::to_value(err_response(
                    id,
                    -32601,
                    format!("Method not found: {}", rpc.method),
                ))
                .unwrap(),
            );
        }
    }
    // Load-bearing fall-through for arms that do not `return` explicitly
    // (e.g. AGENT_SUBSCRIBE, AGENT_CANCEL): `false` = keep the session loop
    // running. Not dead — removing it breaks those arms.
    false
}

/// Flush any pending outgoing frames from the session controller.
fn finalize_ready_turn(
    session: &mut SessionController,
    response_tx: &UnboundedSender<serde_json::Value>,
) {
    for msg in session.take_all_outgoing() {
        emit(response_tx, msg);
    }
}

/// Represents a request to the session loop - either a JSON-RPC call or a transport action.
#[derive(Debug)]
pub enum SessionRequest {
    JsonRpc(JsonRpcRequestEnvelope),
    PermissionGrant {
        request_id: String,
        tool: String,
        approved: bool,
        always: bool,
    },
    SubscribeSession {
        session_id: String,
    },
    UnsubscribeSession {
        session_id: String,
    },
    Cancel,
    Quit,
}

pub async fn run_session_loop(
    session: &mut SessionController,
    mut request_rx: tokio::sync::mpsc::UnboundedReceiver<SessionRequest>,
    response_tx: UnboundedSender<serde_json::Value>,
    perm_tx: UnboundedSender<PendingPermission>,
    mut perm_rx: tokio::sync::mpsc::UnboundedReceiver<PendingPermission>,
    terminal_hub: std::sync::Arc<crate::terminal::TerminalHub>,
) {
    loop {
        finalize_ready_turn(session, &response_tx);

        tokio::select! {
            // Merged broadcast events from all subscribed sessions.
            Some((sid, envelope)) = session.merged_rx.recv() => {
                let project = session.project.clone();
                let binding = session.session_binding(&sid);
                if let Some(proj) = session.projection_mut(&sid) {
                    proj.on_internal(envelope, &project, &binding);
                    for msg in proj.take_outgoing() {
                        emit(&response_tx, msg);
                    }
                }
            }

            Some(perm) = perm_rx.recv() => {
                let sid = perm.session_id.clone();
                let project = session.project.clone();
                let binding = session.session_binding(&sid);
                if let Some(proj) = session.projection_mut(&sid) {
                    proj.on_event(
                        crate::client_protocol::observer::InternalEvent::PermissionAsk {
                            session_id: perm.session_id.clone(),
                            turn_id: perm.turn_id.clone(),
                            request_id: perm.request_id.clone(),
                            tool: perm.tool.clone(),
                            rule_id: perm.rule_id.clone(),
                            summary: perm.summary.clone(),
                        },
                        &project,
                        &binding,
                    );
                    for msg in proj.take_outgoing() {
                        emit(&response_tx, msg);
                    }
                }

                // A grant may have arrived before this wait started — the stray
                // arms below buffer it instead of dropping it, so the wait never
                // hangs on the race. Consume the buffered match first.
                let mut queued: Vec<SessionRequest> = Vec::new();
                let buffered_grant = session
                    .stray_grants
                    .iter()
                    .position(|r| {
                        matches!(
                            r,
                            SessionRequest::PermissionGrant { request_id, .. }
                                if request_id == &perm.request_id
                        )
                    })
                    .map(|i| session.stray_grants.remove(i).unwrap());
                if let Some(SessionRequest::PermissionGrant {
                    request_id,
                    tool,
                    approved,
                    always,
                }) = buffered_grant
                {
                    tracing::info!(
                        request_id = %request_id,
                        tool = %tool,
                        approved,
                        always,
                        "grant_permission consumed from stray buffer"
                    );
                    if approved && always {
                        permission::grant_runtime(
                            &perm.agent_name,
                            &tool,
                            &perm.rule_id,
                            PermissionAction::Allow,
                        );
                    }
                    perm.reply_tx.send(AskOutcome::from_reply(approved, always)).ok();
                    let project = session.project.clone();
                    let binding = session.session_binding(&sid);
                    if let Some(proj) = session.projection_mut(&sid) {
                        proj.on_event(
                            crate::client_protocol::observer::InternalEvent::PermissionResolved {
                                tool: tool.clone(),
                                approved,
                                always,
                            },
                            &project,
                            &binding,
                        );
                        for msg in proj.take_outgoing() {
                            emit(&response_tx, msg);
                        }
                    }
                } else {
                loop {
                    tokio::select! {
                        Some((sid2, envelope)) = session.merged_rx.recv() => {
                            let project = session.project.clone();
                            let binding = session.session_binding(&sid2);
                            if let Some(proj) = session.projection_mut(&sid2) {
                                proj.on_internal(envelope, &project, &binding);
                                for msg in proj.take_outgoing() {
                                    emit(&response_tx, msg);
                                }
                            }
                        }
                        req = request_rx.recv() => {
                            match req {
                                Some(SessionRequest::PermissionGrant {
                                    request_id,
                                    tool,
                                    approved,
                                    always,
                                }) if request_id == perm.request_id => {
                                    tracing::info!(
                                        request_id = %request_id,
                                        tool = %tool,
                                        approved,
                                        always,
                                        "grant_permission received"
                                    );
                                    if approved && always {
                                        permission::grant_runtime(
                                            &perm.agent_name,
                                            &tool,
                                            &perm.rule_id,
                                            PermissionAction::Allow,
                                        );
                                    }
                                    perm.reply_tx.send(AskOutcome::from_reply(approved, always)).ok();
                                    let project = session.project.clone();
                                    let binding = session.session_binding(&sid);
                                    if let Some(proj) = session.projection_mut(&sid) {
                                        proj.on_event(
                                            crate::client_protocol::observer::InternalEvent::PermissionResolved {
                                                tool: tool.clone(),
                                                approved,
                                                always,
                                            },
                                            &project,
                                            &binding,
                                        );
                                        for msg in proj.take_outgoing() {
                                            emit(&response_tx, msg);
                                        }
                                    }
                                    break;
                                }
                                Some(SessionRequest::PermissionGrant { request_id, .. }) => {
                                    tracing::warn!(
                                        request_id = %request_id,
                                        expected = %perm.request_id,
                                        "grant_permission request_id mismatch"
                                    );
                                }
                                Some(SessionRequest::Cancel) | Some(SessionRequest::Quit) => {
                                    // Do not reply Deny: abort must interrupt Ask, not
                                    // continue the loop with permission-denied. Dropping
                                    // the oneshot wakes the wait as Aborted.
                                    drop(perm.reply_tx);
                                    queued.push(SessionRequest::Cancel);
                                    break;
                                }
                                Some(other) => {
                                    queued.push(other);
                                }
                                None => break,
                            }
                        }
                    }
                }
                } // end else: no buffered grant → wait inline
                // Re-inject queued requests into the appropriate projection's deferred queue.
                // If no projection exists for this sid, use the controller's dummy queue.
                if let Some(proj) = session.projection_mut(&sid) {
                    for req in queued {
                        proj.deferred_mut().push_back(req);
                    }
                } else {
                    for req in queued {
                        session._dummy_deferred.push_back(req);
                    }
                }
            }

            req = request_rx.recv() => {
                match req {
                    Some(SessionRequest::Quit) => {
                        // Cancel all running turns for all subscribed sessions.
                        let sids: Vec<String> = session.projections.keys().cloned().collect();
                        for sid in &sids {
                            session.sessions.cancel_turn(sid).await;
                        }
                        break;
                    }
                    Some(SessionRequest::Cancel) => {
                        // Cancel all running turns for all subscribed sessions.
                        let sids: Vec<String> = session.projections.keys().cloned().collect();
                        for sid in &sids {
                            session.sessions.cancel_turn(sid).await;
                        }
                        for msg in session.take_all_outgoing() {
                            emit(&response_tx, msg);
                        }
                    }
                    Some(SessionRequest::JsonRpc(rpc)) => {
                        if handle_jsonrpc(session, &response_tx, &perm_tx, &rpc, &terminal_hub)
                            .await
                        {
                            break;
                        }
                    }
                    Some(stray @ SessionRequest::PermissionGrant { .. }) => {
                        // Handled inline during permission wait; if we get here it's
                        // a stray grant — buffer it so a subsequent wait (which may
                        // have started a moment later) can still consume it.
                        session.stray_grants.push_back(stray);
                    }
                    Some(SessionRequest::SubscribeSession { session_id }) => {
                        session.subscribe(&session_id).await;
                        for msg in session.take_outgoing_for(&session_id) {
                            emit(&response_tx, msg);
                        }
                    }
                    Some(SessionRequest::UnsubscribeSession { session_id }) => {
                        session.unsubscribe(&session_id);
                    }
                    None => break,
                }
            }
        }

        // Process deferred requests from all projections.
        let sids: Vec<String> = session.projections.keys().cloned().collect();
        for sid in sids {
            let deferred: Vec<SessionRequest> = {
                if let Some(proj) = session.projection_mut(&sid) {
                    proj.deferred.drain(..).collect()
                } else {
                    continue;
                }
            };
            for req in deferred {
                match req {
                    SessionRequest::Quit => {
                        session.sessions.cancel_turn(&sid).await;
                        return;
                    }
                    SessionRequest::Cancel => {
                        session.sessions.cancel_turn(&sid).await;
                        if let Some(proj) = session.projection_mut(&sid) {
                            for msg in proj.take_outgoing() {
                                emit(&response_tx, msg);
                            }
                        }
                    }
                    SessionRequest::JsonRpc(rpc) => {
                        if handle_jsonrpc(session, &response_tx, &perm_tx, &rpc, &terminal_hub)
                            .await
                        {
                            return;
                        }
                    }
                    stale @ SessionRequest::PermissionGrant { .. } => {
                        // Stale permission grant — buffer it for a later wait.
                        session.stray_grants.push_back(stale);
                    }
                    SessionRequest::SubscribeSession { session_id } => {
                        session.subscribe(&session_id).await;
                        if let Some(proj) = session.projection_mut(&session_id) {
                            for msg in proj.take_outgoing() {
                                emit(&response_tx, msg);
                            }
                        }
                    }
                    SessionRequest::UnsubscribeSession { session_id } => {
                        session.unsubscribe(&session_id);
                    }
                }
            }
        }

        // Process controller-level deferred requests (no projection).
        let dummy_deferred: Vec<SessionRequest> = session._dummy_deferred.drain(..).collect();
        for req in dummy_deferred {
            match req {
                SessionRequest::Quit => break,
                SessionRequest::Cancel => {}
                SessionRequest::JsonRpc(rpc) => {
                    if handle_jsonrpc(session, &response_tx, &perm_tx, &rpc, &terminal_hub).await {
                        return;
                    }
                }
                stale @ SessionRequest::PermissionGrant { .. } => {
                    session.stray_grants.push_back(stale);
                }
                SessionRequest::SubscribeSession { session_id } => {
                    session.subscribe(&session_id).await;
                    for msg in session.take_outgoing_for(&session_id) {
                        emit(&response_tx, msg);
                    }
                }
                SessionRequest::UnsubscribeSession { session_id } => {
                    session.unsubscribe(&session_id);
                }
            }
        }

        finalize_ready_turn(session, &response_tx);
    }
}

/// In-process connection to `run_session_loop` (CLI / tests).
pub struct ConnectionHandle {
    pub request_tx: mpsc::UnboundedSender<SessionRequest>,
    response_rx: mpsc::UnboundedReceiver<serde_json::Value>,
    loop_handle: JoinHandle<()>,
}

impl ConnectionHandle {
    pub fn spawn(session: SessionController) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, response_rx) = mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = mpsc::unbounded_channel();
        let loop_handle = tokio::spawn(async move {
            let mut session = session;
            run_session_loop(
                &mut session,
                request_rx,
                response_tx,
                perm_tx,
                perm_rx,
                std::sync::Arc::new(crate::terminal::TerminalHub::new()),
            )
            .await;
        });
        Self {
            request_tx,
            response_rx,
            loop_handle,
        }
    }

    pub async fn next_envelope(&mut self) -> Option<serde_json::Value> {
        self.response_rx.recv().await
    }

    pub fn abort(self) {
        self.loop_handle.abort();
    }
}
