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
    hub: Option<Arc<super::SubagentHub>>,
    session_id: Mutex<String>,
}

impl SubagentStopTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            hub: None,
            session_id: Mutex::new(String::new()),
        }
    }

    /// Parent mailbox for `ChildStopped` (K5: written, never wakes the parent).
    pub fn with_hub(mut self, hub: Arc<super::SubagentHub>) -> Self {
        self.hub = Some(hub);
        self
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
        if self.sessions.cancel_turn_sync(child_id).cancelled() {
            if let Some(hub) = &self.hub {
                hub.notify_parent_no_wake(
                    &parent,
                    super::ParentNotice::ChildStopped {
                        child_session_id: child_id.to_string(),
                    },
                );
            }
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
            hub: self.hub.clone(),
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

    async fn parent_and_child() -> (tempfile::TempDir, Arc<SessionManager>, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let parent = sessions
            .open_session("/proj", "default", None)
            .await
            .unwrap();
        let child = sessions
            .open_child_session("/proj", "reviewer", None, &parent, "call_1")
            .unwrap();
        (dir, sessions, parent, child)
    }

    fn stop(sessions: &Arc<SessionManager>, parent: &str, child: &str) -> ToolCallResult {
        let tool = SubagentStopTool::new(Arc::clone(sessions));
        tool.set_active_session(parent.to_string());
        tool.call(serde_json::json!({ "id": child }))
    }

    #[tokio::test]
    async fn stop_is_atomic_against_starting_turn() {
        let (_dir, sessions, parent, child) = parent_and_child().await;
        // StartingTurn reservation: the old get_cached_progress-then-cancel path
        // reported nothing here; CancelOutcome::Starting must count as stopped.
        sessions
            .reserve_turn(&child, "t1".into(), 10, "reviewer", "/proj")
            .unwrap();
        let result = stop(&sessions, &parent, &child);
        assert!(
            result.content.contains("status: stopped"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("turn_id"), "{}", result.content);
    }

    #[tokio::test]
    async fn stop_writes_child_stopped_notice_without_wake() {
        let (_dir, sessions, parent, child) = parent_and_child().await;
        let hub = Arc::new(crate::tools::subagent::SubagentHub::new());
        sessions
            .reserve_turn(&child, "t1".into(), 10, "reviewer", "/proj")
            .unwrap();
        let tool = SubagentStopTool::new(Arc::clone(&sessions)).with_hub(Arc::clone(&hub));
        tool.set_active_session(parent.clone());
        let result = tool.call(serde_json::json!({ "id": child }));
        assert!(
            result.content.contains("status: stopped"),
            "{}",
            result.content
        );
        assert!(hub.has_notices(&parent));
        assert!(!hub.has_pending(&parent));
        assert_eq!(
            hub.take_notices(&parent),
            vec![crate::tools::subagent::ParentNotice::ChildStopped {
                child_session_id: child.clone()
            }]
        );
        // Idle stop writes nothing (repeat cancel inside Starting is idempotent
        // `Starting` per WP-090 §1, so release the reservation first).
        sessions.release_turn_reservation(&child, "t1");
        let idle = tool.call(serde_json::json!({ "id": child }));
        assert!(
            idle.content.contains("status: nothing running"),
            "{}",
            idle.content
        );
        assert!(!hub.has_notices(&parent));
    }

    #[tokio::test]
    async fn reports_nothing_running_when_idle() {
        let (_dir, sessions, parent, child) = parent_and_child().await;
        let result = stop(&sessions, &parent, &child);
        assert!(
            result.content.contains("status: nothing running"),
            "{}",
            result.content
        );
    }

    #[tokio::test]
    async fn does_not_clear_pending() {
        let (_dir, sessions, parent, child) = parent_and_child().await;
        sessions
            .reserve_turn(&child, "t1".into(), 10, "reviewer", "/proj")
            .unwrap();
        sessions
            .enqueue_user_input(&child, crate::types::UserInput::text("queued"))
            .unwrap();
        let result = stop(&sessions, &parent, &child);
        assert!(
            result.content.contains("status: stopped"),
            "{}",
            result.content
        );
        assert_eq!(sessions.pending_messages_snapshot(&child).len(), 1);
    }
}
