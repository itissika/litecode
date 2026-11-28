//! Custom tool — user-defined CLI tool projected into `run_code`.
//!
//! Exit 0 stdout:
//! - JSON object or array is passed through to JS (if the object contains
//!   `media` and/or `level`, those control attachments / warning-or-error
//!   signaling).
//! - Any other stdout is wrapped as `{ text: stdout }`.
//!
//! Non-zero exit is always a pipeline error (catchable JS `Error`). If stdout
//! holds `{ error: { code, message?, details?, retryable? } }`, that code and
//! message are preserved on the JS error.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::schema::CustomToolDefinition;
use crate::tool::Tool;
use crate::tool::custom_spec::{custom_description, custom_input_schema};
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{
    MediaKind, MediaSource, ToolCallResult, ToolErrorKind, ToolOutputPart, ToolSignalLevel,
};

/// Exit code a custom tool uses to signal it declined execution.
const CUSTOM_BLOCKED_EXIT_CODE: i32 = 2;

const ENV_WORKSPACE: &str = "LITECODE_WORKSPACE";
const ENV_CALL_ID: &str = "LITECODE_CALL_ID";
const ENV_SESSION_ID: &str = "LITECODE_SESSION_ID";
const ENV_TOOL_NAME: &str = "LITECODE_TOOL_NAME";

pub struct CustomTool {
    config: CustomToolDefinition,
}

impl CustomTool {
    pub fn new(config: CustomToolDefinition) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &CustomToolDefinition {
        &self.config
    }
}

/// Code-surface failure contract: a non-zero exit may carry a structured
/// error object on stdout. Anything else keeps the message-only shape.
fn structured_failure(stdout: &str, stderr: &str) -> Option<ToolCallResult> {
    let Value::Object(envelope) = serde_json::from_str::<Value>(stdout).ok()? else {
        return None;
    };
    let Value::Object(error) = envelope.get("error")? else {
        return None;
    };
    let code = error.get("code").and_then(Value::as_str)?.trim();
    if code.is_empty() {
        return None;
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| stderr.trim().to_string());
    let retryable = error
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut result = ToolCallResult::error_with_kind(ToolErrorKind::Internal, message, retryable);
    result.error_code = Some(code.to_string());
    result.error_details = error.get("details").cloned();
    Some(result)
}

/// Validate a custom tool definition for `run_code` projection.
pub fn validate_custom_code_contract(
    def: &CustomToolDefinition,
) -> std::result::Result<(), String> {
    crate::tool::dynamic_contract::validate_custom_code_contract(def)
}

impl Tool for CustomTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn dynamic_code_spec(&self) -> Option<crate::tool::DynamicCodeSpec> {
        Some(crate::tool::DynamicCodeSpec {
            description: custom_description(&self.config),
            input_schema: custom_input_schema(&self.config),
            output_schema: None,
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let config = self.config.clone();
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || {
                run_sync_with_lifecycle(&config, &input, &execution)
            });
            match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("custom tool task join failed: {e}")),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let execution = ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            call_id: String::new(),
            cancel: CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        };
        run_sync_with_lifecycle(&self.config, &input, &execution)
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        // Self-enforced inside run_sync_with_lifecycle so cancel/kill stay coupled.
        None
    }
}

