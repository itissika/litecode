//! Plan tool — workspace-scoped markdown plans under `.litecode/plan/`.

use std::sync::Arc;
use std::sync::Mutex;

use petname::{Generator, Petnames};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::permission::{AskOutcome, AskPrompt, PermissionSink};
use crate::session::manager::SessionManager;
use crate::session::task_state::{PlanRef, plan_content_revision};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};

/// Heading-only stub after Approve. `edit` can replace it; a blank file cannot.
/// The plan panel treats this heading as still-planning and opens once the body is written.
const PLAN_STUB: &str = "# Plan\n";

struct PlanStore {
    sessions: Option<Arc<SessionManager>>,
    current_session_id: Mutex<Option<String>>,
}

impl PlanStore {
    fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions: Some(sessions),
            current_session_id: Mutex::new(None),
        }
    }

    fn cold() -> Self {
        Self {
            sessions: None,
            current_session_id: Mutex::new(None),
        }
    }

    fn sessions(&self) -> Result<&Arc<SessionManager>> {
        self.sessions
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("plan tool requires an active session manager".into()))
    }

    fn set_session(&self, session_id: String) {
        *self.current_session_id.lock().unwrap() = Some(session_id);
    }

    fn session_id(&self) -> Result<String> {
        self.current_session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("no active session".into()))
    }
}

pub struct PlanCreateTool {
    store: PlanStore,
    permission_sink: Mutex<Option<Arc<dyn PermissionSink>>>,
}

impl PlanCreateTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            store: PlanStore::new(sessions),
            permission_sink: Mutex::new(None),
        }
    }

    pub fn cold() -> Self {
        Self {
            store: PlanStore::cold(),
            permission_sink: Mutex::new(None),
        }
    }

    fn permission_sink(&self) -> Option<Arc<dyn PermissionSink>> {
        self.permission_sink.lock().unwrap().clone()
    }
}

pub struct PlanFinishTool {
    store: PlanStore,
}

impl PlanFinishTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            store: PlanStore::new(sessions),
        }
    }

    pub fn cold() -> Self {
        Self {
            store: PlanStore::cold(),
        }
    }
}

impl Tool for PlanCreateTool {
    fn name(&self) -> &str {
        "plan.create"
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

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "summary")?;
        Ok(())
    }

    fn set_active_session(&self, session_id: String) {
        self.store.set_session(session_id);
    }

    fn set_permission_sink(&self, sink: Arc<dyn PermissionSink>) {
        *self.permission_sink.lock().unwrap() = Some(sink);
    }
}

