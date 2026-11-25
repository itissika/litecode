//! Shell command execution (`exec_command`).
//!
//! Runs a non-interactive shell command (`stdin` is closed via `/dev/null` and
//! non-interactive env vars are set so accidental prompts fail fast with EOF
//! while still returning all captured stdout/stderr).
//!
//! - Default (`background: false`): waits up to `timeout_ms` (default 60s,
//!   clamped to `1_000..=300_000`) for exit. If still running at timeout,
//!   auto-promotes to background without killing the process.
//! - `background: true`: returns after a brief capture window (250ms) while
//!   the command continues in the background.
//!
//! Background commands deliver a `bash_exit` notice when they exit and keep
//! their log under `.litecode/run/`. Foreground commands that finish inline
//! return `{ exit_code, output }` directly; output overflow spilling is owned
//! solely by the outer `run_code` model view.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::terminal::{
    AgentJobRegistry, PipeDecision, PipeDetach, PipeRequest, supervise_pipe,
};
use crate::tool::Tool;
use crate::tool::clamp::clamp_int;
use crate::tool::outcome::ToolOutcome;
use crate::tool::resource_gate::ToolEffects;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::bash_safety::{is_destructive_command, is_readonly_command};
use crate::tools::command_job;
use crate::tools::file_path::{coded_error, tool_path_error};
use crate::types::ToolCallResult;
use crate::workspace::{ToolPathMode, resolve_tool_path_from};

pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;
pub const BACKGROUND_CAPTURE_MS: u64 = 250;
pub(crate) const CODE_DEFAULT_MAX_OUTPUT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecCommandData {
    pub exit_code: Option<i32>,
    pub output: String,
    /// Metadata-only fields moved out of `data` by `result_projection`:
    /// only populated when the command is running in the background or an arg
    /// was clamped. Never exposed to JS `exec_command` return objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_file: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clamped: Vec<crate::tool::clamp::Clamp>,
}

