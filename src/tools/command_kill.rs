//! Code-surface stop of a `run_command` job (`command_kill`).
//!
//! Not a Direct tool. Killing uses the registry's pid tree kill
//! (`kill_registered`, Unix process group / Windows `taskkill /T /F`) and then
//! `take_notice`, so the exit does not become a user-stopped reminder.
//! An unknown id or another session is `not_found`. Human PTY ids are not in
//! the registry and are left alone.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::AgentJobRegistry;
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// How long to wait for the supervisor to record the real exit code after the
/// tree kill. The wait is `tokio::time::sleep`, not the registry condvar.
const REAP_WAIT: Duration = Duration::from_secs(2);

pub struct CommandKillTool {
    jobs: Arc<AgentJobRegistry>,
}

impl CommandKillTool {
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
        let Some(logs) = command_job::job_logs(&self.jobs, job_id, &execution.workspace_root)
        else {
            return coded_error("io", "command_kill lost the job logs");
        };
        let shown = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "null".into());
        ToolOutcome::ok(KilledData {
            kind: "killed",
            job_id: job_id.to_string(),
            exit_code,
            output_file: logs.output_file,
            stderr_file: logs.stderr_file,
        })
        .into_call_result(format!(
            "kind: killed\njob_id: {job_id}\nexit_code: {shown}\n"
        ))
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let job_id = match crate::tool::require_nonempty_string_trimmed(&input, "job_id") {
            Ok(id) => id.to_string(),
            Err(error) => return coded_error("invalid_input", error),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "command_kill cancelled before the signal");
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

#[derive(Debug, Serialize)]
struct KilledData {
    kind: &'static str,
    job_id: String,
    exit_code: Option<i32>,
    output_file: String,
    stderr_file: String,
}

