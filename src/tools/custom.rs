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

fn start_background(
    config: CustomToolDefinition,
    hub: Arc<CustomToolHub>,
    input: Value,
    execution: ToolExecutionContext,
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

    let _ = std::thread::Builder::new()
        .name(format!("custom-tool-{job_id}"))
        .spawn(move || {
            let result = run_sync_with_lifecycle(
                &config,
                &input,
                &ToolExecutionContext {
                    cancel: job_cancel.clone(),
                    ..execution
                },
                Some(job_id2.as_str()),
            );
            let outcome = if result.content.contains("timed out") {
                CustomToolOutcome::Error {
                    message: result.content,
                }
            } else if result.content.contains("cancelled") || job_cancel.is_cancelled() {
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
        });

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

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return ToolCallResult::error(format!(
                "failed to spawn custom tool '{}': {}",
                config.name, e
            ));
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(input_json.as_bytes()) {
            let _ = kill_child_tree(&mut child);
            return ToolCallResult::error(format!("stdin write failed: {e}"));
        }
        drop(stdin);
    }

    let timeout = Duration::from_secs(config.timeout.max(1));
    match wait_child_cancellable(&mut child, &execution.cancel, timeout) {
        WaitEnd::Finished(output) => {
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

enum WaitEnd {
    Finished(std::process::Output),
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
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(mut s) = stdout {
            let _ = s.read_to_end(&mut out);
        }
        if let Some(mut s) = stderr {
            let _ = s.read_to_end(&mut err);
        }
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
                let status = child.wait().unwrap_or_else(|_| {
                    // Last resort: synthesize a failed wait by re-checking.
                    child.try_wait().ok().flatten().unwrap_or_else(|| {
                        panic!("custom tool wait failed: {e}")
                    })
                });
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
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
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
        let result =
            result_from_stdout(r#"{"content":"wrote 3 of 10","level":"warning"}"#.into());
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
        let result =
            result_from_stdout(r#"{"content":"ok","level":"ok","hint":"nope"}"#.into());
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
}
