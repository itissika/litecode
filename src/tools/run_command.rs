//! Code-surface foreground command runner (`run_command`).
//!
//! Distinct from Direct `bash`: structured `data`, async wait (does not block the
//! current-thread runtime), kill-on-end / kill-on-cancel, no background job /
//! `bash_id`. cwd is always the execution context `workspace_root`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::terminal::{INLINE_FULL, INLINE_HEAD, INLINE_TAIL, FILE_MAX, shell_command};
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::resource_gate::ToolEffects;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::bash_safety::{is_destructive_command, is_readonly_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Default per-call wait before kill (seconds). Aligned with former bash
/// foreground feel, but this tool **kills** instead of backgrounding.
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Hard cap for `timeout_sec`. Kept at/under typical `run_code` default budget
/// (120s) so one call cannot monopolize the whole program by itself.
pub const MAX_TIMEOUT_SECS: u64 = 120;

const OUTPUT_SUBDIR: &str = ".litecode/run_command";

#[derive(Default)]
pub struct RunCommandTool;

#[derive(Debug, Serialize)]
struct StreamView {
    head: String,
    tail: String,
    bytes: u64,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct RunCommandData {
    status: &'static str,
    exit_code: Option<i32>,
    stdout: StreamView,
    stderr: StreamView,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_file: Option<String>,
}

struct BoundedBuf {
    total: usize,
    truncated_on_disk: bool,
    frozen: bool,
    buf: String,
    head: String,
    tail: String,
    file: Option<std::fs::File>,
    file_written: usize,
}

impl BoundedBuf {
    fn new(file: Option<std::fs::File>) -> Self {
        Self {
            total: 0,
            truncated_on_disk: false,
            frozen: false,
            buf: String::new(),
            head: String::new(),
            tail: String::new(),
            file,
            file_written: 0,
        }
    }

    fn push_bytes(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.total = self.total.saturating_add(chunk.len());
        if let Some(file) = self.file.as_mut() {
            if self.file_written < FILE_MAX {
                let room = FILE_MAX - self.file_written;
                let write_n = chunk.len().min(room);
                let _ = std::io::Write::write_all(file, &chunk[..write_n]);
                self.file_written += write_n;
                if chunk.len() > write_n {
                    self.truncated_on_disk = true;
                }
            } else {
                self.truncated_on_disk = true;
            }
        }
        let text = String::from_utf8_lossy(chunk);
        if self.frozen {
            self.tail.push_str(&text);
            if self.tail.len() > INLINE_TAIL * 2 {
                let keep = crate::terminal::strip_ansi(&self.tail);
                self.tail = tail_window(&keep, INLINE_TAIL);
            }
            return;
        }
        self.buf.push_str(&text);
        if self.buf.len() > INLINE_FULL {
            let cleaned = crate::terminal::strip_ansi(&self.buf);
            let (head, rest) = split_at_bytes(&cleaned, INLINE_HEAD);
            self.head = head;
            self.tail = tail_window(rest, INLINE_TAIL);
            self.buf.clear();
            self.frozen = true;
        }
    }

    fn finish(mut self) -> StreamView {
        if !self.frozen {
            let cleaned = crate::terminal::strip_ansi(&self.buf);
            self.head = cleaned;
            self.tail.clear();
        } else {
            self.head = crate::terminal::strip_ansi(&self.head);
            self.tail = tail_window(&crate::terminal::strip_ansi(&self.tail), INLINE_TAIL);
        }
        let truncated = self.frozen || self.truncated_on_disk;
        StreamView {
            head: self.head,
            tail: self.tail,
            bytes: self.total as u64,
            truncated,
        }
    }
}

fn split_at_bytes(s: &str, n: usize) -> (String, &str) {
    if s.len() <= n {
        return (s.to_string(), "");
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), &s[end..])
}

fn tail_window(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut start = s.len() - n;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    s[start..].to_string()
}

fn output_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(OUTPUT_SUBDIR)
}