/// Validate a definition + sample input without registering the tool.
pub fn validate_custom_tool_run(
    def: &CustomToolDefinition,
    sample_input: &Value,
    workspace_root: &Path,
) -> ToolCallResult {
    if let Err(error) = validate_custom_code_contract(def) {
        return ToolCallResult::error(format!(
            "# validate_custom — FAIL\n\n- tool: `{}`\n- detail: {error}\n",
            def.name
        ));
    }

    let execution = ToolExecutionContext {
        path_mode: crate::workspace::ToolPathMode::All,
        workspace_root: workspace_root.to_path_buf(),
        call_id: "validate".into(),
        cancel: CancellationToken::new(),
        output_limit: 8_000,
        session_id: "validate".into(),
        session: None,
    };
    let result = run_sync_with_lifecycle(def, sample_input, &execution);
    let rules_note = if def.rules.is_empty() {
        String::new()
    } else {
        format!(
            "- note: definition has {} rule(s); validate_custom does not evaluate rules — PASS is not a rules check\n",
            def.rules.len()
        )
    };
    let preview = result
        .data
        .as_ref()
        .map(|data| serde_json::to_string_pretty(data).unwrap_or_default())
        .unwrap_or_else(|| result.content.clone());
    match result.level {
        ToolSignalLevel::Error => ToolCallResult::error(format!(
            "# validate_custom — FAIL\n\n- tool: `{}`\n- command: `{}`\n{rules_note}- detail:\n\n{}",
            def.name, def.command, result.content
        )),
        _ => ToolCallResult::ok(format!(
            "# validate_custom — PASS\n\n- tool: `{}`\n- command: `{}`\n{rules_note}- program preview:\n\n{preview}",
            def.name, def.command
        )),
    }
}
fn run_sync_with_lifecycle(
    config: &CustomToolDefinition,
    input: &Value,
    execution: &ToolExecutionContext,
) -> ToolCallResult {
    let input_json = match serde_json::to_string(input) {
        Ok(s) => s,
        Err(e) => return ToolCallResult::error(e.to_string()),
    };

    if execution.cancel.is_cancelled() {
        return ToolCallResult::error(format!(
            "custom tool '{}' cancelled before spawn",
            config.name
        ));
    }

    let mut cmd = StdCommand::new(&config.command);
    cmd.args(&config.args)
        .current_dir(&execution.workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(ENV_WORKSPACE, execution.workspace_root.as_os_str())
        .env(ENV_CALL_ID, &execution.call_id)
        .env(ENV_SESSION_ID, &execution.session_id)
        .env(ENV_TOOL_NAME, &config.name);
    // Own process group so kill_child_tree can reach grandchildren
    // (e.g. `sh -c "a | b"`), matching terminal/pty.rs group kill.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ToolCallResult::error(format!(
                "failed to spawn custom tool '{}': {}",
                config.name, e
            ));
        }
    };
    // Windows Job Object (no-op on Unix): host crash must not leave the tool running.
    // Must run before the pid can be reused. See ISSUE-040.
    crate::proc_lifetime::bind_child_to_parent(child.id());

    // Feed stdin on its own thread so a child that writes a lot of output
    // before reading its input cannot deadlock against us (and so the
    // timeout/cancel loop below always runs). EPIPE is tolerated inside.
    let stdin_writer = child
        .stdin
        .take()
        .map(|stdin| spawn_stdin_writer(stdin, input_json.into_bytes()));

    let timeout = Duration::from_secs(config.timeout.max(1));
    match wait_child_cancellable(&mut child, &execution.cancel, timeout) {
        WaitEnd::Finished(output) => {
            // wait_child_cancellable already group-killed leftovers so pipes
            // close (ISSUE-039 X1/X2). Join the writer with a bound: after the
            // kill, inherited stdin ends yield EPIPE (X3); never hang here.
            let stdin_result = stdin_writer
                .map(|h| join_writer_bounded(h, Duration::from_secs(2)))
                .unwrap_or(Ok(()));
            if let Err(e) = stdin_result {
                return ToolCallResult::error(format!("stdin write failed: {e}"));
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            match output.status.code() {
                Some(0) => result_from_stdout(stdout.into_owned()),
                Some(code) => match structured_failure(&stdout, &stderr) {
                    Some(result) => result,
                    None if code == CUSTOM_BLOCKED_EXIT_CODE => ToolCallResult::error(format!(
                        "custom tool '{}' blocked execution: {}",
                        config.name,
                        stderr.trim()
                    )),
                    None => ToolCallResult::error(format!(
                        "custom tool '{}' exited with code {}: {}",
                        config.name,
                        code,
                        stderr.trim()
                    )),
                },
                None => structured_failure(&stdout, &stderr).unwrap_or_else(|| {
                    ToolCallResult::error(format!(
                        "custom tool '{}' terminated by signal: {}",
                        config.name,
                        stderr.trim()
                    ))
                }),
            }
        }
        WaitEnd::WaitFailed(e) => {
            ToolCallResult::error(format!("custom tool '{}' wait failed: {e}", config.name))
        }
        WaitEnd::Cancelled => {
            let _ = kill_child_tree(&mut child);
            let _ = child.wait();
            ToolCallResult::error(format!("custom tool '{}' cancelled", config.name))
        }
        WaitEnd::TimedOut => {
            let _ = kill_child_tree(&mut child);
            let _ = child.wait();
            ToolCallResult::error(format!(
                "custom tool '{}' timed out after {} seconds",
                config.name, config.timeout
            ))
        }
    }
}

