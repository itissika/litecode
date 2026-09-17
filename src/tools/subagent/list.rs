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
            let Ok(meta) = self.sessions.reader().meta_blocking(&id) else {
                continue;
            };
            out.push_str(&format!("- id: {id}\n"));
            if !meta.agent_id.is_empty() {
                out.push_str(&format!("  agent: {}\n", meta.agent_id));
            }
            if !meta.responsibility.is_empty() {
                out.push_str(&format!("  responsibility: {}\n", meta.responsibility));
            }
            let last_send = meta
                .preview
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !last_send.is_empty() {
                out.push_str(&format!("  last_send: {last_send}\n"));
            }
            out.push_str(&format!("  state: {}\n", status.as_str()));
            if let Some(progress) = self.sessions.get_cached_progress(&id) {
                out.push_str(&format!(
                    "  turn_age: {}\n  step: {}/{}\n",
                    relative_time(progress.started_at_ms),
                    progress.step,
                    progress.step_max
                ));
            } else if status == SessionStatus::Idle {
                if let Ok(Some((_, reason))) =
                    self.sessions.data().latest_turn_end_reason_blocking(&id)
                {
                    out.push_str(&format!("  reason: {reason}\n"));
                }
            }
        }
        ToolCallResult::ok(out)
    }
}

fn relative_time(timestamp_ms: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default();
    let seconds = now.saturating_sub(timestamp_ms).max(0) as u64 / 1000;
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, (seconds % 3600) / 60),
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
        "List child sessions as labeled blocks: id, agent, responsibility, last_send, state; running children also include turn_age and step; idle children include the latest turn reason.".to_string()
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
