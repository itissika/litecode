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

        // K3: check and cancel happen under one records lock (WP-090
        // CancelOutcome). Cancel never touches the child's pending queue.
        if self
            .sessions
            .cancel_turn_sync(child_id, crate::types::StopOrigin::Parent)
            .cancelled()
        {
            // WP-094 L5: the parent stopped it itself — no ChildStopped to self.
            return ToolCallResult::ok(status::format_stopped(child_id));
        }

        match self.sessions.data().latest_turn_result_blocking(child_id) {
            Ok(Some(result)) => {
                let (agent, responsibility) = status::session_labels(&self.sessions, child_id);
                ToolCallResult::ok(format!(
                    "status: nothing running\n{}",
                    status::format_turn_result(
                        child_id,
                        agent.as_deref(),
                        responsibility.as_deref(),
                        &result,
                    )
                ))
            }
            Ok(None) => ToolCallResult::ok(format!(
                "status: nothing running\nchild_session_id: {child_id}\nreason: no completed turn\n"
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
        crate::tools::description_text(include_str!("../descriptions/subagent_stop.md"))
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {
    

}
