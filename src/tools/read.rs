use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::file_identity::{self, locate_resolved};
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// Page size in lines. Not a token budget.
const DEFAULT_LINE_LIMIT: usize = 2_000;
const MAX_LINE_LIMIT: usize = 5_000;
/// Per-line character cap. Longer lines are truncated in the string and noted in message.
const MAX_LINE_CHARS: usize = 1500;

#[derive(Default)]
pub struct ReadTool {
    ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

impl ReadTool {
    pub fn with_ide(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide: Some(ide) }
    }
}

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": format!(
                        "{} Past session transcripts are readable at `.litecode/sessions/<session_id>.md`.",
                        crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                    )
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read, 1-based, inclusive. Default 1."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum lines to return. Default 2000, maximum 5000."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "description": "One page of a text file. Continue with a larger start_line. Guidance lives only in message.",
            "properties": {
                "path": { "type": "string" },
                "start_line": { "type": "integer", "minimum": 1 },
                "lines": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Line contents for this page, in file order."
                },
                "message": { "type": "string" }
            },
            "required": ["path", "start_line", "lines", "message"]
        }))
    }

    fn effects(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        match crate::tools::file_path::code_path(input) {
            Some(path) => crate::tool::resource_gate::ToolEffects::read(
                crate::tool::resource_gate::lock_key(workspace_root, path, path_mode),
            ),
            None => crate::tool::resource_gate::ToolEffects::default(),
        }
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(30))
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<crate::tool::write_lock::ResourceKey> {
        match input["path"].as_str() {
            Some(path) if !path.is_empty() => {
                let key = crate::workspace::resolve_agent(workspace_root, path, path_mode)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string());
                vec![crate::tool::write_lock::ResourceKey::File(key)]
            }
            _ => vec![],
        }
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let execution = ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: self
                .ide
                .as_ref()
                .map(|ide| ide.workspace.sandbox().root().to_path_buf())
                .unwrap_or_else(crate::config::workspace::workspace_root_lap),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: String::new(),
            session: None,
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(self.call_for_execution(input, execution))
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/read.md"))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        ReadTool::validate_input(self, input)
    }
}

impl ReadTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tool::require_nonempty_string(&input, "path") {
            Ok(s) => s,
            Err(e) => return coded("invalid_input", e),
        };
        if let Some(result) = read_virtual_session(raw_path, &input, &execution) {
            return result;
        }
        let path = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => return crate::tools::file_path::tool_path_error(error),
        };
        let located = locate_resolved(&execution.workspace_root, &path, raw_path);
        let workspace_bytes = self.ide.as_ref().and_then(|ide| {
            let relative = ide.workspace.sandbox().rel_path(&path).ok()?;
            match load_workspace_bytes(ide.workspace.as_ref(), &relative, &execution.cancel) {
                WorkspaceBytes::Bytes(bytes) => Some(Ok(bytes)),
                WorkspaceBytes::Missing => None,
                WorkspaceBytes::Failed(error) => Some(Err(error)),
            }
        });
        let result = match workspace_bytes {
            Some(Ok(bytes)) => {
                self.read_resolved(&path, Some(bytes), &input, located, &execution.cancel)
            }
            Some(Err(error)) => error,
            None => self.read_resolved(&path, None, &input, located, &execution.cancel),
        };
        if !matches!(result.level, crate::types::ToolSignalLevel::Error)
            && let Some(ide) = &self.ide
        {
            ide.sync_document_if_ready(&path).await;
        }
        result
    }

    fn read_resolved(
        &self,
        path: &Path,
        workspace_bytes: Option<Vec<u8>>,
        input: &Value,
        located: file_identity::LocatedPath,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> ToolCallResult {
        let path_display = path.to_string_lossy().into_owned();
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return coded("not_found", format!("file not found: {path_display}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return coded(
                    "io",
                    format!(
                        "Permission denied reading {path_display}. Check file permissions or choose a different path."
                    ),
                );
            }
            Err(e) => return coded("io", format!("read {path_display}: {e}")),
        };

        if metadata.is_dir() {
            return coded(
                "invalid_input",
                crate::tools::file_path::directory_not_file_message(&path_display),
            );
        }

        let bytes = match workspace_bytes {
            Some(bytes) => bytes,
            None => match read_file_checking_cancel(path, cancel) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    return ToolCallResult::error_with_kind(
                        crate::types::ToolErrorKind::Cancelled,
                        "read cancelled",
                        false,
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return coded("not_found", format!("file not found: {path_display}"));
                }
                Err(e) => return coded("io", format!("read {path_display}: {e}")),
            },
        };
        if let Some(mime_type) = sniff_image_mime(&bytes) {
            return unsupported_read(
                &located.path,
                mime_type,
                bytes.len() as u64,
                &format!(
                    "unsupported file: {path_display} ({mime_type}, {} bytes)",
                    bytes.len()
                ),
            );
        }
        let content = match crate::workspace::text_codec::decode_utf8_bytes(&bytes) {
            Ok(decoded) => decoded.text,
            Err(error) => {
                let mime_type = match error {
                    crate::workspace::text_codec::Utf8DecodeError::Utf16 => "text/utf-16",
                    _ => "application/octet-stream",
                };
                return unsupported_read(
                    &located.path,
                    mime_type,
                    bytes.len() as u64,
                    &crate::workspace::text_codec::decode_error_for_path(error, &path_display),
                );
            }
        };
        render_text_page(&content, input, &located.path)
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "path")?;
        if let Some(message) = limit_error(input) {
            return Err(message);
        }
        let _ = resolve_start_line(input)?;
        Ok(())
    }
}

