//! Stop a background subagent child.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::hub::SubagentHub;
use super::status;

pub struct SubagentStopTool {
    pub hub: Arc<SubagentHub>,
    session_id: Mutex<String>,
}

impl SubagentStopTool {
    pub fn new(hub: Arc<SubagentHub>) -> Self {
        Self {
            hub,
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
        match self.hub.stop(&sid, child_id) {
            Ok(_notice) => {
                let jobs = self.hub.running(&sid);
                if self.hub.is_alive(child_id) {
                    ToolCallResult::ok(status::format_stopping_status(child_id, &jobs))
                } else {
                    ToolCallResult::ok(status::format_stopped_status(child_id, &jobs))
                }
            }
            Err(unknown) => {
                let jobs = self.hub.running(&sid);
                ToolCallResult::error(status::format_unknown_task(&unknown, &jobs))
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
                    "description": "child_session_id returned by subagent_launch"
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
            hub: Arc::clone(&self.hub),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        Box::pin(async move { tool.call_stop(input) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_stop(input)
    }

    fn description(&self, _ctx: &Context) -> String {
        "Stop a background subagent by child_session_id. Remaining running children are listed.".into()
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }

    fn agent_subagents(&self) -> Option<Arc<SubagentHub>> {
        Some(Arc::clone(&self.hub))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_requires_id() {
        let tool = SubagentStopTool::new(Arc::new(SubagentHub::new()));
        assert_eq!(tool.schema()["required"], serde_json::json!(["id"]));
    }
}
