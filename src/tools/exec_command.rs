//! Codex-compatible unified exec command.
//!
//! `exec_command` is deliberately not a second name for `run`: it starts a
//! session, waits only for `yield_time_ms`, and returns either a final exit or
//! a live session id that `write_stdin` can drive. The process registry owns
//! the stdin, process-group signal, output cursor and cleanup.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::terminal::{
    AgentJobRegistry, PipeDecision, PipeDetach, PipeRequest, PtyRequest, supervise_pipe,
    supervise_pty,
};
use crate::tool::Tool;
use crate::tool::clamp::clamp_int;
use crate::tool::outcome::ToolOutcome;
use crate::tool::resource_gate::ToolEffects;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::bash_safety::{is_destructive_command, is_readonly_command};
use crate::tools::command_job;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

pub const DEFAULT_YIELD_MS: u64 = 10_000;
pub const MIN_YIELD_MS: u64 = 250;
pub const MAX_YIELD_MS: u64 = 30_000;
pub const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub const MAX_OUTPUT_TOKENS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecCommandData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub wall_time_seconds: f64,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_token_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
}

impl ExecCommandData {
    pub fn from_delta(
        session_id: Option<String>,
        exit_code: Option<i32>,
        wall_time_seconds: f64,
        delta: crate::terminal::OutputDelta,
    ) -> Self {
        let mut output = delta.text;
        if delta.truncated {
            output = format!("... (truncated)\n{output}");
        }
        Self {
            session_id,
            exit_code,
            wall_time_seconds,
            output,
            original_token_count: Some(delta.original_bytes / 4),
            truncated: delta.truncated.then_some(true),
        }
    }
}

#[derive(Debug, Clone)]
struct ParsedArgs {
    cmd: String,
    workdir: Option<String>,
    tty: bool,
    yield_ms: u64,
    max_output_tokens: usize,
}

pub struct ExecCommandTool {
    jobs: Arc<AgentJobRegistry>,
}

