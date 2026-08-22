//! Plan tool — workspace-scoped markdown plans under `.litecode/plan/`.

use std::sync::Arc;
use std::sync::Mutex;

use petname::{Generator, Petnames};
use serde_json::Value;

use crate::config::workspace::active_paths;
use crate::context_pipeline::Context;
use crate::session::manager::SessionManager;
use crate::session::task_state::{PlanRef, plan_dir};
use crate::tool::Tool;
use crate::types::{LitecodeError, Result, ToolCallResult};

pub struct PlanTool {
    sessions: Arc<SessionManager>,
    current_session_id: Mutex<Option<String>>,
}

impl PlanTool {
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
            .ok_or_else(|| LitecodeError::ToolExecution("no active session".into()))
    }
}

impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "finish"],
                    "description": "The action to perform: create a new plan or finish/clear the active plan"
                },
                "content": {
                    "type": "string",
                    "description": "Plan content in Markdown format (required for create). The plan filename is auto-generated."
                }
            },
            "required": ["action"]
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        if let Err(e) = self.validate_input(&input) {
            return ToolCallResult::error(e);
        }
        match input["action"].as_str() {
            Some("create") => match self.do_create(&input) {
                Ok(s) => ToolCallResult::ok(s),
                Err(e) => ToolCallResult::error(e.to_string()),
            },
            Some("finish") => match self.do_finish() {
                Ok(s) => ToolCallResult::ok(s),
                Err(e) => ToolCallResult::error(e.to_string()),
            },
            Some(other) => ToolCallResult::error(crate::tool::must_be_one_of(
                "action",
                &["create", "finish"],
                other,
            )),
            None => ToolCallResult::error(crate::tool::missing_parameter("action")),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Create or finish the session plan under .litecode/plan/. \
         create writes a product-owned Markdown file (filename is auto-generated). \
         finish clears the active plan pointer — that is the only way to end a plan. \
         Never delete, move, or overwrite .litecode/plan/ with write, edit, or bash; \
         never rm the plan file or the .litecode directory."
            .into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let action = crate::tool::require_nonempty_string(input, "action")?;
        match action {
            "create" => {
                crate::tool::require_nonempty_string(input, "content")?;
            }
            "finish" => {}
            _ => {
                return Err(crate::tool::must_be_one_of(
                    "action",
                    &["create", "finish"],
                    action,
                ));
            }
        }
        Ok(())
    }

    fn set_active_session(&self, session_id: String) {
        *self.current_session_id.lock().unwrap() = Some(session_id);
    }
}

impl PlanTool {
    fn set_active_plan(&self, session_id: &str, slug: &str) -> Result<()> {
        let plan = PlanRef::new(slug);
        self.sessions
            .with_entry_task_state_mut(session_id, |state| {
                state.set_active_plan(plan);
                Ok(())
            })?;
        self.sessions.save_task_state(session_id)?;
        Ok(())
    }

    fn generate_slug(plan_root: &std::path::Path) -> Result<String> {
        let petnames = Petnames::default();
        for _ in 0..16 {
            let slug = petnames.generate_one(2, "-").unwrap_or_default();
            if slug.is_empty() {
                continue;
            }
            if !plan_root.join(format!("{slug}.md")).exists() {
                return Ok(slug);
            }
        }
        let slug = format!(
            "{}-{}",
            petnames
                .generate_one(2, "-")
                .unwrap_or_else(|| "plan".into()),
            ulid::Ulid::new()
        );
        Ok(slug)
    }

