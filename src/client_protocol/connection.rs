use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::task::JoinHandle;

use crate::client_protocol::controller::SessionController;
use crate::client_protocol::permission_bridge::PendingPermission;
use crate::client_protocol::protocol::{
    ErrorCode, JsonRpcErrorBody, JsonRpcRequestEnvelope, JsonRpcResponse, OperationKind,
    StructuredError,
};
use std::collections::HashMap;

use crate::permission::{self, AskAnswer, AskOutcome, AskReply, PermissionAction};

/// Upper bound for a single `agent/run` input payload (defensive cap).
const MAX_AGENT_RUN_INPUT_BYTES: usize = 256 * 1024;

#[derive(serde::Deserialize)]
struct ImageParam {
    #[serde(rename = "ref")]
    media_ref: String,
}

fn image_refs(images: Vec<ImageParam>) -> Result<Vec<String>, String> {
    crate::session::media::normalize_image_refs(images.into_iter().map(|image| image.media_ref))
}

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
            retryable: false,
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

fn emit_user_anchors(
    session: &SessionController,
    sink: &UnboundedSender<serde_json::Value>,
    id: serde_json::Value,
    session_id: &str,
    anchor_seq: Option<i64>,
    before: u32,
    after: u32,
) {
    match session.user_anchor_window(session_id, anchor_seq, i64::from(before), i64::from(after)) {
        Ok(window) => {
            let anchors = window
                .seqs
                .into_iter()
                .filter_map(|seq| u64::try_from(seq).ok())
                .map(|seq| crate::client_protocol::protocol::UserAnchorWire { seq })
                .collect();
            let result = crate::client_protocol::protocol::UserAnchorsResult {
                session_id: session_id.to_string(),
                anchors,
                anchor_seq: window.anchor.and_then(|seq| u64::try_from(seq).ok()),
                has_more_before: window.has_more_before,
                has_more_after: window.has_more_after,
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
            emit(
                sink,
                serde_json::to_value(err_response(id, -32000, e.to_string())).unwrap(),
            );
        }
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
                #[serde(default)]
                input: String,
                #[serde(default)]
                images: Vec<ImageParam>,
                #[serde(default)]
                session_id: String,
                #[serde(default)]
                plan_execution: bool,
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
            let images = match image_refs(params.images) {
                Ok(images) => images,
                Err(error) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32602, error)).unwrap(),
                    );
                    return false;
                }
            };
            let text = params.input.trim().to_string();
            if text.is_empty() && images.is_empty() {
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32602, "empty message".into())).unwrap(),
                );
                return false;
            }
            let user_input = crate::types::UserInput { text, images };
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
                .start_turn(
                    &sid,
                    user_input,
                    permission_sink,
                    &turn_id,
                    params.plan_execution,
                )
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

        methods::AGENT_RETRY => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                session_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params.clone()) {
                Ok(params) => params,
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
            let sid = resolve_sid(session, &params.session_id);
            if session.sessions.is_child_session(&sid) {
                emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32000,
                        "cannot retry a subagent session".into(),
                    ))
                    .unwrap(),
                );
                return false;
            }
            if session.sessions.is_turn_running(&sid).await
                || !session.sessions.llm_reconnect_retryable(&sid)
            {
                let message = if session.sessions.is_turn_running(&sid).await {
                    "agent already running"
                } else {
                    "nothing to retry"
                };
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, message.into())).unwrap(),
                );
                return false;
            }
            let turn_id = uuid::Uuid::new_v4().to_string();
            let permission_sink = session.permission_sink_for(&sid, perm_tx, &turn_id);
            match session
                .retry_failed_turn(&sid, permission_sink, &turn_id)
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
                Err(error) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
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

        methods::SESSION_PENDING_ENQUEUE => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                text: String,
                #[serde(default)]
                images: Vec<ImageParam>,
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
                            format!("Invalid params: {e}"),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            if params.text.len() > MAX_AGENT_RUN_INPUT_BYTES {
                emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32602,
                        format!(
                            "pending message exceeds {} bytes",
                            MAX_AGENT_RUN_INPUT_BYTES
                        ),
                    ))
                    .unwrap(),
                );
                return false;
            }
            let text = params.text.trim().to_string();
            let images = match image_refs(params.images) {
                Ok(images) => images,
                Err(error) => {
                    emit(
                        sink,
                        serde_json::to_value(err_response(id, -32602, error)).unwrap(),
                    );
                    return false;
                }
            };
            if text.is_empty() && images.is_empty() {
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32602, "empty message".into())).unwrap(),
                );
                return false;
            }
            let sid = resolve_sid(session, &params.session_id);
            if let Err(e) = session.sessions.ensure_entry(&sid).await {
                let msg = e.to_string();
                emit(
                    sink,
                    operation_error(
                        session,
                        &sid,
                        false,
                        &msg,
                        OperationKind::Start,
                        ErrorCode::Internal,
                    ),
                );
                emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                );
                return false;
            }
            match session
                .sessions
                .enqueue_user_input(&sid, crate::types::UserInput { text, images })
            {
                Ok(_) => {
                    // Idle race: the turn ended between the composer's last state
                    // and this RPC. Send the queue as a fresh turn instead of
                    // waiting for a TurnFinished that will never come.
                    if let Some(claimed) = session.sessions.claim_pending_if_idle(&sid)
                        && !claimed.is_empty()
                    {
                        let merged = crate::session::manager::merge_pending(&claimed);
                        let turn_id = uuid::Uuid::new_v4().to_string();
                        let permission_sink = session.permission_sink_for(&sid, perm_tx, &turn_id);
                        if let Err(error) = session
                            .start_turn(&sid, merged, permission_sink, &turn_id, false)
                            .await
                        {
                            session.sessions.restore_pending_messages(&sid, claimed);
                            let msg = error.to_string();
                            emit(
                                sink,
                                operation_error(
                                    session,
                                    &sid,
                                    false,
                                    &msg,
                                    OperationKind::Start,
                                    error.error_code(),
                                ),
                            );
                            emit(
                                sink,
                                serde_json::to_value(err_response(id, -32000, msg)).unwrap(),
                            );
                            return false;
                        }
                    }
                    for msg in session.take_outgoing_for(&sid) {
                        emit(sink, msg);
                    }
                    emit(
                        sink,
                        serde_json::to_value(ok_response(
                            id,
                            serde_json::json!({
                                "queued": true,
                                "pending_messages":
                                    session.sessions.pending_messages_snapshot(&sid),
                            }),
                        ))
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

        methods::SESSION_PENDING_REMOVE => {
            #[derive(serde::Deserialize)]
            struct Params {
                id: String,
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
                            format!("Invalid params: {e}"),
                        ))
                        .unwrap(),
                    );
                    return false;
                }
            };
            let sid = resolve_sid(session, &params.session_id);
            let removed = session.sessions.remove_pending_message(&sid, &params.id);
            emit(
                sink,
                serde_json::to_value(ok_response(
                    id,
                    serde_json::json!({
                        "removed": removed,
                        "pending_messages": session.sessions.pending_messages_snapshot(&sid),
                    }),
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
            snapshot.pending_messages = session.sessions.pending_messages_snapshot(&sid);
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
                seq: u64,
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
            match session.revert_to_user_anchor(&sid, params.seq) {
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
                seq: u64,
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
            match session.revert_files(&sid, params.seq) {
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

        methods::BUFFER_USER_ANCHORS => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default)]
                session_id: String,
                #[serde(default)]
                anchor_seq: Option<u64>,
                #[serde(default)]
                before: u32,
                #[serde(default)]
                after: u32,
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
            let anchor_seq = match params.anchor_seq {
                None => None,
                Some(seq) => match i64::try_from(seq) {
                    Ok(seq) => Some(seq),
                    Err(_) => {
                        emit(
                            sink,
                            serde_json::to_value(err_response(
                                id,
                                -32602,
                                "Invalid params: anchor_seq".to_string(),
                            ))
                            .unwrap(),
                        );
                        return false;
                    }
                },
            };
            let sid = resolve_sid(session, &params.session_id);
            emit_user_anchors(
                session,
                sink,
                id,
                &sid,
                anchor_seq,
                params.before,
                params.after,
            );
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

/// Outstanding Ask: agent blocks on `reply_tx`; the connection loop must not.
struct PendingAsk {
    session_id: String,
    agent_name: String,
    tool: String,
    rule_id: String,
    reply_tx: tokio::sync::oneshot::Sender<AskReply>,
}

/// Result of applying a grant against the pending-ask registry.
enum GrantApply {
    Resolved {
        session_id: String,
        tool: String,
        approved: bool,
        always: bool,
    },
    /// No matching Ask — caller must buffer as stray (do not silently drop).
    Unknown {
        request_id: String,
        tool: String,
        approved: bool,
        always: bool,
        free_text: Option<String>,
        selected: Vec<String>,
        answers: HashMap<String, AskAnswer>,
    },
}

fn apply_permission_grant(
    pending_asks: &mut HashMap<String, PendingAsk>,
    request_id: String,
    tool: String,
    approved: bool,
    always: bool,
    free_text: Option<String>,
    selected: Vec<String>,
    answers: HashMap<String, AskAnswer>,
) -> GrantApply {
    let Some(pending) = pending_asks.remove(&request_id) else {
        return GrantApply::Unknown {
            request_id,
            tool,
            approved,
            always,
            free_text,
            selected,
            answers,
        };
    };
    if approved && always {
        permission::grant_runtime(
            &pending.agent_name,
            &tool,
            &pending.rule_id,
            PermissionAction::Allow,
        );
    }
    let _ = pending
        .reply_tx
        .send(AskReply::from_grant(approved, always, free_text, selected, answers));
    GrantApply::Resolved {
        session_id: pending.session_id,
        tool,
        approved,
        always,
    }
}

/// Non-accept path for outstanding Asks (cancel / quit / disconnect): reject.
fn reject_all_pending_asks(
    pending_asks: &mut HashMap<String, PendingAsk>,
) -> Vec<(String, String)> {
    pending_asks
        .drain()
        .map(|(_, pending)| {
            let _ = pending
                .reply_tx
                .send(AskReply::from_outcome(AskOutcome::Deny));
            (pending.session_id, pending.tool)
        })
        .collect()
}

fn emit_permission_resolved(
    session: &mut SessionController,
    response_tx: &UnboundedSender<serde_json::Value>,
    session_id: &str,
    tool: &str,
    approved: bool,
    always: bool,
) {
    let project = session.project.clone();
    let binding = session.session_binding(session_id);
    if let Some(proj) = session.projection_mut(session_id) {
        proj.on_event(
            crate::client_protocol::observer::InternalEvent::PermissionResolved {
                tool: tool.to_string(),
                approved,
                always,
            },
            &project,
            &binding,
        );
        for msg in proj.take_outgoing() {
            emit(response_tx, msg);
        }
    }
}

fn take_stray_grant_for(
    session: &mut SessionController,
    request_id: &str,
) -> Option<SessionRequest> {
    session
        .stray_grants
        .iter()
        .position(|r| {
            matches!(
                r,
                SessionRequest::PermissionGrant { request_id: rid, .. } if rid == request_id
            )
        })
        .map(|i| session.stray_grants.remove(i).unwrap())
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
        free_text: Option<String>,
        selected: Vec<String>,
        answers: HashMap<String, AskAnswer>,
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
    // Ask registry: agent waits on oneshot; outer loop keeps handling JsonRpc
    // for every session (including the asking one). No nested select that
    // parks other sessions' RPCs into deferred.
    let mut pending_asks: HashMap<String, PendingAsk> = HashMap::new();

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
                            kind: perm.kind,
                            free_text: perm.free_text,
                            options: perm.options.clone(),
                            multi_select: perm.multi_select,
                            questions: perm.questions.clone(),
                        },
                        &project,
                        &binding,
                    );
                    for msg in proj.take_outgoing() {
                        emit(&response_tx, msg);
                    }
                }

                // Race: grant may have arrived before this Ask was registered.
                let buffered_grant = take_stray_grant_for(session, &perm.request_id);
                if let Some(SessionRequest::PermissionGrant {
                    request_id,
                    tool,
                    approved,
                    always,
                    free_text,
                    selected,
                    answers,
                }) = buffered_grant
                {
                    tracing::debug!(
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
                    let _ = perm.reply_tx.send(AskReply::from_grant(
                        approved,
                        always,
                        free_text,
                        selected,
                        answers,
                    ));
                    emit_permission_resolved(
                        session,
                        &response_tx,
                        &sid,
                        &tool,
                        approved,
                        always,
                    );
                } else {
                    pending_asks.insert(
                        perm.request_id.clone(),
                        PendingAsk {
                            session_id: sid,
                            agent_name: perm.agent_name,
                            tool: perm.tool,
                            rule_id: perm.rule_id,
                            reply_tx: perm.reply_tx,
                        },
                    );
                }
            }

            req = request_rx.recv() => {
                match req {
                    Some(SessionRequest::Quit) => {
                        for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &sid, &tool, false, false,
                            );
                        }
                        let sids: Vec<String> = session.projections.keys().cloned().collect();
                        for sid in &sids {
                            session.sessions.cancel_turn(sid).await;
                        }
                        break;
                    }
                    Some(SessionRequest::Cancel) => {
                        // Non-accept ⇒ reject outstanding Asks, then cancel turns.
                        for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &sid, &tool, false, false,
                            );
                        }
                        let sids: Vec<String> = session.projections.keys().cloned().collect();
                        for sid in &sids {
                            session.sessions.cancel_turn(sid).await;
                        }
                        for msg in session.take_all_outgoing() {
                            emit(&response_tx, msg);
                        }
                    }
                    Some(SessionRequest::JsonRpc(rpc)) => {
                        // Human interactive RPC stays live during Ask (any session).
                        if handle_jsonrpc(session, &response_tx, &perm_tx, &rpc, &terminal_hub)
                            .await
                        {
                            for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                                emit_permission_resolved(
                                    session, &response_tx, &sid, &tool, false, false,
                                );
                            }
                            break;
                        }
                    }
                    Some(SessionRequest::PermissionGrant {
                        request_id,
                        tool,
                        approved,
                        always,
                        free_text,
                        selected,
                        answers,
                    }) => {
                        match apply_permission_grant(
                            &mut pending_asks,
                            request_id,
                            tool,
                            approved,
                            always,
                            free_text,
                            selected,
                            answers,
                        ) {
                            GrantApply::Resolved {
                                session_id,
                                tool,
                                approved,
                                always,
                            } => {
                                tracing::info!(
                                    session_id = %session_id,
                                    tool = %tool,
                                    approved,
                                    always,
                                    "grant_permission received"
                                );
                                emit_permission_resolved(
                                    session,
                                    &response_tx,
                                    &session_id,
                                    &tool,
                                    approved,
                                    always,
                                );
                            }
                            GrantApply::Unknown {
                                request_id,
                                tool,
                                approved,
                                always,
                                free_text,
                                selected,
                                answers,
                            } => {
                                // Mismatch / early grant: keep in stray, never drop.
                                tracing::warn!(
                                    request_id = %request_id,
                                    "grant_permission with no matching pending ask; buffering as stray"
                                );
                                session.stray_grants.push_back(SessionRequest::PermissionGrant {
                                    request_id,
                                    tool,
                                    approved,
                                    always,
                                    free_text,
                                    selected,
                                    answers,
                                });
                            }
                        }
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
                    None => {
                        for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &sid, &tool, false, false,
                            );
                        }
                        break;
                    }
                }
            }
        }

        // Process deferred requests from all projections (legacy queue; Ask
        // no longer parks RPCs here).
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
                        for (rej_sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &rej_sid, &tool, false, false,
                            );
                        }
                        session.sessions.cancel_turn(&sid).await;
                        return;
                    }
                    SessionRequest::Cancel => {
                        for (rej_sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &rej_sid, &tool, false, false,
                            );
                        }
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
                            for (rej_sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                                emit_permission_resolved(
                                    session, &response_tx, &rej_sid, &tool, false, false,
                                );
                            }
                            return;
                        }
                    }
                    stale @ SessionRequest::PermissionGrant { .. } => {
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
                SessionRequest::Quit => {
                    for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                        emit_permission_resolved(
                            session, &response_tx, &sid, &tool, false, false,
                        );
                    }
                    break;
                }
                SessionRequest::Cancel => {
                    for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                        emit_permission_resolved(
                            session, &response_tx, &sid, &tool, false, false,
                        );
                    }
                }
                SessionRequest::JsonRpc(rpc) => {
                    if handle_jsonrpc(session, &response_tx, &perm_tx, &rpc, &terminal_hub).await {
                        for (sid, tool) in reject_all_pending_asks(&mut pending_asks) {
                            emit_permission_resolved(
                                session, &response_tx, &sid, &tool, false, false,
                            );
                        }
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


#[cfg(test)]
mod pending_ask_tests {
    use super::{GrantApply, PendingAsk, apply_permission_grant, reject_all_pending_asks};
    use crate::permission::{AskOutcome, AskReply};
    use std::collections::HashMap;
    use tokio::sync::oneshot;

    fn insert_ask(
        map: &mut HashMap<String, PendingAsk>,
        request_id: &str,
        session_id: &str,
    ) -> oneshot::Receiver<AskReply> {
        let (reply_tx, reply_rx) = oneshot::channel();
        map.insert(
            request_id.to_string(),
            PendingAsk {
                session_id: session_id.to_string(),
                agent_name: "default".into(),
                tool: "bash".into(),
                rule_id: "rule".into(),
                reply_tx,
            },
        );
        reply_rx
    }

    #[test]
    fn matching_accept_resolves_allow() {
        let mut pending = HashMap::new();
        let rx = insert_ask(&mut pending, "req-1", "sess-a");
        match apply_permission_grant(
            &mut pending,
            "req-1".into(),
            "bash".into(),
            true,
            false,
            None,
            Vec::new(),
            HashMap::new(),
        ) {
            GrantApply::Resolved {
                session_id,
                approved,
                always,
                ..
            } => {
                assert_eq!(session_id, "sess-a");
                assert!(approved);
                assert!(!always);
            }
            GrantApply::Unknown { .. } => panic!("expected resolved"),
        }
        let reply = rx.blocking_recv().unwrap();
        assert_eq!(reply.outcome, AskOutcome::Allow { always: false });
        assert!(pending.is_empty());
    }

    #[test]
    fn matching_reject_resolves_deny() {
        let mut pending = HashMap::new();
        let rx = insert_ask(&mut pending, "req-2", "sess-a");
        match apply_permission_grant(
            &mut pending,
            "req-2".into(),
            "bash".into(),
            false,
            false,
            None,
            Vec::new(),
            HashMap::new(),
        ) {
            GrantApply::Resolved { approved, .. } => assert!(!approved),
            GrantApply::Unknown { .. } => panic!("expected resolved"),
        }
        let reply = rx.blocking_recv().unwrap();
        assert_eq!(reply.outcome, AskOutcome::Deny);
    }

    #[test]
    fn unknown_grant_is_not_silently_dropped() {
        let mut pending = HashMap::new();
        let _rx = insert_ask(&mut pending, "req-live", "sess-a");
        match apply_permission_grant(
            &mut pending,
            "req-other".into(),
            "bash".into(),
            true,
            false,
            None,
            Vec::new(),
            HashMap::new(),
        ) {
            GrantApply::Unknown { request_id, .. } => assert_eq!(request_id, "req-other"),
            GrantApply::Resolved { .. } => panic!("mismatch must be Unknown"),
        }
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("req-live"));
    }

    #[test]
    fn reject_all_pending_is_deny_not_abort() {
        let mut pending = HashMap::new();
        let rx_a = insert_ask(&mut pending, "a", "sess-a");
        let rx_b = insert_ask(&mut pending, "b", "sess-b");
        let rejected = reject_all_pending_asks(&mut pending);
        assert_eq!(rejected.len(), 2);
        assert!(pending.is_empty());
        assert_eq!(rx_a.blocking_recv().unwrap().outcome, AskOutcome::Deny);
        assert_eq!(rx_b.blocking_recv().unwrap().outcome, AskOutcome::Deny);
    }

    #[test]
    fn concurrent_asks_resolve_independently() {
        let mut pending = HashMap::new();
        let mut rx_a = insert_ask(&mut pending, "a", "sess-a");
        let rx_b = insert_ask(&mut pending, "b", "sess-b");
        apply_permission_grant(&mut pending, "b".into(), "bash".into(), true, false, None, Vec::new(), HashMap::new());
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("a"));
        assert_eq!(
            rx_b.blocking_recv().unwrap().outcome,
            AskOutcome::Allow { always: false }
        );
        assert!(rx_a.try_recv().is_err());
        apply_permission_grant(&mut pending, "a".into(), "bash".into(), false, false, None, Vec::new(), HashMap::new());
        assert_eq!(rx_a.blocking_recv().unwrap().outcome, AskOutcome::Deny);
        assert!(pending.is_empty());
    }
}

