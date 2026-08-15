//! Todo tool — session-scoped task tracking (SQLite authority).

use std::sync::Arc;
use std::sync::Mutex;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::session::manager::SessionManager;
use crate::session::task_state::{TodoItem, TodoStatus};
use crate::tool::Tool;
use crate::types::{Result, ToolCallResult};

pub struct TodoWriteTool {
    sessions: Arc<SessionManager>,
    current_session_id: Mutex<Option<String>>,
}

impl TodoWriteTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
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
        "todo"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string", "description": "Task content"},
                            "status": {"type": "string", "enum": ["pending", "in_progress", "completed"], "description": "Task status"}
                        },
                        "required": ["content", "status"]
                    },
                    "description": "Complete task list (submits full list, overwrites previous state)"
                }
            },
            "required": ["todos"]
        })
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
                // Count from the committed (normalized) state, not the raw input.
                let pending = committed
                    .iter()
                    .filter(|t| t.status == TodoStatus::Pending)
                    .count();
                let in_progress = committed
                    .iter()
                    .filter(|t| t.status == TodoStatus::InProgress)
                    .count();
                let completed = committed
                    .iter()
                    .filter(|t| t.status == TodoStatus::Completed)
                    .count();
                // Count-only ack. The full list already lives in the call
                // arguments (and in TaskState for compact); echoing it here
                // duplicated noise into every later LLM view.
                ToolCallResult::ok(format!(
                    "OK. Status — pending: {}, in_progress: {}, completed: {}",
                    pending, in_progress, completed
                ))
            }
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Manage the session task list; submitting todos replaces the entire list.\n\
         \n\
         Use proactively for multi-step work (3+ distinct steps), when the user gives\n\
         several tasks, or when new instructions arrive. Skip for single, straightforward\n\
         tasks or purely conversational/informational requests.\n\
         \n\
         States: pending, in_progress (exactly ONE at a time), completed.\n\
         \n\
         Rules:\n\
         - Update status in real time; don't batch completions.\n\
         - Mark completed only after the work is actually done, including verification.\n\
         - Keep exactly one in_progress while work remains.\n\
         - Mark a step completed before starting the next one.\n\
         - Mark all steps completed when finished."
            .into()
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
        let sid = self.session_id()?;
        let committed = self.sessions.with_entry_task_state_mut(&sid, |state| {
            state.todos = todos.to_vec();
            state.normalize();
            Ok(state.todos.clone())
        })?;
        self.sessions.save_task_state(&sid)?;
        Ok(committed)
    }
}