#[derive(Debug, Clone)]
struct ParsedArgs {
    cmd: String,
    workdir: Option<String>,
    timeout_ms: u64,
    background: bool,
    clamped: Vec<crate::tool::clamp::Clamp>,
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
        let background = match input.get("background") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err("parameter 'background' expected a boolean".into()),
        };
        let mut clamped = Vec::new();
        let timeout_value =
            clamp_int(input, "timeout_ms", MIN_TIMEOUT_MS, MAX_TIMEOUT_MS, "ms")?;
        if let Some(note) = timeout_value
            .as_ref()
            .and_then(|value| value.as_clamp("timeout_ms"))
        {
            clamped.push(note);
        }
        let timeout_ms = timeout_value.map_or(DEFAULT_TIMEOUT_MS, |value| value.value);
        Ok(ParsedArgs {
            cmd,
            workdir,
            timeout_ms,
            background,
            clamped,
        })
    }

    fn resolve_workdir(
        raw: Option<&str>,
        root: &Path,
        mode: ToolPathMode,
    ) -> Result<PathBuf, ToolCallResult> {
        let Some(raw) = raw else {
            return Ok(root.to_path_buf());
        };
        let resolved = resolve_tool_path_from(root, raw, mode).map_err(tool_path_error)?;
        if !resolved.exists() {
            return Err(coded_error(
                "not_found",
                format!("workdir '{raw}' does not exist"),
            ));
        }
        if !resolved.is_dir() {
            return Err(coded_error(
                "invalid_input",
                format!("workdir '{raw}' is not a directory"),
            ));
        }
        Ok(resolved)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let args = match Self::parse_args(&input) {
            Ok(args) => args,
            Err(error) => return coded_error("invalid_input", error),
        };
        let cwd = match Self::resolve_workdir(
            args.workdir.as_deref(),
            &execution.workspace_root,
            execution.path_mode,
        ) {
            Ok(cwd) => cwd,
            Err(error) => return error,
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "exec_command cancelled before spawn");
        }
        tracing::info!(
            tool = "exec_command",
            session_id = %execution.session_id,
            background = args.background,
            timeout_ms = args.timeout_ms,
            "exec_command start"
        );
        let scope = crate::tool::run_scope::current().unwrap_or_default();
        let (spawned_tx, spawned_rx) = tokio::sync::oneshot::channel();
        let detach = if args.background {
            PipeDetach::Background {
                window: Duration::from_millis(BACKGROUND_CAPTURE_MS),
            }
        } else {
            PipeDetach::Foreground {
                timeout: Duration::from_millis(args.timeout_ms),
            }
        };
        let rx = supervise_pipe(
            Arc::clone(&self.jobs),
            PipeRequest {
                command: args.cmd,
                cwd,
                workspace_root: execution.workspace_root.clone(),
                session_id: execution.session_id.clone(),
                call_id: execution.call_id.clone(),
                cancel: execution.cancel.clone(),
                detach,
                merge_output: true,
                on_spawned: Some(spawned_tx),
            },
        );
        let _ = spawned_rx.await;
        scope.spawn_locks.release();
        let decision = match rx.await {
            Ok(decision) => decision,
            Err(_) => return coded_error("io", "exec supervisor dropped the result"),
        };
        match decision {
            PipeDecision::Exited { job_id, exit_code } => {
                self.jobs
                    .wait_readers(&job_id, command_job::READER_GRACE)
                    .await;
                let tee_path = self.jobs.get(&job_id).map(|(_, _, _, path)| path);
                let output = self
                    .jobs
                    .output_delta(&job_id, CODE_DEFAULT_MAX_OUTPUT_BYTES)
                    .map(|delta| delta.text)
                    .unwrap_or_default();
                self.jobs.discard_finished(&job_id);
                if let Some(path) = tee_path {
                    let _ = std::fs::remove_file(path);
                }
                let data = ExecCommandData {
                    exit_code,
                    output,
                    session_id: None,
                    output_file: None,
                    clamped: args.clamped,
                };
                ToolOutcome::ok(data).into_call_result(format!(
                    "exit_code: {}\n",
                    exit_code
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "null".into())
                ))
            }
            PipeDecision::Background { job_id } | PipeDecision::Promoted { job_id } => {
                let delta = self.jobs.output_delta(&job_id, CODE_DEFAULT_MAX_OUTPUT_BYTES);
                let (output, output_file) = match delta {
                    Some(delta) => (
                        delta.text,
                        (!delta.output_file.is_empty()).then_some(delta.output_file),
                    ),
                    None => (String::new(), None),
                };
                let data = ExecCommandData {
                    exit_code: None,
                    output,
                    session_id: Some(job_id.clone()),
                    output_file,
                    clamped: args.clamped,
                };
                tracing::info!(
                    tool = "exec_command",
                    session_id = %job_id,
                    background = args.background,
                    timeout_ms = args.timeout_ms,
                    "exec_command backgrounded"
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
    #[cfg(unix)]
    use crate::tools::command_job::test_execution;

    #[cfg(unix)]
    fn data(result: &ToolCallResult) -> ExecCommandData {
        serde_json::from_value(result.data.clone().expect("data")).expect("exec data")
    }

    #[test]
    fn defaults_timeout_to_sixty_seconds_and_clamps_bounds() {
        let parsed = ExecCommandTool::parse_args(&serde_json::json!({ "cmd": "true" }))
            .expect("parse default");
        assert_eq!(parsed.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(parsed.timeout_ms, 60_000);
        assert!(!parsed.background);
        assert!(parsed.clamped.is_empty());

        let low = ExecCommandTool::parse_args(&serde_json::json!({
            "cmd": "true",
            "timeout_ms": 100
        }))
        .expect("clamp low");
        assert_eq!(low.timeout_ms, MIN_TIMEOUT_MS);
        assert_eq!(low.clamped.len(), 1);

        let high = ExecCommandTool::parse_args(&serde_json::json!({
            "cmd": "true",
            "timeout_ms": 999_999
        }))
        .expect("clamp high");
        assert_eq!(high.timeout_ms, MAX_TIMEOUT_MS);
        assert_eq!(high.clamped.len(), 1);
    }

    #[test]
    fn resolve_workdir_respects_safe_vs_all_path_mode() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sub = root.path().join("sub");
        std::fs::create_dir(&sub).unwrap();

        let in_ws = ExecCommandTool::resolve_workdir(Some("sub"), root.path(), ToolPathMode::Safe)
            .expect("subdir allowed in SAFE");
        assert!(in_ws.ends_with("sub"));

        let outside_raw = outside.path().to_string_lossy().into_owned();
        let denied =
            ExecCommandTool::resolve_workdir(Some(&outside_raw), root.path(), ToolPathMode::Safe)
                .expect_err("external workdir rejected in SAFE");
        assert_eq!(denied.error_code.as_deref(), Some("invalid_input"));

        let allowed =
            ExecCommandTool::resolve_workdir(Some(&outside_raw), root.path(), ToolPathMode::All)
                .expect("external workdir allowed in ALL");
        assert_eq!(
            allowed.canonicalize().unwrap(),
            outside.path().canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_returns_exit_code_and_output_without_session_or_log_leak() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s1");

        let ok = exec
            .execute(serde_json::json!({ "cmd": "printf 'hello\\n'" }), execution.clone())
            .await;
        assert!(ok.error.is_none(), "{ok:?}");
        let ok_data = data(&ok);
        assert_eq!(ok_data.exit_code, Some(0));
        assert_eq!(ok_data.output, "hello\n");
        assert!(ok_data.session_id.is_none());
        assert!(ok_data.output_file.is_none());

        let fail = exec
            .execute(
                serde_json::json!({ "cmd": "printf 'boom\\n' >&2; exit 7" }),
                execution,
            )
            .await;
        assert!(fail.error.is_none(), "{fail:?}");
        let fail_data = data(&fail);
        assert_eq!(fail_data.exit_code, Some(7));
        assert_eq!(fail_data.output, "boom\n");

        // Foreground temporary tee files must be cleaned up.
        let run_dir = dir.path().join(".litecode").join("run");
        if run_dir.exists() {
            assert_eq!(std::fs::read_dir(&run_dir).unwrap().count(), 0);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_is_closed_so_interactive_prompts_fail_fast_with_output() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s_eof");

        let result = exec
            .execute(
                serde_json::json!({
                    "cmd": "printf 'Prompt: '; read line || exit 1",
                    "timeout_ms": 5000
                }),
                execution,
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let d = data(&result);
        assert_eq!(d.exit_code, Some(1), "read on closed stdin must hit EOF");
        assert!(d.output.contains("Prompt:"), "prompt output preserved: {d:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn background_true_detaches_and_delivers_exit_notice() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s_bg");

        let started = exec
            .execute(
                serde_json::json!({
                    "cmd": "printf 'booting\\n'; sleep 0.5; printf 'done\\n'",
                    "background": true
                }),
                execution,
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let started = data(&started);
        assert!(started.exit_code.is_none(), "{started:?}");
        let session_id = started.session_id.expect("background session_id");
        assert!(started.output_file.is_some(), "background log path expected");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !jobs.mailbox_pending("s_bg") && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let notices = jobs.take_mailbox("s_bg");
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].bash_id, session_id);
        assert_eq!(notices[0].exit_code, Some(0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn foreground_timeout_auto_promotes_to_background_without_killing() {
        let dir = tempfile::tempdir().unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s_promo");

        let promoted = exec
            .execute(
                serde_json::json!({
                    "cmd": "printf 'step1\\n'; sleep 1.4; printf 'step2\\n'",
                    "timeout_ms": 1000
                }),
                execution,
            )
            .await;
        assert!(promoted.error.is_none(), "{promoted:?}");
        let promoted = data(&promoted);
        assert!(promoted.exit_code.is_none(), "{promoted:?}");
        let session_id = promoted.session_id.expect("promoted session_id");
        assert!(promoted.output.contains("step1"), "{promoted:?}");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !jobs.mailbox_pending("s_promo") && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let notices = jobs.take_mailbox("s_promo");
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].bash_id, session_id);
        assert_eq!(notices[0].exit_code, Some(0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_command_honours_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let jobs = Arc::new(AgentJobRegistry::new());
        let exec = ExecCommandTool::new(Arc::clone(&jobs));
        let execution = test_execution(dir.path(), "s_wd");

        let result = exec
            .execute(
                serde_json::json!({
                    "cmd": "pwd",
                    "workdir": "sub"
                }),
                execution,
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        assert!(data(&result).output.contains("sub"), "{result:?}");
    }
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    let full = ExecCommandData {
        exit_code: Some(0),
        output: "hello\n".into(),
        session_id: Some("bg_00000001".into()),
        output_file: Some(".litecode/run/bg_00000001.stdout".into()),
        clamped: vec![crate::tool::clamp::Clamp {
            parameter: "timeout_ms".into(),
            requested: serde_json::json!(900000),
            applied: serde_json::json!(300000),
        }],
    };
    let bare = ExecCommandData {
        exit_code: None,
        output: String::new(),
        session_id: None,
        output_file: None,
        clamped: Vec::new(),
    };
    vec![crate::tool::code_contract::DriftCase {
        tool: "exec_command",
        all_optional_present: ToolOutcome::ok(full).into_call_result("full"),
        all_optional_absent: ToolOutcome::ok(bare).into_call_result("bare"),
    }]
}
