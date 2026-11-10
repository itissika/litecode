use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

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
        parse_limit(input)?;
        resolve_start_line(input)?;
        Ok(())
    }
}

fn coded(code: &str, message: impl Into<String>) -> ToolCallResult {
    crate::tools::file_path::coded_error(code, message)
}

fn parse_limit(
    input: &Value,
) -> Result<(usize, Option<crate::tool::clamp::Clamp>), String> {
    let clamped = crate::tool::clamp::clamp_int(input, "limit", 1, MAX_LINE_LIMIT as u64, "lines")?;
    Ok(match clamped {
        None => (DEFAULT_LINE_LIMIT, None),
        Some(clamped) => {
            let applied = clamped.value as usize;
            (applied, clamped.as_clamp("limit"))
        }
    })
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
    let (start_line, start_note) = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let (limit, limit_note) = match parse_limit(input) {
        Ok(limit) => limit,
        Err(msg) => return coded("invalid_input", msg),
    };
    let clamped = merge_clamps(start_note, limit_note);
    let total_lines = file.total_lines();
    if start_line > total_lines {
        return past_eof_page(data_path, start_line, total_lines, clamped);
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
            end - start,
            clamped,
            Vec::new(),
        );
    }
    let mut lines = Vec::new();
    let mut line_capped: Vec<usize> = Vec::new();
    let mut skipped_in_context = 0usize;
    for i in start..end {
        let line_no = (i + 1) as u32;
        if file
            .seq_at(line_no)
            .is_some_and(|seq| hidden_seqs.contains(&seq))
        {
            skipped_in_context += 1;
            continue;
        }
        let raw = file.lines.get(i).map(String::as_str).unwrap_or("");
        let (text, capped) = data_line_text(raw);
        if capped {
            line_capped.push(i + 1);
        }
        lines.push(text);
    }
    finish_read(
        data_path,
        start_line,
        lines,
        end,
        total_lines,
        skipped_in_context,
        clamped,
        line_capped,
    )
}

fn render_text_page(content: &str, input: &Value, path: &str) -> ToolCallResult {
    let lines: Vec<&str> = content.lines().collect();
    let (start_line, start_note) = match resolve_start_line(input) {
        Ok(line) => line,
        Err(msg) => return coded("invalid_input", msg),
    };
    let (limit, limit_note) = match parse_limit(input) {
        Ok(limit) => limit,
        Err(msg) => return coded("invalid_input", msg),
    };
    let clamped = merge_clamps(start_note, limit_note);
    let total_lines = lines.len();
    if start_line > total_lines {
        return past_eof_page(path, start_line, total_lines, clamped);
    }
    let start = start_line.saturating_sub(1);
    let end = (start + limit).min(total_lines);
    let mut out = Vec::new();
    let mut line_capped: Vec<usize> = Vec::new();
    for i in start..end {
        let (text, capped) = data_line_text(lines[i]);
        if capped {
            line_capped.push(i + 1);
        }
        out.push(text);
    }
    finish_read(
        path,
        start_line,
        out,
        end,
        total_lines,
        0,
        clamped,
        line_capped,
    )
}

/// `start_line` beyond the last line (including an empty file): resolve an
/// empty window and say how long the file is, instead of rejecting.
fn past_eof_page(
    path: &str,
    start_line: usize,
    total_lines: usize,
    clamped: Vec<crate::tool::clamp::Clamp>,
) -> ToolCallResult {
    finish_read(
        path,
        start_line,
        Vec::new(),
        total_lines,
        total_lines,
        0,
        clamped,
        Vec::new(),
    )
}

/// Sole success shape for read: one page of lines plus paging facts.
#[derive(serde::Serialize)]
struct ReadData {
    path: String,
    start_line: usize,
    lines: Vec<String>,
    /// Lines in the file; [`Self::has_more`] compares against it.
    total_lines: usize,
    /// True when this page stops before the end of the file.
    has_more: bool,
    /// Lines the window covered but left out because they are already in the
    /// model's context. A gap in `lines` is explained by this number.
    skipped_in_context: usize,
    /// Lines shortened to the per-line character cap, by number.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lines_capped: Vec<usize>,
    /// Parameters adjusted to a legal value; absent when nothing moved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    clamped: Vec<crate::tool::clamp::Clamp>,
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    vec![DriftCase {
        tool: "read",
        all_optional_present: finish_read(
            "note.txt",
            1,
            vec!["alpha".into()],
            1,
            2,
            1,
            vec![crate::tool::clamp::Clamp {
                parameter: "limit".into(),
                requested: serde_json::json!(0),
                applied: serde_json::json!(1),
            }],
            vec![1],
        ),
        all_optional_absent: finish_read(
            "note.txt",
            1,
            vec!["alpha".into()],
            1,
            1,
            0,
            Vec::new(),
            Vec::new(),
        ),
    }]
}