fn relative_output_path(workspace_root: &Path, abs: &Path) -> String {
    abs.strip_prefix(workspace_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| abs.display().to_string().replace('\\', "/"))
}

fn kill_process_group(child: &mut tokio::process::Child) {
    let Some(pid) = child.id() else {
        let _ = child.start_kill();
        return;
    };
    #[cfg(unix)]
    {
        let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.start_kill();
}

async fn drain_pipe<R: AsyncRead + Unpin>(
    mut reader: R,
    mut buf: BoundedBuf,
    cancel: CancellationToken,
) -> BoundedBuf {
    let mut chunk = vec![0u8; 8192];
    loop {
        if cancel.is_cancelled() {
            break;
        }
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => buf.push_bytes(&chunk[..n]),
            Err(_) => break,
        }
    }
    buf
}

impl RunCommandTool {
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

    async fn run(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let command = match crate::tool::require_nonempty_string_trimmed(&input, "command") {
            Ok(c) => c.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        let timeout_sec = match Self::parse_timeout_sec(&input) {
            Ok(n) => n,
            Err(e) => return coded_error("invalid_input", e),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "run_command cancelled before spawn");
        }

        let workspace_root = execution.workspace_root.clone();
        let call_id = if execution.call_id.is_empty() {
            format!("rc-{}", uuid_like())
        } else {
            execution.call_id.clone()
        };

        let out_dir = output_dir(&workspace_root);
        if let Err(e) = std::fs::create_dir_all(&out_dir) {
            return coded_error("io", format!("failed to create output dir: {e}"));
        }
        let stdout_path = out_dir.join(format!("{call_id}.stdout"));
        let stderr_path = out_dir.join(format!("{call_id}.stderr"));
        let stdout_file = match std::fs::File::create(&stdout_path) {
            Ok(f) => f,
            Err(e) => return coded_error("io", format!("failed to create stdout log: {e}")),
        };
        let stderr_file = match std::fs::File::create(&stderr_path) {
            Ok(f) => f,
            Err(e) => return coded_error("io", format!("failed to create stderr log: {e}")),
        };

        let spec = shell_command(&command);
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .current_dir(&workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_TERMINAL_PROMPT", "0");
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return coded_error("io", format!("failed to spawn run_command: {e}"));
            }
        };
        crate::proc_lifetime::bind_child_to_parent(child.id().unwrap_or(0));

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let pipe_cancel = CancellationToken::new();
        let stdout_task = {
            let cancel = pipe_cancel.clone();
            let buf = BoundedBuf::new(Some(stdout_file));
            tokio::spawn(async move {
                match stdout {
                    Some(r) => drain_pipe(r, buf, cancel).await,
                    None => buf,
                }
            })
        };
        let stderr_task = {
            let cancel = pipe_cancel.clone();
            let buf = BoundedBuf::new(Some(stderr_file));
            tokio::spawn(async move {
                match stderr {
                    Some(r) => drain_pipe(r, buf, cancel).await,
                    None => buf,
                }
            })
        };

        let timeout = Duration::from_secs(timeout_sec);
        let outcome = tokio::select! {
            biased;
            _ = execution.cancel.cancelled() => WaitEnd::Cancelled,
            _ = tokio::time::sleep(timeout) => WaitEnd::TimedOut,
            status = child.wait() => match status {
                Ok(s) => WaitEnd::Exited(s),
                Err(e) => WaitEnd::WaitFailed(e.to_string()),
            },
        };

