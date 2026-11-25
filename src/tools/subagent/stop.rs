//! Cancel the current turn of a child Session.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::session::SessionService;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::status;

pub struct SubagentStopTool {
    service: Option<Arc<SessionService>>,
    session_id: Mutex<String>,
}

impl SubagentStopTool {
    pub fn new(service: Arc<SessionService>) -> Self {
        Self {
            service: Some(service),
            session_id: Mutex::new(String::new()),
        }
    }

    pub fn cold() -> Self {
        Self {
            service: None,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_stop(&self, input: Value) -> ToolCallResult {
        let Some(service) = self.service.as_ref() else {
            return ToolCallResult::error("subagent.stop requires an active session service");
        };
        let child_id = match crate::tool::require_nonempty_string(&input, "id") {
            Ok(id) => id,
            Err(error) => return ToolCallResult::error(error),
        };
        let parent = self.session_id();
        if !service
            .descendant_session_ids(&parent)
            .iter()
            .any(|id| id == child_id)
        {
            return ToolCallResult::error(status::format_unknown_child(child_id));
        }

        // K3: check and cancel happen under one records lock (WP-090
        // CancelOutcome). Cancel never touches the child's pending queue.
        if service
            .cancel_turn(child_id, crate::types::StopOrigin::Parent)
            .cancelled()
        {
            // WP-094 L5: the parent stopped it itself — no ChildStopped to self.
            let msg = status::format_stopped(child_id);
            return ToolCallResult::ok(msg).with_data(serde_json::json!({
                "status": "stopped",
            }));
        }

        match service.latest_turn_result(child_id) {
            Ok(Some(result)) => {
                let (agent, responsibility) = status::session_labels(service, child_id);
                let msg = format!(
                    "status: nothing running\n{}",
                    status::format_turn_result(
                        child_id,
                        agent.as_deref(),
                        responsibility.as_deref(),
                        &result,
                    )
                );
                let cancelled_by = match service.latest_turn_end(child_id) {
                    Ok(Some((tid, _, by))) if tid == result.turn_id => by,
                    _ => None,
                };
                let mut data = serde_json::Map::new();
                data.insert("status".into(), Value::String("nothing_running".into()));
                data.insert(
                    "reason".into(),
                    Value::String(status::merged_reason(&result.reason, false, cancelled_by)),
                );
                data.insert("output".into(), Value::String(result.output.clone()));
                ToolCallResult::ok(msg).with_data(Value::Object(data))
            }
            Ok(None) => {
                let msg = format!(
                    "status: nothing running\nchild_session_id: {child_id}\nreason: no completed turn\n"
                );
                ToolCallResult::ok(msg).with_data(serde_json::json!({
                    "status": "nothing_running",
                    "reason": "no completed turn",
                }))
            }
            Err(error) => ToolCallResult::error(format!(
                "subagent '{child_id}' result is unavailable: {error}"
            )),
        }
    }
}

impl Tool for SubagentStopTool {
    fn name(&self) -> &str {
        "subagent.stop"
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentStopTool {
            service: self.service.clone(),
            session_id: Mutex::new(execution.session_id),
        };
        Box::pin(async move { tool.call_stop(input) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_stop(input)
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {}
