use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::file_identity::{self, file_ref, locate_resolved};
use crate::tool::outcome::{FileRef, Page};
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

/// Page size in lines. Not a token budget.
const DEFAULT_LINE_LIMIT: usize = 2_000;
const MAX_LINE_LIMIT: usize = 5_000;
/// Per-line character cap. A longer line is marked `line_truncated`.
const MAX_LINE_CHARS: usize = 1500;

#[derive(Debug, Serialize)]
struct ReadLineData {
    number: usize,
    text: String,
    line_truncated: bool,
}

#[derive(Debug, Serialize)]
struct ReadPageData {
    file: FileRef,
    lines: Vec<ReadLineData>,
    page: Page,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

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
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read, 1-based, inclusive. The page ends at min(end_line, start_line + limit - 1)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum lines in this page. Default 2000, maximum 5000."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "description": "One page of a text file, with the file's sha256 identity.",
            "type": "object",
            "properties": {
                "file": {
                    "type": "object",
                    "description": "File identity. path is workspace-relative inside the workspace. Outside the workspace, path is the requested path and abs is absolute.",
                    "properties": {
                        "path": { "type": "string" },
                        "abs": { "type": "string" },
                        "hash": { "type": "string", "description": "sha256 hex of the file bytes." },
                        "bytes": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["path", "hash", "bytes"]
                },
                "lines": {
                    "type": "array",
                    "description": "Lines in this page, in file order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "number": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "1-based line number."
                            },
                            "text": {
                                "type": "string",
                                "description": "Line text without a line-number prefix."
                            },
                            "line_truncated": {
                                "type": "boolean",
                                "description": "True when text was capped at the per-line character limit."
                            }
                        },
                        "required": ["number", "text", "line_truncated"]
                    }
                },
                "page": {
                    "type": "object",
                    "properties": {
                        "has_more": {
                            "type": "boolean",
                            "description": "True when any line remains after this page, including after an explicit window."
                        },
                        "next": {
                            "type": ["string", "null"],
                            "description": "Next 1-based line number, or null when the file is finished."
                        },
                        "total": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "Total lines in the file."
                        }
                    },
                    "required": ["has_more", "next"]
                },
                "reason": {
                    "type": "string",
                    "enum": ["in_context_window"],
                    "description": "Set when lines is empty because those session lines are already in the context window."
                }
            },
            "required": ["file", "lines", "page"]
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
        // Unit-test helper only. Production Agent turns enter through execute().
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
                &format!("unsupported file: {path_display} ({mime_type}, {} bytes)", bytes.len()),
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
        let file = file_ref(located, &bytes);
        render_text_page(&content, input, file)
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "path")?;
        if let Some(message) = limit_error(input) {
            return Err(message);
        }
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
    let identity = file_ref(
        file_identity::LocatedPath {
            base: file_identity::PathBase::Workspace,
            path: data_path.to_string(),
            abs: None,
        },
        file.lines.join("\n").as_bytes(),
    );
    let start_line = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let end_line = match resolve_end_line(input, start_line) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let total_lines = file.total_lines();
    if start_line > total_lines && total_lines > 0 {
        return coded(
            "invalid_input",
            format!("start_line {start_line} exceeds file length ({total_lines} lines)"),
        );
    }
    let start = start_line.saturating_sub(1);
    let end = end_line.min(total_lines);
    let range_hidden = (start..end).all(|i| {
        file.seq_at((i + 1) as u32)
            .is_some_and(|seq| hidden_seqs.contains(&seq))
    });
    if total_lines > 0 && start < end && range_hidden && !hidden_seqs.is_empty() {
        return finish_read(
            identity,
            Vec::new(),
            end,
            total_lines,
            Some("in_context_window"),
        );
    }
    render_line_window(identity, start, end, total_lines, None, |i| {
            let line_no = (i + 1) as u32;
            if file
                .seq_at(line_no)
                .is_some_and(|seq| hidden_seqs.contains(&seq))
            {
                return None;
            }
            Some((i + 1, file.lines.get(i).map(String::as_str).unwrap_or("")))
        },
    )
}

