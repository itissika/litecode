//! Delete a direct child Session and its descendants (`subagent_delete`).

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::delete::{DeleteSessionError, delete_session_tree};
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::SubagentHub;

pub struct SubagentDeleteTool {
    sessions: Arc<SessionManager>,
    hub: Arc<SubagentHub>,
    session_id: Mutex<String>,
}

impl SubagentDeleteTool {
    pub fn new(sessions: Arc<SessionManager>, hub: Arc<SubagentHub>) -> Self {
        Self {
            sessions,
            hub,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    async fn call_delete(&self, input: Value) -> ToolCallResult {
        let child_id = match crate::tool::require_nonempty_string(&input, "session_id") {
            Ok(id) => id.to_string(),
            Err(error) => return ToolCallResult::error(error),
        };
        let parent = self.session_id();
        if parent.is_empty() {
            return ToolCallResult::error(
                "subagent_delete requires an active session execution context",
            );
        }
        // Only the caller's own direct child (not a grandchild, not a foreign session).
        let is_direct_child = self
            .sessions
            .reader()
            .meta_blocking(&child_id)
            .ok()
            .and_then(|meta| meta.parent_session_id)
            .is_some_and(|p| p == parent);
        if !is_direct_child {
            return ToolCallResult::error(format!(
                "subagent '{child_id}' is not a direct child session of this session"
            ));
        }
        match delete_session_tree(&self.sessions, &self.hub, &child_id).await {
            Ok(deleted) => {
                // WP-094 L5: the parent deleted it itself — no ChildDeleted to self.
                let mut result = ToolCallResult::ok(format!(
                    "status: deleted\nchild_session_id: {child_id}\ndeleted_session_ids: {}\n",
                    deleted.removed.join(",")
                ));
                result.metadata = Some(serde_json::json!({
                    "child_session_id": child_id,
                    "deleted_session_ids": deleted.removed,
                }));
                result
            }
            Err(DeleteSessionError::AlreadyRunning) => ToolCallResult::error(format!(
                "subagent '{child_id}' is running; stop it and wait for the turn to settle before deleting"
            )),
            Err(DeleteSessionError::DescendantRunning {
                running_session_ids,
            }) => ToolCallResult::error(format!(
                "cannot delete: a descendant subagent session is running (descendant_running): {}",
                running_session_ids.join(",")
            )),
            Err(DeleteSessionError::Other(error)) => {
                ToolCallResult::error(format!("subagent_delete failed: {error}"))
            }
        }
    }
}

impl Tool for SubagentDeleteTool {
    fn name(&self) -> &str {
        "subagent_delete"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "child_session_id of a direct child of this session"
                }
            },
            "required": ["session_id"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentDeleteTool {
            sessions: Arc::clone(&self.sessions),
            hub: Arc::clone(&self.hub),
            session_id: Mutex::new(execution.session_id),
        };
        Box::pin(async move { tool.call_delete(input).await })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("subagent_delete must be invoked via execute (async tool path)")
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("../descriptions/subagent_delete.md"))
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {
    

}