impl ExecCommandTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    fn parse_args(input: &Value) -> Result<ParsedArgs, String> {
        let cmd = crate::tool::require_nonempty_string_trimmed(input, "cmd")?.to_string();
        let workdir = match input.get("workdir") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.clone()),
            Some(Value::String(_)) => None,
            Some(_) => return Err("parameter 'workdir' expected a string".into()),
        };
        let tty = match input.get("tty") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err("parameter 'tty' expected a boolean".into()),
        };
        let yield_ms = clamp_int(input, "yield_time_ms", MIN_YIELD_MS, MAX_YIELD_MS, "ms")?
            .map_or(DEFAULT_YIELD_MS, |value| value.value);
        let max_output_tokens = clamp_int(
            input,
            "max_output_tokens",
            1,
            MAX_OUTPUT_TOKENS as u64,
            "tokens",
        )?
        .map_or(DEFAULT_MAX_OUTPUT_TOKENS, |value| value.value as usize);
        Ok(ParsedArgs {
            cmd,
            workdir,
            tty,
            yield_ms,
            max_output_tokens,
        })
    }

    fn resolve_workdir(raw: Option<&str>, root: &Path) -> Result<PathBuf, String> {
        let Some(raw) = raw else {
            return Ok(root.to_path_buf());
        };
        let requested = Path::new(raw);
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            root.join(requested)
        };
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let resolved = joined.canonicalize().unwrap_or(joined);
        if !resolved.starts_with(&root) {
            return Err("workdir escapes the workspace root".into());
        }
        Ok(resolved)
    }

    fn output_max_bytes(max_output_tokens: usize) -> usize {
        max_output_tokens.saturating_mul(4).max(1024)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let args = match Self::parse_args(&input) {
            Ok(args) => args,
            Err(error) => return coded_error("invalid_input", error),
        };
        let cwd = match Self::resolve_workdir(args.workdir.as_deref(), &execution.workspace_root) {
            Ok(cwd) => cwd,
            Err(error) => return coded_error("invalid_input", error),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "exec_command cancelled before spawn");
        }
        tracing::info!(
            tool = "exec_command",
            session_id = %execution.session_id,
            tty = args.tty,
            yield_ms = args.yield_ms,
            max_output_tokens = args.max_output_tokens,
            "unified exec start"
        );
        let started = Instant::now();
        let scope = crate::tool::run_scope::current().unwrap_or_default();
        let (spawned_tx, spawned_rx) = tokio::sync::oneshot::channel();
        let rx = if args.tty {
            supervise_pty(
                Arc::clone(&self.jobs),
                PtyRequest {
                    command: args.cmd,
                    cwd,
                    workspace_root: execution.workspace_root.clone(),
                    session_id: execution.session_id.clone(),
                    call_id: execution.call_id.clone(),
                    yield_window: Duration::from_millis(args.yield_ms),
                    cancel: execution.cancel.clone(),
                    on_spawned: Some(spawned_tx),
                },
            )
        } else {
            supervise_pipe(
                Arc::clone(&self.jobs),
                PipeRequest {
                    command: args.cmd,
                    cwd,
                    workspace_root: execution.workspace_root.clone(),
                    session_id: execution.session_id.clone(),
                    call_id: execution.call_id.clone(),
                    cancel: execution.cancel.clone(),
                    detach: PipeDetach::Background {
                        window: Duration::from_millis(args.yield_ms),
                    },
                    open_stdin: true,
                    merge_output: true,
                    on_spawned: Some(spawned_tx),
                },
            )
        };
        let _ = spawned_rx.await;
        scope.spawn_locks.release();
        let decision = match rx.await {
            Ok(decision) => decision,
            Err(_) => return coded_error("io", "exec supervisor dropped the result"),
        };
        let max_bytes = Self::output_max_bytes(args.max_output_tokens);
        match decision {
            PipeDecision::Exited { job_id, exit_code } => {
                self.jobs
                    .wait_readers(&job_id, command_job::READER_GRACE)
                    .await;
                let delta = self
                    .jobs
                    .output_delta(&job_id, max_bytes)
                    .unwrap_or_else(|| crate::terminal::OutputDelta {
                        text: String::new(),
                        original_bytes: 0,
                        alive: false,
                        exit_code: None,
                        truncated: false,
                    });
                self.jobs.discard_finished(&job_id);
                let data = ExecCommandData::from_delta(
                    None,
                    exit_code,
                    started.elapsed().as_secs_f64(),
                    delta,
                );
                ToolOutcome::ok(data).into_call_result(format!(
                    "exit_code: {}\n",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "null".into())
                ))
            }
            PipeDecision::Background { job_id } | PipeDecision::Promoted { job_id } => {
                let delta = self
                    .jobs
                    .output_delta(&job_id, max_bytes)
                    .unwrap_or_else(|| crate::terminal::OutputDelta {
                        text: String::new(),
                        original_bytes: 0,
                        alive: true,
                        exit_code: None,
                        truncated: false,
                    });
                let data = ExecCommandData::from_delta(
                    Some(job_id.clone()),
                    None,
                    started.elapsed().as_secs_f64(),
                    delta,
                );
                tracing::info!(
                    tool = "exec_command",
                    session_id = %job_id,
                    tty = args.tty,
                    yield_ms = args.yield_ms,
                    "unified exec yielded"
                );
                ToolOutcome::ok(data).into_call_result(format!("session_id: {job_id}\n"))
            }
            PipeDecision::Cancelled { .. } => coded_error("cancelled", "exec_command cancelled"),
            PipeDecision::Failed { message } => coded_error("io", message),
        }
    }
}

impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
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
            .get("cmd")
            .and_then(Value::as_str)
            .is_some_and(is_readonly_command)
    }

    fn effects(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &Path,
    ) -> ToolEffects {
        let Some(command) = input.get("cmd").and_then(Value::as_str) else {
            return ToolEffects::write(ResourceKey::Workspace);
        };
        if is_readonly_command(command) {
            ToolEffects::default()
        } else {
            ToolEffects::write(ResourceKey::Workspace)
        }
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &Path,
    ) -> bool {
        input
            .get("cmd")
            .and_then(Value::as_str)
            .is_some_and(is_destructive_command)
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        Self::parse_args(input).map(|_| ())
    }

    fn max_result_size(&self) -> usize {
        200_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::command_job::test_execution;
    use crate::tools::write_stdin::WriteStdinTool;

    fn data(result: &ToolCallResult) -> ExecCommandData {
        serde_json::from_value(result.data.clone().expect("data")).expect("exec data")
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_yields_then_ctrl_c_runs_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let stdin = WriteStdinTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s1");

        let started = exec
            .execute(
                serde_json::json!({
                    "cmd": "trap 'echo CLEANED; exit 0' INT; while :; do sleep 1; done",
                    "yield_time_ms": 250
                }),
                execution.clone(),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let started = data(&started);
        let session_id = started.session_id.expect("session id");
        assert!(started.exit_code.is_none());

        let stopped = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "chars": "\u{0003}",
                    "yield_time_ms": 3000
                }),
                execution,
            )
            .await;
        assert!(stopped.error.is_none(), "{stopped:?}");
        let stopped = data(&stopped);
        assert!(stopped.output.contains("CLEANED"), "{stopped:?}");
        assert!(stopped.session_id.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_tty_yields_then_ctrl_c_runs_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let stdin = WriteStdinTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s2");

        let started = exec
            .execute(
                serde_json::json!({
                    "cmd": "trap 'echo PTY_CLEANED; exit 0' INT; while :; do sleep 1; done",
                    "tty": true,
                    "yield_time_ms": 250
                }),
                execution.clone(),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let started = data(&started);
        let session_id = started.session_id.expect("session id");

        let stopped = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "chars": "\u{0003}",
                    "yield_time_ms": 3000
                }),
                execution,
            )
            .await;
        assert!(stopped.error.is_none(), "{stopped:?}");
        let stopped = data(&stopped);
        assert!(stopped.output.contains("PTY_CLEANED"), "{stopped:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_stdin_empty_chars_polls_new_output() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let stdin = WriteStdinTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s4");

        let started = exec
            .execute(
                serde_json::json!({
                    "cmd": "printf first; sleep 0.3; printf second; sleep 30",
                    "yield_time_ms": 250
                }),
                execution.clone(),
            )
            .await;
        let started = data(&started);
        let session_id = started.session_id.expect("session id");
        let polled = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "yield_time_ms": 2000
                }),
                execution.clone(),
            )
            .await;
        assert!(polled.error.is_none(), "{polled:?}");
        assert!(data(&polled).output.contains("second"));
        let _ = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "chars": "\u{0003}",
                    "yield_time_ms": 1000
                }),
                execution,
            )
            .await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_honours_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s5");

        let result = exec
            .execute(
                serde_json::json!({
                    "cmd": "pwd",
                    "workdir": "sub",
                    "yield_time_ms": 250
                }),
                execution,
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        assert!(data(&result).output.contains("sub"), "{result:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_truncates_to_output_budget() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s6");

        let result = exec
            .execute(
                serde_json::json!({
                    "cmd": "printf '%02000d' 0",
                    "max_output_tokens": 1,
                    "yield_time_ms": 250
                }),
                execution,
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let result = data(&result);
        assert_eq!(result.truncated, Some(true), "{result:?}");
        assert!(result.output.len() < 1500, "{}", result.output.len());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_stdin_feeds_a_live_pipe_session() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let stdin = WriteStdinTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s3");

        let started = exec
            .execute(
                serde_json::json!({"cmd": "cat", "yield_time_ms": 250}),
                execution.clone(),
            )
            .await;
        let started = data(&started);
        let session_id = started.session_id.expect("session id");
        let echoed = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "chars": "hello from stdin\n",
                    "yield_time_ms": 1000
                }),
                execution.clone(),
            )
            .await;
        assert!(echoed.error.is_none(), "{echoed:?}");
        assert!(data(&echoed).output.contains("hello from stdin"));

        let _ = stdin
            .execute(
                serde_json::json!({
                    "session_id": session_id,
                    "chars": "\u{0003}",
                    "yield_time_ms": 1000
                }),
                execution,
            )
            .await;
    }
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    let full = ExecCommandData {
        session_id: Some("bg_00000001".into()),
        exit_code: Some(0),
        wall_time_seconds: 0.5,
        output: "hello\n".into(),
        original_token_count: Some(2),
        truncated: Some(true),
    };
    let bare = ExecCommandData {
        session_id: None,
        exit_code: None,
        wall_time_seconds: 0.0,
        output: String::new(),
        original_token_count: None,
        truncated: None,
    };
    ["exec_command", "write_stdin"]
        .into_iter()
        .map(|tool| crate::tool::code_contract::DriftCase {
            tool,
            all_optional_present: ToolOutcome::ok(full.clone()).into_call_result("full"),
            all_optional_absent: ToolOutcome::ok(bare.clone()).into_call_result("bare"),
        })
        .collect()
}
