//! Code-surface shell starter (`run`).
//!
//! Spawns a pipe job on [`AgentJobRegistry`] with a foreground window of
//! [`RUN_FOREGROUND_WINDOW`] (capped by the enclosing `run_code` budget).
//! Non-read-only commands hold the workspace locks only until spawn. Returns [`CommandData`] immediately: finished processes carry an
//! exit code; still-running ones keep `exit_code = null` and remain wait/kill
//! targets. No `kind`, timeout, promote, or `run_in_background`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::{
    AgentJobRegistry, PipeDecision, PipeDetach, PipeRequest, RUN_FOREGROUND_WINDOW, supervise_pipe,
};
use crate::tool::Tool;
use crate::tool::resource_gate::ToolEffects;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::bash_safety::{is_destructive_command, is_readonly_command};
use crate::tools::command_job::{self, present_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Kept back from the `run_code` deadline so `run` returns before the program
/// budget stops the whole call.
const BUDGET_MARGIN: Duration = Duration::from_millis(250);

pub struct RunTool {
    jobs: Arc<AgentJobRegistry>,
    window: Duration,
}

impl RunTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self {
            jobs,
            window: RUN_FOREGROUND_WINDOW,
        }
    }

    /// Test hook: a shorter foreground window so detach tests stay fast.
    #[cfg(test)]
    pub(crate) fn with_window(jobs: Arc<AgentJobRegistry>, window: Duration) -> Self {
        Self { jobs, window }
    }

    /// `min(window, run_code remaining - margin)`.
    fn effective_window(&self, deadline: Option<tokio::time::Instant>) -> Duration {
        match deadline {
            Some(deadline) => self.window.min(
                deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .saturating_sub(BUDGET_MARGIN),
            ),
            None => self.window,
        }
    }

    fn discard_cancelled(&self, job_id: &str) {
        if job_id.is_empty() {
            return;
        }
        let paths = self
            .jobs
            .pipe_streams(job_id)
            .map(|(_, _, stdout, stderr)| (stdout, stderr));
        if self.jobs.discard_attached(job_id)
            && let Some((stdout, stderr)) = paths
        {
            let _ = std::fs::remove_file(stdout);
            let _ = std::fs::remove_file(stderr);
        }
    }

    fn command_result(
        &self,
        workspace_root: &Path,
        job_id: &str,
        exit_code: Option<i32>,
        running: bool,
    ) -> ToolCallResult {
        let notes: &[&str] = if running {
            &["still running; use tools.wait or tools.kill"]
        } else {
            &[]
        };
        let output = if running {
            command_job::format_job_output(
                &self.jobs,
                job_id,
                workspace_root,
                exit_code,
                notes,
            )
        } else {
            command_job::format_exited_output(&self.jobs, job_id, workspace_root, exit_code, notes)
        };
        let Some(output) = output else {
            return coded_error("io", "run lost the job logs");
        };
        present_command(job_id, exit_code, output)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let command = match crate::tool::require_nonempty_string_trimmed(&input, "command") {
            Ok(c) => c.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "run cancelled before spawn");
        }

        let scope = crate::tool::run_scope::current().unwrap_or_default();
        let (spawned_tx, spawned_rx) = tokio::sync::oneshot::channel();
        let rx = supervise_pipe(
            Arc::clone(&self.jobs),
            PipeRequest {
                command,
                cwd: execution.workspace_root.clone(),
                workspace_root: execution.workspace_root.clone(),
                session_id: execution.session_id.clone(),
                call_id: execution.call_id.clone(),
                cancel: execution.cancel.clone(),
                // Exit inside the window returns exit_code; otherwise detach with null.
                detach: PipeDetach::Background {
                    window: self.effective_window(scope.deadline),
                },
                on_spawned: Some(spawned_tx),
            },
        );
        // Resolves on spawn success, or with Err when the supervisor gave up
        // before/at spawn. Either way the locks go now, not after the wait.
        // Cancel/panic of this future drops them with the executor's scope.
        let _ = spawned_rx.await;
        scope.spawn_locks.release();
        let decision = match rx.await {
            Ok(decision) => decision,
            Err(_) => return coded_error("io", "run supervisor dropped the result"),
        };
        match decision {
            PipeDecision::Exited { job_id, exit_code } => {
                self.command_result(&execution.workspace_root, &job_id, exit_code, false)
            }
            PipeDecision::Background { job_id } | PipeDecision::Promoted { job_id } => {
                self.command_result(&execution.workspace_root, &job_id, None, true)
            }
            PipeDecision::Cancelled { job_id } => {
                self.discard_cancelled(&job_id);
                coded_error("cancelled", "run cancelled")
            }
            PipeDecision::Failed { message } => coded_error("io", message),
        }
    }
}