impl Tool for CommandKillTool {
    fn name(&self) -> &str {
        "command_kill"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "job_id from a background or promoted run_command in this session."
                }
            },
            "required": ["job_id"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "description": "The process tree was killed and the exit notice was consumed.",
            "properties": {
                "kind": { "const": "killed" },
                "job_id": { "type": "string" },
                "exit_code": { "type": ["integer", "null"] },
                "output_file": { "type": "string" },
                "stderr_file": { "type": "string" }
            },
            "required": ["kind", "job_id", "exit_code", "output_file", "stderr_file"]
        }))
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/command_kill.md"))
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
        crate::tool::require_nonempty_string_trimmed(input, "job_id")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::terminal::TerminalHub;
    use crate::tool::Tool;
    use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
    use crate::tool::{ToolSurface, project_tools};
    use crate::tools::command_job::{StopJob, pid_alive, sleep_shell, test_execution};
    use crate::tools::run_command::RunCommandTool;

    fn tool() -> (CommandKillTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (CommandKillTool::new(Arc::clone(&jobs)), jobs)
    }

    async fn start_sleep(
        jobs: &Arc<AgentJobRegistry>,
        root: &std::path::Path,
        session: &str,
    ) -> (String, u32) {
        let runner = RunCommandTool::new(Arc::clone(jobs));
        let started = runner
            .execute(
                serde_json::json!({
                    "command": sleep_shell(30),
                    "run_in_background": true
                }),
                test_execution(root, session),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        assert_eq!(started.data.as_ref().unwrap()["kind"], "background");
        let job_id = started.data.as_ref().unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let pid = jobs.job_pid(&job_id).expect("pid");
        (job_id, pid)
    }

    #[test]
    fn surface_schema_and_admission() {
        let (tool, _) = tool();
        assert_eq!(tool.surface(), ToolSurface::Code);
        assert!(tool.timeout().is_none());
        let description = tool.description(&crate::context_pipeline::Context {
            cwd: std::path::PathBuf::new(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(std::path::Path::new(
                ".",
            )),
            agents_md: None,
            claude_md: None,
        });
        assert!(!description.contains("wait_shell"));
        assert!(!description.contains("kill_shell"));
        assert!(validate_code_contract(&tool).is_ok());
        let rendered = render_code_tool_description(&[Arc::new(tool)]).expect("sdk");
        assert!(rendered.contains("command_kill"));
        assert!(rendered.contains("killed"));
        assert!(!rendered.contains("kill_shell"));
        let projected = project_tools(vec![Arc::new(CommandKillTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))]);
        assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
        assert_eq!(projected.whitelist, vec!["command_kill".to_string()]);
    }

    #[test]
    fn permission_is_fixed_allow_without_command_policy() {
        use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
        use crate::config::schema::{GlobalSettings, PermissionSurface};
        use crate::permission::floor::check_floor;
        use crate::permission::{
            BindingPathMode, MatchContext, PermissionAction, PermissionEngine, permission_surface,
        };

        assert_eq!(permission_surface("command_kill"), PermissionSurface::Fixed);
        let root = std::path::Path::new(".");
        let ctx = MatchContext {
            workspace_root: root,
            path_mode: BindingPathMode::Unrestricted,
        };
        assert!(
            check_floor(
                "command_kill",
                &serde_json::json!({"job_id": "bg_a", "command": "rm -rf /"}),
                &ctx
            )
            .is_none()
        );
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
        let engine = PermissionEngine::resolver(resolved, "default", 0);
        let decision = engine.evaluate_tool(
            "command_kill",
            &serde_json::json!({"job_id": "bg_a", "command": "rm -rf /"}),
            root,
        );
        assert_eq!(decision.action, PermissionAction::Allow);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_id_is_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let result = tool
            .run(
                serde_json::json!({"job_id": "pty_human_1"}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert_eq!(
            result.error_code.as_deref(),
            Some("not_found"),
            "{result:?}"
        );
        assert!(result.data.is_none());
        assert!(!jobs.mailbox_pending("sess"));
        assert!(jobs.get("pty_human_1").is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn other_session_is_not_found_and_stays_alive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        let (job_id, pid) = start_sleep(&jobs, dir.path(), "other").await;
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: job_id.clone(),
            session_id: "other".into(),
        };
        jobs.insert_live_pipe_job("bg_mine001", "sess", "c", "sleep", dir.path())
            .unwrap();
        jobs.insert_live_pipe_job("bg_third01", "third", "c", "sleep", dir.path())
            .unwrap();
        let tool = CommandKillTool::new(Arc::clone(&jobs));
        let result = tool
            .run(
                serde_json::json!({"job_id": job_id}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert_eq!(
            result.error_code.as_deref(),
            Some("not_found"),
            "{result:?}"
        );
        assert!(result.content.contains("bg_mine001"), "{}", result.content);
        assert!(!result.content.contains("bg_third01"), "{}", result.content);
        assert!(pid_alive(pid), "cross-session kill signaled the process");
        assert!(jobs.get(&job_id).unwrap().0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kills_process_tree_and_takes_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        let (job_id, pid) = start_sleep(&jobs, dir.path(), "sess").await;
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: job_id.clone(),
            session_id: "sess".into(),
        };
        let tool = CommandKillTool::new(Arc::clone(&jobs));
        let mut exec = test_execution(dir.path(), "sess");
        exec.call_id = "parent:code:0".into();
        let result = tool.run(serde_json::json!({"job_id": job_id}), exec).await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["kind"], "killed");
        assert_eq!(data["job_id"], job_id);
        assert!(data.get("exit_code").is_some());
        let output_file = data["output_file"].as_str().unwrap();
        assert!(output_file.contains(".litecode/run_command/"));
        assert!(output_file.contains(&job_id));
        assert!(!output_file.contains("parent:code"));
        assert!(data["stderr_file"].as_str().unwrap().contains(&job_id));
        assert!(!jobs.mailbox_pending("sess"));
        let deadline = Instant::now() + Duration::from_secs(3);
        while pid_alive(pid) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!pid_alive(pid), "process tree still alive");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !jobs.mailbox_pending("sess"),
            "supervisor pushed a second exit after take_notice"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ui_bash_kill_leaves_user_killed_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hub = TerminalHub::new();
        let (job_id, pid) = start_sleep(&hub.jobs, dir.path(), "sess").await;
        let _stop = StopJob {
            jobs: Arc::clone(&hub.jobs),
            id: job_id.clone(),
            session_id: "sess".into(),
        };
        let info = hub.kill_from_ui(&job_id).expect("ui kill");
        assert!(!info.alive);
        assert!(hub.jobs.mailbox_pending("sess"));
        let notices = hub.jobs.take_mailbox("sess");
        assert_eq!(notices.len(), 1);
        assert!(
            notices[0].user_killed,
            "UI kill must stay a user-stopped notice"
        );
        assert_eq!(notices[0].bash_id, job_id);
        let deadline = Instant::now() + Duration::from_secs(3);
        while pid_alive(pid) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!pid_alive(pid));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !hub.jobs.mailbox_pending("sess"),
            "a later finish must not replace the consumed user notice with a second one"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_call_is_routed_and_does_not_kill() {
        use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
        use crate::config::schema::GlobalSettings;
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tool::Tool;
        use crate::tool::executor::run_tool;
        use crate::tool::write_lock::WorkspaceWriteLock;
        use crate::types::FunctionToolCall;

        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        jobs.insert_live_pipe_job("bg_direct1", "sess", "c", "sleep", dir.path())
            .unwrap();
        let tool: Arc<dyn Tool> = Arc::new(CommandKillTool::new(Arc::clone(&jobs)));
        let tools = vec![tool];
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(dir.path()));
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let call = FunctionToolCall {
            arguments: r#"{"job_id":"bg_direct1"}"#.into(),
            call_id: "call_1".into(),
            name: "command_kill".into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let sink = deny_permission_sink();
        let result = run_tool(
            &call,
            &tools,
            &permission,
            &ctx,
            "sess",
            "default",
            sink.as_ref(),
            tokio_util::sync::CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        assert!(
            result
                .content
                .contains("only available as tools.command_kill inside run_code"),
            "{}",
            result.content
        );
        assert!(
            jobs.get("bg_direct1").unwrap().0,
            "direct call killed the job"
        );
        assert!(!jobs.mailbox_pending("sess"));
    }
}