fn render_text_page(content: &str, input: &Value, file: FileRef) -> ToolCallResult {
    let lines: Vec<&str> = content.lines().collect();
    let start_line = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let end_line = match resolve_end_line(input, start_line) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let total_lines = lines.len();
    if start_line > total_lines && total_lines > 0 {
        return coded(
            "invalid_input",
            format!("start_line {start_line} exceeds file length ({total_lines} lines)"),
        );
    }
    let start = start_line.saturating_sub(1);
    let end = end_line.min(total_lines);
    render_line_window(file, start, end, total_lines, None, |i| {
        Some((i + 1, lines[i]))
    })
}

fn render_line_window<'a>(
    file: FileRef,
    start: usize,
    end: usize,
    total_lines: usize,
    reason: Option<&str>,
    mut line_at: impl FnMut(usize) -> Option<(usize, &'a str)>,
) -> ToolCallResult {
    let mut data_lines = Vec::new();
    for i in start..end {
        let Some((line_no, line)) = line_at(i) else {
            continue;
        };
        let (text, line_truncated) = data_line_text(line);
        data_lines.push(ReadLineData {
            number: line_no,
            text,
            line_truncated,
        });
    }
    finish_read(file, data_lines, end, total_lines, reason)
}

fn finish_read(
    file: FileRef,
    lines: Vec<ReadLineData>,
    covered_end: usize,
    total_lines: usize,
    reason: Option<&str>,
) -> ToolCallResult {
    let has_more = covered_end < total_lines;
    let next = has_more.then(|| (covered_end + 1).to_string());
    let page = Page {
        has_more,
        next,
        total: Some(total_lines as u64),
    };
    let data = serde_json::to_value(ReadPageData {
        file,
        lines,
        page,
        reason: reason.map(str::to_string),
    })
    .unwrap_or(Value::Null);
    crate::tool::outcome::ok_data(data)
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

fn data_line_text(line: &str) -> (String, bool) {
    if line.chars().count() <= MAX_LINE_CHARS {
        (line.to_string(), false)
    } else {
        (line.chars().take(MAX_LINE_CHARS).collect(), true)
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

fn resolve_end_line(input: &Value, start_line: usize) -> Result<usize, String> {
    if let Some(message) = limit_error(input) {
        return Err(message);
    }
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|limit| limit as usize)
        .unwrap_or(DEFAULT_LINE_LIMIT);
    let max_end = start_line.saturating_add(limit - 1);
    let Some(value) = input.get("end_line").filter(|value| !value.is_null()) else {
        return Ok(max_end);
    };
    let Some(n) = line_number(value) else {
        return Err("parameter 'end_line' must be an integer >= 1".into());
    };
    if n < 1 {
        return Err("parameter 'end_line' must be an integer >= 1".into());
    }
    let requested = n as usize;
    if requested < start_line {
        return Err("end_line must be >= start_line".into());
    }
    Ok(requested.min(max_end))
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
        assert_eq!(
            read.data.as_ref().unwrap()["file"]["bytes"],
            body.len() as u64
        );
        assert!(
            read.content.contains("IDE_CANCEL_SENTINEL")
                || read.data.as_ref().unwrap()["lines"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("IDE_CANCEL_SENTINEL"),
            "{}",
            read.content
        );

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
        assert!(!cancelled.content.contains("IDE_CANCEL_SENTINEL"));
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

    #[test]
    fn test_read_basic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "path": file_path.to_str().expect("path"),
        });

        let result = tool.call(input);
        let text = page_text(&result);
        assert!(text.contains("line1"));
        assert!(text.contains("line2"));
        assert!(text.contains("line3"));
    }

    #[test]
    fn test_read_with_start_end_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\n").expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "start_line": 2,
            "end_line": 3
        });

        let result = tool.call(input);
        let data = result.data.expect("data");
        assert_eq!(data["lines"][0]["text"], "line2");
        assert_eq!(data["lines"][1]["text"], "line3");
        assert_eq!(data["lines"].as_array().unwrap().len(), 2);
        assert_eq!(data["page"]["has_more"], true);
        assert_eq!(data["page"]["next"], "4");
    }

    #[test]
    fn test_read_start_line_past_end_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\n").expect("write");

        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "start_line": 3,
        }));

        assert_eq!(
            result.content,
            "Error: start_line 3 exceeds file length (2 lines)"
        );
    }

    #[test]
    fn test_read_binary_detection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("binary.dat");
        std::fs::write(&file_path, b"hello\x00world\x00" as &[u8]).expect("write binary");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "path": file_path.to_str().expect("path")
        });

        let result = tool.call(input);
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.parts.is_empty());
        assert!(result.data.is_none());
        let details = result.error_details.expect("details");
        assert_eq!(details["mime"], "application/octet-stream");
        assert!(details["bytes"].as_u64().unwrap() > 0);
        assert!(result.content.to_lowercase().contains("binary"));
    }

    #[test]
    fn test_read_strips_utf8_bom_from_display() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("bom.txt");
        std::fs::write(&file_path, "\u{feff}hello\n").expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        let text = page_text(&result);
        assert!(text.contains("hello"));
        assert!(!text.contains('\u{feff}'));
    }

    #[test]
    fn test_read_utf16_error_is_explicit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("utf16.txt");
        std::fs::write(&file_path, [0xFF, 0xFE, b'A', 0x00]).expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.parts.is_empty());
        assert!(result.data.is_none());
        assert_eq!(result.error_details.as_ref().unwrap()["mime"], "text/utf-16");
        assert!(result.content.contains("UTF-16"));
    }

    #[test]
    fn test_read_preserves_trailing_spaces_on_last_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("spaces.txt");
        std::fs::write(&file_path, "keep   ").expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        let text = page_text(&result);
        assert!(
            text.contains("keep   "),
            "last-line trailing spaces must survive display, got: {text:?}"
        );
    }

    #[test]
    fn test_read_image_returns_media_part() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("screenshot.png");
        std::fs::write(&file_path, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
            .expect("write png signature");

        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));

        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.parts.is_empty(), "read must not return media parts");
        assert!(result.data.is_none());
        let details = result.error_details.expect("details");
        assert_eq!(details["mime"], "image/png");
        assert_eq!(details["bytes"], 8);
    }

    #[test]
    fn test_read_page_ignores_token_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("big.txt");
        let line = (0..80)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let content: String = (0..200).map(|_| format!("{line}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        assert!(result.error.is_none(), "{}", result.content);
        let lines = result.data.unwrap()["lines"].as_array().unwrap().len();
        assert_eq!(lines, 200, "page size follows the line limit, not a token budget");
    }

    #[test]
    fn test_read_default_limit_pages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("long.txt");
        let content: String = (1..=2_001).map(|i| format!("{i}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let data = ReadTool::default()
            .call(serde_json::json!({
                "path": file_path.to_str().expect("path"),
            }))
            .data
            .unwrap();
        assert_eq!(data["lines"].as_array().unwrap().len(), DEFAULT_LINE_LIMIT);
        assert_eq!(data["page"]["has_more"], true);
        assert_eq!(data["page"]["next"], "2001");
        assert_eq!(data["lines"][0]["text"], "1");
    }

    #[test]
    fn test_read_long_line_is_capped_not_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("fat.txt");
        let huge = (0..20_000)
            .map(|i| format!("tok{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(&file_path, format!("{huge}\nnext\n")).expect("write");

        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        assert!(result.error.is_none(), "{}", result.content);
        let data = result.data.unwrap();
        assert_eq!(data["lines"][0]["line_truncated"], true);
        assert_eq!(data["lines"][1]["text"], "next");
        assert!(data["lines"][0]["text"].as_str().unwrap().chars().count() <= MAX_LINE_CHARS);
    }

    #[test]
    fn test_read_schema_omits_token_budget() {
        let props = ReadTool::default().schema()["properties"].clone();
        assert!(props.get("token_budget").is_none());
        assert!(props.get("start_line").is_some());
        assert!(props.get("end_line").is_some());
    }

    #[test]
    fn test_read_clamps_huge_end_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("long.txt");
        let content: String = (1..=2_500).map(|i| format!("{i}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "end_line": 99999
        }));
        assert!(result.error.is_none(), "{}", result.content);
        let data = result.data.unwrap();
        assert_eq!(data["lines"].as_array().unwrap().len(), DEFAULT_LINE_LIMIT);
        assert_eq!(data["page"]["has_more"], true);
        assert_eq!(data["page"]["next"], "2001");
    }

    #[test]
    fn test_read_truncates_long_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("wide.txt");
        let long = "x".repeat(2000);
        std::fs::write(&file_path, format!("{long}\nshort\n")).expect("write");

        let data = ReadTool::default()
            .call(serde_json::json!({
                "path": file_path.to_str().expect("path"),
            }))
            .data
            .unwrap();
        assert_eq!(data["lines"][0]["line_truncated"], true);
        assert_eq!(data["lines"][0]["text"].as_str().unwrap().chars().count(), MAX_LINE_CHARS);
        assert_eq!(data["lines"][1]["text"], "short");
    }

    #[test]
    fn test_read_size_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("large.txt");
        let chunk = "a".repeat(1024);
        let mut file = std::fs::File::create(&file_path).expect("create");
        for _ in 0..10241 {
            use std::io::Write;
            writeln!(file, "{}", chunk).expect("write chunk");
        }
        drop(file);

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "path": file_path.to_str().expect("path")
        });

        let result = tool.call(input);
        assert!(result.error.is_none(), "large text is paged, not a media cap: {}", result.content);
        assert!(result.data.as_ref().unwrap()["file"]["hash"].is_string());
        assert_eq!(result.data.as_ref().unwrap()["page"]["has_more"], true);
        assert!(result.parts.is_empty());
    }

    #[test]
    fn test_validate_input() {
        let tool = ReadTool::default();

        // Missing file_path
        assert!(tool.validate_input(&serde_json::json!({})).is_err());

        // Empty file_path
        assert!(
            tool.validate_input(&serde_json::json!({"path": ""}))
                .is_err()
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "start_line": 0
        }));
        assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
        assert!(
            result.content.contains("start_line"),
            "got: {}",
            result.content
        );
        assert!(result.data.is_none());

        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "start_line": 2,
            "end_line": 1
        }));
        assert!(result.content.starts_with("Error:"));
        assert!(
            result.content.contains("end_line must be >= start_line"),
            "got: {}",
            result.content
        );

        // Valid
        assert!(
            tool.validate_input(&serde_json::json!({"path": "/tmp/x"}))
                .is_ok()
        );
    }

    #[test]
    fn negative_start_line_is_invalid_input() {
        let tool = ReadTool::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let path = file_path.to_str().expect("path");

        let start = tool.call(serde_json::json!({
            "path": path,
            "start_line": -5
        }));
        assert_eq!(start.error_code.as_deref(), Some("invalid_input"));
        assert!(start.content.contains("start_line"), "got: {}", start.content);
        assert!(start.data.is_none());
    }

    #[test]
    fn known_path_read_ignores_discovery_filters() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join(".git")).expect("git");
        std::fs::write(dir.path().join(".gitignore"), "secret.env\n").expect("gitignore");
        let file_path = dir.path().join("secret.env");
        std::fs::write(&file_path, "TOKEN=known-path\n").expect("write");

        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "path": file_path.to_str().expect("path"),
        }));
        assert!(
            page_text(&result).contains("TOKEN=known-path"),
            "read must not apply discovery FilterPreset; got: {}",
            page_text(&result)
        );
    }

    fn seed_session(root: &std::path::Path, text: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(&id, &[crate::types::user_text(text)])
            .unwrap();
        id
    }

    fn read_virtual(
        root: &std::path::Path,
        path: &str,
        extra: serde_json::Value,
        session_id: &str,
    ) -> crate::types::ToolCallResult {
        let mut input = extra;
        input["path"] = serde_json::Value::String(path.to_string());
        let tool = ReadTool::default();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(tool.execute(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: session_id.to_string(),
                session: Some(crate::session::SessionDataReader::open(
                    &root.join(".litecode").join("sessions.db"),
                )),
            },
        ))
    }

    #[test]
    fn virtual_session_read_keeps_canonical_line_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let sid = seed_session(root, "alpha\nVIRTUAL_READ_NEEDLE here\ndelta");
        let path = crate::session::transcript_file::virtual_path_for(&sid);
        let other = read_virtual(root, &path, serde_json::json!({}), "");
        assert_eq!(
            other.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            other.content
        );
        let other_text = page_text(&other);
        assert!(
            other_text.contains("VIRTUAL_READ_NEEDLE"),
            "{other_text}"
        );
        let needle_line = other
            .data
            .as_ref()
            .unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["text"].as_str().unwrap_or("").contains("VIRTUAL_READ_NEEDLE"))
            .expect("needle line");
        assert_eq!(needle_line["number"], 3, "canonical body line should stay numbered, got: {needle_line}");

        let self_read = read_virtual(root, &path, serde_json::json!({}), &sid);
        assert_eq!(
            self_read.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            self_read.content
        );
        assert_eq!(
            self_read.data.as_ref().unwrap()["reason"], "in_context_window",
            "live surface should be filtered: {:?}",
            self_read.data
        );
        assert!(
            !page_text(&self_read).contains("VIRTUAL_READ_NEEDLE"),
            "filtered read must not re-number remaining lines around the live item: {}",
            page_text(&self_read)
        );
    }

    fn seed_compacted(root: &std::path::Path, archived: &str, live: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
        let data = crate::session::SessionData::open(&lease, &db).unwrap();
        let id = data
            .create_session(root.to_str().unwrap(), "default", None)
            .unwrap();
        data.insert_items(
            &id,
            &[
                crate::types::user_text(archived),
                crate::types::user_text("filler middle"),
                crate::types::user_text(live),
            ],
        )
        .unwrap();
        data.compact_from(
            &id,
            &crate::types::user_text("[summary] compact"),
            Some(2),
            10,
        )
        .unwrap();
        id
    }

    #[test]
    fn read_peels_current_surface_keeps_canonical_lines_and_other_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let current = seed_compacted(
            root,
            "ARCHIVED_READ_TOKEN buried",
            "LIVE_READ_TOKEN still in window",
        );
        let other = seed_session(root, "LIVE_READ_TOKEN in other");
        let current_path = crate::session::transcript_file::virtual_path_for(&current);
        let other_path = crate::session::transcript_file::virtual_path_for(&other);

        let full = read_virtual(root, &current_path, serde_json::json!({}), "");
        let archived_no = line_number(&full, "ARCHIVED_READ_TOKEN");
        let live_no = line_number(&full, "LIVE_READ_TOKEN still in window");
        assert!(live_no > archived_no);

        let peeled = read_virtual(root, &current_path, serde_json::json!({}), &current);
        assert!(
            page_text(&peeled).contains("ARCHIVED_READ_TOKEN"),
            "archived rows stay readable:\n{}",
            page_text(&peeled)
        );
        assert!(
            !page_text(&peeled).contains("LIVE_READ_TOKEN still in window"),
            "current live surface must be peeled:\n{}",
            page_text(&peeled)
        );
        let peeled_no = line_number(&peeled, "ARCHIVED_READ_TOKEN");
        assert_eq!(
            peeled_no, archived_no,
            "peel must keep canonical line numbers"
        );

        let other_as_current = read_virtual(root, &other_path, serde_json::json!({}), &current);
        assert!(
            page_text(&other_as_current).contains("LIVE_READ_TOKEN in other"),
            "other sessions are not peeled:\n{}",
            page_text(&other_as_current)
        );

        let live_only = read_virtual(
            root,
            &current_path,
            serde_json::json!({ "start_line": live_no, "end_line": live_no }),
            &current,
        );
        assert_eq!(
            live_only.data.as_ref().unwrap()["reason"], "in_context_window",
            "offset into the live surface must stay in-window: {:?}",
            live_only.data
        );
        assert!(
            !page_text(&live_only).contains("LIVE_READ_TOKEN still in window"),
            "live-only window must not leak surface text:\n{}",
            page_text(&live_only)
        );
    }

    fn line_number(result: &ToolCallResult, needle: &str) -> usize {
        result.data.as_ref().expect("data")["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["text"].as_str().unwrap_or("").contains(needle))
            .unwrap_or_else(|| panic!("missing {needle} in {}", result.data.as_ref().unwrap()))
            ["number"]
            .as_u64()
            .unwrap() as usize
    }

    fn page_text(result: &ToolCallResult) -> String {
        result
            .data
            .as_ref()
            .and_then(|data| data.get("lines"))
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| line["text"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    fn read_data(result: &ToolCallResult) -> Value {
        result
            .data
            .clone()
            .expect("read must return structured data")
    }

    #[test]
    fn text_result_has_structured_lines_without_display_prefixes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").expect("write");
        let result = ReadTool::default().call(serde_json::json!({
            "path": path.to_str().expect("path"),
        }));
        let data = read_data(&result);
        assert!(data["file"]["path"].as_str().unwrap().contains("a.txt"));
        assert_eq!(data["file"]["hash"].as_str().unwrap().len(), 64);
        assert_eq!(data["page"]["total"], 3);
        assert_eq!(data["page"]["has_more"], false);
        assert!(data["page"]["next"].is_null());
        let lines = data["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["number"], 1);
        assert_eq!(lines[0]["text"], "alpha");
        assert_eq!(lines[2]["text"], "gamma");
        assert!(!lines[0]["text"].as_str().unwrap().contains(':'));
    }

    #[test]
    fn text_result_pages_with_next_start_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("long.txt");
        let content: String = (1..=5).map(|i| format!("{i}\n")).collect();
        std::fs::write(&path, &content).expect("write");
        let result = ReadTool::default().call(serde_json::json!({
            "path": path.to_str().expect("path"),
            "limit": 2
        }));
        let data = read_data(&result);
        assert_eq!(data["page"]["total"], 5);
        assert_eq!(data["page"]["has_more"], true);
        let lines = data["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(data["page"]["next"], "3");
    }

    #[test]
    fn data_line_text_is_capped_and_marked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("wide.txt");
        std::fs::write(&path, format!("{}\n", "x".repeat(1600))).expect("write");
        let result = ReadTool::default().call(serde_json::json!({
            "path": path.to_str().expect("path"),
        }));
        let data = read_data(&result);
        let line = &data["lines"][0];
        assert_eq!(line["line_truncated"], true);
        assert_eq!(line["text"].as_str().unwrap().chars().count(), 1500);
    }

    #[test]
    fn image_result_has_structured_metadata() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shot.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]).expect("write png");
        let result = ReadTool::default().call(serde_json::json!({
            "path": path.to_str().expect("path"),
        }));
        assert_eq!(result.error_code.as_deref(), Some("unsupported"));
        assert!(result.parts.is_empty());
        assert!(result.data.is_none());
        assert_eq!(result.error_details.as_ref().unwrap()["mime"], "image/png");
        assert_eq!(result.error_details.as_ref().unwrap()["bytes"], 8);
    }

    #[test]
    fn data_schema_accepts_text_results() {
        let dir = tempfile::tempdir().expect("tempdir");
        let text_path = dir.path().join("a.txt");
        std::fs::write(&text_path, "alpha\nbeta\n").expect("write");
        let image_path = dir.path().join("shot.png");
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .expect("write png");

        let tool = ReadTool::default();
        let schema = tool.data_schema().expect("read data schema");
        let result = tool.call(serde_json::json!({
            "path": text_path.to_str().expect("path"),
        }));
        let data = result.data.expect("read must return data");
        crate::tool::schema_validate(&schema, &data).unwrap_or_else(|error| {
            panic!("read data does not match its schema: {error}: {data}")
        });
        let image = tool.call(serde_json::json!({
            "path": image_path.to_str().expect("path"),
        }));
        assert_eq!(image.error_code.as_deref(), Some("unsupported"));
        assert!(image.data.is_none());
        assert!(image.parts.is_empty());
    }
}
