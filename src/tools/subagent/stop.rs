//! Stop a background subagent child.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::jobs::{StopMark, SubagentJobBoard};
use super::status;

pub struct SubagentStopTool {
    sessions: Arc<SessionManager>,
    jobs: Arc<SubagentJobBoard>,
    session_id: Mutex<String>,
}

impl SubagentStopTool {
    pub fn new(sessions: Arc<SessionManager>, jobs: Arc<SubagentJobBoard>) -> Self {
        Self {
            sessions,
            jobs,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_stop(&self, input: Value) -> ToolCallResult {
        let child_id = match crate::tool::require_nonempty_string(&input, "id") {
            Ok(id) => id,
            Err(e) => return ToolCallResult::error(e),
        };
        let sid = self.session_id();
        match self.jobs.mark_stop(&sid, child_id) {
            Ok(StopMark::CancelRequested(_notice)) => {
                self.sessions.cancel_turn_sync(child_id);
                ToolCallResult::ok(status::format_stopping_status(
                    child_id,
                    &self.jobs.running(&sid),
                ))
            }
            Ok(StopMark::AlreadyEnded(notice)) => ToolCallResult::ok(
                status::format_already_ended_status(&notice, &self.jobs.running(&sid)),
            ),
            Err(unknown) => {
                ToolCallResult::error(status::format_unknown_task(&unknown, &self.jobs.running(&sid)))
            }
        }
    }
}

impl Tool for SubagentStopTool {
    fn name(&self) -> &str {
        "subagent_stop"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "child_session_id"
                }
            },
            "required": ["id"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentStopTool {
            sessions: Arc::clone(&self.sessions),
            jobs: Arc::clone(&self.jobs),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        Box::pin(async move { tool.call_stop(input) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_stop(input)
    }

    fn description(&self, _ctx: &Context) -> String {
        "Cancel the current turn of a child session. The session remains.".into()
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_id() {
        let sessions = Arc::new(crate::session::manager::SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let jobs = Arc::new(SubagentJobBoard::new());
        let tool = SubagentStopTool::new(sessions, jobs);
        assert_eq!(tool.schema()["required"], serde_json::json!(["id"]));
    }
}
