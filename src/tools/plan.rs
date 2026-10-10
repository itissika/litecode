//! Plan tool — workspace-scoped markdown plans under `.litecode/plan/`.

use std::sync::Arc;
use std::sync::Mutex;

use petname::{Generator, Petnames};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskOutcome, AskPrompt, PermissionSink};
use crate::session::manager::SessionManager;
use crate::session::task_state::{PlanRef, plan_content_revision};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};

/// Empty placeholder after Approve. FE treats blank / legacy "# Plan" as
/// still-planning (capsule bounce only); opens the panel once the body is written.
const PLAN_STUB: &str = "";

pub struct PlanTool {
    sessions: Arc<SessionManager>,
    current_session_id: Mutex<Option<String>>,
    permission_sink: Mutex<Option<Arc<dyn PermissionSink>>>,
}

impl PlanTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            current_session_id: Mutex::new(None),
            permission_sink: Mutex::new(None),
        }
    }

    fn session_id(&self) -> Result<String> {
        self.current_session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("no active session".into()))
    }

    fn permission_sink(&self) -> Option<Arc<dyn PermissionSink>> {
        self.permission_sink.lock().unwrap().clone()
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
                    "description": "create: propose a plan (user Approve/Reject). finish: clear the active plan pointer."
                },
                "summary": {
                    "type": "string",
                    "description": "Short summary of the plan you intend to write (required for create). Shown to the user for Approve/Reject; not the plan body."
                }
            },
            "required": ["action"]
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.run(
            &input,
            &CancellationToken::new(),
            self.permission_sink().as_deref(),
        )
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let sink = self.permission_sink();
        let cancel = execution.cancel.clone();
        Box::pin(async move { self.run(&input, &cancel, sink.as_deref()) })
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/plan.md"))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let action = crate::tool::require_nonempty_string(input, "action")?;
        match action {
            "create" => {
                crate::tool::require_nonempty_string(input, "summary")?;
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

    fn set_permission_sink(&self, sink: Arc<dyn PermissionSink>) {
        *self.permission_sink.lock().unwrap() = Some(sink);
    }
}

impl PlanTool {
    fn run(
        &self,
        input: &Value,
        cancel: &CancellationToken,
        sink: Option<&dyn PermissionSink>,
    ) -> ToolCallResult {
        if let Err(e) = self.validate_input(input) {
            return ToolCallResult::error(e);
        }
        match input["action"].as_str() {
            Some("create") => match self.do_create(input, cancel, sink) {
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

    fn set_active_plan(
        &self,
        session_id: &str,
        slug: &str,
        revision: Option<String>,
    ) -> Result<()> {
        let plan = PlanRef::with_revision(slug, revision);
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

    fn do_create(
        &self,
        input: &Value,
        cancel: &CancellationToken,
        sink: Option<&dyn PermissionSink>,
    ) -> Result<String> {
        let session_id = self.session_id()?;
        let summary = crate::tool::require_nonempty_string(input, "summary")
            .map_err(LitecodeError::ToolExecution)?;

        let Some(sink) = sink else {
            return Err(LitecodeError::ToolExecution(
                "plan create requires a permission Ask bridge (no sink)".into(),
            ));
        };

        let prompt = AskPrompt::approval("plan", "create", summary);
        let reply = sink.ask(&prompt, cancel);
        match reply.outcome {
            AskOutcome::Aborted => {
                return Err(LitecodeError::ToolExecution(
                    "plan create aborted (turn cancelled)".into(),
                ));
            }
            AskOutcome::Deny => {
                let mut msg = "Plan create rejected by the user. No plan file was created.".to_string();
                if let Some(opinion) = reply.free_text.as_deref() {
                    msg.push_str("\nUser opinion: ");
                    msg.push_str(opinion);
                }
                return Ok(msg);
            }
            AskOutcome::Allow { .. } => {}
        }

        let plan_root = self.sessions.plan_dir_path();
        std::fs::create_dir_all(&plan_root)?;

        let slug = Self::generate_slug(&plan_root)?;
        let relative = format!(".litecode/plan/{slug}.md");
        let file_path = plan_root.join(format!("{slug}.md"));
        let tmp_path = plan_root.join(format!(".{slug}.md.tmp"));

        // Atomic create (REV-10): stage stub, persist active-plan pointer, rename.
        std::fs::write(&tmp_path, PLAN_STUB)?;
        let revision = Some(plan_content_revision(PLAN_STUB.as_bytes()));
        if let Err(e) = self.set_active_plan(&session_id, &slug, revision) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
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

        let mut out = format!(
            "Created plan at {relative}\nUser approved. Write the full plan body with the edit tool at that path (do not use write)."
        );
        if let Some(opinion) = reply.free_text.as_deref() {
            out.push_str("\nUser opinion: ");
            out.push_str(opinion);
        }
        Ok(out)
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
    use crate::permission::{AskOutcome, RecordingPermissionSink};
    use crate::session::manager::SessionManager;
    use crate::session::task_state::TaskReminders;
    use crate::types::ToolSignalLevel;
    use std::sync::Arc;

    fn make_manager(db_path: &str) -> (Arc<SessionManager>, String) {
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            db_path.to_string(),
        ));
        let sid = manager
            .open_session_sync("/proj", "default", Some("m"))
            .unwrap();
        (manager, sid)
    }

    fn make_tool(manager: Arc<SessionManager>, session_id: &str) -> PlanTool {
        let tool = PlanTool::new(manager);
        tool.set_active_session(session_id.to_string());
        tool.set_permission_sink(Arc::new(RecordingPermissionSink::from_reply(true, false)));
        tool
    }

    fn make_tool_with_sink(
        manager: Arc<SessionManager>,
        session_id: &str,
        sink: Arc<dyn PermissionSink>,
    ) -> PlanTool {
        let tool = PlanTool::new(manager);
        tool.set_active_session(session_id.to_string());
        tool.set_permission_sink(sink);
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

    fn slug_from_create(content: &str) -> String {
        content
            .lines()
            .find(|l| l.contains(".litecode/plan/"))
            .and_then(|l| l.split(".litecode/plan/").nth(1))
            .and_then(|rest| rest.strip_suffix(".md"))
            .expect("slug in create output")
            .to_string()
    }

    #[test]
    fn test_create_plan_asks_and_writes_stub() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let sink = Arc::new(
            RecordingPermissionSink::from_reply(true, false).with_free_text("looks good"),
        );
        let tool = make_tool_with_sink(Arc::clone(&manager), &sid, sink.clone());

        let result = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Refactor auth to use sessions"
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
            result.content.contains("User opinion: looks good"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("edit tool"),
            "got: {}",
            result.content
        );
        assert!(
            !result.content.contains(&format!("/{sid}/")),
            "path should not include session id: {}",
            result.content
        );

        let calls = sink.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "plan");
        assert_eq!(calls[0].1, "create");
        assert_eq!(calls[0].2, "Refactor auth to use sessions");

        let plan_dir = dir.path().join(".litecode/plan");
        let entries: Vec<_> = std::fs::read_dir(&plan_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].extension().is_some_and(|e| e == "md"));
        let file_content = std::fs::read_to_string(&entries[0]).unwrap();
        assert_eq!(file_content, PLAN_STUB);

        let slug = entries[0]
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let state = load_task_state(&manager, &sid);
        let plan = state.active_plan.as_ref().expect("active plan");
        assert_eq!(plan.slug, slug);
        assert_eq!(plan.relative_path, format!(".litecode/plan/{slug}.md"));
        let expected_revision =
            crate::session::task_state::plan_content_revision(file_content.as_bytes());
        assert_eq!(plan.revision.as_deref(), Some(expected_revision.as_str()));
    }

    #[test]
    fn test_create_rejected_does_not_create_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let sink = Arc::new(
            RecordingPermissionSink::new(AskOutcome::Deny).with_free_text("too vague"),
        );
        let tool = make_tool_with_sink(Arc::clone(&manager), &sid, sink);

        let result = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Do stuff"
        }));
        assert!(
            result.content.contains("rejected"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("No plan file was created"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("User opinion: too vague"),
            "got: {}",
            result.content
        );
        assert_eq!(result.level, ToolSignalLevel::Ok);
        let plan_dir = dir.path().join(".litecode/plan");
        assert!(
            !plan_dir.exists() || std::fs::read_dir(&plan_dir).unwrap().count() == 0,
            "reject must not create a plan file"
        );
        assert!(load_task_state(&manager, &sid).active_plan.is_none());
    }

    #[test]
    fn test_finish_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let _ = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Ship feature X"
        }));
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
            "summary": "Plan v1"
        }));
        let second = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Plan v2"
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

        let created = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Keep file"
        }));
        let slug = slug_from_create(&created.content);

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

        tool.call(serde_json::json!({"action": "create", "summary": "Plan v1"}));
        let second = tool.call(serde_json::json!({"action": "create", "summary": "Plan v2"}));

        let slug = slug_from_create(&second.content);

        let state = load_task_state(&manager, &sid);
        let plan = state.active_plan.as_ref().expect("active plan");
        assert_eq!(plan.slug, slug);
        let file_content =
            std::fs::read_to_string(dir.path().join(".litecode/plan").join(format!("{slug}.md")))
                .unwrap();
        assert_eq!(file_content, PLAN_STUB);
    }

    #[test]
    fn test_session_resume_restores_flat_active_plan() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(Arc::clone(&manager), &sid);

        let created = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Persisted"
        }));
        let slug = slug_from_create(&created.content);

        let plan = manager
            .with_entry_task_state(&sid, |state| Ok(state.active_plan.clone()))
            .unwrap()
            .expect("active plan after resume");
        assert_eq!(plan.slug, slug);
        assert_eq!(plan.relative_path, format!(".litecode/plan/{slug}.md"));
    }

    #[test]
    fn test_validate_input_rejects_empty_summary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(manager, &sid);

        let err = tool
            .validate_input(&serde_json::json!({"action": "create", "summary": ""}))
            .unwrap_err();
        assert!(err.contains("summary"));
    }

    #[test]
    fn test_create_without_sink_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = PlanTool::new(manager);
        tool.set_active_session(sid);
        let result = tool.call(serde_json::json!({
            "action": "create",
            "summary": "No sink"
        }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(
            result.content.contains("permission Ask bridge"),
            "got: {}",
            result.content
        );
    }

    #[test]
    fn test_requires_active_session() {
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ));
        let tool = PlanTool::new(manager);
        tool.set_permission_sink(Arc::new(RecordingPermissionSink::from_reply(true, false)));
        let result = tool.call(serde_json::json!({
            "action": "create",
            "summary": "Plan"
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

    #[test]
    fn test_schema_has_summary_not_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let tool = make_tool(manager, &sid);
        let schema = tool.schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("summary").is_some());
        assert!(props.get("content").is_none());
    }
}