fn coded(code: &str, message: impl Into<String>) -> ToolCallResult {
    crate::tools::file_path::coded_error(code, message)
}

fn limit_error(input: &Value) -> Option<String> {
    let Some(value) = input.get("limit").filter(|value| !value.is_null()) else {
        return None;
    };
    let Some(limit) = value.as_u64() else {
        return Some("parameter 'limit' must be an integer".into());
    };
    if !(1..=MAX_LINE_LIMIT as u64).contains(&limit) {
        return Some(format!(
            "parameter 'limit' must be between 1 and {MAX_LINE_LIMIT}"
        ));
    }
    None
}

fn read_virtual_session(
    raw_path: &str,
    input: &Value,
    execution: &ToolExecutionContext,
) -> Option<ToolCallResult> {
    let stem = crate::session::transcript_file::try_parse_virtual_path(raw_path)?;
    let reader = match execution.session_reader() {
        Ok(r) => r,
        Err(e) => return Some(crate::tools::file_path::litecode_error(e)),
    };
    let session_id = match crate::engines::session_search::resolve_session_ref(reader, &stem) {
        Ok(id) => id,
        Err(e) => return Some(crate::tools::file_path::litecode_error(e)),
    };
    let file = match reader.transcript_file_blocking(&session_id) {
        Ok(f) => f,
        Err(e) => return Some(crate::tools::file_path::litecode_error(e)),
    };
    let hidden = hidden_surface_seqs(reader, &session_id, &execution.session_id);
    Some(render_projection_page(&file, input, &hidden, raw_path))
}

fn hidden_surface_seqs(
    reader: &crate::session::SessionDataReader,
    target_session: &str,
    caller_session: &str,
) -> Vec<i64> {
    if caller_session.is_empty() || caller_session != target_session {
        return Vec::new();
    }
    crate::engines::session_search::load_surface_seqs(reader, target_session).unwrap_or_default()
}

fn render_projection_page(
    file: &crate::session::transcript_file::TranscriptFile,
    input: &Value,
    hidden_seqs: &[i64],
    data_path: &str,
) -> ToolCallResult {
    let start_line = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    if let Some(message) = limit_error(input) {
        return coded("invalid_input", message);
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LINE_LIMIT);
    let total_lines = file.total_lines();
    if start_line > total_lines && total_lines > 0 {
        return coded(
            "invalid_input",
            format!("start_line {start_line} exceeds file length ({total_lines} lines)"),
        );
    }
    let start = start_line.saturating_sub(1);
    let end = (start + limit).min(total_lines);
    let range_hidden = (start..end).all(|i| {
        file.seq_at((i + 1) as u32)
            .is_some_and(|seq| hidden_seqs.contains(&seq))
    });
    if total_lines > 0 && start < end && range_hidden && !hidden_seqs.is_empty() {
        return finish_read(
            data_path,
            start_line,
            Vec::new(),
            end,
            total_lines,
            "These session lines are already in the context window.",
            false,
        );
    }
    let mut lines = Vec::new();
    let mut line_capped = false;
    for i in start..end {
        let line_no = (i + 1) as u32;
        if file
            .seq_at(line_no)
            .is_some_and(|seq| hidden_seqs.contains(&seq))
        {
            continue;
        }
        let raw = file.lines.get(i).map(String::as_str).unwrap_or("");
        let (text, capped) = data_line_text(raw);
        if capped {
            line_capped = true;
        }
        lines.push(text);
    }
    finish_read(
        data_path,
        start_line,
        lines,
        end,
        total_lines,
        "",
        line_capped,
    )
}