// Removed: do_add, do_update, do_remove, do_list, do_clear, parse_status, next_id

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::workspace::set_runtime_paths;
    use crate::session::manager::SessionManager;
    use crate::session::store::Session;
    use crate::session::task_state::TaskReminders;
    use crate::session::task_state::render_todos;

    fn test_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn make_manager(db_path: &str) -> (Arc<SessionManager>, String) {
        let session = Session::open(db_path, "/proj", "default", Some("m")).unwrap();
        let sid = session.id.clone();
        let manager = Arc::new(SessionManager::new(
            Arc::new(crate::config::TurnGuard::new()),
            db_path.to_string(),
        ));
        manager.register_for_test(session);
        (manager, sid)
    }

    fn make_tool(manager: Arc<SessionManager>, session_id: &str) -> TodoWriteTool {
        let tool = TodoWriteTool::new(manager);
        tool.set_active_session(session_id.to_string());
        tool
    }

    fn install_paths(
        dir: &std::path::Path,
    ) -> crate::session::snapshot_paths::test_home::HomeGuard {
        // Isolate the host home so concurrent todo tests (and other --lib tests)
        // cannot race on the shared workspace-registry under the same home env.
        let _home = crate::session::snapshot_paths::test_home::isolate_home();
        crate::config::init_workspace(dir).unwrap();
        set_runtime_paths(WorkspacePaths::for_legacy_root(&dir));
        _home
    }

    fn setup(
        dir: &std::path::Path,
    ) -> (
        Arc<SessionManager>,
        String,
        crate::session::snapshot_paths::test_home::HomeGuard,
    ) {
        let home = install_paths(dir);
        let db = dir.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let (manager, sid) = make_manager(&db.to_string_lossy());
        (manager, sid, home)
    }

    fn load_task_state(manager: &SessionManager, session_id: &str) -> TaskReminders {
        manager
            .with_entry_task_state(session_id, |s| Ok(s.clone()))
            .unwrap()
    }

    #[test]
    fn test_add_todo() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let result = tool.call(serde_json::json!({
            "todos": [{"content": "Implement feature X", "status": "pending"}]
        }));
        assert!(
            result.content.contains("pending: 1"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("completed: 0"),
            "got: {}",
            result.content
        );
        assert!(
            !result.content.contains("Implement feature X"),
            "tool result must not echo the list: {}",
            result.content
        );
    }

    #[test]
    fn test_list_todos() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task A", "status": "pending"},
                {"content": "Task B", "status": "completed"}
            ]
        }));

        let result = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task A", "status": "pending"},
                {"content": "Task B", "status": "completed"}
            ]
        }));
        assert!(
            result.content.contains("pending: 1"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("completed: 1"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_update_todo() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        // First submit two tasks
        let _ = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task A", "status": "pending"},
                {"content": "Task B", "status": "pending"}
            ]
        }));

        // Update by resubmitting full array with changed status
        let result = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task A", "status": "completed"},
                {"content": "Task B", "status": "pending"}
            ]
        }));
        assert!(
            result.content.contains("completed: 1"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("pending: 1"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_remove_todo() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task A", "status": "pending"},
                {"content": "Task B", "status": "pending"}
            ]
        }));

        // Remove Task A by submitting array without it
        let result = tool.call(serde_json::json!({
            "todos": [
                {"content": "Task B", "status": "pending"}
            ]
        }));
        assert!(
            result.content.contains("pending: 1"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_clear_todos() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({
            "todos": [
                {"content": "A", "status": "pending"},
                {"content": "B", "status": "pending"}
            ]
        }));

        let result = tool.call(serde_json::json!({"todos": []}));
        assert!(
            result.content.contains("pending: 0") && result.content.contains("completed: 0"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("in_progress: 0"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_in_progress_todo() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let result = tool.call(serde_json::json!({
            "todos": [
                {"content": "Active task", "status": "in_progress"},
                {"content": "Queued", "status": "pending"}
            ]
        }));
        assert!(
            result.content.contains("in_progress: 1"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("pending: 1"),
            "got: {}",
            result.content
        );

        let reloaded = load_task_state(&manager, &sid);
        assert_eq!(reloaded.todos[0].status, TodoStatus::InProgress);
    }

    #[test]
    fn test_persisted_to_session_db() {
        let dir = test_dir();
        let (manager, sid, _home) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({
            "todos": [{"content": "Persist me", "status": "pending"}]
        }));

        let reloaded = load_task_state(&manager, &sid);
        assert_eq!(reloaded.todos.len(), 1);
        assert_eq!(reloaded.todos[0].content, "Persist me");
    }

    #[test]
    fn test_requires_active_session() {
        let manager = Arc::new(SessionManager::new(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let tool = TodoWriteTool::new(manager);
        // No session set
        let result = tool.call(serde_json::json!({
            "todos": [{"content": "orphan", "status": "pending"}]
        }));
        assert!(
            result.content.contains("no active session"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_validate_input() {
        let manager = Arc::new(SessionManager::new(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let tool = TodoWriteTool::new(manager);
        // Missing todos
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
        // Empty array is valid (clear)
        assert!(
            tool.validate_input(&serde_json::json!({"todos": []}))
                .is_ok()
        );
        // Valid single item
        assert!(
            tool.validate_input(
                &serde_json::json!({"todos": [{"content": "test", "status": "pending"}]})
            )
            .is_ok()
        );
        // Missing content
        assert!(
            tool.validate_input(&serde_json::json!({"todos": [{"status": "pending"}]}))
                .is_err()
        );
        // Invalid status
        assert!(
            tool.validate_input(
                &serde_json::json!({"todos": [{"content": "test", "status": "bogus"}]})
            )
            .is_err()
        );
    }

    #[test]
    fn test_render_todos() {
        let items = vec![
            TodoItem {
                id: "t1".into(),
                content: "First".into(),
                status: TodoStatus::Pending,
                priority: None,
            },
            TodoItem {
                id: "t2".into(),
                content: "Active".into(),
                status: TodoStatus::InProgress,
                priority: None,
            },
            TodoItem {
                id: "t3".into(),
                content: "Second".into(),
                status: TodoStatus::Completed,
                priority: None,
            },
        ];
        let rendered = render_todos(&items);
        assert!(rendered.contains("○ [t1] First"));
        assert!(rendered.contains("◐ [t2] Active"));
        assert!(rendered.contains("● [t3] Second"));
    }
}
