//! Workspace-domain WebSocket requests.
//!
//! These requests share `/ws` with agent sessions but deliberately do not
//! depend on session subscriptions or client-side agent projections.

use tokio::sync::mpsc::UnboundedSender;

use crate::client_protocol::connection::emit;
use crate::client_protocol::protocol::{
    JsonRpcErrorBody, JsonRpcRequestEnvelope, JsonRpcResponse, methods,
};
use crate::engines::EngineState;
use crate::runtime::RuntimeHandle;

fn ok_response(id: serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    }
}

fn err_response(id: serde_json::Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(JsonRpcErrorBody {
            code,
            message: message.into(),
        }),
    }
}

fn lsp_unavailable_message(runtime: &RuntimeHandle) -> String {
    let root = runtime.workspace_root().display();
    match runtime.workspace_engines.state("lsp") {
        Some(EngineState::Warming) => {
            format!(
                "LSP is loading for the current workspace (root={root}); please wait until it is Warm and retry."
            )
        }
        Some(EngineState::Failed) => format!(
            "LSP is unavailable for the current workspace (root={root}): {}. \
             Check Settings → Engines → LSP, then retry.",
            runtime
                .workspace_engines
                .last_error("lsp")
                .unwrap_or_else(|| "language engine failed to start".into())
        ),
        _ => format!(
            "LSP is not enabled for the current workspace (root={root}). \
             Configure and start an LSP server in Settings → Engines → LSP."
        ),
    }
}

pub fn is_workspace_method(method: &str) -> bool {
    matches!(
        method,
        methods::LSP_REQUEST
            | methods::TERMINAL_CREATE
            | methods::TERMINAL_WRITE
            | methods::TERMINAL_RESIZE
            | methods::TERMINAL_CLOSE
            | methods::BASH_TAIL
            | methods::BASH_KILL
    )
}

