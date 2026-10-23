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

use crate::config::schema::{CallSurface, CustomToolDefinition};
use crate::context_pipeline::Context;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::{Tool, ToolSurface};
use crate::tools::custom_hub::{CustomToolExitNotice, CustomToolHub, CustomToolOutcome};
use crate::types::{
    MediaKind, MediaSource, ToolCallResult, ToolErrorKind, ToolOutputPart, ToolSignalLevel,
};

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

/// The only result shape a custom tool can expose inside `run_code` in v1.
///
/// Custom tools are user-owned CLI processes; giving each one its own output
/// schema would turn this into a second public API. For Code calls the stdout
/// text stays opaque. Programs that want JSON can `JSON.parse(result.text)`
/// themselves.
fn custom_code_data_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "text": { "type": "string" }
        },
        "required": ["text"]
    })
}

fn custom_description(config: &CustomToolDefinition) -> String {
    let trimmed = config.description.trim();
    if trimmed.is_empty() {
        format!("External tool: {}", config.name)
    } else {
        trimmed.to_string()
    }
}

/// One schema for both the direct and code projections. The background knob is
/// a direct-only affordance: inside `run_code` a job id is not an awaited result.
fn custom_tool_schema(config: &CustomToolDefinition) -> Value {
    let mut schema = config.to_json_schema();
    if config.surface == CallSurface::Direct
        && let Some(obj) = schema.as_object_mut()
    {
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

fn code_result(mut result: ToolCallResult) -> ToolCallResult {
    let success = result.error.is_none()
        && matches!(result.level, ToolSignalLevel::Ok | ToolSignalLevel::Warning);
    if success {
        let text = result.content.clone();
        result.data = Some(serde_json::json!({ "text": text }));
    }
    result
}

struct CustomCodeProbe<'a> {
    config: &'a CustomToolDefinition,
}

impl Tool for CustomCodeProbe<'_> {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        custom_tool_schema(self.config)
    }

    fn data_schema(&self) -> Option<Value> {
        Some(custom_code_data_schema())
    }

    fn description(&self, _ctx: &Context) -> String {
        custom_description(self.config)
    }
}

/// Validate the Code-facing contract without registering or running the tool.
/// The error is deliberately suitable for settings UI and `validate_custom`.
pub fn validate_custom_code_contract(
    def: &CustomToolDefinition,
) -> std::result::Result<(), String> {
    if def.surface != CallSurface::Code {
        return Ok(());
    }
    crate::tool::code_manifest::validate_code_contract(&CustomCodeProbe { config: def })
        .map(|_| ())
        .map_err(|reason| {
            format!(
                "surface 'code' input schema cannot be admitted into run_code: {reason}. Use a simple object schema (no unresolved $ref) or declare surface 'direct'."
            )
        })
}