#[cfg(test)]
mod ask_loop_tests {
    use super::{SessionRequest, run_session_loop};
    use std::collections::HashMap;
    use crate::client_protocol::controller::SessionController;
    use crate::client_protocol::permission_bridge::PendingPermission;
    use crate::client_protocol::protocol::JsonRpcRequestEnvelope;
    use crate::config::TurnGuard;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::engines::WorkspaceEngines;
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
    use crate::permission::{AskKind, AskOutcome, AskReply};
    use crate::runtime::RuntimeHandle;
    use crate::session::SessionManager;
    use crate::workspace::WorkspaceService;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};

    fn test_runtime(root: &std::path::Path) -> (RuntimeHandle, Arc<SessionManager>) {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                ..Default::default()
            },
        );
        let workspace_state = WorkspaceState::new(root);
        let resolved = resolve_without_catalog(global, workspace_state.clone());
        let workspace = WorkspaceService::new(root.to_path_buf()).unwrap();
        let engines = Arc::new(WorkspaceEngines::new());
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let ide = IdeBaseHandle::new(workspace, Arc::clone(&engines), Arc::clone(&hub));
        let runtime = RuntimeHandle::new(
            resolved,
            "default".into(),
            workspace_state,
            Arc::new(EngineManager::new()),
            engines,
            ide,
            Arc::new(AtomicU64::new(0)),
            root.join("global.db"),
        );
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        (runtime, sessions)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ask_does_not_block_other_session_rpc() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let mut controller = SessionController::with_turn_guard(runtime, None, sessions).unwrap();

        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, mut response_rx) = mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = mpsc::unbounded_channel();
        let perm_tx_loop = perm_tx.clone();
        let hub = Arc::new(crate::terminal::TerminalHub::new());

        let loop_handle = tokio::spawn(async move {
            run_session_loop(
                &mut controller,
                request_rx,
                response_tx,
                perm_tx_loop,
                perm_rx,
                hub,
            )
            .await;
        });

        let (reply_tx, mut reply_rx) = oneshot::channel::<AskReply>();
        perm_tx
            .send(PendingPermission {
                session_id: "sess-ask".into(),
                agent_name: "default".into(),
                turn_id: "turn-1".into(),
                request_id: "req-ask".into(),
                tool: "bash".into(),
                rule_id: "rule".into(),
                summary: "run".into(),
                kind: AskKind::Permission,
                free_text: false,
                options: Vec::new(),
                multi_select: false,
                questions: Vec::new(),
                reply_tx,
            })
            .unwrap();

        // Give the loop a tick to register the Ask.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Human RPC on another session must be answered while Ask is outstanding.
        request_tx
            .send(SessionRequest::JsonRpc(JsonRpcRequestEnvelope {
                jsonrpc: "2.0".into(),
                id: serde_json::json!(42),
                method: "definitely/not-a-method".into(),
                params: serde_json::json!({}),
            }))
            .unwrap();

        let resp = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let msg = response_rx.recv().await.expect("response channel closed");
                if msg.get("id") == Some(&serde_json::json!(42)) {
                    return msg;
                }
            }
        })
        .await
        .expect("other-session RPC must not wait for Ask grant");

        assert_eq!(resp["error"]["code"], -32601);
        // Ask still outstanding until explicit grant.
        assert!(reply_rx.try_recv().is_err());

        request_tx
            .send(SessionRequest::PermissionGrant {
                request_id: "req-ask".into(),
                tool: "bash".into(),
                approved: false,
                always: false,
                free_text: None,
                selected: Vec::new(),
                answers: HashMap::new(),
            })
            .unwrap();

        let reply = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .expect("grant should resolve Ask")
            .expect("oneshot open");
        assert_eq!(reply.outcome, AskOutcome::Deny);

        let _ = request_tx.send(SessionRequest::Quit);
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancel_rejects_pending_ask() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let mut controller = SessionController::with_turn_guard(runtime, None, sessions).unwrap();

        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, _response_rx) = mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = mpsc::unbounded_channel();
        let perm_tx_loop = perm_tx.clone();
        let hub = Arc::new(crate::terminal::TerminalHub::new());

        let loop_handle = tokio::spawn(async move {
            run_session_loop(
                &mut controller,
                request_rx,
                response_tx,
                perm_tx_loop,
                perm_rx,
                hub,
            )
            .await;
        });

        let (reply_tx, reply_rx) = oneshot::channel::<AskReply>();
        perm_tx
            .send(PendingPermission {
                session_id: "sess-ask".into(),
                agent_name: "default".into(),
                turn_id: "turn-1".into(),
                request_id: "req-ask".into(),
                tool: "bash".into(),
                rule_id: "rule".into(),
                summary: "run".into(),
                kind: AskKind::Permission,
                free_text: false,
                options: Vec::new(),
                multi_select: false,
                questions: Vec::new(),
                reply_tx,
            })
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        request_tx.send(SessionRequest::Cancel).unwrap();

        let reply = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .expect("cancel should reject Ask")
            .expect("oneshot open");
        assert_eq!(reply.outcome, AskOutcome::Deny);

        let _ = request_tx.send(SessionRequest::Quit);
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_grant_is_buffered_as_stray_then_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let mut controller = SessionController::with_turn_guard(runtime, None, sessions).unwrap();

        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (response_tx, _response_rx) = mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = mpsc::unbounded_channel();
        let perm_tx_loop = perm_tx.clone();
        let hub = Arc::new(crate::terminal::TerminalHub::new());

        let loop_handle = tokio::spawn(async move {
            run_session_loop(
                &mut controller,
                request_rx,
                response_tx,
                perm_tx_loop,
                perm_rx,
                hub,
            )
            .await;
        });

        // Early grant before Ask registration.
        request_tx
            .send(SessionRequest::PermissionGrant {
                request_id: "req-early".into(),
                tool: "bash".into(),
                approved: true,
                always: false,
                free_text: None,
                selected: Vec::new(),
                answers: HashMap::new(),
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let (reply_tx, reply_rx) = oneshot::channel::<AskReply>();
        perm_tx
            .send(PendingPermission {
                session_id: "sess-ask".into(),
                agent_name: "default".into(),
                turn_id: "turn-1".into(),
                request_id: "req-early".into(),
                tool: "bash".into(),
                rule_id: "rule".into(),
                summary: "run".into(),
                kind: AskKind::Permission,
                free_text: false,
                options: Vec::new(),
                multi_select: false,
                questions: Vec::new(),
                reply_tx,
            })
            .unwrap();

        let reply = tokio::time::timeout(Duration::from_secs(2), reply_rx)
            .await
            .expect("stray grant should resolve Ask")
            .expect("oneshot open");
        assert_eq!(reply.outcome, AskOutcome::Allow { always: false });

        let _ = request_tx.send(SessionRequest::Quit);
        let _ = tokio::time::timeout(Duration::from_secs(2), loop_handle).await;
    }
}