/// Handle a workspace-domain request independently of `SessionController`.
pub async fn handle_jsonrpc(
    runtime: &RuntimeHandle,
    sink: &UnboundedSender<serde_json::Value>,
    terminal_hub: &std::sync::Arc<crate::terminal::TerminalHub>,
    caller: &crate::terminal::ConnectionId,
    rpc: JsonRpcRequestEnvelope,
) {
    let id = rpc.id;
    match rpc.method.as_str() {
        methods::LSP_REQUEST => {
            #[derive(serde::Deserialize)]
            struct Params {
                method: String,
                params: serde_json::Value,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            if !runtime.workspace_engines.is_warmed("lsp") {
                emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32001,
                        lsp_unavailable_message(runtime),
                    ))
                    .unwrap(),
                );
                return;
            }
            let hub = runtime.workspace_engines.lsp_hub();
            match hub.request(&params.method, params.params).await {
                Ok(result) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "result": result })))
                        .unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        methods::TERMINAL_CREATE => {
            #[derive(serde::Deserialize)]
            struct Params {
                #[serde(default = "default_cols")]
                cols: u16,
                #[serde(default = "default_rows")]
                rows: u16,
                #[serde(default)]
                cwd: Option<String>,
            }
            fn default_cols() -> u16 {
                80
            }
            fn default_rows() -> u16 {
                24
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            let hub = std::sync::Arc::clone(terminal_hub);
            let caller = caller.clone();
            // Default the terminal's working directory to the workspace root
            // explicitly. The spawn_blocking thread has no RUNTIME_PATHS, and the
            // process cwd no longer mirrors the workspace (chdir removed), so the
            // active_paths() fallback would point at the launch directory.
            let default_cwd = runtime.workspace_root().to_path_buf();
            let cwd = match params
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                None => Some(default_cwd),
                Some(rel) => match crate::workspace::Sandbox::new(default_cwd.clone())
                    .and_then(|sandbox| sandbox.resolve(rel))
                {
                    Ok(abs) => Some(abs),
                    Err(error) => {
                        emit(
                            sink,
                            serde_json::to_value(err_response(
                                id,
                                -32602,
                                format!("Invalid terminal cwd: {error}"),
                            ))
                            .unwrap(),
                        );
                        return;
                    }
                },
            };
            let opts = crate::terminal::CreateOptions {
                cols: params.cols,
                rows: params.rows,
                cwd,
            };
            match tokio::task::spawn_blocking(move || hub.create(&caller, opts)).await {
                Ok(Ok(term_id)) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "id": term_id })))
                        .unwrap(),
                ),
                Ok(Err(error)) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        methods::TERMINAL_WRITE => {
            #[derive(serde::Deserialize)]
            struct Params {
                id: String,
                data: String,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            let hub = std::sync::Arc::clone(terminal_hub);
            let caller = caller.clone();
            match tokio::task::spawn_blocking(move || {
                hub.write(&caller, &params.id, params.data.as_bytes())
            })
            .await
            {
                Ok(Ok(())) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "ok": true })))
                        .unwrap(),
                ),
                Ok(Err(error)) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        methods::TERMINAL_RESIZE => {
            #[derive(serde::Deserialize)]
            struct Params {
                id: String,
                cols: u16,
                rows: u16,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            let hub = std::sync::Arc::clone(terminal_hub);
            let caller = caller.clone();
            match tokio::task::spawn_blocking(move || {
                hub.resize(&caller, &params.id, params.cols, params.rows)
            })
            .await
            {
                Ok(Ok(())) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "ok": true })))
                        .unwrap(),
                ),
                Ok(Err(error)) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        methods::TERMINAL_CLOSE => {
            #[derive(serde::Deserialize)]
            struct Params {
                id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            let hub = std::sync::Arc::clone(terminal_hub);
            let caller = caller.clone();
            match tokio::task::spawn_blocking(move || hub.close(&caller, &params.id)).await {
                Ok(Ok(())) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "ok": true })))
                        .unwrap(),
                ),
                Ok(Err(error)) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        methods::BASH_TAIL => {
            #[derive(serde::Deserialize)]
            struct Params {
                bash_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            match terminal_hub.jobs.tail_view(&params.bash_id) {
                Some(view) => emit(
                    sink,
                    serde_json::to_value(ok_response(
                        id,
                        serde_json::json!({
                            "text": view.text,
                            "truncated_on_disk": view.truncated_on_disk,
                            "alive": view.alive,
                            "exit_code": view.exit_code,
                        }),
                    ))
                    .unwrap(),
                ),
                None => emit(
                    sink,
                    serde_json::to_value(err_response(
                        id,
                        -32000,
                        format!("bash job '{}' not found", params.bash_id),
                    ))
                    .unwrap(),
                ),
            }
        }
        methods::BASH_KILL => {
            #[derive(serde::Deserialize)]
            struct Params {
                bash_id: String,
            }
            let params: Params = match serde_json::from_value(rpc.params) {
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
                    return;
                }
            };
            let hub = std::sync::Arc::clone(terminal_hub);
            match tokio::task::spawn_blocking(move || {
                let info = hub.kill_from_ui(&params.bash_id)?;
                let _ = hub.close_agent(&params.bash_id);
                Ok::<_, crate::terminal::TerminalError>(info)
            })
            .await
            {
                Ok(Ok(_)) => emit(
                    sink,
                    serde_json::to_value(ok_response(id, serde_json::json!({ "ok": true })))
                        .unwrap(),
                ),
                Ok(Err(error)) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
                Err(error) => emit(
                    sink,
                    serde_json::to_value(err_response(id, -32000, error.to_string())).unwrap(),
                ),
            }
        }
        _ => unreachable!("workspace router only receives workspace methods"),
    }
}

#[cfg(test)]
mod tests {
    use super::is_workspace_method;

    #[test]
    fn classifies_workspace_methods() {
        assert!(is_workspace_method("lsp/request"));
        assert!(is_workspace_method("terminal/create"));
        assert!(is_workspace_method("bash/tail"));
        assert!(is_workspace_method("bash/kill"));
        assert!(!is_workspace_method("session/subscribe"));
        assert!(!is_workspace_method("agent/run"));
        assert!(!is_workspace_method("bash/jobs"));
    }
}