/// Write the whole input to the child's stdin, then close it.
///
/// `BrokenPipe` means the child exited or closed stdin without reading all of
/// it; that is not a tool failure (exit status decides). Windows reports
/// ERROR_BROKEN_PIPE / ERROR_NO_DATA, which std maps to the same
/// `ErrorKind::BrokenPipe`. Any other error is returned.
fn spawn_stdin_writer(
    mut stdin: std::process::ChildStdin,
    input: Vec<u8>,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::spawn(move || {
        let r = stdin.write_all(&input);
        drop(stdin);
        match r {
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            other => other,
        }
    })
}

enum WaitEnd {
    Finished(std::process::Output),
    WaitFailed(String),
    Cancelled,
    TimedOut,
}

/// Join the stdin writer without hanging if a grandchild still held stdin
/// open past the child's exit (ISSUE-039 X3). After group-kill, EPIPE should
/// unblock it quickly; if not, abandon the join so the tool can return.
fn join_writer_bounded(
    handle: std::thread::JoinHandle<std::io::Result<()>>,
    limit: Duration,
) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = handle
            .join()
            .unwrap_or_else(|_| Err(std::io::Error::other("stdin writer panicked")));
        let _ = tx.send(result);
    });
    match rx.recv_timeout(limit) {
        Ok(result) => result,
        // Writer still blocked: group kill already happened; do not fail the tool.
        Err(_) => Ok(()),
    }
}

/// After the direct child exits, kill leftovers in the process group so any
/// grandchild holding stdout/stderr/stdin cannot stall the drain (X1/X2), then
/// wait for the reader with a deadline (never unbounded `recv`).
fn finish_after_child_exit(
    child: &mut Child,
    rx: &mpsc::Receiver<(Vec<u8>, Vec<u8>)>,
    status: std::process::ExitStatus,
    cancel: &CancellationToken,
    deadline: Instant,
    stderr_prefix: Option<String>,
) -> WaitEnd {
    let _ = kill_child_tree(child);
    let (out, err, terminal) = drain_output_bounded(rx, cancel, deadline);
    if let Some(end) = terminal {
        return end;
    }
    let stderr = match stderr_prefix {
        Some(prefix) if err.is_empty() => prefix.into_bytes(),
        _ => err,
    };
    WaitEnd::Finished(std::process::Output {
        status,
        stdout: out,
        stderr,
    })
}

fn drain_output_bounded(
    rx: &mpsc::Receiver<(Vec<u8>, Vec<u8>)>,
    cancel: &CancellationToken,
    deadline: Instant,
) -> (Vec<u8>, Vec<u8>, Option<WaitEnd>) {
    // After group-kill, pipes should EOF promptly. Keep a small grace past the
    // deadline so a just-killed grandchild can close fds before we give up.
    let grace_deadline = deadline + Duration::from_millis(500);
    loop {
        if cancel.is_cancelled() {
            return (Vec::new(), Vec::new(), Some(WaitEnd::Cancelled));
        }
        let now = Instant::now();
        if now >= grace_deadline {
            return (Vec::new(), Vec::new(), Some(WaitEnd::TimedOut));
        }
        let slice = (grace_deadline - now).min(Duration::from_millis(50));
        match rx.recv_timeout(slice) {
            Ok((out, err)) => return (out, err, None),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return (Vec::new(), Vec::new(), None);
            }
        }
    }
}