impl Tool for PlanFinishTool {
    fn name(&self) -> &str {
        "plan.finish"
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        match self.do_finish() {
            Ok((content, data)) => ToolCallResult::ok(content).with_data(data),
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn validate_input(&self, _input: &Value) -> std::result::Result<(), String> {
        Ok(())
    }

    fn set_active_session(&self, session_id: String) {
        self.store.set_session(session_id);
    }
}

impl PlanCreateTool {
    fn run(
        &self,
        input: &Value,
        cancel: &CancellationToken,
        sink: Option<&dyn PermissionSink>,
    ) -> ToolCallResult {
        if let Err(e) = self.validate_input(input) {
            return ToolCallResult::error(e);
        }
        match self.do_create(input, cancel, sink) {
            Ok((content, data)) => ToolCallResult::ok(content).with_data(data),
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn set_active_plan(
        &self,
        session_id: &str,
        slug: &str,
        revision: Option<String>,
    ) -> Result<()> {
        let sessions = self.store.sessions()?;
        let plan = PlanRef::with_revision(slug, revision);
        sessions.with_entry_task_state_mut(session_id, |state| {
            state.set_active_plan(plan);
            Ok(())
        })?;
        sessions.save_task_state(session_id)?;
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
    ) -> Result<(String, Value)> {
        let session_id = self.store.session_id()?;
        let summary = crate::tool::require_nonempty_string(input, "summary")
            .map_err(LitecodeError::ToolExecution)?;

        let Some(sink) = sink else {
            return Err(LitecodeError::ToolExecution(
                "plan.create requires a permission Ask bridge (no sink)".into(),
            ));
        };

        let prompt = AskPrompt::approval("plan", "create", summary);
        let reply = sink.ask(&prompt, cancel);
        match reply.outcome {
            AskOutcome::Aborted => {
                return Err(LitecodeError::ToolExecution(
                    "plan.create aborted (turn cancelled)".into(),
                ));
            }
            AskOutcome::Deny => {
                let mut msg =
                    "Plan create rejected by the user. No plan file was created.".to_string();
                let mut data = serde_json::Map::new();
                data.insert("approved".into(), Value::Bool(false));
                if let Some(opinion) = reply.free_text.as_deref() {
                    msg.push_str("\nUser opinion: ");
                    msg.push_str(opinion);
                    data.insert("opinion".into(), Value::String(opinion.to_string()));
                }
                return Ok((msg, Value::Object(data)));
            }
            AskOutcome::Allow { .. } => {}
        }

        let sessions = self.store.sessions()?;
        let plan_root = sessions.plan_dir_path();
        std::fs::create_dir_all(&plan_root)?;

        let slug = Self::generate_slug(&plan_root)?;
        let relative = format!(".litecode/plan/{slug}.md");
        let file_path = plan_root.join(format!("{slug}.md"));
        let tmp_path = plan_root.join(format!(".{slug}.md.tmp"));

        // Atomic create (REV-10): stage stub, persist active-plan pointer, rename.
        std::fs::write(&tmp_path, PLAN_STUB)?;
        let revision = plan_content_revision(PLAN_STUB.as_bytes());
        if let Err(e) = self.set_active_plan(&session_id, &slug, Some(revision.clone())) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
            let _ = sessions.with_entry_task_state_mut(&session_id, |state| {
                state.clear_plan();
                Ok(())
            });
            let _ = sessions.save_task_state(&session_id);
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e.into());
        }

        let mut out = format!(
            "Created plan at {relative}\nUser approved. The file contains only `# Plan`. Replace that heading with the full plan body, then present it and ask the user to confirm before implementing."
        );
        let mut data = serde_json::Map::new();
        data.insert("approved".into(), Value::Bool(true));
        data.insert("plan_path".into(), Value::String(relative));
        data.insert("revision".into(), Value::String(revision));
        if let Some(opinion) = reply.free_text.as_deref() {
            out.push_str("\nUser opinion: ");
            out.push_str(opinion);
            data.insert("opinion".into(), Value::String(opinion.to_string()));
        }
        Ok((out, Value::Object(data)))
    }
}

impl PlanFinishTool {
    fn do_finish(&self) -> Result<(String, Value)> {
        let sessions = self.store.sessions()?;
        let sid = self.store.session_id()?;
        sessions.with_entry_task_state_mut(&sid, |state| {
            state.clear_plan();
            Ok(())
        })?;
        sessions.save_task_state(&sid)?;
        Ok(("Active plan cleared.".into(), json!({ "cleared": true })))
    }
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let finish_sample = ToolCallResult::ok("Active plan cleared.")
        .with_data(json!({ "cleared": true }));
    vec![
        DriftCase {
            tool: "plan.create",
            all_optional_present: ToolCallResult::ok("Created plan").with_data(json!({
                "approved": true,
                "plan_path": ".litecode/plan/demo.md",
                "revision": "abc123",
                "opinion": "looks good",
            })),
            all_optional_absent: ToolCallResult::ok("Plan create rejected")
                .with_data(json!({ "approved": false })),
        },
        DriftCase {
            tool: "plan.finish",
            all_optional_present: finish_sample.clone(),
            all_optional_absent: finish_sample,
        },
    ]
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

    fn make_create_tool_with_sink(
        manager: Arc<SessionManager>,
        session_id: &str,
        sink: Arc<dyn PermissionSink>,
    ) -> PlanCreateTool {
        let tool = PlanCreateTool::new(manager);
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

    #[test]
    fn test_create_rejected_does_not_create_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manager, sid) = setup(dir.path());
        let sink =
            Arc::new(RecordingPermissionSink::new(AskOutcome::Deny).with_free_text("too vague"));
        let tool = make_create_tool_with_sink(Arc::clone(&manager), &sid, sink);

        let result = tool.call(serde_json::json!({
            "summary": "Do stuff"
        }));
        assert!(
            result.content.contains("rejected"),
            "got: {}",
            result.content
        );
        assert!(
            result.content.contains("User opinion: too vague"),
            "got: {}",
            result.content
        );
        assert_eq!(result.level, ToolSignalLevel::Ok);
        assert_eq!(result.data.as_ref().unwrap()["approved"], false);
        let plan_dir = dir.path().join(".litecode/plan");
        assert!(
            !plan_dir.exists() || std::fs::read_dir(&plan_dir).unwrap().count() == 0,
            "reject must not create a plan file"
        );
        assert!(load_task_state(&manager, &sid).active_plan.is_none());
    }
}