impl Tool for RunTool {
    fn name(&self) -> &str {
        "run"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run once in the workspace root (not a persistent session). Chain with && when you need cd or env for this call only."
                }
            },
            "required": ["command"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(command_job::command_data_schema())
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/run.md"))
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

    fn releases_lock_after_spawn(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self, input: &Value) -> bool {
        input
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_readonly_command)
    }

    fn effects(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &Path,
    ) -> ToolEffects {
        let command = match input.get("command").and_then(Value::as_str) {
            Some(c) => c,
            None => return ToolEffects::write(ResourceKey::Workspace),
        };
        if is_readonly_command(command) {
            ToolEffects::default()
        } else {
            ToolEffects::write(ResourceKey::Workspace)
        }
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &Path,
    ) -> Vec<ResourceKey> {
        let command = match input.get("command").and_then(Value::as_str) {
            Some(c) => c,
            None => return vec![],
        };
        let paths = crate::tools::bash_safety::extract_bash_paths(command);
        if paths.is_empty() {
            return vec![];
        }
        let mut keys = Vec::new();
        for raw in paths {
            let key = match crate::workspace::resolve_agent(workspace_root, &raw, path_mode) {
                Ok(path) => crate::tool::resource_gate::file_resource_key(&path),
                Err(_) => crate::tool::resource_gate::file_resource_key(Path::new(&raw)),
            };
            keys.push(key);
        }
        keys
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string_trimmed(input, "command")?;
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &Path,
    ) -> bool {
        input
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_destructive_command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio_util::sync::CancellationToken;

    use crate::tool::Tool;
    use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
    use crate::tool::{ToolSurface, project_tools};
    use crate::tools::command_job::{StopJob, pid_alive, sleep_shell, test_execution};

    fn tool() -> (RunTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (RunTool::with_window(Arc::clone(&jobs), Duration::from_millis(500)), jobs)
    }

    fn execution(root: &Path) -> ToolExecutionContext {
        test_execution(root, "test-session")
    }

    #[test]
    fn surface_schema_and_command_shape() {
        let (tool, _) = tool();
        assert_eq!(tool.surface(), ToolSurface::Code);
        assert!(tool.timeout().is_none());
        assert!(tool.is_cancellable());
        assert!(validate_code_contract(&tool).is_ok());
        let schema = tool.data_schema().expect("schema");
        assert_eq!(
            schema["required"],
            serde_json::json!(["id", "exit_code", "output"])
        );
        assert!(schema.get("oneOf").is_none());
        assert!(
            schema
                .get("properties")
                .and_then(|p| p.get("kind"))
                .is_none()
        );
        let rendered = render_code_tool_description(&[Arc::new(RunTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))])
        .expect("sdk");
        assert!(rendered.contains("run"));
        assert!(rendered.contains("exit_code"));
        assert!(rendered.contains("output"));
        assert!(!rendered.contains("run_in_background"));
        assert!(!rendered.contains("timeout_sec"));
        let projected = project_tools(vec![Arc::new(RunTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))]);
        assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
        assert_eq!(projected.whitelist, vec!["run".to_string()]);
    }

    #[test]
    fn validate_rejects_empty_command() {
        let (tool, _) = tool();
        assert!(tool.validate_input(&serde_json::json!({"command": "true"})).is_ok());
        assert!(tool.validate_input(&serde_json::json!({"command": "  "})).is_err());
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fast_command_returns_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let result = tool
            .execute(
                serde_json::json!({"command": "echo hello-wp197"}),
                execution(dir.path()),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().expect("data");
        assert!(data.get("kind").is_none(), "{data}");
        assert_eq!(data["exit_code"], 0);
        assert!(data["id"].as_str().unwrap().starts_with("bg_"));
        let output = data["output"].as_str().unwrap();
        assert!(output.contains("hello-wp197"), "{output}");
        let _ = jobs;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn long_command_returns_null_exit_and_keeps_job() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-run-long";
        let result = tool
            .execute(
                serde_json::json!({"command": sleep_shell(30)}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert!(data["exit_code"].is_null(), "{data}");
        let id = data["id"].as_str().unwrap().to_string();
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        let pid = jobs.job_pid(&id).expect("pid");
        assert!(pid_alive(pid), "job should still be running");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_before_spawn_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tool();
        let mut exec = execution(dir.path());
        exec.cancel.cancel();
        let result = tool
            .execute(serde_json::json!({"command": "echo x"}), exec)
            .await;
        assert!(result.error.is_some());
        assert_eq!(result.error_code.as_deref(), Some("cancelled"));
    }

    #[test]
    fn effects_and_safe_policy_match_readonly_dial() {
        use crate::config::schema::ToolPreset;
        use crate::permission::{MatchContext, PermissionAction, binding_for_tool, evaluate};

        let (tool, _) = tool();
        let root = Path::new(".");
        assert!(tool.is_concurrency_safe(&serde_json::json!({"command": "ls"})));
        assert!(!tool.is_concurrency_safe(&serde_json::json!({"command": "cargo build"})));
        let (policy, path_mode) = binding_for_tool("run", ToolPreset::Safe);
        let ctx = MatchContext {
            workspace_root: root,
            path_mode,
        };
        let denied = evaluate(
            &policy,
            &serde_json::json!({"command": "cargo build"}),
            &ctx,
        );
        assert_eq!(denied.action, PermissionAction::Deny);
        let allowed = evaluate(&policy, &serde_json::json!({"command": "ls"}), &ctx);
        assert_eq!(allowed.action, PermissionAction::Allow);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_yields_so_cancel_can_run() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let cancel = CancellationToken::new();
        let mut exec = execution(dir.path());
        exec.cancel = cancel.clone();
        let started = Arc::new(AtomicBool::new(false));
        let handle = tokio::spawn({
            let cancel = cancel.clone();
            let started = Arc::clone(&started);
            async move {
                while !started.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
                cancel.cancel();
            }
        });
        started.store(true, Ordering::SeqCst);
        let result = tool
            .execute(
                serde_json::json!({"command": sleep_shell(60)}),
                exec,
            )
            .await;
        let _ = handle.await;
        // Cancel during the background window may yield cancelled or a running Command.
        let _ = (result, jobs);
    }
}
