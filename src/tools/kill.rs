//! Code-surface stop of a `run` job (`kill`).
//!
//! Killing uses the registry's pid tree kill (`kill_registered`, Unix process
//! group / Windows `taskkill /T /F`) and then `take_notice`, so the exit does
//! not become a user-stopped reminder. An unknown id or another session is
//! `not_found`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::AgentJobRegistry;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job::{self, present_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// How long to wait for the supervisor to record the real exit code after the
/// tree kill. The wait is `tokio::time::sleep`, not the registry condvar.
const REAP_WAIT: Duration = Duration::from_secs(2);

pub struct KillTool {
    jobs: Arc<AgentJobRegistry>,
}

impl KillTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    async fn observed_exit(
        &self,
        job_id: &str,
        seed: Option<u32>,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Option<i32> {
        if let Some(code) = seed {
            return Some(code as i32);
        }
        let deadline = tokio::time::Instant::now() + REAP_WAIT;
        loop {
            if let Some((_, Some(code), _, _)) = self.jobs.get(job_id) {
                return Some(code as i32);
            }
            if cancel.is_cancelled() || tokio::time::Instant::now() >= deadline {
                return self
                    .jobs
                    .get(job_id)
                    .and_then(|(_, code, _, _)| command_job::code_i32(code));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn present(
        &self,
        execution: &ToolExecutionContext,
        job_id: &str,
        exit_code: Option<i32>,
    ) -> ToolCallResult {
        let Some(output) = command_job::format_job_output(
            &self.jobs,
            job_id,
            &execution.workspace_root,
            exit_code,
            &["process was killed"],
        ) else {
            return coded_error("io", "kill lost the job logs");
        };
        present_command(job_id, exit_code, output)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let job_id = match crate::tool::require_nonempty_string_trimmed(&input, "id") {
            Ok(id) => id.to_string(),
            Err(error) => return coded_error("invalid_input", error),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "kill cancelled before the signal");
        }
        if !command_job::owns_session(&self.jobs, &execution.session_id, &job_id) {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        }

        // `user_killed` stays false. The human click is `kill_from_ui`, which
        // leaves the mailbox notice. This path consumes it.
        let Some(killed) = self.jobs.kill_registered(&job_id, false) else {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        };
        self.jobs.take_notice(&execution.session_id, &job_id);
        let exit_code = self
            .observed_exit(&job_id, killed.exit_code, &execution.cancel)
            .await;
        self.present(&execution, &job_id, exit_code)
    }
}

impl Tool for KillTool {
    fn name(&self) -> &str {
        "kill"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Command.id from tools.run or tools.wait in this session."
                }
            },
            "required": ["id"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(command_job::command_data_schema())
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/kill.md"))
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.run(input, execution))
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string_trimmed(input, "id")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::tool::Tool;
    use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
    use crate::tool::{ToolSurface, project_tools};
    use crate::tools::command_job::{StopJob, pid_alive, sleep_shell, test_execution};
    use crate::tools::run::RunTool;

    fn tool() -> (KillTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (KillTool::new(Arc::clone(&jobs)), jobs)
    }

    async fn start_sleep(
        jobs: &Arc<AgentJobRegistry>,
        root: &std::path::Path,
        session: &str,
    ) -> (String, u32) {
        let runner = RunTool::with_window(Arc::clone(jobs), Duration::from_millis(500));
        let started = runner
            .execute(
                serde_json::json!({"command": sleep_shell(30)}),
                test_execution(root, session),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let data = started.data.as_ref().unwrap();
        assert!(data["exit_code"].is_null(), "{data}");
        let job_id = data["id"].as_str().unwrap().to_string();
        let pid = jobs.job_pid(&job_id).expect("pid");
        (job_id, pid)
    }

    #[test]
    fn surface_schema_and_admission() {
        let (tool, _) = tool();
        assert_eq!(tool.surface(), ToolSurface::Code);
        assert!(tool.timeout().is_none());
        assert!(validate_code_contract(&tool).is_ok());
        let rendered = render_code_tool_description(&[Arc::new(KillTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))])
        .expect("sdk");
        assert!(rendered.contains("kill"));
        assert!(rendered.contains("exit_code"));
        let projected = project_tools(vec![Arc::new(KillTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))]);
        assert_eq!(projected.whitelist, vec!["kill".to_string()]);
    }

    #[test]
    fn permission_is_fixed_allow_without_command_policy() {
        use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
        use crate::config::schema::{GlobalSettings, PermissionSurface};
        use crate::permission::floor::check_floor;
        use crate::permission::{
            BindingPathMode, MatchContext, PermissionAction, PermissionEngine, permission_surface,
        };

        assert_eq!(permission_surface("kill"), PermissionSurface::Fixed);
        let root = std::path::Path::new(".");
        let ctx = MatchContext {
            workspace_root: root,
            path_mode: BindingPathMode::Unrestricted,
        };
        assert!(
            check_floor(
                "kill",
                &serde_json::json!({"id": "bg_a", "command": "rm -rf /"}),
                &ctx
            )
            .is_none()
        );
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
        let engine = PermissionEngine::resolver(resolved, "default", 0);
        let decision = engine.evaluate_tool(
            "kill",
            &serde_json::json!({"id": "bg_a", "command": "rm -rf /"}),
            root,
        );
        assert_eq!(decision.action, PermissionAction::Allow);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kill_stops_process_and_notes_output() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-kill";
        let (id, pid) = start_sleep(&jobs, dir.path(), session).await;
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        assert!(pid_alive(pid));
        let started = Instant::now();
        let result = tool
            .execute(
                serde_json::json!({"id": id}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["id"].as_str().unwrap().len() > 0, true);
        assert!(
            data["output"]
                .as_str()
                .unwrap()
                .contains("process was killed"),
            "{data}"
        );
        // Give the OS a moment; pid should die.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && pid_alive(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!pid_alive(pid), "pid should be dead");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_id_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tool();
        let result = tool
            .execute(
                serde_json::json!({"id": "bg_missing"}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert_eq!(result.error_code.as_deref(), Some("not_found"));
    }
}