    fn do_create(&self, input: &Value) -> Result<String> {
        let session_id = self.session_id()?;
        let paths = active_paths();

        let content = crate::tool::require_nonempty_string(input, "content")
            .map_err(LitecodeError::ToolExecution)?;

        let plan_root = plan_dir(&paths);
        std::fs::create_dir_all(&plan_root)?;

        let slug = Self::generate_slug(&plan_root)?;
        let relative = format!(".litecode/plan/{slug}.md");
        let file_path = plan_root.join(format!("{slug}.md"));
        let tmp_path = plan_root.join(format!(".{slug}.md.tmp"));

        // Atomic create (REV-10): stage the .md as a temp file, persist the
        // active-plan pointer, then publish via rename — a failure before the
        // rename leaves neither a visible .md nor a DB pointer.
        std::fs::write(&tmp_path, content)?;
        if let Err(e) = self.set_active_plan(&session_id, &slug) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
            // Roll back the DB pointer so no orphan pointer survives.
            let _ = self
                .sessions
                .with_entry_task_state_mut(&session_id, |state| {
                    state.clear_plan();
                    Ok(())
                });
            let _ = self.sessions.save_task_state(&session_id);
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }
        Ok(format!(
            "Created plan at {}\nPlan filename was auto-generated; content saved.",
            relative
        ))
    }

    fn do_finish(&self) -> Result<String> {
        let sid = self.session_id()?;
        self.sessions.with_entry_task_state_mut(&sid, |state| {
            state.clear_plan();
            Ok(())
        })?;
        self.sessions.save_task_state(&sid)?;
        Ok("Active plan cleared.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::workspace::set_runtime_paths;
    use crate::session::manager::SessionManager;
    use crate::session::store::Session;
    use crate::session::task_state::TaskReminders;
    use std::sync::Arc;

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

    fn make_tool(manager: Arc<SessionManager>, session_id: &str) -> PlanTool {
        let tool = PlanTool::new(manager);
        tool.set_active_session(session_id.to_string());
        tool
    }

    fn install_paths(dir: &std::path::Path) {
        set_runtime_paths(WorkspacePaths::for_legacy_root(&dir));
    }

    fn setup(dir: &std::path::Path) -> (Arc<SessionManager>, String) {
        install_paths(dir);
        let db = dir.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        make_manager(&db.to_string_lossy())
    }

    fn load_task_state(manager: &SessionManager, session_id: &str) -> TaskReminders {
        manager
            .with_entry_task_state(session_id, |s| Ok(s.clone()))
            .unwrap()
    }

    #[test]
    fn test_create_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let result = tool.call(serde_json::json!({
            "action": "create",
            "content": "# Plan\n\n- Step 1\n- Step 2"
        }));
        assert!(
            result.content.contains("Created plan"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains(".litecode/plan/"),
            "got: {}",
            result.content
        );
        assert!(
            !result.content.contains(&format!("/{sid}/")),
            "path should not include session id: {}",
            result.content
        );
        assert!(
            result.content.contains("auto-generated"),
            "got: {}",
            result.content
        );

        let plan_dir = dir.path().join(".litecode/plan");
        let entries: Vec<_> = std::fs::read_dir(&plan_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].extension().is_some_and(|e| e == "md"));
        let file_content = std::fs::read_to_string(&entries[0]).unwrap();
        assert_eq!(file_content, "# Plan\n\n- Step 1\n- Step 2");

        let slug = entries[0]
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let state = load_task_state(&manager, &sid);
        let plan = state.active_plan.as_ref().expect("active plan");
        assert_eq!(plan.slug, slug);
        assert_eq!(plan.relative_path, format!(".litecode/plan/{slug}.md"));
    }

    #[test]
    fn test_finish_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({"action": "create", "content": "# Plan"}));
        let result = tool.call(serde_json::json!({"action": "finish"}));
        assert!(
            result.content.contains("cleared"),
            "got: {}",
            result.content
        );
        let state = load_task_state(&manager, &sid);
        assert!(state.active_plan.is_none());
    }

    #[test]
    fn test_create_plan_generates_unique_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let first = tool.call(serde_json::json!({
            "action": "create",
            "content": "# Plan v1"
        }));
        let second = tool.call(serde_json::json!({
            "action": "create",
            "content": "# Plan v2"
        }));
        assert!(first.content.contains("Created plan"));
        assert!(second.content.contains("Created plan"));

        let plan_dir = dir.path().join(".litecode/plan");
        let count = std::fs::read_dir(&plan_dir).unwrap().count();
        assert_eq!(count, 2);

        let state = load_task_state(&manager, &sid);
        assert!(state.active_plan.is_some());
    }

    #[test]
    fn test_finish_keeps_plan_file_on_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let created = tool.call(serde_json::json!({"action": "create", "content": "# Plan"}));
        let slug = created
            .content
            .lines()
            .find(|l| l.contains(".litecode/plan/"))
            .and_then(|l| l.split(".litecode/plan/").nth(1))
            .and_then(|rest| rest.strip_suffix(".md"))
            .expect("slug in create output");

        let plan_file = dir.path().join(".litecode/plan").join(format!("{slug}.md"));
        assert!(plan_file.is_file());

        let result = tool.call(serde_json::json!({"action": "finish"}));
        assert!(result.content.contains("cleared"));
        assert!(load_task_state(&manager, &sid).active_plan.is_none());
        assert!(
            plan_file.is_file(),
            "finish should not delete the plan file"
        );
    }

    #[test]
    fn test_second_create_points_active_plan_at_latest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        tool.call(serde_json::json!({"action": "create", "content": "# Plan v1"}));
        let second = tool.call(serde_json::json!({"action": "create", "content": "# Plan v2"}));

        let slug = second
            .content
            .lines()
            .find(|l| l.contains(".litecode/plan/"))
            .and_then(|l| l.split(".litecode/plan/").nth(1))
            .and_then(|rest| rest.strip_suffix(".md"))
            .expect("slug in second create output");

        let state = load_task_state(&manager, &sid);
        let plan = state.active_plan.as_ref().expect("active plan");
        assert_eq!(plan.slug, slug);
        let file_content =
            std::fs::read_to_string(dir.path().join(".litecode/plan").join(format!("{slug}.md")))
                .unwrap();
        assert_eq!(file_content, "# Plan v2");
    }

    #[test]
    fn test_session_resume_restores_flat_active_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let created = tool.call(serde_json::json!({"action": "create", "content": "# Persisted"}));
        let slug = created
            .content
            .lines()
            .find(|l| l.contains(".litecode/plan/"))
            .and_then(|l| l.split(".litecode/plan/").nth(1))
            .and_then(|rest| rest.strip_suffix(".md"))
            .expect("slug in create output");

        let db = dir.path().join(".litecode").join("sessions.db");
        let resumed = Session::resume(&db.to_string_lossy(), &sid).unwrap();
        let state = resumed.load_task_state().unwrap();
        let plan = state
            .active_plan
            .as_ref()
            .expect("active plan after resume");
        assert_eq!(plan.slug, slug);
        assert_eq!(plan.relative_path, format!(".litecode/plan/{slug}.md"));
    }

    #[test]
    fn test_validate_input_rejects_empty_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(manager, &sid);

        let err = tool
            .validate_input(&serde_json::json!({"action": "create", "content": ""}))
            .unwrap_err();
        assert!(err.contains("content"));
    }

    #[test]
    fn test_requires_active_session() {
        let manager = Arc::new(SessionManager::new(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let tool = PlanTool::new(manager);
        let result = tool.call(serde_json::json!({
            "action": "create",
            "content": "# Plan"
        }));
        assert!(
            result.content.contains("no active session"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_list_action_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(manager, &sid);

        let result = tool.call(serde_json::json!({"action": "list"}));
        assert!(
            result.content.contains("expected one of create, finish"),
            "got: {}",
            result.content
        );
    }
}
