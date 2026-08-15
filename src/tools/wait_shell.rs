//! Wait for agent background bash jobs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::terminal::{TerminalHub, WaitOutcome};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::bash_status;

const MAX_WAIT_SECS: u64 = 600;

pub struct WaitShellTool {
    pub hub: Arc<TerminalHub>,
    cancel: CancellationToken,
    session_id: Mutex<String>,
}

impl WaitShellTool {
    pub fn new(hub: Arc<TerminalHub>) -> Self {
        Self {
            hub,
            cancel: CancellationToken::new(),
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_with_root(&self, input: Value, workspace_root: std::path::PathBuf) -> ToolCallResult {
        let id = input["id"].as_str().filter(|s| !s.is_empty());
        let sec = input["sec"].as_u64();
        let sid = self.session_id();
        let timeout = sec.map(Duration::from_secs);
        match self.hub.jobs.wait(&sid, id, timeout, &self.cancel, true) {
            WaitOutcome::Exited(notice) => {
                let jobs = self.hub.jobs.running(&sid);
                ToolCallResult::ok(bash_status::format_exited_status(
                    &notice,
                    &workspace_root,
                    &jobs,
                ))
            }
            WaitOutcome::TimedOut => {
                let jobs = self.hub.jobs.running(&sid);
                ToolCallResult::ok(bash_status::format_waited_status(&jobs, &workspace_root))
            }
            WaitOutcome::Cancelled => ToolCallResult::error("wait_shell cancelled"),
            WaitOutcome::UnknownId(unknown) => {
                let jobs = self.hub.jobs.running(&sid);
                ToolCallResult::error(bash_status::format_unknown_task(
                    &unknown,
                    &jobs,
                    &workspace_root,
                ))
            }
        }
    }
}

impl Tool for WaitShellTool {
    fn name(&self) -> &str {
        "wait_shell"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "bash_id to wait for. Omit to wait only on sec, or until any session job exits."
                },
                "sec": {
                    "type": "integer",
                    "description": "Seconds to wait (1-600). With id: return when that job exits or time elapses, whichever first. Without id: sleep, or return sooner if any session job exits."
                }
            }
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = WaitShellTool {
            hub: Arc::clone(&self.hub),
            cancel: execution.cancel.clone(),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        let workspace_root = execution.workspace_root.clone();
        Box::pin(async move { tool.call_with_root(input, workspace_root) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_with_root(input, crate::config::workspace::workspace_root_lap())
    }

    fn description(&self, _ctx: &Context) -> String {
        "Wait for a background bash job. Pass id (one job), sec (pure wait), or both (whichever happens first). Any other bash from this session exiting also returns. Does not kill the process.".into()
    }

    fn timeout(&self) -> Option<u64> {
        None
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }

    fn agent_terminal(&self) -> Option<Arc<TerminalHub>> {
        Some(Arc::clone(&self.hub))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let sec = input.get("sec");
        if id.is_none() && sec.is_none() {
            return Err("missing required parameter 'id' or 'sec'".into());
        }
        if let Some(v) = sec {
            let n = v
                .as_u64()
                .ok_or_else(|| crate::tool::expected_type("sec", "integer", v))?;
            if n < 1 || n > MAX_WAIT_SECS {
                return Err(crate::tool::must_be("sec", "between 1 and 600"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_id_or_sec() {
        let tool = WaitShellTool::new(Arc::new(TerminalHub::new()));
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
        assert!(
            tool.validate_input(&serde_json::json!({"id": "bg_a"}))
                .is_ok()
        );
        assert!(tool.validate_input(&serde_json::json!({"sec": 2})).is_ok());
        assert!(tool.validate_input(&serde_json::json!({"sec": 0})).is_err());
    }
}