fn render_text_page(content: &str, input: &Value, path: &str) -> ToolCallResult {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    if let Some(message) = limit_error(input) {
        return coded("invalid_input", message);
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_LINE_LIMIT);
    let total_lines = lines.len();
    if start_line > total_lines && total_lines > 0 {
        return coded(
            "invalid_input",
            format!("start_line {start_line} exceeds file length ({total_lines} lines)"),
        );
    }
    let start = start_line.saturating_sub(1);
    let end = (start + limit).min(total_lines);
    let mut out = Vec::new();
    let mut line_capped = false;
    for i in start..end {
        let (text, capped) = data_line_text(lines[i]);
        if capped {
            line_capped = true;
        }
        out.push(text);
    }
    finish_read(path, start_line, out, end, total_lines, "", line_capped)
}

fn finish_read(
    path: &str,
    start_line: usize,
    lines: Vec<String>,
    covered_end: usize,
    total_lines: usize,
    preset_message: &str,
    line_capped: bool,
) -> ToolCallResult {
    let has_more = covered_end < total_lines;
    let mut message = preset_message.to_string();
    if message.is_empty() {
        if has_more {
            message = format!(
                "Showing lines {start_line}–{covered_end} of {total_lines}. Continue with start_line={}.",
                covered_end + 1
            );
        } else if line_capped {
            message = "One or more lines were truncated.".into();
        }
    } else if line_capped {
        message = format!("{message} One or more lines were truncated.");
    }
    crate::tool::outcome::ok_data(serde_json::json!({
        "path": path,
        "start_line": start_line,
        "lines": lines,
        "message": message,
    }))
}

fn data_line_text(line: &str) -> (String, bool) {
    if line.chars().count() <= MAX_LINE_CHARS {
        (line.to_string(), false)
    } else {
        (
            format!("{}...", line.chars().take(MAX_LINE_CHARS).collect::<String>()),
            true,
        )
    }
}

fn line_number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
}

fn resolve_start_line(input: &Value) -> Result<usize, String> {
    let Some(value) = input.get("start_line").filter(|value| !value.is_null()) else {
        return Ok(1);
    };
    match line_number(value) {
        Some(n) if n >= 1 => Ok(n as usize),
        _ => Err("parameter 'start_line' must be an integer >= 1".into()),
    }
}


enum WorkspaceBytes {
    Bytes(Vec<u8>),
    /// The workspace sandbox does not have this path. Fall through to the
    /// direct read, which still reports a missing file.
    Missing,
    Failed(ToolCallResult),
}