fn wait_child_cancellable(
    child: &mut Child,
    cancel: &CancellationToken,
    timeout: Duration,
) -> WaitEnd {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        // Drain stderr concurrently: reading the two pipes one after another
        // stalls a child that fills the stderr pipe before closing stdout.
        let err_reader = std::thread::spawn(move || {
            let mut err = Vec::new();
            if let Some(mut s) = stderr {
                let _ = s.read_to_end(&mut err);
            }
            err
        });
        let mut out = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut out);
        }
        let err = err_reader.join().unwrap_or_default();
        let _ = tx.send((out, err));
    });

    let deadline = Instant::now() + timeout;
    loop {
        if cancel.is_cancelled() {
            return WaitEnd::Cancelled;
        }
        if Instant::now() >= deadline {
            return WaitEnd::TimedOut;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                // ISSUE-039: never unbounded recv; group-kill then bounded drain.
                return finish_after_child_exit(child, &rx, status, cancel, deadline, None);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let status = match child.wait() {
                    Ok(status) => status,
                    // Never panic here: in a background worker that would lose
                    // the exit notice forever. Report the failure instead.
                    Err(_) => {
                        let _ = kill_child_tree(child);
                        return WaitEnd::WaitFailed(e.to_string());
                    }
                };
                return finish_after_child_exit(
                    child,
                    &rx,
                    status,
                    cancel,
                    deadline,
                    Some(format!("wait error: {e}")),
                );
            }
        }
    }
}

fn kill_child_tree(child: &mut Child) -> std::io::Result<()> {
    let pid = child.id();
    #[cfg(windows)]
    {
        let _ = StdCommand::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        Ok(())
    }
    #[cfg(unix)]
    {
        let _ = child;
        // Negative pid = the whole group created via process_group(0).
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        Ok(())
    }
    #[cfg(not(any(windows, unix)))]
    {
        child.kill()
    }
}