fn finish_read(
    path: &str,
    start_line: usize,
    lines: Vec<String>,
    covered_end: usize,
    total_lines: usize,
    skipped_in_context: usize,
    clamped: Vec<crate::tool::clamp::Clamp>,
    lines_capped: Vec<usize>,
) -> ToolCallResult {
    // Paging is a fact, not a sentence: has_more + total_lines say it, and the
    // renderer turns the pair into one reminder line for the model.
    let has_more = covered_end < total_lines;
    crate::tool::outcome::ok_data(ReadData {
        path: path.to_string(),
        start_line,
        lines,
        total_lines,
        has_more,
        skipped_in_context,
        lines_capped,
        clamped,
    })
}

fn data_line_text(line: &str) -> (String, bool) {
    if line.chars().count() <= MAX_LINE_CHARS {
        (line.to_string(), false)
    } else {
        (
            format!(
                "{}...",
                line.chars().take(MAX_LINE_CHARS).collect::<String>()
            ),
            true,
        )
    }
}

fn resolve_start_line(
    input: &Value,
) -> Result<(usize, Option<crate::tool::clamp::Clamp>), String> {
    let clamped = crate::tool::clamp::clamp_int(input, "start_line", 1, u64::MAX, "")?;
    Ok(match clamped {
        None => (1, None),
        Some(clamped) => {
            let applied = clamped.value as usize;
            (applied, clamped.as_clamp("start_line"))
        }
    })
}

fn merge_clamps(
    first: Option<crate::tool::clamp::Clamp>,
    second: Option<crate::tool::clamp::Clamp>,
) -> Vec<crate::tool::clamp::Clamp> {
    first.into_iter().chain(second).collect()
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
    use serde_json::json;

    #[test]
    fn cancelled_read_stops_between_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, vec![b'a'; READ_CHUNK_BYTES + 8]).unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();
        assert!(read_file_checking_cancel(&path, &cancel).unwrap().is_none());
    }

    fn page_lines(result: &ToolCallResult) -> Vec<String> {
        result.data.as_ref().unwrap()["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
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
        assert_eq!(data["has_more"], true, "{data}");
        assert_eq!(data["total_lines"], 5, "{data}");
        assert!(data.get("message").is_none(), "{data}");
    }

    fn page_file(lines: &[&str]) -> crate::session::transcript_file::TranscriptFile {
        crate::session::transcript_file::TranscriptFile {
            session_id: "session-1".to_string(),
            virtual_path: crate::session::transcript_file::virtual_path_for("session-1"),
            line_index: (1..=lines.len())
                .map(|line| crate::session::transcript_file::LineSpan {
                    line: line as u32,
                    seq: line as i64,
                    kind: "item/user".to_string(),
                    item_type: "message".to_string(),
                    is_header: false,
                    char_start_in_item: 0,
                    char_end_in_item: 0,
                })
                .collect(),
            lines: lines.iter().map(|line| line.to_string()).collect(),
            tool_names: Default::default(),
            result_call_ids: Default::default(),
            session_read_calls: Default::default(),
        }
    }

    /// A page that is entirely in the context window says so instead of
    /// handing back an unexplained empty list.
    #[test]
    fn a_page_already_in_context_counts_what_it_left_out() {
        let file = page_file(&["one", "two", "three"]);
        let result = render_projection_page(
            &file,
            &json!({"start_line": 1, "limit": 3}),
            &[1, 2, 3],
            "session://session-1",
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["lines"], json!([]));
        assert_eq!(data["skipped_in_context"], 3, "{data}");
        assert_eq!(data["has_more"], false, "{data}");
    }

    /// A partly hidden page leaves a gap; the count explains it.
    #[test]
    fn a_partly_hidden_page_explains_the_gap() {
        let file = page_file(&["one", "two", "three"]);
        let result = render_projection_page(
            &file,
            &json!({"start_line": 1, "limit": 3}),
            &[2],
            "session://session-1",
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["lines"], json!(["one", "three"]), "{data}");
        assert_eq!(data["skipped_in_context"], 1, "{data}");
    }

    #[test]
    fn test_read_start_line_past_end_resolves_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "only\n").unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "start_line": 5
        }));
        assert!(result.error_code.is_none(), "{}", result.content);
        assert!(page_lines(&result).is_empty());
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["start_line"], 5);
        assert_eq!(data["total_lines"], 1, "{data}");
        assert_eq!(data["has_more"], false, "{data}");
        assert_eq!(data["skipped_in_context"], 0, "{data}");
    }

    #[test]
    fn test_read_empty_file_past_end_resolves_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        std::fs::write(&file_path, "").unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "start_line": 3
        }));
        assert!(result.error_code.is_none(), "{}", result.content);
        assert!(page_lines(&result).is_empty());
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["total_lines"], 0, "{data}");
        assert_eq!(data["has_more"], false, "{data}");
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
        assert_eq!(data["total_lines"], 2500, "{data}");
        assert_eq!(data["has_more"], true, "{data}");
    }

    #[test]
    fn test_read_limit_and_start_line_are_clamped_with_notes() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").unwrap();
        let result = ReadTool::default().call(serde_json::json!({
            "path": file_path.to_str().unwrap(),
            "start_line": 0,
            "limit": 6000
        }));
        assert!(result.error_code.is_none(), "{}", result.content);
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["start_line"], 1);
        assert_eq!(page_lines(&result), ["line1", "line2", "line3"]);
        assert_eq!(
            data["clamped"],
            json!([
                {"parameter": "start_line", "requested": 0, "applied": 1},
                {"parameter": "limit", "requested": 6000, "applied": 5000},
            ]),
            "{data}"
        );
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
        assert_eq!(data["lines_capped"], json!([1]), "{data}");
        assert!(data["lines"][0].is_string());
    }
}

