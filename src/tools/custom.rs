//! Custom tool — user-defined CLI tool wrapped as a platform tool.
//!
//! # Stdout JSON envelope (opt-in)
//!
//! A custom tool that emits a JSON object on stdout **and** includes `media`
//! and/or `level` is parsed as an envelope. Plain text, and JSON without those
//! keys, is returned verbatim. Envelope parse runs only on process exit 0;
//! a non-zero exit is always a pipeline Error (stdout cannot wash a crash
//! into Ok). `Hint` is not part of this protocol (LSP only).
//!
//! Sync is the default. Pass `run_in_background: true` to register a job on
//! [`CustomToolHub`], return immediately, and deliver a [`CustomToolExitNotice`]
//! to the session mailbox when the process finishes (Bobo drains it).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::schema::CustomToolDefinition;
use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::custom_hub::{CustomToolExitNotice, CustomToolHub, CustomToolOutcome};
use crate::types::{MediaKind, MediaSource, ToolCallResult, ToolOutputPart, ToolSignalLevel};

/// Exit code a custom tool uses to signal it declined execution.
const CUSTOM_BLOCKED_EXIT_CODE: i32 = 2;

const ENV_WORKSPACE: &str = "LITECODE_WORKSPACE";
const ENV_CALL_ID: &str = "LITECODE_CALL_ID";
const ENV_SESSION_ID: &str = "LITECODE_SESSION_ID";
const ENV_TOOL_NAME: &str = "LITECODE_TOOL_NAME";
const ENV_JOB_ID: &str = "LITECODE_JOB_ID";

pub struct CustomTool {
    config: CustomToolDefinition,
    hub: Arc<CustomToolHub>,
}

impl CustomTool {
    pub fn new(config: CustomToolDefinition, hub: Arc<CustomToolHub>) -> Self {
        Self { config, hub }
    }

    pub fn config(&self) -> &CustomToolDefinition {
        &self.config
    }
}

impl Tool for CustomTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn schema(&self) -> Value {
        let mut schema = self.config.to_json_schema();
        if let Some(obj) = schema.as_object_mut() {
            let props = obj
                .entry("properties")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut();
            if let Some(props) = props {
                props.insert(
                    "run_in_background".into(),
                    serde_json::json!({
                        "type": "boolean",
                        "description": "When true, start as a background job and return job_id immediately; result is delivered later via CustomToolSettled mailbox (default false)."
                    }),
                );
            }
        }
        schema
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let hub = Arc::clone(&self.hub);
        let config = self.config.clone();
        Box::pin(async move {
            let background = input
                .get("run_in_background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let mut child_input = input;
            if let Some(obj) = child_input.as_object_mut() {
                obj.remove("run_in_background");
            }

            if background {
                return start_background(config, hub, child_input, execution);
            }

            let config2 = config.clone();
            let exec2 = execution.clone();
            let join = tokio::task::spawn_blocking(move || {
                run_sync_with_lifecycle(&config2, &child_input, &exec2, None)
            });
            match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("custom tool task join failed: {e}")),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let mut child_input = input;
        if let Some(obj) = child_input.as_object_mut() {
            obj.remove("run_in_background");
        }
        let execution = ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            call_id: String::new(),
            cancel: CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        };
        run_sync_with_lifecycle(&self.config, &child_input, &execution, None)
    }

    fn description(&self, _ctx: &Context) -> String {
        let trimmed = self.config.description.trim();
        if trimmed.is_empty() {
            format!("External tool: {}", self.config.name)
        } else {
            trimmed.to_string()
        }
    }

    fn timeout(&self) -> Option<u64> {
        // Self-enforced inside run_sync_with_lifecycle so cancel/kill stay coupled.
        None
    }

    fn is_cancellable(&self) -> bool {
        true
    }
}