/// Workspace policy stays with the sandbox: resolve, not-a-file, and the 10MB
/// cap. The bytes themselves are read in chunks so cancel can drop a partial
/// buffer before decode.
fn load_workspace_bytes(
    workspace: &crate::workspace::WorkspaceService,
    relative: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> WorkspaceBytes {
    let sandbox = workspace.sandbox();
    let abs = match sandbox.resolve(relative) {
        Ok(abs) => abs,
        Err(error) => {
            return WorkspaceBytes::Failed(crate::tools::file_path::workspace_error(error.into()));
        }
    };
    if !abs.exists() {
        return WorkspaceBytes::Missing;
    }
    if !abs.is_file() {
        let display = sandbox
            .rel_path(&abs)
            .unwrap_or_else(|_| relative.to_string());
        return WorkspaceBytes::Failed(coded(
            "invalid_input",
            crate::tools::file_path::directory_not_file_message(&display),
        ));
    }
    let meta = match abs.metadata() {
        Ok(meta) => meta,
        Err(error) => {
            return WorkspaceBytes::Failed(crate::tools::file_path::workspace_error(error.into()));
        }
    };
    if meta.len() > crate::workspace::MAX_FILE_SIZE {
        return WorkspaceBytes::Failed(crate::tools::file_path::workspace_error(
            crate::workspace::WorkspaceError::TooLarge,
        ));
    }
    match read_file_checking_cancel(&abs, cancel) {
        Ok(Some(bytes)) => WorkspaceBytes::Bytes(bytes),
        Ok(None) => WorkspaceBytes::Failed(ToolCallResult::error_with_kind(
            crate::types::ToolErrorKind::Cancelled,
            "read cancelled",
            false,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => WorkspaceBytes::Missing,
        Err(error) => {
            WorkspaceBytes::Failed(crate::tools::file_path::workspace_error(error.into()))
        }
    }
}

const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Read the file in chunks so a cancel that arrives between chunks can stop
/// the read. A single `read` syscall itself cannot be interrupted.
fn read_file_checking_cancel(
    path: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> std::io::Result<Option<Vec<u8>>> {
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let mut file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    let mut buf = [0u8; READ_CHUNK_BYTES];
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let read = std::io::Read::read(&mut file, &mut buf)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..read]);
    }
    Ok(Some(bytes))
}

fn unsupported_read(path: &str, mime: &str, bytes: u64, message: &str) -> ToolCallResult {
    crate::tool::outcome::ToolOutcome::<Value>::failure("unsupported", message, false)
        .with_error_details(serde_json::json!({
            "path": path,
            "mime": mime,
            "bytes": bytes,
        }))
        .into_call_result(message)
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_declares_a_thirty_second_timeout() {
        assert_eq!(
            ReadTool::default().timeout(),
            Some(std::time::Duration::from_secs(30))
        );
    }

    #[test]
    fn cancelled_read_stops_between_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, vec![b'a'; READ_CHUNK_BYTES + 8]).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert!(read_file_checking_cancel(&path, &cancel).unwrap().is_none());
    }

    #[tokio::test]
    async fn ide_read_checks_cancel_between_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let mut body = b"IDE_CANCEL_SENTINEL\n".to_vec();
        body.resize(READ_CHUNK_BYTES + 32, b'q');
        std::fs::write(dir.path().join("note.txt"), &body).unwrap();
        let ide = crate::ide_base::IdeBaseHandle::open(
            dir.path(),
            std::sync::Arc::new(crate::engines::WorkspaceEngines::new()),
        )
        .unwrap();
        let tool = ReadTool::with_ide(ide);
        let execution = |cancel| ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: dir.path().to_path_buf(),
            call_id: "read-ide".into(),
            cancel,
            output_limit: usize::MAX,
            session_id: String::new(),
            session: None,
        };
        let read = tool
            .execute(
                serde_json::json!({"path": "note.txt"}),
                execution(tokio_util::sync::CancellationToken::new()),
            )
            .await;
        assert!(read.error.is_none(), "{}", read.content);
        let data = read.data.as_ref().unwrap();
        assert_eq!(data["path"], "note.txt");
        assert!(
            data["lines"][0].as_str().unwrap().contains("IDE_CANCEL_SENTINEL"),
            "{data}"
        );
        assert!(read.content.is_empty());

        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        let cancelled = tool
            .execute(serde_json::json!({"path": "note.txt"}), execution(cancel))
            .await;
        assert_eq!(
            cancelled.error.as_ref().map(|error| error.kind),
            Some(crate::types::ToolErrorKind::Cancelled)
        );
        assert!(cancelled.data.is_none());
    }

    #[test]
    fn chunked_read_returns_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let body = vec![b'b'; READ_CHUNK_BYTES + 8];
        std::fs::write(&path, &body).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let read = read_file_checking_cancel(&path, &cancel).unwrap().unwrap();
        assert_eq!(read, body);
    }

    fn page_lines(result: &ToolCallResult) -> Vec<String> {
        result
            .data
            .as_ref()
            .unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn test_read_basic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        let lines = page_lines(&result);
        assert_eq!(lines, ["line1", "line2", "line3"]);
        assert!(result.content.is_empty());
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["start_line"], 1);
        assert!(data.get("file").is_none());
        assert!(data.get("page").is_none());
    }

    #[test]
    fn test_read_with_start_line_and_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\n").expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "start_line": 2,
            "limit": 2
        }));
        assert_eq!(page_lines(&result), ["line2", "line3"]);
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["start_line"], 2);
        assert!(
            data["message"].as_str().unwrap().contains("start_line=4"),
            "{}",
            data["message"]
        );
    }

    #[test]
    fn test_read_start_line_past_end_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "only\n").unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "start_line": 5
        }));
        assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
        assert!(result.content.contains("start_line"), "{}", result.content);
    }

    #[test]
    fn test_read_binary_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.bin");
        std::fs::write(&path, [0u8, 1, 2, 3, 0, 5]).unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": path.to_str().unwrap()
        }));
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.data.is_none());
    }

    #[test]
    fn test_read_strips_utf8_bom_from_display() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.txt");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hello\n");
        std::fs::write(&path, bytes).unwrap();
        let result = ReadTool::default().call(serde_json::json!({"path": path.to_str().unwrap()}));
        assert_eq!(page_lines(&result), ["hello"]);
    }

    #[test]
    fn test_read_utf16_error_is_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u16.txt");
        // UTF-16 LE BOM + "hi"
        std::fs::write(&path, [0xFF, 0xFE, b'h', 0, b'i', 0]).unwrap();
        let result = ReadTool::default().call(serde_json::json!({"path": path.to_str().unwrap()}));
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
    }

    #[test]
    fn test_read_preserves_trailing_spaces_on_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sp.txt");
        std::fs::write(&path, "alpha  \n").unwrap();
        let result = ReadTool::default().call(serde_json::json!({"path": path.to_str().unwrap()}));
        assert_eq!(page_lines(&result), ["alpha  "]);
    }

    #[test]
    fn test_read_image_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        std::fs::write(
            &path,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00],
        )
        .unwrap();
        let result = ReadTool::default().call(serde_json::json!({"path": path.to_str().unwrap()}));
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.parts.is_empty());
    }

    #[test]
    fn test_read_page_ignores_token_count() {
        let dir = tempfile::tempdir().unwrap();
        let line = (0..80).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
        let body: String = (0..200).map(|_| format!("{line}\n")).collect();
        std::fs::write(dir.path().join("big.txt"), body).unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": dir.path().join("big.txt").to_str().unwrap()
        }));
        assert_eq!(page_lines(&result).len(), 200);
    }

    #[test]
    fn test_read_default_limit_pages() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..2500).map(|i| format!("L{i}\n")).collect();
        std::fs::write(dir.path().join("many.txt"), body).unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": dir.path().join("many.txt").to_str().unwrap()
        }));
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["lines"].as_array().unwrap().len(), 2000);
        assert!(data["message"].as_str().unwrap().contains("start_line=2001"), "{}", data["message"]);
    }

    #[test]
    fn test_read_long_line_is_capped_in_text() {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(2000);
        std::fs::write(dir.path().join("wide.txt"), format!("{long}\n")).unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": dir.path().join("wide.txt").to_str().unwrap()
        }));
        let line = page_lines(&result)[0].clone();
        assert!(line.ends_with("..."));
        assert!(line.chars().count() <= MAX_LINE_CHARS + 3);
        let data = result.data.as_ref().unwrap();
        assert!(
            data["message"].as_str().unwrap().contains("truncated"),
            "{}",
            data["message"]
        );
        assert!(data["lines"][0].is_string());
    }

    #[test]
    fn test_read_schema_omits_retired_fields() {
        let tool = ReadTool::default();
        let schema = tool.schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("start_line"));
        assert!(props.contains_key("limit"));
        assert!(!props.contains_key("end_line"));
        let data_schema = tool.data_schema().unwrap();
        let dprops = data_schema["properties"].as_object().unwrap();
        assert!(dprops.contains_key("path"));
        assert!(dprops.contains_key("start_line"));
        assert!(dprops.contains_key("lines"));
        assert!(dprops.contains_key("message"));
        for retired in ["file", "page", "reason", "end_line"] {
            assert!(!dprops.contains_key(retired), "retired {retired}");
        }
    }

    #[test]
    fn test_validate_input() {
        let tool = ReadTool::default();
        assert!(tool.validate_input(&serde_json::json!({"path": "a.txt"})).is_ok());
        assert!(tool.validate_input(&serde_json::json!({"path": ""})).is_err());
        assert!(tool.validate_input(&serde_json::json!({"path": "a.txt", "limit": 0})).is_err());
        assert!(tool.validate_input(&serde_json::json!({"path": "a.txt", "start_line": 0})).is_err());
    }

    #[test]
    fn data_schema_accepts_live_result() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": dir.path().join("a.txt").to_str().unwrap()
        }));
        let data = result.data.as_ref().unwrap();
        let schema = ReadTool::default().data_schema().unwrap();
        crate::tool::schema_validate(&schema, data)
            .unwrap_or_else(|e| panic!("{e}: {data}"));
    }
}
