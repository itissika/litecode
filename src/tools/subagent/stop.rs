//! Cancel the current turn of a child Session.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::status;

pub struct SubagentStopTool {
    sessions: Arc<SessionManager>,
    session_id: Mutex<String>,
}

impl SubagentStopTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_stop(&self, input: Value) -> ToolCallResult {
        let child_id = match crate::tool::require_nonempty_string(&input, "id") {
            Ok(id) => id,
            Err(error) => return ToolCallResult::error(error),
        };
        let parent = self.session_id();
        if !self
            .sessions
            .descendant_session_ids(&parent)
            .iter()
            .any(|id| id == child_id)
        {
            return ToolCallResult::error(status::format_unknown_child(child_id));
        }

        if let Some(progress) = self.sessions.get_cached_progress(child_id) {
            if self.sessions.cancel_turn_sync(child_id) {
                return ToolCallResult::ok(status::format_stop_requested(
                    child_id,
                    &progress.turn_id,
                ));
            }
        }

        match self.sessions.data().latest_turn_result_blocking(child_id) {
            Ok(Some(result)) => {
                let (agent, responsibility) = status::session_labels(&self.sessions, child_id);
                ToolCallResult::ok(format!(
                    "status: already ended\n{}",
                    status::format_turn_result(
                        child_id,
                        agent.as_deref(),
                        responsibility.as_deref(),
                        &result,
                    )
                ))
            }
            Ok(None) => ToolCallResult::ok(format!(
                "status: idle\nchild_session_id: {child_id}\nreason: no completed turn\n"
            )),
            Err(error) => ToolCallResult::error(format!(
                "subagent '{child_id}' result is unavailable: {error}"
            )),
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
            session_id: Mutex::new(execution.session_id),
        };
        Box::pin(async move { tool.call_stop(input) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_stop(input)
    }

    fn description(&self, _ctx: &Context) -> String {
        "Request cancellation of a child session's current turn. The child session and its context remain.".into()
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
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let tool = SubagentStopTool::new(sessions);
        assert_eq!(tool.schema()["required"], serde_json::json!(["id"]));
    }
}
