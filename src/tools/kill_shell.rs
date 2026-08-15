//! KillShell tool — terminates TerminalHub background sessions.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::TerminalHub;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::bash_status;

pub struct KillShellTool {
    pub hub: Arc<TerminalHub>,
    session_id: Mutex<String>,
}

impl KillShellTool {
    pub fn new(hub: Arc<TerminalHub>) -> Self {
        Self {
            hub,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_with_root(&self, input: Value, workspace_root: std::path::PathBuf) -> ToolCallResult {
        let bash_id = match crate::tool::require_nonempty_string(&input, "bash_id") {
            Ok(id) => id,
            Err(e) => return ToolCallResult::error(e),
        };
        let sid = self.session_id();

        let hub = &self.hub;
        match hub.kill(bash_id) {
            Ok(info) => {
                hub.jobs.take_notice(&sid, bash_id);
                let _ = hub.close_agent(bash_id);
                let jobs = hub.jobs.running(&sid);
                let code = info.exit_code.map(|c| c as i32).unwrap_or(-1);
                ToolCallResult::ok(bash_status::format_killed_status(
                    bash_id,
                    code,
                    info.output_path.as_deref(),
                    &workspace_root,
                    &jobs,
                ))
            }
            Err(_) => {
                let jobs = hub.jobs.running(&sid);
                ToolCallResult::error(bash_status::format_unknown_task(
                    bash_id,
                    &jobs,
                    &workspace_root,
                ))
            }
        }
    }
}

impl Tool for KillShellTool {
    fn name(&self) -> &str {
        "kill_shell"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "bash_id": {
                    "type": "string",
                    "description": "bash_id returned by bash when a command is running in the background"
                }
            },
            "required": ["bash_id"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = KillShellTool {
            hub: Arc::clone(&self.hub),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        let workspace_root = execution.workspace_root.clone();
        Box::pin(async move { tool.call_with_root(input, workspace_root) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_with_root(input, crate::config::workspace::workspace_root_lap())
    }

    fn description(&self, _ctx: &Context) -> String {
        "Stop a background bash task by bash_id. The output file stays; inspect with read/grep. Remaining running jobs are listed.".into()
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }

    fn agent_terminal(&self) -> Option<Arc<TerminalHub>> {
        Some(Arc::clone(&self.hub))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_bash_id() {
        let tool = KillShellTool::new(Arc::new(TerminalHub::new()));
        assert_eq!(tool.schema()["required"], serde_json::json!(["bash_id"]));
    }
}
