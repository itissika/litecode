//! Code-surface command runner (`run_command`).
//!
//! The process is a pipe job on [`AgentJobRegistry`]. This tool waits for one
//! supervisor decision. It does not spawn a second child, and it does not set
//! `detached` after that decision arrives. A foreground timeout promotes a
//! still-running process. Cancellation before a job id is returned kills the tree.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::{
    AgentJobRegistry, INLINE_FULL, PipeDecision, PipeDetach, PipeRequest, TeeCapture,
    display_output_path, supervise_pipe,
};
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::resource_gate::ToolEffects;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::bash_safety::{is_destructive_command, is_readonly_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Default foreground wait (seconds). Only the wait is capped; a promoted or
/// background process has no lifetime limit from this field.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Hard cap for `timeout_sec`. Kept at the default `run_code` budget (120s) so
/// one foreground await cannot cover the whole program.
pub const MAX_TIMEOUT_SECS: u64 = 120;

pub struct RunCommandTool {
    jobs: Arc<AgentJobRegistry>,
}

impl RunCommandTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }
}

#[derive(Debug, Serialize)]
struct StreamView {
    head: String,
    tail: String,
    bytes: u64,
    truncated: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RunCommandData {
    Exited {
        exit_code: Option<i32>,
        stdout: StreamView,
        stderr: StreamView,
        #[serde(skip_serializing_if = "Option::is_none")]
        output_file: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr_file: Option<String>,
    },
    Background {
        job_id: String,
        stdout: StreamView,
        stderr: StreamView,
        output_file: String,
        stderr_file: String,
    },
    Promoted {
        job_id: String,
        stdout: StreamView,
        stderr: StreamView,
        output_file: String,
        stderr_file: String,
    },
}

fn stream_view(capture: &TeeCapture) -> StreamView {
    StreamView {
        head: capture.head.clone(),
        tail: capture.tail.clone(),
        bytes: capture.total_bytes as u64,
        truncated: capture.frozen || capture.truncated_on_disk,
    }
}

fn stream_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "head": { "type": "string" },
            "tail": { "type": "string" },
            "bytes": { "type": "integer", "minimum": 0 },
            "truncated": { "type": "boolean" }
        },
        "required": ["head", "tail", "bytes", "truncated"]
    })
}

impl RunCommandTool {
    fn parse_background(input: &Value) -> Result<bool, String> {
        match input.get("run_in_background").filter(|v| !v.is_null()) {
            None => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(v) => Err(crate::tool::expected_type(
                "run_in_background",
                "boolean",
                v,
            )),
        }
    }

    fn parse_timeout_sec(input: &Value) -> Result<u64, String> {
        match input.get("timeout_sec").filter(|v| !v.is_null()) {
            None => Ok(DEFAULT_TIMEOUT_SECS),
            Some(v) => {
                let n = v
                    .as_u64()
                    .ok_or_else(|| crate::tool::expected_type("timeout_sec", "integer", v))?;
                if !(1..=MAX_TIMEOUT_SECS).contains(&n) {
                    return Err(crate::tool::must_be(
                        "timeout_sec",
                        &format!("between 1 and {MAX_TIMEOUT_SECS}"),
                    ));
                }
                Ok(n)
            }
        }
    }

