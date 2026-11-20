use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::session::SessionService;
use crate::session::manager::SessionStatus;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// List every descendant session of the active session as a raw session fact.
/// No subagent-specific status translation: the values are the session's own
/// `idle` / `running` / `stopping` / `running_with_subagent` status.
pub struct SubagentListTool {
    service: Option<Arc<SessionService>>,
    session_id: Mutex<String>,
}

impl SubagentListTool {
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

    fn call_list(&self) -> ToolCallResult {
        let Some(service) = self.service.as_ref() else {
            return ToolCallResult::error("subagent.list requires an active session service");
        };
        let sid = self.session_id();
        if sid.is_empty() {
            return ToolCallResult::error(
                "subagent_list requires an active session execution context",
            );
        }
        let ids = service.descendant_session_ids(&sid);
        let mut out = format!("sessions: {}\n", ids.len());
        let mut rows: Vec<Value> = Vec::new();
        for id in ids {
            let status = service.status(&id).unwrap_or(SessionStatus::Idle);
            let Ok(meta) = service.meta(&id) else {
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
            let mut row = serde_json::Map::new();
            row.insert("id".into(), Value::String(id.clone()));
            row.insert("agent".into(), Value::String(meta.agent_id.clone()));
            row.insert(
                "responsibility".into(),
                Value::String(meta.responsibility.clone()),
            );
            row.insert("state".into(), Value::String(status.as_str().to_string()));
            // Live from session data (same reader status.rs/turn_result uses), not
            // from the search index: the parent can `read` by these line numbers.
            match service.transcript_file(&id) {
                Ok(file) => {
                    out.push_str(&format!("  transcript: {}\n", file.virtual_path));
                    out.push_str(&format!("  transcript_lines: {}\n", file.lines.len()));
                    row.insert("transcript".into(), Value::String(file.virtual_path));
                    row.insert("transcript_lines".into(), json!(file.lines.len()));
                }
                Err(error) => {
                    out.push_str(&format!("  transcript: unavailable ({error})\n"));
                    row.insert("transcript".into(), Value::String(String::new()));
                }
            }
            if let Some(progress) = service.cached_progress(&id) {
                out.push_str(&format!(
                    "  turn_age: {}\n  step: {}/{}\n",
                    relative_time(progress.started_at_ms),
                    progress.step,
                    progress.step_max
                ));
                row.insert("step".into(), json!(progress.step));
                row.insert("step_max".into(), json!(progress.step_max));
            } else if status == SessionStatus::Idle {
                if let Ok(Some((_, reason))) = service.latest_turn_end_reason(&id) {
                    out.push_str(&format!("  reason: {reason}\n"));
                    row.insert("reason".into(), Value::String(reason));
                }
            }
            rows.push(Value::Object(row));
        }
        let count = rows.len();
        ToolCallResult::ok(out).with_data(json!({
            "count": count,
            "sessions": rows,
        }))
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
        "subagent.list"
    }

    fn execute(
        &self,
        _input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentListTool {
            service: self.service.clone(),
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