        match outcome {
            WaitEnd::Exited(status) => {
                pipe_cancel.cancel();
                let stdout_view = stdout_task.await.unwrap_or_else(|_| BoundedBuf::new(None)).finish();
                let stderr_view = stderr_task.await.unwrap_or_else(|_| BoundedBuf::new(None)).finish();
                let exit_code = status.code();
                let need_file = stdout_view.truncated
                    || stderr_view.truncated
                    || stdout_view.bytes > INLINE_FULL as u64
                    || stderr_view.bytes > INLINE_FULL as u64;
                // Prefer a single combined pointer when either stream spilled.
                let output_file = if need_file {
                    // Point agents at the larger / primary log dir entry (stdout).
                    Some(relative_output_path(&workspace_root, &stdout_path))
                } else {
                    // Small result: drop the temp logs to avoid litter.
                    let _ = std::fs::remove_file(&stdout_path);
                    let _ = std::fs::remove_file(&stderr_path);
                    None
                };
                let data = RunCommandData {
                    status: "exited",
                    exit_code,
                    stdout: stdout_view,
                    stderr: stderr_view,
                    output_file,
                };
                let content = format!(
                    "status: exited\nexit_code: {}\n",
                    exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "null".into())
                );
                ToolOutcome::ok(data).into_call_result(content)
            }
            WaitEnd::Cancelled => {
                kill_process_group(&mut child);
                let _ = child.wait().await;
                pipe_cancel.cancel();
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                let _ = std::fs::remove_file(&stdout_path);
                let _ = std::fs::remove_file(&stderr_path);
                coded_error("cancelled", "run_command cancelled")
            }
            WaitEnd::TimedOut => {
                kill_process_group(&mut child);
                let _ = child.wait().await;
                pipe_cancel.cancel();
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                // Keep logs on timeout so the agent can inspect via tools.read.
                let rel = relative_output_path(&workspace_root, &stdout_path);
                ToolOutcome::<Value>::failure(
                    "timeout",
                    format!(
                        "run_command timed out after {timeout_sec}s (process group killed). \
                         Partial log: {rel}"
                    ),
                    true,
                )
                .with_error_details(serde_json::json!({
                    "timeout_sec": timeout_sec,
                    "output_file": rel,
                }))
                .into_call_result(format!(
                    "run_command timed out after {timeout_sec}s"
                ))
            }
            WaitEnd::WaitFailed(msg) => {
                kill_process_group(&mut child);
                let _ = child.wait().await;
                pipe_cancel.cancel();
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                coded_error("io", format!("run_command wait failed: {msg}"))
            }
        }
    }
}

