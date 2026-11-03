use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::SessionService;
use crate::session::manager::SessionStatus;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// List every descendant session of the active session as a raw session fact.
/// No subagent-specific status translation: the values are the session's own
/// `idle` / `running` / `stopping` / `running_with_subagent` status.
pub struct SubagentListTool {
    service: Arc<SessionService>,
    session_id: Mutex<String>,
}

impl SubagentListTool {
    pub fn new(service: Arc<SessionService>) -> Self {
        Self {
            service,
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
        let ids = self.service.descendant_session_ids(&sid);
        let mut out = format!("sessions: {}\n", ids.len());
        for id in ids {
            let status = self.service.status(&id).unwrap_or(SessionStatus::Idle);
            let Ok(meta) = self.service.meta(&id) else {
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
            // Live from session data (same reader status.rs/turn_result uses), not
            // from the search index: the parent can `read` by these line numbers.
            match self.service.transcript_file(&id) {
                Ok(file) => {
                    out.push_str(&format!("  transcript: {}\n", file.virtual_path));
                    out.push_str(&format!("  transcript_lines: {}\n", file.lines.len()));
                }
                Err(error) => {
                    out.push_str(&format!("  transcript: unavailable ({error})\n"));
                }
            }
            if let Some(progress) = self.service.cached_progress(&id) {
                out.push_str(&format!(
                    "  turn_age: {}\n  step: {}/{}\n",
                    relative_time(progress.started_at_ms),
                    progress.step,
                    progress.step_max
                ));
            } else if status == SessionStatus::Idle {
                if let Ok(Some((_, reason))) = self.service.latest_turn_end_reason(&id) {
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

    fn direct_spec(&self, _ctx: &Context) -> Option<crate::tool::DirectToolSpec> {
        let description =
            crate::tools::description_text(include_str!("../descriptions/subagent_list.md"));
        let input_schema = {
            serde_json::json!({
                "type": "object",
                "properties": {},
            })
        };
        Some(crate::tool::DirectToolSpec {
            description,
            input_schema,
        })
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentListTool {
            service: Arc::clone(&self.service),
            session_id: Mutex::new(execution.session_id.clone()),
        };
        Box::pin(async move { tool.call_list() })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        self.call_list()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {}
