use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::manager::{SessionManager, SessionStatus};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// List every descendant session of the active session as a raw session fact.
/// No subagent-specific status translation: the values are the session's own
/// `idle` / `running` / `stopping` / `running_with_subagent` status.
pub struct SubagentListTool {
    sessions: Arc<SessionManager>,
    session_id: Mutex<String>,
}

impl SubagentListTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_list(&self) -> ToolCallResult {
        let sid = self.session_id();
        if sid.is_empty() {
            return ToolCallResult::error(
                "subagent_list requires an active session execution context",
            );
        }
        let ids = self.sessions.descendant_session_ids(&sid);
        let mut out = format!("sessions: {}\n", ids.len());
        for id in ids {
            let status = self
                .sessions
                .session_status(&id)
                .unwrap_or(SessionStatus::Idle);
            let (preview, updated_at) = self
                .sessions
                .reader()
                .meta_blocking(&id)
                .map(|meta| (meta.preview, meta.updated_at))
                .unwrap_or_default();
            let preview = if preview.trim().is_empty() {
                "-".to_string()
            } else {
                preview
            };
            out.push_str(&format!(
                "- {id}  {}  {updated_at}  {preview}\n",
                status.as_str()
            ));
        }
        ToolCallResult::ok(out)
    }
}

impl Tool for SubagentListTool {
    fn name(&self) -> &str {
        "subagent_list"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentListTool {
            sessions: Arc::clone(&self.sessions),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        Box::pin(async move { tool.call_list() })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        self.call_list()
    }

    fn description(&self, _ctx: &Context) -> String {
        "List this session's child sessions and each one's raw session status.".to_string()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;

    #[test]
    fn schema_is_empty_object() {
        let tool = SubagentListTool::new(Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )));
        assert_eq!(tool.schema()["type"], "object");
    }
}