enum WaitEnd {
    Exited(std::process::ExitStatus),
    Cancelled,
    TimedOut,
    WaitFailed(String),
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
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
                "timeout_sec": {
                    "type": "integer",
                    "description": format!(
                        "Seconds to wait for exit before killing the process group (1-{MAX_TIMEOUT_SECS}, default {DEFAULT_TIMEOUT_SECS}). On timeout the call rejects ToolError timeout; it does not background."
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
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "description": "Foreground command result. Non-zero exit_code is still a successful resolve; cancellation and timeout reject ToolError.",
            "properties": {
                "status": {
                    "type": "string",
                    "const": "exited",
                    "description": "Always exited on success. Running/background is not part of this tool."
                },
                "exit_code": {
                    "type": ["integer", "null"],
                    "description": "Process exit code. null when terminated by signal without a code."
                },
                "stdout": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "head": { "type": "string" },
                        "tail": { "type": "string" },
                        "bytes": { "type": "integer", "minimum": 0 },
                        "truncated": { "type": "boolean" }
                    },
                    "required": ["head", "tail", "bytes", "truncated"]
                },
                "stderr": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "head": { "type": "string" },
                        "tail": { "type": "string" },
                        "bytes": { "type": "integer", "minimum": 0 },
                        "truncated": { "type": "boolean" }
                    },
                    "required": ["head", "tail", "bytes", "truncated"]
                },
                "output_file": {
                    "type": "string",
                    "description": "Workspace-relative path to the stdout log when output was truncated or large. Use tools.read to page. Omitted for small results."
                }
            },
            "required": ["status", "exit_code", "stdout", "stderr"]
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
        // Enforcement is the input timeout_sec select; no second wrapper clock.
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
            // Minimal Q5: may write anywhere under the workspace.
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
        Self::parse_timeout_sec(input)?;
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

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

    #[test]
    fn surface_is_code_only() {
        assert_eq!(RunCommandTool.surface(), crate::tool::ToolSurface::Code);
    }

    #[test]
    fn data_schema_is_nonempty_object() {
        let schema = RunCommandTool.data_schema().expect("schema");
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("status").is_some());
        assert!(schema["properties"].get("exit_code").is_some());
    }

    #[test]
    fn validate_timeout_bounds() {
        let tool = RunCommandTool;
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
            tool.validate_input(&serde_json::json!({"command": "true", "timeout_sec": 120}))
                .is_ok()
        );
    }

    #[test]
    fn effects_workspace_write_for_mutating_commands() {
        let tool = RunCommandTool;
        let root = Path::new("/tmp");
        let mode = crate::workspace::ToolPathMode::All;
        let write = tool.effects(
            &serde_json::json!({"command": "rm -rf ./out"}),
            mode,
            root,
        );
        assert_eq!(write.writes, vec![ResourceKey::Workspace]);
        let read = tool.effects(&serde_json::json!({"command": "ls"}), mode, root);
        assert!(!read.declares());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn success_data_and_nonzero_exit_still_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = RunCommandTool;
        let ok = tool
            .run(
                serde_json::json!({"command": "echo hello_rc"}),
                execution(dir.path()),
            )
            .await;
        assert!(ok.error.is_none(), "zero exit should succeed: {:?}", ok);
        let data = ok.data.expect("data");
        assert_eq!(data["status"], "exited");
        assert_eq!(data["exit_code"], 0);
        assert!(
            data["stdout"]["head"].as_str().unwrap_or("").contains("hello_rc"),
            "stdout head={:?}",
            data["stdout"]["head"]
        );

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
        assert_eq!(data["status"], "exited");
        assert_eq!(data["exit_code"], 7);
        assert!(
            data["stderr"]["head"].as_str().unwrap_or("").contains("nope"),
            "stderr={:?}",
            data["stderr"]["head"]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_kills_process_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("child_alive");
        let script = format!(
            "sh -c 'echo start > {0}; sleep 60; echo done >> {0}'",
            marker.display()
        );
        let mut exec = execution(dir.path());
        let cancel = exec.cancel.clone();
        let tool = RunCommandTool;
        let handle = tokio::spawn(async move {
            tool.run(serde_json::json!({"command": script, "timeout_sec": 60}), exec)
                .await
        });
        // Wait until the child has started.
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(marker.exists(), "child should have started");
        cancel.cancel();
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
        // Give the group a moment to die; the marker file must not gain "done".
        tokio::time::sleep(Duration::from_millis(200)).await;
        let body = std::fs::read_to_string(&marker).unwrap_or_default();
        assert!(
            !body.contains("done"),
            "process group should have been killed before finishing: {body}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_yields_so_budget_watcher_can_run() {
        // Proof that waiting is async: a sibling task on the same current-thread
        // runtime must be able to run while run_command waits on a short sleep.
        let dir = tempfile::tempdir().expect("tempdir");
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = Arc::clone(&flag);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            flag2.store(true, Ordering::SeqCst);
        });
        let tool = RunCommandTool;
        let result = tool
            .run(
                serde_json::json!({"command": "sleep 0.25", "timeout_sec": 5}),
                execution(dir.path()),
            )
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        assert!(
            flag.load(Ordering::SeqCst),
            "sibling task must progress during async wait (budget watcher analogue)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn uses_workspace_root_as_cwd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool = RunCommandTool;
        let result = tool
            .run(serde_json::json!({"command": "pwd"}), execution(dir.path()))
            .await;
        assert!(result.error.is_none(), "{:?}", result);
        let head = result.data.unwrap()["stdout"]["head"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        let expected = dir
            .path()
            .canonicalize()
            .unwrap_or_else(|_| dir.path().to_path_buf());
        let got = PathBuf::from(&head)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&head));
        assert_eq!(got, expected, "cwd head={head}");
    }
}
