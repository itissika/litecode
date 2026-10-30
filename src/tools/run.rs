//! Code-surface shell starter (`run`).
//!
//! Spawns a pipe job on [`AgentJobRegistry`] with a foreground window of
//! [`RUN_FOREGROUND_WINDOW`] (capped by the enclosing `run_code` budget).
//! Non-read-only commands hold the workspace locks only until spawn. Returns [`CommandData`] immediately: finished processes carry an
//! exit code; still-running ones keep `exit_code = null` and remain wait/kill
//! targets. Pass `run_in_background: true` to detach immediately; otherwise a
//! fast command returns its exit code and a command still alive after the
//! window returns `exit_code = null`. No `kind`, timeout, or promote knob.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

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
        if self.jobs.discard_cancelled(job_id)
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
        message: Option<String>,
    ) -> ToolCallResult {
        let notes: &[&str] = if running {
            &["still running; use tools.process.wait or tools.process.kill"]
        } else {
            &[]
        };
        let output = if running {
            command_job::format_job_output(&self.jobs, job_id, workspace_root, exit_code, notes)
        } else {
            command_job::format_exited_output(&self.jobs, job_id, workspace_root, exit_code, notes)
        };
        let Some(output) = output else {
            return coded_error("io", "run lost the job logs");
        };
        present_command(job_id, exit_code, output, message)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let command = match crate::tool::require_nonempty_string_trimmed(&input, "command") {
            Ok(c) => c.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        let run_in_background = match input.get("run_in_background") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => {
                return coded_error(
                    "invalid_input",
                    "parameter 'run_in_background' must be a boolean",
                );
            }
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "run cancelled before spawn");
        }

        let message = command_message(&command);
        let scope = crate::tool::run_scope::current().unwrap_or_default();
        let background_window = if run_in_background {
            Duration::ZERO
        } else {
            self.effective_window(scope.deadline)
        };
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
                    window: background_window,
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
                // The exit is already recorded; give the readers a brief grace
                // so a fast command still returns its full output.
                self.jobs
                    .wait_readers(&job_id, command_job::READER_GRACE)
                    .await;
                self.command_result(
                    &execution.workspace_root,
                    &job_id,
                    exit_code,
                    false,
                    message,
                )
            }
            PipeDecision::Background { job_id } | PipeDecision::Promoted { job_id } => {
                self.command_result(&execution.workspace_root, &job_id, None, true, message)
            }
            PipeDecision::Cancelled { job_id } => {
                self.discard_cancelled(&job_id);
                coded_error("cancelled", "run cancelled")
            }
            PipeDecision::Failed { message } => coded_error("io", message),
        }
    }
}

/// MSYS path-conversion hint, only when Git Bash is the shell (WP-210 F/Q8).
fn command_message(command: &str) -> Option<String> {
    if crate::terminal::decode::shell_identity().contains("MINGW") {
        crate::terminal::decode::msys_path_hint(command)
    } else {
        None
    }
}

impl Tool for RunTool {
    fn name(&self) -> &str {
        "run"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
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

    use crate::tool::Tool;

    use crate::tools::command_job::{StopJob, pid_alive, sleep_shell, test_execution};
    use std::time::Instant;

    fn tool() -> (RunTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (
            RunTool::with_window(Arc::clone(&jobs), Duration::from_millis(500)),
            jobs,
        )
    }

    fn execution(root: &Path) -> ToolExecutionContext {
        test_execution(root, "test-session")
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
        // A small foreground exit keeps its log only until the readers finish,
        // then the row is dropped so no stale job lingers.
        let id = data["id"].as_str().unwrap();
        assert!(
            jobs.get(id).is_none(),
            "sealed foreground job must be dropped"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_flag_returns_before_command_exits() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-run-background";
        let started = Instant::now();
        let result = tool
            .execute(
                serde_json::json!({
                    "command": sleep_shell(30),
                    "run_in_background": true
                }),
                test_execution(dir.path(), session),
            )
            .await;
        let elapsed = started.elapsed();
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().expect("data");
        assert!(data["exit_code"].is_null(), "{data}");
        assert!(
            elapsed < Duration::from_millis(400),
            "run blocked for {elapsed:?}"
        );
        let id = data["id"].as_str().expect("id").to_string();
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        assert!(
            jobs.get(&id).is_some(),
            "background job must remain addressable"
        );
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
    async fn run_returns_when_the_descendant_still_holds_the_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let started = Instant::now();
        let result = tool
            .execute(
                serde_json::json!({"command": "(sleep 1; echo late > late.txt) & sleep 0"}),
                execution(dir.path()),
            )
            .await;
        let elapsed = started.elapsed();
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().expect("data");
        assert_eq!(data["exit_code"], 0, "{data}");
        assert!(
            elapsed < Duration::from_millis(900),
            "run must not wait for the descendant's pipe: {elapsed:?}"
        );
        let id = data["id"].as_str().unwrap().to_string();
        assert!(
            jobs.readers_open(&id),
            "the descendant must still hold the pipe: {data}"
        );
        assert!(
            data["output"].as_str().unwrap().contains("still draining"),
            "{data}"
        );
        assert!(
            jobs.running("test-session").is_empty(),
            "an exited shell must not stay in the running list: {data}"
        );
        assert!(!dir.path().join("late.txt").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_returns_when_the_process_exited_even_if_the_pipe_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let started = tool
            .execute(
                serde_json::json!({"command": "(sleep 1; echo late > late.txt) & sleep 0"}),
                execution(dir.path()),
            )
            .await;
        let data = started.data.as_ref().expect("data");
        let id = data["id"].as_str().unwrap().to_string();

        let waiter = crate::tools::wait::WaitTool::new(Arc::clone(&jobs));
        let began = Instant::now();
        let result = waiter
            .execute(
                serde_json::json!({"id": id, "sec": 5}),
                test_execution(dir.path(), "test-session"),
            )
            .await;
        assert!(
            began.elapsed() < Duration::from_millis(900),
            "wait must not block on a pipe the process already left"
        );
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["exit_code"], 0, "{data}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_separates_stdout_from_stderr_and_names_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _jobs) = tool();
        let result = tool
            .execute(
                serde_json::json!({"command": "echo to-out; echo to-err 1>&2"}),
                execution(dir.path()),
            )
            .await;
        let data = result.data.as_ref().expect("data");
        assert_eq!(data["exit_code"], 0, "{data}");
        assert_eq!(data["output"].as_str().unwrap().trim(), "to-out", "{data}");
        assert_eq!(data["stderr"].as_str().unwrap().trim(), "to-err", "{data}");
        assert!(
            !data["output"].as_str().unwrap().contains("[stderr]"),
            "{data}"
        );
        assert!(!data["shell"].as_str().unwrap().is_empty(), "{data}");
        assert_eq!(
            crate::terminal::decode::msys_path_hint("cmd /c dir").is_some(),
            true
        );
        assert!(crate::terminal::decode::msys_path_hint("ls -la src/a").is_none());
    }
}