    fn exited_result(
        &self,
        workspace_root: &Path,
        job_id: &str,
        exit_code: Option<i32>,
    ) -> ToolCallResult {
        let Some((stdout_cap, stderr_cap, stdout_path, stderr_path)) =
            self.jobs.pipe_streams(job_id)
        else {
            return coded_error("io", "run_command lost the exited job");
        };
        let stdout = stream_view(&stdout_cap);
        let stderr = stream_view(&stderr_cap);
        let need_file = stdout.truncated
            || stderr.truncated
            || stdout.bytes > INLINE_FULL as u64
            || stderr.bytes > INLINE_FULL as u64;
        let (output_file, stderr_file) = if need_file {
            (
                Some(display_output_path(&stdout_path, workspace_root)),
                Some(display_output_path(&stderr_path, workspace_root)),
            )
        } else if self.jobs.discard_attached(job_id) {
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
            (None, None)
        } else {
            (None, None)
        };
        let shown = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "null".into());
        ToolOutcome::ok(RunCommandData::Exited {
            exit_code,
            stdout,
            stderr,
            output_file,
            stderr_file,
        })
        .into_call_result(format!("kind: exited\nexit_code: {shown}\n"))
    }

    fn detached_result(
        &self,
        workspace_root: &Path,
        job_id: &str,
        promoted: bool,
    ) -> ToolCallResult {
        let Some((stdout_cap, stderr_cap, stdout_path, stderr_path)) =
            self.jobs.pipe_streams(job_id)
        else {
            return coded_error("io", "run_command lost the detached job");
        };
        let stdout = stream_view(&stdout_cap);
        let stderr = stream_view(&stderr_cap);
        let output_file = display_output_path(&stdout_path, workspace_root);
        let stderr_file = display_output_path(&stderr_path, workspace_root);
        let kind = if promoted { "promoted" } else { "background" };
        let data = if promoted {
            RunCommandData::Promoted {
                job_id: job_id.to_string(),
                stdout,
                stderr,
                output_file,
                stderr_file,
            }
        } else {
            RunCommandData::Background {
                job_id: job_id.to_string(),
                stdout,
                stderr,
                output_file,
                stderr_file,
            }
        };
        ToolOutcome::ok(data).into_call_result(format!("kind: {kind}\njob_id: {job_id}\n"))
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

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let command = match crate::tool::require_nonempty_string_trimmed(&input, "command") {
            Ok(c) => c.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        let background = match Self::parse_background(&input) {
            Ok(value) => value,
            Err(e) => return coded_error("invalid_input", e),
        };
        let detach = if background {
            PipeDetach::Background
        } else {
            let timeout_sec = match Self::parse_timeout_sec(&input) {
                Ok(n) => n,
                Err(e) => return coded_error("invalid_input", e),
            };
            PipeDetach::Foreground {
                timeout: Duration::from_secs(timeout_sec),
            }
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "run_command cancelled before spawn");
        }

        let rx = supervise_pipe(
            Arc::clone(&self.jobs),
            PipeRequest {
                command,
                cwd: execution.workspace_root.clone(),
                workspace_root: execution.workspace_root.clone(),
                session_id: execution.session_id.clone(),
                call_id: execution.call_id.clone(),
                cancel: execution.cancel.clone(),
                detach,
            },
        );
        let decision = match rx.await {
            Ok(decision) => decision,
            Err(_) => return coded_error("io", "run_command supervisor dropped the result"),
        };
        match decision {
            PipeDecision::Exited { job_id, exit_code } => {
                self.exited_result(&execution.workspace_root, &job_id, exit_code)
            }
            PipeDecision::Background { job_id } => {
                self.detached_result(&execution.workspace_root, &job_id, false)
            }
            PipeDecision::Promoted { job_id } => {
                self.detached_result(&execution.workspace_root, &job_id, true)
            }
            PipeDecision::Cancelled { job_id } => {
                self.discard_cancelled(&job_id);
                coded_error("cancelled", "run_command cancelled")
            }
            PipeDecision::Failed { message } => coded_error("io", message),
        }
    }
}

impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
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
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Default false. When true, timeout_sec is ignored. If the process is still running, detach and return kind background with job_id. If it has already exited, return kind exited and no job_id.",
                    "default": false
                },
                "timeout_sec": {
                    "type": "integer",
                    "description": format!(
                        "Foreground wait in seconds (1-{MAX_TIMEOUT_SECS}, default {DEFAULT_TIMEOUT_SECS}). Ignored when run_in_background is true. When it elapses and the process is still running, the call returns kind promoted and leaves the process running."
                    ),
                    "minimum": 1,
                    "maximum": MAX_TIMEOUT_SECS,
                    "default": DEFAULT_TIMEOUT_SECS
                }
            },
            "required": ["command"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        let streams = stream_schema();
        Some(serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "The process had already exited when the call decided. Non-zero exit_code still resolves. No job_id.",
                    "properties": {
                        "kind": { "const": "exited" },
                        "exit_code": {
                            "type": ["integer", "null"],
                            "description": "Process exit code. null when the status has no code."
                        },
                        "stdout": streams.clone(),
                        "stderr": streams.clone(),
                        "output_file": {
                            "type": "string",
                            "description": "Workspace-relative stdout log. Omitted when both streams are small."
                        },
                        "stderr_file": {
                            "type": "string",
                            "description": "Workspace-relative stderr log. Omitted when both streams are small."
                        }
                    },
                    "required": ["kind", "exit_code", "stdout", "stderr"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "run_in_background was set and the process was still running after detach.",
                    "properties": {
                        "kind": { "const": "background" },
                        "job_id": { "type": "string" },
                        "stdout": streams.clone(),
                        "stderr": streams.clone(),
                        "output_file": { "type": "string" },
                        "stderr_file": { "type": "string" }
                    },
                    "required": ["kind", "job_id", "stdout", "stderr", "output_file", "stderr_file"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "The foreground wait elapsed and the process was still running after detach.",
                    "properties": {
                        "kind": { "const": "promoted" },
                        "job_id": { "type": "string" },
                        "stdout": streams.clone(),
                        "stderr": streams,
                        "output_file": { "type": "string" },
                        "stderr_file": { "type": "string" }
                    },
                    "required": ["kind", "job_id", "stdout", "stderr", "output_file", "stderr_file"]
                }
            ]
        }))
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/run_command.md"))
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
        let background = Self::parse_background(input)?;
        if !background {
            Self::parse_timeout_sec(input)?;
        }
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio_util::sync::CancellationToken;

    fn tool() -> (RunCommandTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (RunCommandTool::new(Arc::clone(&jobs)), jobs)
    }

    fn execution(root: &Path) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: root.to_path_buf(),
            call_id: format!("test-{}", uuid_like()),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: "test-session".into(),
            session: None,
        }
    }

    fn uuid_like() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{nanos:x}")
    }

    fn pid_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        #[cfg(unix)]
        {
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            };
            unsafe {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if handle.is_null() {
                    return false;
                }
                CloseHandle(handle);
                true
            }
        }
    }

    fn marker_sleep(marker: &Path, seconds: u64) -> String {
        let path = marker.display().to_string().replace('\\', "/");
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                format!("echo start > '{path}'; sleep {seconds}; echo done >> '{path}'")
            } else {
                format!(
                    "Set-Content -LiteralPath '{path}' -Value start; Start-Sleep -Seconds {seconds}; Add-Content -LiteralPath '{path}' -Value done"
                )
            }
        }
        #[cfg(not(windows))]
        {
            format!("echo start > '{path}'; sleep {seconds}; echo done >> '{path}'")
        }
    }

    fn marker_then_blob(marker: &Path) -> String {
        let path = marker.display().to_string().replace('\\', "/");
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                format!(
                    "echo start > '{path}'; sleep 2; head -c 262144 /dev/zero; echo done >> '{path}'"
                )
            } else {
                format!(
                    "Set-Content -LiteralPath '{path}' -Value start; Start-Sleep -Seconds 2; Write-Output -NoNewline ('x' * 262144); Add-Content -LiteralPath '{path}' -Value done"
                )
            }
        }
        #[cfg(not(windows))]
        {
            format!(
                "echo start > '{path}'; sleep 2; head -c 262144 /dev/zero; echo done >> '{path}'"
            )
        }
    }

    fn sleep_secs(seconds: u64) -> String {
        #[cfg(windows)]
        {
            if crate::config::git_install::find_git_bash().is_some() {
                format!("sleep {seconds}")
            } else {
                format!("Start-Sleep -Seconds {seconds}")
            }
        }
        #[cfg(not(windows))]
        {
            format!("sleep {seconds}")
        }
    }

    fn assert_log_names(data: &Value, call_id: &str) {
        for key in ["output_file", "stderr_file"] {
            if let Some(path) = data.get(key).and_then(Value::as_str) {
                let name = path.rsplit('/').next().unwrap_or(path);
                assert!(!name.contains(':'), "{path}");
                assert!(!name.contains(call_id), "{path}");
                assert!(name.starts_with("bg_"), "{path}");
            }
        }
    }

    async fn wait_until(mut ready: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if ready() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        ready()
    }

    #[test]
    fn surface_is_code_only() {
        let (tool, _) = tool();
        assert_eq!(tool.surface(), crate::tool::ToolSurface::Code);
    }

    #[test]
    fn data_schema_is_three_kinds() {
        let (tool, _) = tool();
        let schema = tool.data_schema().expect("schema");
        let variants = schema["oneOf"].as_array().expect("oneOf");
        let kinds: Vec<&str> = variants
            .iter()
            .map(|variant| variant["properties"]["kind"]["const"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["exited", "background", "promoted"]);
        assert!(
            schema
                .get("properties")
                .and_then(|p| p.get("status"))
                .is_none()
        );
        let rendered = crate::tool::code_manifest::render_code_tool_description(&[Arc::new(
            RunCommandTool::new(Arc::new(AgentJobRegistry::new())),
        )])
        .expect("schema renders");
        assert!(rendered.contains("kind: \"exited\""));
        assert!(rendered.contains("kind: \"background\""));
        assert!(rendered.contains("kind: \"promoted\""));
        assert!(!rendered.contains("status: \"exited\""));
    }

    #[test]
    fn validate_timeout_bounds() {
        let (tool, _) = tool();
        assert!(
            tool.validate_input(&serde_json::json!({"command": "true"}))
                .is_ok()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"command": "true", "timeout_sec": 0}))
                .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"command": "true", "timeout_sec": 121}))
                .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"command": "true", "timeout_sec": 600}))
                .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"command": "true", "timeout_sec": 120}))
                .is_ok()
        );
        assert!(
            tool.validate_input(&serde_json::json!({
                "command": "true",
                "run_in_background": true,
                "timeout_sec": 0
            }))
            .is_ok()
        );
        assert!(
            tool.validate_input(&serde_json::json!({
                "command": "true",
                "run_in_background": "yes"
            }))
            .is_err()
        );
    }

    #[test]
    fn safe_background_cargo_build_is_denied_before_spawn() {
        use crate::config::schema::ToolPreset;
        use crate::permission::{MatchContext, PermissionAction, binding_for_tool, evaluate};

        let (policy, path_mode) = binding_for_tool("run_command", ToolPreset::Safe);
        let ctx = MatchContext {
            workspace_root: Path::new("."),
            path_mode,
        };
        let denied = evaluate(
            &policy,
            &serde_json::json!({
                "command": "cargo build",
                "run_in_background": true
            }),
            &ctx,
        );
        assert_eq!(denied.action, PermissionAction::Deny);
        let allowed = evaluate(
            &policy,
            &serde_json::json!({
                "command": "cargo check",
                "run_in_background": true
            }),
            &ctx,
        );
        assert_eq!(allowed.action, PermissionAction::Allow);
        assert!(!is_readonly_command("cargo build"));
        assert!(is_readonly_command("cargo check"));
    }

    #[test]
    fn readonly_commands_are_concurrency_safe_including_cargo_prefixes() {
        use crate::config::schema::ToolPreset;
        use crate::permission::{MatchContext, PermissionAction, binding_for_tool, evaluate};

        let (tool, _) = tool();
        let (policy, path_mode) = binding_for_tool("run_command", ToolPreset::Safe);
        let ctx = MatchContext {
            workspace_root: Path::new("."),
            path_mode,
        };
        for command in [
            "ls",
            "cargo check -q",
            "cargo test -q --lib",
            "cargo clippy -q",
        ] {
            let input = serde_json::json!({ "command": command });
            assert!(is_readonly_command(command), "{command}");
            assert!(tool.is_concurrency_safe(&input), "{command}");
            assert_eq!(
                evaluate(&policy, &input, &ctx).action,
                PermissionAction::Allow,
                "{command}"
            );
        }
        let build = serde_json::json!({ "command": "cargo build" });
        assert!(!tool.is_concurrency_safe(&build));
        assert_eq!(
            evaluate(&policy, &build, &ctx).action,
            PermissionAction::Deny
        );
    }

    #[test]
    fn effects_workspace_write_for_mutating_commands() {
        let (tool, _) = tool();
        let root = Path::new("/tmp");
        let mode = crate::workspace::ToolPathMode::All;
        let write = tool.effects(&serde_json::json!({"command": "rm -rf ./out"}), mode, root);
        assert_eq!(write.writes, vec![ResourceKey::Workspace]);
        let read = tool.effects(&serde_json::json!({"command": "ls"}), mode, root);
        assert!(!read.declares());
        let background_write = tool.effects(
            &serde_json::json!({"command": "cargo build", "run_in_background": true}),
            mode,
            root,
        );
        assert_eq!(background_write.writes, vec![ResourceKey::Workspace]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn success_data_and_nonzero_exit_still_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let ok = tool
            .run(
                serde_json::json!({"command": "echo hello_rc"}),
                execution(dir.path()),
            )
            .await;
        assert!(ok.error.is_none(), "zero exit should succeed: {:?}", ok);
        let data = ok.data.expect("data");
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 0);
        assert!(data.get("job_id").is_none(), "{data}");
        assert!(
            data["stdout"]["head"]
                .as_str()
                .unwrap_or("")
                .contains("hello_rc"),
            "stdout head={:?}",
            data["stdout"]["head"]
        );
        assert!(!jobs.mailbox_pending("test-session"));
        assert!(!jobs.session_has_job("test-session"));

        let fail = tool
            .run(
                serde_json::json!({"command": "sh -c 'echo nope >&2; exit 7'"}),
                execution(dir.path()),
            )
            .await;
        assert!(
            fail.error.is_none(),
            "non-zero exit must still resolve ok: {:?}",
            fail
        );
        let data = fail.data.expect("data");
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 7);
        assert!(data.get("job_id").is_none(), "{data}");
        assert!(
            data["stderr"]["head"]
                .as_str()
                .unwrap_or("")
                .contains("nope"),
            "stderr={:?}",
            data["stderr"]["head"]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn colon_in_call_id_does_not_break_log_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, _) = tool();
        let mut exec = execution(dir.path());
        exec.call_id = "call_abc:code:0".into();
        let result = tool
            .run(serde_json::json!({"command": "echo hello_rc"}), exec)
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.expect("data");
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 0);
        assert!(
            data["stdout"]["head"]
                .as_str()
                .unwrap_or("")
                .contains("hello_rc"),
            "stdout head={:?}",
            data["stdout"]["head"]
        );
        assert_log_names(&data, "call_abc:code:0");
        let logs = dir.path().join(".litecode").join("run_command");
        if let Ok(entries) = std::fs::read_dir(logs) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                assert!(!name.contains(':'), "{name}");
                assert!(!name.contains("call_abc"), "{name}");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_echo_returns_exited_without_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let mut exec = execution(dir.path());
        exec.call_id = "parent:code:0".into();
        let result = tool
            .run(
                serde_json::json!({"command": "echo hello_rc", "run_in_background": true}),
                exec,
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.expect("data");
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 0);
        assert!(data.get("job_id").is_none(), "{data}");
        assert!(!jobs.mailbox_pending("test-session"));
        assert!(!jobs.session_has_job("test-session"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn background_live_process_survives_turn_cancel() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("bg_marker");
        let (tool, jobs) = tool();
        let turn = CancellationToken::new();
        let run_code = turn.child_token();
        let mut exec = execution(dir.path());
        exec.call_id = "parent:code:2".into();
        exec.session_id = "sess-bg".into();
        exec.cancel = run_code.child_token();
        let result = tool
            .run(
                serde_json::json!({
                    "command": marker_then_blob(&marker),
                    "run_in_background": true
                }),
                exec,
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.expect("data");
        assert_eq!(data["kind"], "background");
        let job_id = data["job_id"].as_str().expect("job_id").to_string();
        assert_eq!(job_id.len(), "bg_".len() + 8, "{job_id}");
        assert!(
            job_id["bg_".len()..].chars().all(|c| c.is_ascii_hexdigit()),
            "{job_id}"
        );
        assert!(!job_id.contains(':'));
        assert_log_names(&data, "parent:code:2");
        let stdout_rel = data["output_file"].as_str().expect("output_file");
        let stderr_rel = data["stderr_file"].as_str().expect("stderr_file");
        assert_eq!(stdout_rel, format!(".litecode/run_command/{job_id}.stdout"));
        assert_eq!(stderr_rel, format!(".litecode/run_command/{job_id}.stderr"));
        assert!(dir.path().join(stderr_rel).is_file(), "{stderr_rel}");
        assert!(jobs.is_detached(&job_id));
        let pid = jobs.job_pid(&job_id).expect("pid");
        assert!(
            pid_alive(pid),
            "process must be alive when background returns"
        );
        let stdout_path = dir.path().join(stdout_rel);
        let early = std::fs::read(&stdout_path).unwrap_or_default();
        assert!(
            early.len() < 4096,
            "blob must not have been written before detach, got {} bytes",
            early.len()
        );
        drop(run_code);
        turn.cancel();
        assert!(
            pid_alive(pid),
            "turn cancel after background must not kill the process"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            pid_alive(pid),
            "process died after the turn token was cancelled"
        );
        let mailed = wait_until(|| jobs.mailbox_pending("sess-bg"), Duration::from_secs(15)).await;
        assert!(
            mailed,
            "natural exit must reach the mailbox after turn cancel"
        );
        let late = std::fs::read(&stdout_path).unwrap_or_default();
        assert!(
            late.len() > 200_000,
            "stdout after turn cancel was only {} bytes",
            late.len()
        );
        let body = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(
            body.contains("done"),
            "process should finish after the blob: {body}"
        );
        let mail = jobs.take_mailbox("sess-bg");
        assert_eq!(mail.len(), 1);
        assert_eq!(mail[0].bash_id, job_id);
        assert!(!mail[0].user_killed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn timeout_promotes_live_process_and_mails_exit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let mut exec = execution(dir.path());
        exec.session_id = "sess-promote".into();
        exec.call_id = "parent:code:5".into();
        let result = tool
            .run(
                serde_json::json!({
                    "command": sleep_secs(5),
                    "timeout_sec": 1
                }),
                exec,
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.expect("data");
        assert_eq!(data["kind"], "promoted", "{data}");
        let job_id = data["job_id"].as_str().expect("job_id").to_string();
        assert_log_names(&data, "parent:code:5");
        assert!(jobs.is_detached(&job_id));
        let pid = jobs.job_pid(&job_id).expect("pid");
        assert!(pid_alive(pid), "promote must leave the process running");
        assert!(!jobs.mailbox_pending("sess-promote"));
        let mailed = wait_until(
            || jobs.mailbox_pending("sess-promote"),
            Duration::from_secs(12),
        )
        .await;
        assert!(mailed, "promoted exit must reach the mailbox");
        let mail = jobs.take_mailbox("sess-promote");
        assert_eq!(mail.len(), 1);
        assert!(!mail[0].user_killed);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fast_command_with_timeout_returns_exited() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let result = tool
            .run(
                serde_json::json!({"command": "echo hello_rc", "timeout_sec": 1}),
                execution(dir.path()),
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.expect("data");
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 0);
        assert!(data.get("job_id").is_none(), "{data}");
        assert!(!jobs.mailbox_pending("test-session"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_before_return_kills_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("child_alive");
        let (tool, jobs) = tool();
        let turn = CancellationToken::new();
        let run_code = turn.child_token();
        let mut exec = execution(dir.path());
        exec.session_id = "sess-cancel".into();
        exec.cancel = run_code.child_token();
        let command = marker_sleep(&marker, 60);
        let handle = tokio::spawn(async move {
            tool.run(
                serde_json::json!({"command": command, "timeout_sec": 60}),
                exec,
            )
            .await
        });
        assert!(
            wait_until(|| marker.exists(), Duration::from_secs(5)).await,
            "child should have started"
        );
        run_code.cancel();
        let result = handle.await.expect("join");
        assert!(result.error.is_some(), "cancel must error: {:?}", result);
        assert!(
            result
                .error
                .as_ref()
                .map(|e| e.message.contains("cancelled"))
                .unwrap_or(false),
            "expected cancelled error: {:?}",
            result.error
        );
        assert!(!jobs.session_has_job("sess-cancel"));
        assert!(!jobs.mailbox_pending("sess-cancel"));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let body = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(
            !body.contains("done"),
            "process tree should have been killed before finishing: {body}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_yields_so_budget_watcher_can_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = Arc::clone(&flag);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            flag2.store(true, Ordering::SeqCst);
        });
        let (tool, _) = tool();
        let result = tool
            .run(
                serde_json::json!({"command": "sleep 0.25", "timeout_sec": 5}),
                execution(dir.path()),
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        assert_eq!(
            result.data.as_ref().and_then(|d| d.get("kind")),
            Some(&serde_json::json!("exited"))
        );
        assert!(
            flag.load(Ordering::SeqCst),
            "sibling task must progress during async wait (budget watcher analogue)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn uses_workspace_root_as_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, _) = tool();
        let result = tool
            .run(
                serde_json::json!({"command": "echo here > cwd_marker"}),
                execution(dir.path()),
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let data = result.data.unwrap();
        assert_eq!(data["kind"], "exited");
        let marker = dir.path().join("cwd_marker");
        assert!(
            marker.is_file(),
            "command must run in the workspace root, marker missing"
        );
        let body = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(body.contains("here"), "{body}");
    }
}