impl Tool for CustomTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn surface(&self) -> ToolSurface {
        match self.config.surface {
            CallSurface::Direct => ToolSurface::Direct,
            CallSurface::Code => ToolSurface::Code,
        }
    }

    fn schema(&self) -> Value {
        custom_tool_schema(&self.config)
    }

    fn data_schema(&self) -> Option<Value> {
        (self.config.surface == CallSurface::Code).then(custom_code_data_schema)
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let hub = Arc::clone(&self.hub);
        let config = self.config.clone();
        Box::pin(async move {
            let code_surface = config.surface == CallSurface::Code;
            let background = input
                .get("run_in_background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if code_surface && background {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    "run_in_background is not available for a custom tool on the code surface; remove it or declare surface 'direct'",
                    false,
                );
            }
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
            let result = match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("custom tool task join failed: {e}")),
            };
            if code_surface {
                code_result(result)
            } else {
                result
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
        custom_description(&self.config)
    }

    fn timeout(&self) -> Option<std::time::Duration> {
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
    if let Err(error) = validate_custom_code_contract(def) {
        return ToolCallResult::error(format!(
            "# validate_custom — FAIL\n\n- tool: `{}`\n- surface: `code`\n- code admission: FAIL\n- detail: {error}\n",
            def.name
        ));
    }

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
    let surface_note = match def.surface {
        CallSurface::Direct => {
            "- surface: `direct`\n- program result: n/a (direct tools are called outside run_code)\n- background: available\n".to_string()
        }
        CallSurface::Code => {
            "- surface: `code`\n- code admission: PASS\n- program result: `{ ok: true, text: <stdout/content> }`; parse text yourself if it is JSON\n- background: unavailable on the code surface\n".to_string()
        }
    };
    match result.level {
        ToolSignalLevel::Error => ToolCallResult::error(format!(
            "# validate_custom — FAIL\n\n- tool: `{}`\n- command: `{}`\n{surface_note}{rules_note}- detail:\n\n{}",
            def.name, def.command, result.content
        )),
        _ => {
            let preview = if def.surface == CallSurface::Code {
                let value = serde_json::json!({ "ok": true, "text": result.content });
                let mut preview = serde_json::to_string(&value)
                    .unwrap_or_else(|_| "{\"ok\":true,\"text\":\"<unrenderable>\"}".into());
                if preview.chars().count() > 1200 {
                    preview = preview.chars().take(1200).collect::<String>() + "…";
                }
                format!("- code preview:\n\n{preview}\n")
            } else {
                String::new()
            };
            ToolCallResult::ok(format!(
                "# validate_custom — PASS\n\n- tool: `{}`\n- command: `{}`\n{surface_note}{rules_note}- output:\n\n{}\n{preview}",
                def.name, def.command, result.content
            ))
        }
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
            surface: Default::default(),
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
            surface: Default::default(),
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
            None,
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
            None,
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
    fn code_surface_has_opaque_data_and_no_background_knob() {
        let mut def = sh_def("code_echo", "echo code-marker", 10);
        def.surface = CallSurface::Code;
        let tool = CustomTool::new(def.clone(), Arc::new(CustomToolHub::new()));
        assert_eq!(tool.surface(), ToolSurface::Code);
        assert!(tool.schema().get("properties").is_some());
        assert!(
            tool.schema()["properties"]
                .get("run_in_background")
                .is_none(),
            "code surface must not advertise background jobs"
        );
        let data_schema = tool.data_schema().expect("code data schema");
        assert_eq!(data_schema["properties"]["text"]["type"], "string");
        validate_custom_code_contract(&def).expect("simple object schema is code-admissible");

        let projected = crate::tool::project_tools(vec![Arc::new(CustomTool::new(
            def,
            Arc::new(CustomToolHub::new()),
        )) as Arc<dyn Tool>]);
        assert!(projected.direct.is_empty());
        assert_eq!(projected.whitelist, vec!["code_echo".to_string()]);
        assert!(projected.sdk.contains("code_echo"));
        assert!(projected.sdk.contains("text"));
    }

    #[test]
    fn code_surface_rejects_unrenderable_schema_with_actionable_reason() {
        let mut def = sh_def("bad_shape", "echo nope", 10);
        def.surface = CallSurface::Code;
        def.schema.properties = serde_json::json!({
            "item": { "$ref": "#/Item" }
        });
        let error = validate_custom_code_contract(&def).expect_err("ref must fail admission");
        assert!(error.contains("surface 'code'"), "{error}");
        assert!(error.contains("direct"), "{error}");
    }

    #[tokio::test]
    async fn code_surface_rejects_background_before_spawning() {
        let mut def = sh_def("bg_code", "echo should-not-run", 10);
        def.surface = CallSurface::Code;
        let tool = CustomTool::new(def, Arc::new(CustomToolHub::new()));
        let result = tool
            .execute(
                serde_json::json!({ "run_in_background": true }),
                test_exec(CancellationToken::new()),
            )
            .await;
        let error = result.error.expect("background must fail validation");
        assert_eq!(error.kind, ToolErrorKind::InvalidInput);
        assert!(
            error.message.contains("run_in_background"),
            "{}",
            error.message
        );
    }

    #[tokio::test]
    async fn code_surface_wraps_success_as_opaque_text_data() {
        let marker = "code-surface-marker";
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C".into(), format!("echo {marker}")]);
        #[cfg(unix)]
        let (command, args) = ("sh", vec!["-c".into(), format!("printf '%s' '{marker}'")]);

        let def = CustomToolDefinition {
            name: "code_echo".into(),
            description: "echo marker".into(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
            command: command.into(),
            args,
            timeout: 10,
            rules: Vec::new(),
            surface: CallSurface::Code,
            suite: None,
            suite_label: None,
        };
        let tool = CustomTool::new(def, Arc::new(CustomToolHub::new()));
        let result = tool
            .execute(serde_json::json!({}), test_exec(CancellationToken::new()))
            .await;
        assert!(result.error.is_none(), "{}", result.content);
        let data = result.data.expect("code success must carry data");
        let text = data["text"].as_str().expect("text field");
        assert!(text.contains(marker), "{text}");
    }

    #[test]
    fn validate_custom_reports_code_admission_and_program_preview() {
        let marker = "validate-code-marker";
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C".into(), format!("echo {marker}")]);
        #[cfg(unix)]
        let (command, args) = ("sh", vec!["-c".into(), format!("printf '%s' '{marker}'")]);
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
            surface: CallSurface::Code,
            suite: None,
            suite_label: None,
        };
        let result = validate_custom_tool_run(&def, &serde_json::json!({}), &std::env::temp_dir());
        assert!(result.error.is_none(), "{}", result.content);
        assert!(
            result.content.contains("code admission: PASS"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("background: unavailable"),
            "{}",
            result.content
        );
        assert!(
            result.content.contains("code preview:"),
            "{}",
            result.content
        );
        assert!(result.content.contains(marker), "{}", result.content);
    }
    #[test]
    fn direct_surface_keeps_background_and_has_no_code_data_schema() {
        let def = sh_def("direct_echo", "echo direct-marker", 10);
        let tool = CustomTool::new(def, Arc::new(CustomToolHub::new()));
        assert_eq!(tool.surface(), ToolSurface::Direct);
        assert_eq!(
            tool.schema()["properties"]["run_in_background"]["type"],
            "boolean"
        );
        assert!(tool.data_schema().is_none());
    }


}