/// read paging and tool chains (merged from phase2_gate, WP-207).
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod chain_contract_tests {
    use std::path::Path;

    use serde_json::{Value, json};

    use crate::tool::Tool;
    use crate::tool::trait_::ToolExecutionContext;
    use crate::tools::edit::EditTool;
    use crate::tools::glob::GlobTool;
    use crate::tools::grep::GrepTool;
    use crate::tools::read::ReadTool;
    use crate::types::ToolCallResult;

    fn execution(root: &Path, path_mode: crate::workspace::ToolPathMode) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode,
            workspace_root: root.to_path_buf(),
            call_id: String::new(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: String::new(),
            session: None,
        }
    }

    fn call<T: Tool>(
        tool: &T,
        root: &Path,
        input: Value,
        path_mode: crate::workspace::ToolPathMode,
    ) -> ToolCallResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tool.execute(input, execution(root, path_mode)))
    }

    fn safe<T: Tool>(tool: &T, root: &Path, input: Value) -> ToolCallResult {
        call(tool, root, input, crate::workspace::ToolPathMode::Safe)
    }

    #[test]
    fn read_missing_file_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = safe(
            &ReadTool::default(),
            dir.path(),
            json!({"path": "missing.txt"}),
        );
        assert_eq!(result.error_code.as_deref(), Some("not_found"));
    }

    #[test]
    fn outside_workspace_paths_noted() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("note.txt"), "outside-body\n").unwrap();
        let listed = call(
            &GlobTool,
            workspace.path(),
            json!({"pattern": "*.txt", "path": outside.path().display().to_string()}),
            crate::workspace::ToolPathMode::All,
        );
        assert!(listed.error.is_none(), "{}", listed.content);
        let data = listed.data.unwrap();
        assert!(data.get("base").is_none());
        let path = data["paths"][0].as_str().unwrap().to_string();
        assert!(path.contains("note.txt"), "{path}");
        assert_eq!(data["outside_workspace"], json!(true), "{data}");
        assert!(path.starts_with('/') || path.contains(":\\"), "{path}");
        let read = call(
            &ReadTool::default(),
            workspace.path(),
            json!({"path": path}),
            crate::workspace::ToolPathMode::All,
        );
        assert!(read.error.is_none(), "{}", read.content);
        let file = read.data.unwrap();
        assert_eq!(file["lines"][0], "outside-body");
        assert!(file.get("file").is_none());
    }

    fn with_excluded_dir<R>(f: impl FnOnce() -> R) -> R {
        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("hidden_dir".into());
        let mut out = None;
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            out = Some(f());
        });
        out.expect("cache test body ran")
    }
}
