//! Todo tool — session-scoped task tracking (SQLite authority).

use std::sync::Arc;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::session::manager::SessionManager;
use crate::session::task_state::{TodoItem, TodoStatus};
use crate::tool::Tool;
use crate::types::{Result, ToolCallResult};

pub struct TodoWriteTool {
    sessions: Option<Arc<SessionManager>>,
    current_session_id: Mutex<Option<String>>,
}

impl TodoWriteTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions: Some(sessions),
            current_session_id: Mutex::new(None),
        }
    }

    pub fn cold() -> Self {
        Self {
            sessions: None,
            current_session_id: Mutex::new(None),
        }
    }

    fn session_id(&self) -> Result<String> {
        self.current_session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| crate::types::LitecodeError::ToolExecution("no active session".into()))
    }
}

impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "todo.write"
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        if let Err(e) = self.validate_input(&input) {
            return ToolCallResult::error(e);
        }
        let todos_array = input["todos"].as_array().expect("validated");

        let mut todos = Vec::with_capacity(todos_array.len());
        for (i, item) in todos_array.iter().enumerate() {
            let content = item["content"].as_str().expect("validated").to_string();
            let status = match item["status"].as_str().expect("validated") {
                "pending" => TodoStatus::Pending,
                "in_progress" => TodoStatus::InProgress,
                "completed" => TodoStatus::Completed,
                other => {
                    return ToolCallResult::error(crate::tool::must_be_one_of(
                        &format!("todos[{i}].status"),
                        &["pending", "in_progress", "completed"],
                        other,
                    ));
                }
            };

            todos.push(TodoItem {
                id: format!("t{}", i + 1),
                content,
                status,
                priority: None,
            });
        }

        match self.commit_todos(&todos) {
            Ok(committed) => {
                // Count the SUBMITTED list, not the committed one. The only
                // normalization is "all completed -> dropped", which would
                // otherwise report 0/0/0 for a fully-finished list.
                let pending = todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::Pending)
                    .count();
                let in_progress = todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::InProgress)
                    .count();
                let completed = todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::Completed)
                    .count();
                // Count-only ack. The full list already lives in the call
                // arguments (and in TaskState for compact); echoing it here
                // duplicated noise into every later LLM view.
                let cleared = committed.is_empty() && completed > 0;
                let data = json!({
                    "total": todos.len(),
                    "pending": pending,
                    "in_progress": in_progress,
                    "completed": completed,
                    "cleared": cleared,
                });
                if cleared {
                    // Everything finished: the overlay was dropped, so say that
                    // instead of reporting a bare 0/0/0 that reads like data loss.
                    ToolCallResult::ok(format!(
                        "OK. All {completed} todos completed; task list cleared."
                    ))
                    .with_data(data)
                } else {
                    ToolCallResult::ok(format!(
                        "OK. Status — pending: {}, in_progress: {}, completed: {}",
                        pending, in_progress, completed
                    ))
                    .with_data(data)
                }
            }
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let todos = match input.get("todos") {
            None => return Err(crate::tool::missing_parameter("todos")),
            Some(v) => v
                .as_array()
                .ok_or_else(|| crate::tool::expected_type("todos", "array", v))?,
        };

        for (i, item) in todos.iter().enumerate() {
            let content_path = format!("todos[{i}].content");
            let status_path = format!("todos[{i}].status");
            match item.get("content") {
                None => return Err(crate::tool::missing_parameter(&content_path)),
                Some(v) => {
                    let content = crate::tool::require_string_value(v, &content_path)?;
                    if content.is_empty() {
                        return Err(crate::tool::must_be_nonempty_string(&content_path));
                    }
                }
            }
            match item.get("status") {
                None => return Err(crate::tool::missing_parameter(&status_path)),
                Some(v) => {
                    let status = crate::tool::require_string_value(v, &status_path)?;
                    let valid = ["pending", "in_progress", "completed"];
                    if !valid.contains(&status) {
                        return Err(crate::tool::must_be_one_of(&status_path, &valid, status));
                    }
                }
            }
        }

        Ok(())
    }

    fn set_active_session(&self, session_id: String) {
        *self.current_session_id.lock().unwrap() = Some(session_id);
    }
}

impl TodoWriteTool {
    /// Commit the submitted list, returning the POST-normalize state so callers
    /// report counts that match what was actually persisted (not the raw input).
    fn commit_todos(&self, todos: &[TodoItem]) -> Result<Vec<TodoItem>> {
        let sessions = self.sessions.as_ref().ok_or_else(|| {
            crate::types::LitecodeError::ToolExecution(
                "todo.write requires an active session manager".into(),
            )
        })?;
        let sid = self.session_id()?;
        let committed = sessions.with_entry_task_state_mut(&sid, |state| {
            state.todos = todos.to_vec();
            state.normalize();
            Ok(state.todos.clone())
        })?;
        sessions.save_task_state(&sid)?;
        Ok(committed)
    }
}

// Removed: do_add, do_update, do_remove, do_list, do_clear, parse_status, next_id

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    let sample = ToolCallResult::ok("OK. Status — pending: 1, in_progress: 1, completed: 1").with_data(
        json!({
            "total": 3,
            "pending": 1,
            "in_progress": 1,
            "completed": 1,
            "cleared": false,
        }),
    );
    vec![crate::tool::code_contract::DriftCase {
        tool: "todo.write",
        all_optional_present: sample.clone(),
        all_optional_absent: sample,
    }]
}

#[cfg(test)]
mod tests {}
