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
//! ```json
//! { "content": "optional text",
//!   "level": "warning",
//!   "media": [
//!     { "url": "https://example.com/a.png", "mime_type": "image/png" },
//!     { "file_path": "/tmp/clip.mp4", "mime_type": "video/mp4" }
//!   ] }
//! ```
//!
//! - `level` — `ok` | `warning` | `error` (case-insensitive). Omitted with
//!   `media` only → `ok`. Unknown values hard-fail.
//! - `url` — passed through to the provider as-is (no client-side fetch).
//! - `file_path` — materialized by the executor into a base64 blob (10 MB cap).
//! - `kind` is derived from the `mime_type` prefix (image/video/audio).
//! - A malformed envelope **hard-fails** — never silently downgraded.

use std::process::Command as StdCommand;

use serde_json::Value;

use crate::config::schema::CustomToolDefinition;
use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::types::{MediaKind, MediaSource, ToolCallResult, ToolOutputPart, ToolSignalLevel};

/// Exit code a custom tool uses to signal it declined execution.
const CUSTOM_BLOCKED_EXIT_CODE: i32 = 2;

pub struct CustomTool {
    config: CustomToolDefinition,
}

impl CustomTool {
    pub fn new(config: CustomToolDefinition) -> Self {
        Self { config }
    }
}

impl Tool for CustomTool {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn schema(&self) -> Value {
        self.config.to_json_schema()
    }

    fn execute(
        &self,
        input: Value,
        _execution: crate::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // 2.6: the child process wait is blocking; run it in `spawn_blocking` so a
        // blocked custom tool does not stall the async executor and its timeout
        // stays effective.
        let tool = CustomTool::new(self.config.clone());
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || tool.call_inner(input));
            match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("custom tool task join failed: {e}")),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let input_json = match serde_json::to_string(&input) {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let mut child = match StdCommand::new(&self.config.command)
            .args(&self.config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return ToolCallResult::error(format!(
                    "failed to spawn custom tool '{}': {}",
                    self.config.name, e
                ));
            }
        };

        // Write input JSON to stdin.
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            if let Err(e) = stdin.write_all(input_json.as_bytes()) {
                return ToolCallResult::error(format!("stdin write failed: {}", e));
            }
            drop(stdin); // Close stdin to signal EOF.
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        match output.status.code() {
            Some(0) => result_from_stdout(stdout.into_owned()),
            Some(code) if code == CUSTOM_BLOCKED_EXIT_CODE => ToolCallResult::error(format!(
                "custom tool '{}' blocked execution: {}",
                self.config.name,
                stderr.trim()
            )),
            Some(code) => ToolCallResult::error(format!(
                "custom tool '{}' exited with code {}: {}",
                self.config.name,
                code,
                stderr.trim()
            )),
            None => ToolCallResult::error(format!(
                "custom tool '{}' terminated by signal: {}",
                self.config.name,
                stderr.trim()
            )),
        }
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
        Some(self.config.timeout)
    }
}

/// Map custom-tool stdout to a tool result.
///
/// Plain text (and JSON without `media` / `level`) keeps the legacy text-only
/// behavior. A JSON envelope with those keys carries signal and/or media; any
/// malformed envelope is a hard error — never silently downgraded.
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

/// Parse the `media` array of the JSON envelope into [`ToolOutputPart`]s.
///
/// Each entry: `url` (remote passthrough) XOR `file_path` (executor
/// materializes to base64), plus a required `mime_type`; the media kind is
/// derived from the mime prefix. Unknown mimes or missing fields hard-fail.
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
        CustomTool::new(CustomToolDefinition {
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
        })
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
        let result = result_from_stdout(r#"{"level":"Warning","content":"wrote 3 of 10"}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert_eq!(result.content, "wrote 3 of 10");
        assert!(result.hint.is_none());
        assert!(result.parts.is_empty());
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "Warning: wrote 3 of 10");
    }

    #[test]
    fn level_error_envelope_sets_error_signal() {
        let result =
            result_from_stdout(r#"{"level":"error","content":"missing ticket id"}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert_eq!(result.content, "missing ticket id");
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "Error: missing ticket id");
    }

    #[test]
    fn level_ok_with_media_keeps_parts() {
        let stdout = r#"{"level":"ok","content":"shot","media":[{"url":"https://example.com/a.png","mime_type":"image/png"}]}"#;
        let result = result_from_stdout(stdout.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert_eq!(result.content, "shot");
        assert_eq!(result.parts.len(), 1);
    }

    #[test]
    fn level_warning_with_media_keeps_parts() {
        let stdout = r#"{"level":"warning","content":"partial","media":[{"url":"https://example.com/a.png","mime_type":"image/png"}]}"#;
        let result = result_from_stdout(stdout.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert_eq!(result.content, "partial");
        assert_eq!(result.parts.len(), 1);
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "Warning: partial");
        assert_eq!(wire.parts.len(), 1);
    }

    #[test]
    fn unknown_level_hard_fails() {
        let result = result_from_stdout(r#"{"level":"hint","content":"nope"}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(result.content.contains("unknown level"));
    }

    #[test]
    fn non_string_level_hard_fails() {
        let result = result_from_stdout(r#"{"level":1,"content":"x"}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(result.content.contains("level must be a string"));
    }

    #[test]
    fn envelope_hint_key_is_ignored() {
        let result =
            result_from_stdout(r#"{"level":"ok","content":"body","hint":"lsp-only"}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert_eq!(result.content, "body");
        assert!(result.hint.is_none());
        let wire = result.finalize_signals();
        assert_eq!(wire.content, "body");
        assert!(!wire.content.contains("Hint:"));
    }

    #[test]
    fn envelope_with_url_image_produces_media_part() {
        let stdout = r#"{"content":"shot","media":[{"url":"https://example.com/a.png","mime_type":"image/png"}]}"#;
        let result = result_from_stdout(stdout.into());
        assert_eq!(result.content, "shot");
        assert_eq!(result.parts.len(), 1);
        assert!(matches!(
            &result.parts[0],
            ToolOutputPart::Media { artifact }
                if artifact.kind == MediaKind::Image
                    && artifact.mime_type == "image/png"
                    && matches!(&artifact.source, MediaSource::Url { url } if url == "https://example.com/a.png")
        ));
    }

    #[test]
    fn envelope_with_file_path_produces_local_file_part() {
        let stdout = r#"{"media":[{"file_path":"/tmp/clip.mp4","mime_type":"video/mp4"}]}"#;
        let result = result_from_stdout(stdout.into());
        assert_eq!(result.parts.len(), 1);
        assert!(matches!(
            &result.parts[0],
            ToolOutputPart::Media { artifact }
                if artifact.kind == MediaKind::Video
                    && matches!(&artifact.source, MediaSource::LocalFile { .. })
        ));
    }

    #[test]
    fn malformed_envelope_hard_fails() {
        let result =
            result_from_stdout(r#"{"media":[{"url":"https://example.com/a.png"}]}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(result.content.contains("mime_type"));

        let result = result_from_stdout(r#"{"media":[{"mime_type":"image/png"}]}"#.into());
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(result.content.contains("url or file_path"));

        let result = result_from_stdout(
            r#"{"media":[{"url":"https://example.com/a.xyz","mime_type":"application/octet-stream"}]}"#.into(),
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(result.content.contains("unsupported mime_type"));
    }

    #[test]
    fn schema_is_config_schema() {
        let schema = tool("t").schema();
        assert!(schema.is_object());
    }
}