fn result_from_stdout(stdout: String) -> ToolCallResult {
    let parsed = serde_json::from_str::<Value>(&stdout);
    match parsed {
        Ok(Value::Object(map)) => {
            let level = match parse_envelope_level(map.get("level")) {
                Ok(level) => level,
                Err(msg) => return ToolCallResult::error(format!("custom tool signal: {msg}")),
            };
            if level == ToolSignalLevel::Error {
                let message = map
                    .get("message")
                    .or_else(|| map.get("content"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| stdout.clone());
                return ToolCallResult::error(message);
            }
            let parts = if map.contains_key("media") {
                match parse_media_parts(&map["media"]) {
                    Ok(parts) => parts,
                    Err(msg) => {
                        return ToolCallResult::error(format!("custom tool media output: {msg}"));
                    }
                }
            } else {
                Vec::new()
            };
            let mut result = if parts.is_empty() {
                match level {
                    ToolSignalLevel::Warning => ToolCallResult::warning(stdout),
                    _ => ToolCallResult::ok(stdout),
                }
            } else {
                ToolCallResult::ok_with_parts(stdout, parts)
            };
            result.level = level;
            result.data = Some(Value::Object(map));
            result
        }
        Ok(Value::Array(arr)) => ToolCallResult::ok(stdout).with_data(Value::Array(arr)),
        _ => {
            let text = stdout.clone();
            ToolCallResult::ok(stdout).with_data(serde_json::json!({ "text": text }))
        }
    }
}

fn parse_envelope_level(value: Option<&Value>) -> std::result::Result<ToolSignalLevel, String> {
    let Some(value) = value else {
        return Ok(ToolSignalLevel::Ok);
    };
    let Some(raw) = value.as_str() else {
        return Err("level must be a string (ok, warning, or error)".into());
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "ok" => Ok(ToolSignalLevel::Ok),
        "warning" => Ok(ToolSignalLevel::Warning),
        "error" => Ok(ToolSignalLevel::Error),
        other => Err(format!("unknown level '{other}'")),
    }
}

fn parse_media_parts(value: &Value) -> std::result::Result<Vec<ToolOutputPart>, String> {
    let arr = value.as_array().ok_or("expected a JSON array")?;
    let mut parts = Vec::with_capacity(arr.len());
    for (i, entry) in arr.iter().enumerate() {
        let mime = entry
            .get("mime_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if mime.is_empty() {
            return Err(format!("media[{i}]: mime_type is required"));
        }
        let source = if let Some(url) = entry
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            MediaSource::Url {
                url: url.trim().to_string(),
            }
        } else if let Some(path) = entry
            .get("file_path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            MediaSource::LocalFile {
                path: path.trim().to_string(),
            }
        } else {
            return Err(format!("media[{i}]: url or file_path is required"));
        };
        let kind = if mime.starts_with("image/") {
            MediaKind::Image
        } else if mime.starts_with("video/") {
            MediaKind::Video
        } else if mime.starts_with("audio/") {
            MediaKind::Audio
        } else {
            return Err(format!("media[{i}]: unsupported mime_type '{mime}'"));
        };
        parts.push(match kind {
            MediaKind::Image => ToolOutputPart::image(source, mime),
            MediaKind::Video => ToolOutputPart::video(source, mime),
            MediaKind::Audio => ToolOutputPart::audio(source, mime),
        });
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// WP-018: child that never reads stdin must not fail with Broken pipe.
    /// 1 MiB input exceeds the pipe buffer, so the write deterministically
    /// hits EPIPE once the child exits.
    #[cfg(unix)]
    #[test]
    fn child_ignoring_stdin_is_not_a_broken_pipe_failure() {
        use crate::tool::trait_::ToolExecutionContext;
        let def = CustomToolDefinition {
            name: "no_stdin".into(),
            description: "ignores stdin".into(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
            command: "sh".into(),
            args: vec!["-c".into(), "exec 0<&-; echo no-stdin-marker".into()],
            timeout: 15,
            rules: Vec::new(),
            suite: None,
        };
        let big = "x".repeat(1 << 20);
        let exec = ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::env::temp_dir(),
            call_id: "call-np".into(),
            cancel: CancellationToken::new(),
            output_limit: 8_000,
            session_id: "sess-np".into(),
            session: None,
        };
        let result = run_sync_with_lifecycle(&def, &serde_json::json!({ "pad": big }), &exec);
        assert!(
            !result.content.contains("Broken pipe"),
            "EPIPE must not surface: {}",
            result.content
        );
        assert!(
            result.content.contains("no-stdin-marker"),
            "{}",
            result.content
        );
    }

    fn sh_def(name: &str, script: &str, timeout: u64) -> CustomToolDefinition {
        CustomToolDefinition {
            name: name.into(),
            description: name.into(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            timeout,
            rules: Vec::new(),
            suite: None,
        }
    }

    #[cfg(unix)]
    fn test_exec(cancel: CancellationToken) -> crate::tool::trait_::ToolExecutionContext {
        crate::tool::trait_::ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::env::temp_dir(),
            call_id: "call-t".into(),
            cancel,
            output_limit: 8_000,
            session_id: "sess-t".into(),
            session: None,
        }
    }

    /// ISSUE-039 / X1: grandchild holds stdout after the shell exits — must not
    /// hang past the deadline, and the leftover sleep must be reaped (X2).
    #[cfg(unix)]
    #[test]
    fn finished_reaps_grandchild_holding_stdout() {
        let started = Instant::now();
        let def = sh_def("orphan-out", "sleep 30 & echo orphan-out-marker", 2);
        let result = run_sync_with_lifecycle(
            &def,
            &serde_json::json!({}),
            &test_exec(CancellationToken::new()),
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "hung waiting on grandchild pipe: {elapsed:?}"
        );
        assert!(
            result.content.contains("orphan-out-marker"),
            "expected tool stdout, got: {}",
            result.content
        );
        assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    }

    /// ISSUE-039 / X2 companion: grandchild scheduled to touch a file after we
    /// return must not get to run — group kill on Finished.
    #[cfg(unix)]
    #[test]
    fn finished_kills_orphaned_grandchild_before_side_effect() {
        let marker = std::env::temp_dir().join(format!("lc-fin-{}", ulid::Ulid::new()));
        let script = format!("(sleep 2; touch {}) & echo fin-marker", marker.display());
        let def = sh_def("fin-gc", &script, 10);
        let started = Instant::now();
        let result = run_sync_with_lifecycle(
            &def,
            &serde_json::json!({}),
            &test_exec(CancellationToken::new()),
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "should return as soon as the shell exits, not wait for sleep"
        );
        assert!(result.content.contains("fin-marker"), "{}", result.content);
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !marker.exists(),
            "grandchild survived Finished path (ISSUE-039 X2)"
        );
    }

    #[test]
    fn custom_tool_projects_to_code_with_promise_unknown() {
        let def = sh_def("code_echo", "echo code-marker", 10);
        let tool = CustomTool::new(def.clone());
        assert!(!tool.is_code_transport());
        let spec = tool.dynamic_code_spec().expect("code dynamic contract");
        assert!(spec.input_schema.get("properties").is_some());
        assert!(
            spec.input_schema["properties"]
                .get("run_in_background")
                .is_none(),
            "code surface must not advertise background jobs"
        );
        assert!(spec.output_schema.is_none());
        validate_custom_code_contract(&def).expect("simple object schema is code-admissible");

        let projected =
            crate::tool::project_tools(vec![Arc::new(CustomTool::new(def)) as Arc<dyn Tool>]);
        assert_eq!(projected.whitelist(), vec!["code_echo".to_string()]);
        assert!(projected.sdk().contains("code_echo"));
        assert!(projected.sdk().contains("Promise<unknown>"));
    }

    #[test]
    fn plain_text_stdout_wraps_as_text_object() {
        let result = result_from_stdout("hello world\n".into());
        assert_eq!(result.data, Some(serde_json::json!({ "text": "hello world\n" })));
    }

    #[test]
    fn json_object_stdout_passes_through_and_extracts_media() {
        let json = r#"{"files":3,"lines":120,"media":[{"mime_type":"image/png","url":"https://example.com/a.png"}]}"#;
        let result = result_from_stdout(json.into());
        assert!(result.error.is_none(), "{}", result.content);
        let data = result.data.expect("data");
        assert_eq!(data["files"], 3);
        assert_eq!(data["lines"], 120);
        assert_eq!(result.parts.len(), 1);
    }

    #[test]
    fn json_array_stdout_passes_through() {
        let result = result_from_stdout(r#"[1, "two", {"three": 3}]"#.into());
        assert_eq!(
            result.data,
            Some(serde_json::json!([1, "two", { "three": 3 }]))
        );
    }

    #[test]
    fn code_failure_envelope_sets_code_details_for_rejection() {
        let envelope = r#"{"error":{"code":"not_found","message":"missing","details":{"id":42},"retryable":true}}"#;
        let result = structured_failure(envelope, "stderr").expect("envelope");
        assert_eq!(result.error_code.as_deref(), Some("not_found"));
        let error = result.error.as_ref().expect("error");
        assert_eq!(error.message, "missing");
        assert!(error.retryable);
        assert_eq!(result.error_details.as_ref().unwrap()["id"], 42);

        let rejected = crate::tool::outcome::program_result(&result)
            .expect_err("the program must see a rejection");
        match rejected.kind {
            crate::ptc::HostErrorKind::Tool {
                code, details, ..
            } => {
                assert_eq!(code, "not_found");
                assert_eq!(details.as_ref().unwrap()["id"], 42);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(rejected.message, "missing");
        assert!(structured_failure("not json", "stderr").is_none());
    }

    #[test]
    fn validate_custom_reports_program_preview() {
        let output = r#"{"ok":true}"#;
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C".into(), format!("echo {output}")]);
        #[cfg(unix)]
        let (command, args) = ("sh", vec!["-c".into(), format!("printf '%s' '{output}'")]);
        let def = CustomToolDefinition {
            name: "validate_code".into(),
            description: "validation probe".into(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
            command: command.into(),
            args,
            timeout: 10,
            rules: Vec::new(),
            suite: None,
        };
        let result = validate_custom_tool_run(&def, &serde_json::json!({}), &std::env::temp_dir());
        assert!(result.error.is_none(), "{}", result.content);
        assert!(
            result.content.contains("program preview:"),
            "{}",
            result.content
        );
    }
}