/// Validate a definition + sample input without registering the tool.
pub fn validate_custom_tool_run(
    def: &CustomToolDefinition,
    sample_input: &Value,
    workspace_root: &Path,
) -> ToolCallResult {
    // Match production execute/call_inner: scheduling knobs never reach stdin.
    let mut child_input = sample_input.clone();
    if let Some(obj) = child_input.as_object_mut() {
        obj.remove("run_in_background");
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
    let result = run_sync_with_lifecycle(def, &child_input, &execution, None);
    let rules_note = if def.rules.is_empty() {
        String::new()
    } else {
        format!(
            "- note: definition has {} rule(s); validate_custom does not evaluate rules — PASS is not a rules check\n",
            def.rules.len()
        )
    };
    match result.level {
        ToolSignalLevel::Error => ToolCallResult::error(format!(
            "# validate_custom — FAIL\n\n- tool: `{}`\n- command: `{}`\n{}- detail:\n\n{}",
            def.name, def.command, rules_note, result.content
        )),
        _ => ToolCallResult::ok(format!(
            "# validate_custom — PASS\n\n- tool: `{}`\n- command: `{}`\n{}- output:\n\n{}",
            def.name, def.command, rules_note, result.content
        )),
    }
}

type WorkerSpawner = fn(String, Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()>;

fn spawn_os_thread(name: String, f: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
    std::thread::Builder::new().name(name).spawn(f).map(|_| ())
}

fn start_background(
    config: CustomToolDefinition,
    hub: Arc<CustomToolHub>,
    input: Value,
    execution: ToolExecutionContext,
) -> ToolCallResult {
    start_background_with(config, hub, input, execution, spawn_os_thread)
}

fn start_background_with(
    config: CustomToolDefinition,
    hub: Arc<CustomToolHub>,
    input: Value,
    execution: ToolExecutionContext,
    spawner: WorkerSpawner,
) -> ToolCallResult {
    let job_id = ulid::Ulid::new().to_string();
    let session_id = execution.session_id.clone();
    let call_id = execution.call_id.clone();
    let tool_name = config.name.clone();
    // Register with Bobo's hub so cancel_job / turn-cancel reach this worker.
    let job_cancel = hub.begin_job(&session_id, &job_id, &execution.cancel);

    let hub2 = Arc::clone(&hub);
    let session_id2 = session_id.clone();
    let call_id2 = call_id.clone();
    let tool_name2 = tool_name.clone();
    let job_id2 = job_id.clone();

    let spawned = spawner(
        format!("custom-tool-{job_id}"),
        Box::new(move || {
            let result = run_sync_with_lifecycle(
                &config,
                &input,
                &ToolExecutionContext {
                    cancel: job_cancel.clone(),
                    ..execution
                },
                Some(job_id2.as_str()),
            );
            // Classify on our own lifecycle signals only: the child's stdout may
            // legitimately contain words like "cancelled" / "timed out".
            let outcome = if job_cancel.is_cancelled() {
                CustomToolOutcome::Cancelled
            } else if result.level == ToolSignalLevel::Error {
                CustomToolOutcome::Error {
                    message: result.content,
                }
            } else {
                CustomToolOutcome::Ok {
                    output: result.content,
                }
            };
            hub2.push_notice(CustomToolExitNotice {
                session_id: session_id2,
                call_id: call_id2,
                tool_name: tool_name2,
                job_id: job_id2,
                outcome,
                revision_hint: None,
            });
        }),
    );
    if let Err(e) = spawned {
        // No worker → no notice would ever arrive; release the job and fail now.
        hub.abandon_job(&job_id);
        return ToolCallResult::error(format!(
            "failed to start background worker for custom tool '{tool_name}': {e}"
        ));
    }

    ToolCallResult::ok(format!(
        "started custom tool '{tool_name}' as background job `{job_id}` (call_id `{call_id}`).\nResult arrives later as reminder/custom_tool_settled (CustomToolSettled); do not invent a FunctionCallOutput."
    ))
}

fn run_sync_with_lifecycle(
    config: &CustomToolDefinition,
    input: &Value,
    execution: &ToolExecutionContext,
    job_id: Option<&str>,
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
    if let Some(job_id) = job_id {
        cmd.env(ENV_JOB_ID, job_id);
    }
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
            // Child exited, so its stdin read end is closed (unless a
            // grandchild inherited it) and the writer has finished or hit EPIPE.
            // On Cancelled/TimedOut the writer is left detached: it unblocks
            // with EPIPE once the kill below closes the pipe.
            let stdin_result = stdin_writer
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(std::io::Error::other("stdin writer panicked")))
                })
                .unwrap_or(Ok(()));
            if let Err(e) = stdin_result {
                return ToolCallResult::error(format!("stdin write failed: {e}"));
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            match output.status.code() {
                Some(0) => result_from_stdout(stdout.into_owned()),
                Some(code) if code == CUSTOM_BLOCKED_EXIT_CODE => ToolCallResult::error(format!(
                    "custom tool '{}' blocked execution: {}",
                    config.name,
                    stderr.trim()
                )),
                Some(code) => ToolCallResult::error(format!(
                    "custom tool '{}' exited with code {}: {}",
                    config.name,
                    code,
                    stderr.trim()
                )),
                None => ToolCallResult::error(format!(
                    "custom tool '{}' terminated by signal: {}",
                    config.name,
                    stderr.trim()
                )),
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
                let (out, err) = rx.recv().unwrap_or_default();
                return WaitEnd::Finished(std::process::Output {
                    status,
                    stdout: out,
                    stderr: err,
                });
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
                let (out, err) = rx.recv().unwrap_or_default();
                return WaitEnd::Finished(std::process::Output {
                    status,
                    stdout: out,
                    stderr: if err.is_empty() {
                        format!("wait error: {e}").into_bytes()
                    } else {
                        err
                    },
                });
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
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&stdout) else {
        return ToolCallResult::ok(stdout);
    };
    if !map.contains_key("media") && !map.contains_key("level") {
        return ToolCallResult::ok(stdout);
    }

    let level = match parse_envelope_level(map.get("level")) {
        Ok(level) => level,
        Err(msg) => return ToolCallResult::error(format!("custom tool signal: {msg}")),
    };
    let parts = if map.contains_key("media") {
        match parse_media_parts(&map["media"]) {
            Ok(parts) => parts,
            Err(msg) => return ToolCallResult::error(format!("custom tool media output: {msg}")),
        }
    } else {
        Vec::new()
    };
    let content = map
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut result = if parts.is_empty() {
        match level {
            ToolSignalLevel::Ok => ToolCallResult::ok(content),
            ToolSignalLevel::Warning => ToolCallResult::warning(content),
            ToolSignalLevel::Error => ToolCallResult::error(content),
        }
    } else {
        ToolCallResult::ok_with_parts(content, parts)
    };
    result.level = level;
    result
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
    use crate::types::MediaSource;

    fn tool(name: &str) -> CustomTool {
        CustomTool::new(
            CustomToolDefinition {
                name: name.into(),
                description: "test".into(),
                schema: crate::config::schema::ToolSchema {
                    schema_type: "object".into(),
                    properties: serde_json::json!({}),
                    required: vec![],
                },
                command: "true".into(),
                args: vec![],
                timeout: 10,
                rules: Vec::new(),
                suite: None,
                suite_label: None,
            },
            Arc::new(CustomToolHub::new()),
        )
    }

    #[test]
    fn plain_text_keeps_legacy_behavior() {
        let result = result_from_stdout("hello world".into());
        assert_eq!(result.content, "hello world");
        assert!(result.parts.is_empty());
    }

    #[test]
    fn json_without_media_is_plain_text() {
        let result = result_from_stdout(r#"{"name":"data","ok":true}"#.into());
        assert!(result.content.contains("data"));
        assert!(result.parts.is_empty());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
    }

    #[test]
    fn level_warning_envelope_sets_warning_signal() {
        let result = result_from_stdout(r#"{"content":"wrote 3 of 10","level":"warning"}"#.into());
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "Warning: wrote 3 of 10");
    }

    #[test]
    fn level_error_envelope_sets_error_signal() {
        let result =
            result_from_stdout(r#"{"content":"missing ticket id","level":"error"}"#.into());
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "Error: missing ticket id");
    }

    #[test]
    fn schema_exposes_run_in_background() {
        let schema = tool("t").schema();
        assert!(
            schema["properties"]["run_in_background"]["type"]
                .as_str()
                .is_some()
        );
    }

    #[test]
    fn envelope_hint_key_is_ignored() {
        let result = result_from_stdout(r#"{"content":"ok","level":"ok","hint":"nope"}"#.into());
        assert_eq!(result.level, ToolSignalLevel::Ok);
        assert!(result.hint.is_none());
    }

    #[test]
    fn envelope_with_url_image_produces_media_part() {
        let result = result_from_stdout(
            r#"{"content":"","level":"ok","media":[{"url":"https://example.com/a.png","mime_type":"image/png"}]}"#.into(),
        );
        assert_eq!(result.parts.len(), 1);
        let ToolOutputPart::Media { artifact } = &result.parts[0] else {
            panic!("expected media");
        };
        assert!(matches!(artifact.source, MediaSource::Url { .. }));
    }

    #[test]
    fn envelope_with_file_path_produces_local_file_part() {
        let result = result_from_stdout(
            r#"{"media":[{"file_path":"/tmp/x.png","mime_type":"image/png"}]}"#.into(),
        );
        assert_eq!(result.parts.len(), 1);
    }

    #[test]
    fn malformed_envelope_hard_fails() {
        let result = result_from_stdout(r#"{"level":123}"#.into());
        assert_eq!(result.level, ToolSignalLevel::Error);
    }

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
            suite_label: None,
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
        let result = run_sync_with_lifecycle(&def, &serde_json::json!({ "pad": big }), &exec, None);
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
            suite_label: None,
        }
    }

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

    /// Child writes 1 MiB to stdout AND 1 MiB to stderr before reading its
    /// 1 MiB stdin. Old code deadlocked (sync stdin write / sequential reads)
    /// until timeout.
    #[cfg(unix)]
    #[test]
    fn large_output_before_reading_stdin_does_not_deadlock() {
        let def = sh_def(
            "big_io",
            "head -c 1048576 /dev/zero | tr '\\0' a; head -c 1048576 /dev/zero >&2; cat >/dev/null; echo",
            20,
        );
        let big = "x".repeat(1 << 20);
        let start = Instant::now();
        let result = run_sync_with_lifecycle(
            &def,
            &serde_json::json!({ "pad": big }),
            &test_exec(CancellationToken::new()),
            None,
        );
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "took {:?}",
            start.elapsed()
        );
        assert_ne!(
            result.level,
            ToolSignalLevel::Error,
            "{}",
            &result.content[..200.min(result.content.len())]
        );
    }

    /// Background outcome must not be classified by words in the child's output.
    #[tokio::test]
    async fn background_output_mentioning_cancelled_is_ok() {
        let hub = Arc::new(CustomToolHub::new());
        let tool = CustomTool::new(
            sh_def(
                "talky",
                "echo 'request cancelled; upload timed out (retrying)'",
                15,
            ),
            Arc::clone(&hub),
        );
        let mut exec = test_exec(CancellationToken::new());
        exec.session_id = "sess-talky".into();
        tool.execute(serde_json::json!({ "run_in_background": true }), exec)
            .await;
        let deadline = Instant::now() + Duration::from_secs(10);
        let notices = loop {
            let n = hub.take_mailbox("sess-talky");
            if !n.is_empty() {
                break n;
            }
            assert!(Instant::now() < deadline, "no notice");
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        match &notices[0].outcome {
            CustomToolOutcome::Ok { output } => assert!(output.contains("cancelled")),
            other => panic!("misclassified: {other:?}"),
        }
    }

    /// Timeout kill must reach grandchildren (process group kill).
    #[cfg(unix)]
    #[test]
    fn timeout_kills_grandchild_process() {
        let marker = std::env::temp_dir().join(format!("lc-gc-{}", ulid::Ulid::new()));
        let script = format!("(sleep 3; touch {}) & wait", marker.display());
        let def = sh_def("gc", &script, 1);
        let result = run_sync_with_lifecycle(
            &def,
            &serde_json::json!({}),
            &test_exec(CancellationToken::new()),
            None,
        );
        assert!(result.content.contains("timed out"), "{}", result.content);
        std::thread::sleep(Duration::from_secs(4));
        assert!(!marker.exists(), "grandchild survived the kill");
    }

    /// B5: a failing wait (child reaped elsewhere → ECHILD) returns an error, no panic.
    #[cfg(unix)]
    #[test]
    fn wait_failure_returns_error_instead_of_panicking() {
        let mut child = StdCommand::new("sh")
            .args(["-c", "exit 0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut st = 0;
        unsafe {
            libc::waitpid(child.id() as i32, &mut st, 0);
        }
        let end = wait_child_cancellable(
            &mut child,
            &CancellationToken::new(),
            Duration::from_secs(5),
        );
        assert!(matches!(end, WaitEnd::WaitFailed(_)));
    }

    /// B6: worker spawn failure releases the job and reports an error.
    #[test]
    fn background_spawn_failure_is_reported_and_job_released() {
        fn failing(_: String, _: Box<dyn FnOnce() + Send + 'static>) -> std::io::Result<()> {
            Err(std::io::Error::other("no threads"))
        }
        let hub = Arc::new(CustomToolHub::new());
        let mut exec = test_exec(CancellationToken::new());
        exec.session_id = "sess-sf".into();
        let result = start_background_with(
            sh_def("sf", "true", 5),
            Arc::clone(&hub),
            serde_json::json!({}),
            exec,
            failing,
        );
        assert_eq!(result.level, ToolSignalLevel::Error, "{}", result.content);
        assert!(result.content.contains("failed to start background worker"));
        let job_id = result.content.split('`').nth(1);
        assert!(job_id.is_none(), "must not announce a job id");
        // Nothing left in flight for this session.
        hub.cancel_session("sess-sf");
        assert!(hub.take_mailbox("sess-sf").is_empty());
    }

    #[tokio::test]
    async fn background_real_process_begin_job_then_mailbox_notice() {
        use crate::tool::trait_::ToolExecutionContext;
        use std::time::Duration;

        let hub = Arc::new(CustomToolHub::new());
        let tool = CustomTool::new(
            CustomToolDefinition {
                name: "echo_bg".into(),
                description: "bg echo".into(),
                schema: crate::config::schema::ToolSchema {
                    schema_type: "object".into(),
                    properties: serde_json::json!({}),
                    required: vec![],
                },
                // Real process (not a stub helper): shell echoes a marker.
                command: "sh".into(),
                args: vec!["-c".into(), "echo custom-bg-marker".into()],
                timeout: 15,
                rules: Vec::new(),
                suite: None,
                suite_label: None,
            },
            Arc::clone(&hub),
        );

        let cwd = std::env::temp_dir();
        let result = tool
            .execute(
                serde_json::json!({ "run_in_background": true }),
                ToolExecutionContext {
                    path_mode: crate::workspace::ToolPathMode::All,
                    workspace_root: cwd,
                    call_id: "call-bg-1".into(),
                    cancel: CancellationToken::new(),
                    output_limit: 8_000,
                    session_id: "sess-bg-1".into(),
                    session: None,
                },
            )
            .await;

        assert!(
            result.content.contains("background job"),
            "immediate ack must mention background job: {}",
            result.content
        );
        assert!(
            result.content.contains('`'),
            "ack should include backtick-wrapped job_id: {}",
            result.content
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !hub.mailbox_pending("sess-bg-1") {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for custom mailbox notice"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let notices = hub.take_mailbox("sess-bg-1");
        assert_eq!(notices.len(), 1, "exactly one settled notice");
        assert_eq!(notices[0].session_id, "sess-bg-1");
        assert_eq!(notices[0].call_id, "call-bg-1");
        assert_eq!(notices[0].tool_name, "echo_bg");
        assert!(!notices[0].job_id.is_empty());
        match &notices[0].outcome {
            CustomToolOutcome::Ok { output } => {
                assert!(
                    output.contains("custom-bg-marker"),
                    "stdout must reach mailbox: {output}"
                );
            }
            other => panic!("expected Ok outcome, got {other:?}"),
        }
        assert!(
            !hub.mailbox_pending("sess-bg-1"),
            "take_mailbox drains pending"
        );
    }
}
