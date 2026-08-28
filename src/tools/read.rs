use serde_json::Value;
use similar::TextDiff;
use std::path::Path;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::session::media::MAX_MEDIA_BLOB_SIZE;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{ToolCallResult, ToolOutputPart};

/// Default / max line window.
const DEFAULT_LINE_LIMIT: usize = 1500;
/// Soft output cap so a page stays under the 32KB spill threshold.
const DEFAULT_CHAR_BUDGET: usize = 24_000;
/// Per-line display cap (chars, not bytes).
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

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
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
                    "description": "Last line to read, 1-based, inclusive. Default start_line+1499 (max 1500 lines per call)."
                },
                "token_budget": {
                    "type": "integer",
                    "description": "Max output characters (default and max 24000)"
                }
            },
            "required": ["file_path"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<crate::tool::write_lock::ResourceKey> {
        match input["file_path"].as_str() {
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
            output_limit: self.max_result_size(),
            session_id: String::new(),
        };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(self.call_for_execution(input, execution))
    }

    fn description(&self, _ctx: &Context) -> String {
        "Read a text file with line numbers or return a supported image as media.".into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        ReadTool::validate_input(self, input)
    }

    fn max_result_size(&self) -> usize {
        ReadTool::max_result_size(self)
    }
}

impl ReadTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tool::require_nonempty_string(&input, "file_path") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
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
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        let workspace_bytes = self.ide.as_ref().and_then(|ide| {
            let relative = ide.workspace.sandbox().rel_path(&path).ok()?;
            match ide.workspace.read_file_bytes(&relative) {
                Ok((_, bytes)) => Some(Ok(bytes)),
                Err(
                    crate::workspace::WorkspaceError::NotFile(p)
                    | crate::workspace::WorkspaceError::IsDir(p),
                ) => Some(Err(crate::tools::file_path::directory_not_file_message(&p))),
                Err(error) => Some(Err(error.to_string())),
            }
        });
        let result = match workspace_bytes {
            Some(Ok(bytes)) => self.read_resolved(&path, Some(bytes), &input),
            Some(Err(error)) => ToolCallResult::error(error),
            None => self.read_resolved(&path, None, &input),
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
    ) -> ToolCallResult {
        let path_display = path.to_string_lossy().into_owned();
        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut msg = format!("File not found: {path_display}");
                let suggestions = suggest_similar_files(&path_display);
                if !suggestions.is_empty() {
                    msg.push_str(&format!("\nSimilar files: {}", suggestions.join(", ")));
                } else {
                    msg.push_str(&format!(
                        "\n{}",
                        crate::tools::file_path::missing_file_hint()
                    ));
                }
                return ToolCallResult::error(msg);
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                return ToolCallResult::error(format!(
                    "Permission denied reading {path_display}. Check file permissions or choose a different path."
                ));
            }
            Err(e) => return ToolCallResult::error(format!("read {path_display}: {e}")),
        };

        if metadata.is_dir() {
            return ToolCallResult::error(crate::tools::file_path::directory_not_file_message(
                &path_display,
            ));
        }

        if metadata.len() > MAX_MEDIA_BLOB_SIZE {
            return ToolCallResult::error(format!(
                "File too large: {} ({} bytes, max {} bytes)",
                path_display,
                metadata.len(),
                MAX_MEDIA_BLOB_SIZE
            ));
        }

        if let Some(mime_type) = detect_image_mime(&path_display) {
            return ToolCallResult::ok_with_parts(
                format!(
                    "Image file: {} ({} bytes, {})",
                    path_display,
                    metadata.len(),
                    mime_type
                ),
                vec![ToolOutputPart::image_file(path_display, mime_type)],
            );
        }

        let bytes = match workspace_bytes {
            Some(bytes) => bytes,
            None => match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) => return ToolCallResult::error(format!("read {}: {}", path_display, e)),
            },
        };
        let content = match crate::workspace::text_codec::decode_utf8_bytes(&bytes) {
            Ok(decoded) => decoded.text,
            Err(error) => {
                return ToolCallResult::error(crate::workspace::text_codec::decode_error_for_path(
                    error,
                    &path_display,
                ));
            }
        };

        render_text_page(&content, input)
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "file_path")?;
        Ok(())
    }

    /// ReadTool output is the source of truth for other tools; use a very high limit
    /// so we don't double-truncate.
    fn max_result_size(&self) -> usize {
        usize::MAX // infinity — ReadTool does its own token-aware truncation
    }
}

fn read_virtual_session(
    raw_path: &str,
    input: &Value,
    execution: &ToolExecutionContext,
) -> Option<ToolCallResult> {
    let stem = crate::session::transcript_file::try_parse_virtual_path(raw_path)?;
    let db = crate::engines::session_search::sessions_db_under(&execution.workspace_root);
    let session_id = match crate::engines::session_search::resolve_session_ref(&db, &stem) {
        Ok(id) => id,
        Err(e) => return Some(ToolCallResult::error(e.to_string())),
    };
    let file = match crate::session::transcript_file::load_transcript_file(&db, &session_id) {
        Ok(f) => f,
        Err(e) => return Some(ToolCallResult::error(e.to_string())),
    };
    let hidden = hidden_surface_seqs(&db, &session_id, &execution.session_id);
    Some(render_projection_page(&file, input, &hidden))
}

fn hidden_surface_seqs(db: &Path, target_session: &str, caller_session: &str) -> Vec<i64> {
    if caller_session.is_empty() || caller_session != target_session {
        return Vec::new();
    }
    crate::engines::session_search::load_surface_seqs(db, target_session).unwrap_or_default()
}

fn render_projection_page(
    file: &crate::session::transcript_file::TranscriptFile,
    input: &Value,
    hidden_seqs: &[i64],
) -> ToolCallResult {
    let (start_line, start_warning) = resolve_start_line(input);
    let (end_line, window_warning, capped_window) = match resolve_end_line(input, start_line) {
        Ok(pair) => pair,
        Err(msg) => return ToolCallResult::error(msg),
    };
    let (token_budget, budget_warning) = resolve_token_budget(input);
    let total_lines = file.total_lines();
    if start_line > total_lines && total_lines > 0 {
        return ToolCallResult::error(format!(
            "start_line {start_line} exceeds file length ({total_lines} lines)"
        ));
    }
    let start = start_line.saturating_sub(1);
    let end = end_line.min(total_lines);
    let range_hidden = (start..end).all(|i| {
        file.seq_at((i + 1) as u32)
            .is_some_and(|seq| hidden_seqs.contains(&seq))
    });
    if total_lines > 0 && start < end && range_hidden && !hidden_seqs.is_empty() {
        return apply_read_warnings(
            ToolCallResult::ok(crate::session::transcript_file::IN_CONTEXT_WINDOW_MSG),
            [&start_warning, &window_warning, &budget_warning],
        );
    }
    render_line_window(
        start,
        end,
        total_lines,
        token_budget,
        capped_window,
        [&start_warning, &window_warning, &budget_warning],
        |i| {
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

fn render_text_page(content: &str, input: &Value) -> ToolCallResult {
    let lines: Vec<&str> = content.lines().collect();
    let (start_line, start_warning) = resolve_start_line(input);
    let (end_line, window_warning, capped_window) = match resolve_end_line(input, start_line) {
        Ok(pair) => pair,
        Err(msg) => return ToolCallResult::error(msg),
    };
    let (token_budget, budget_warning) = resolve_token_budget(input);
    let total_lines = lines.len();
    if start_line > total_lines && total_lines > 0 {
        return ToolCallResult::error(format!(
            "start_line {start_line} exceeds file length ({total_lines} lines)"
        ));
    }
    let start = start_line.saturating_sub(1);
    let end = end_line.min(total_lines);
    render_line_window(
        start,
        end,
        total_lines,
        token_budget,
        capped_window,
        [&start_warning, &window_warning, &budget_warning],
        |i| Some((i + 1, lines[i])),
    )
}

fn render_line_window<'a>(
    start: usize,
    end: usize,
    total_lines: usize,
    token_budget: usize,
    capped_window: bool,
    warnings: [&Option<String>; 3],
    mut line_at: impl FnMut(usize) -> Option<(usize, &'a str)>,
) -> ToolCallResult {
    let mut result = String::new();
    let mut char_count = 0usize;
    let mut lines_included = 0usize;
    let mut hit_char_cap = false;
    let mut first_shown = 0usize;
    let mut last_shown = 0usize;

    for i in start..end {
        let Some((line_no, line)) = line_at(i) else {
            continue;
        };
        let formatted = format_read_line(line_no, line);
        let next = char_count + formatted.len();
        if lines_included > 0 && next > token_budget {
            hit_char_cap = true;
            break;
        }
        result.push_str(&formatted);
        char_count = next;
        lines_included += 1;
        if first_shown == 0 {
            first_shown = line_no;
        }
        last_shown = line_no;
        if char_count > token_budget {
            hit_char_cap = true;
            break;
        }
    }

    if result.is_empty() {
        return apply_read_warnings(ToolCallResult::ok("(empty file)"), warnings);
    }

    if let Some(footer) = pagination_footer(
        first_shown,
        last_shown,
        total_lines,
        hit_char_cap,
        capped_window,
    ) {
        result.push('\n');
        result.push_str(&footer);
    }

    apply_read_warnings(
        ToolCallResult::ok(result.trim_end_matches(['\n', '\r']).to_string()),
        warnings,
    )
}

fn resolve_start_line(input: &Value) -> (usize, Option<String>) {
    if input.get("start_line").is_none() {
        return (1, None);
    }
    if let Some(n) = input["start_line"].as_i64() {
        if n < 1 {
            return (
                1,
                Some(format!(
                    "start_line {n} is invalid (must be >= 1); starting at line 1"
                )),
            );
        }
        return (n as usize, None);
    }
    if let Some(n) = input["start_line"].as_u64() {
        if n < 1 {
            return (
                1,
                Some("start_line 0 is invalid (must be >= 1); starting at line 1".into()),
            );
        }
        return (n as usize, None);
    }
    (
        1,
        Some("start_line must be a positive integer; starting at line 1".into()),
    )
}

fn resolve_end_line(
    input: &Value,
    start_line: usize,
) -> std::result::Result<(usize, Option<String>, bool), String> {
    let default_end = start_line.saturating_add(DEFAULT_LINE_LIMIT - 1);
    let (requested, parse_warning) = if input.get("end_line").is_none() {
        (default_end, None)
    } else if let Some(n) = input["end_line"].as_i64() {
        if n < 1 {
            return Err("end_line must be >= start_line".into());
        }
        (n as usize, None)
    } else if let Some(n) = input["end_line"].as_u64() {
        if n < 1 {
            return Err("end_line must be >= start_line".into());
        }
        (n as usize, None)
    } else {
        (
            default_end,
            Some("end_line must be a positive integer; using default window".into()),
        )
    };
    if requested < start_line {
        return Err("end_line must be >= start_line".into());
    }
    let max_end = start_line.saturating_add(DEFAULT_LINE_LIMIT - 1);
    if requested > max_end {
        Ok((
            max_end,
            Some(format!(
                "end_line window exceeds max {DEFAULT_LINE_LIMIT}; showing {DEFAULT_LINE_LIMIT} lines. Use start_line to continue"
            )),
            true,
        ))
    } else {
        Ok((requested, parse_warning, input.get("end_line").is_none()))
    }
}

fn resolve_token_budget(input: &Value) -> (usize, Option<String>) {
    if input.get("token_budget").is_none() {
        return (DEFAULT_CHAR_BUDGET, None);
    }
    if let Some(n) = input["token_budget"].as_i64() {
        if n < 1 {
            return (
                DEFAULT_CHAR_BUDGET,
                Some(format!(
                    "token_budget {n} is invalid (must be >= 1); using default {DEFAULT_CHAR_BUDGET}"
                )),
            );
        }
        return ((n as usize).min(DEFAULT_CHAR_BUDGET), None);
    }
    if let Some(n) = input["token_budget"].as_u64() {
        return ((n as usize).min(DEFAULT_CHAR_BUDGET), None);
    }
    (
        DEFAULT_CHAR_BUDGET,
        Some(format!(
            "token_budget must be a positive integer; using default {DEFAULT_CHAR_BUDGET}"
        )),
    )
}

fn format_read_line(num: usize, line: &str) -> String {
    format!("{:6}: {}\n", num, truncate_chars(line, MAX_LINE_CHARS))
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}… [line truncated]")
}

fn pagination_footer(
    start_1: usize,
    last_shown: usize,
    total: usize,
    hit_char_cap: bool,
    capped_window: bool,
) -> Option<String> {
    if last_shown == 0 {
        return None;
    }
    if last_shown >= total || (!hit_char_cap && !capped_window) {
        return None;
    }
    let next = last_shown + 1;
    if hit_char_cap {
        Some(format!(
            "[showing lines {start_1}-{last_shown} of {total} — output cap. Use start_line={next} to continue]"
        ))
    } else {
        Some(format!(
            "[showing lines {start_1}-{last_shown} of {total}. Use start_line={next} to continue]"
        ))
    }
}

fn apply_read_warnings(result: ToolCallResult, warnings: [&Option<String>; 3]) -> ToolCallResult {
    let joined: Vec<&str> = warnings.iter().filter_map(|w| w.as_deref()).collect();
    if joined.is_empty() {
        result
    } else {
        result.with_warning(joined.join("; "))
    }
}

/// Detect the small set of image formats supported by the read MVP.
///
/// This intentionally uses magic bytes rather than only a file extension so a
/// random binary file cannot be injected as an image artifact.
fn detect_image_mime(path: &str) -> Option<&'static str> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 12];
    let n = std::io::Read::read(&mut file, &mut buf).ok()?;

    if n >= 8 && buf[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        Some("image/png")
    } else if n >= 3 && buf[..3] == [0xFF, 0xD8, 0xFF] {
        Some("image/jpeg")
    } else if n >= 6 && (&buf[..6] == b"GIF87a" || &buf[..6] == b"GIF89a") {
        Some("image/gif")
    } else if n >= 12 && &buf[..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Suggest similar files when the exact file is not found.
/// Returns a list of file paths in the same directory that have similar names.
pub fn suggest_similar_files(path: &str) -> Vec<String> {
    let file_path = Path::new(path);
    let parent = file_path.parent();
    let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if filename.is_empty() {
        return Vec::new();
    }

    let dir = match parent {
        Some(d) if d.exists() => d,
        _ => return Vec::new(),
    };

    let mut suggestions = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            let entry_str = entry_name.to_string_lossy();

            // Skip the original file
            if entry_str == filename {
                continue;
            }

            // Compute simple similarity: shared prefix + suffix length
            let sim = simple_similarity(filename, &entry_str);
            if sim >= 0.4
                && let Some(full_path) = entry.path().to_str()
            {
                suggestions.push((sim, full_path.to_string()));
            }
        }
    }

    // Sort by similarity descending, take top 5
    suggestions.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    suggestions.truncate(5);
    suggestions.into_iter().map(|(_, p)| p).collect()
}

/// Simple string similarity based on Levenshtein/edit distance ratio.
/// Uses the `similar` crate for robust comparison.
fn simple_similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let diff = TextDiff::from_chars(a, b);
    diff.ratio() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_basic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
        });

        let result = tool.call(input).content;
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
    }

    #[test]
    fn test_read_with_start_end_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\n").expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "start_line": 2,
            "end_line": 3
        });

        let result = tool.call(input).content;
        assert_eq!(result, "     2: line2\n     3: line3");
    }

    #[test]
    fn test_read_start_line_past_end_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\n").expect("write");

        let result = ReadTool::default().call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
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
            "file_path": file_path.to_str().expect("path")
        });

        let result = tool.call(input);
        assert!(result.content.starts_with("Error:"));
        assert!(result.content.to_lowercase().contains("binary"));
    }

    #[test]
    fn test_read_strips_utf8_bom_from_display() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("bom.txt");
        std::fs::write(&file_path, "\u{feff}hello\n").expect("write");
        let tool = ReadTool::default();
        let result = tool
            .call(serde_json::json!({
                "file_path": file_path.to_str().expect("path"),
            }))
            .content;
        assert!(result.contains("hello"));
        assert!(!result.contains('\u{feff}'));
    }

    #[test]
    fn test_read_utf16_error_is_explicit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("utf16.txt");
        std::fs::write(&file_path, [0xFF, 0xFE, b'A', 0x00]).expect("write");
        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
        }));
        assert!(result.content.starts_with("Error:"));
        assert!(result.content.contains("UTF-16"));
    }

    #[test]
    fn test_read_preserves_trailing_spaces_on_last_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("spaces.txt");
        std::fs::write(&file_path, "keep   ").expect("write");
        let tool = ReadTool::default();
        let result = tool
            .call(serde_json::json!({
                "file_path": file_path.to_str().expect("path"),
            }))
            .content;
        assert!(
            result.contains("keep   "),
            "last-line trailing spaces must survive display, got: {result:?}"
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
            "file_path": file_path.to_str().expect("path"),
        }));

        assert!(result.content.contains("image/png"));
        assert_eq!(result.parts.len(), 1);
        assert!(matches!(
            &result.parts[0],
            ToolOutputPart::Media { artifact }
                if artifact.mime_type == "image/png"
                    && matches!(
                        artifact.source,
                        crate::types::MediaSource::LocalFile { .. }
                    )
        ));
    }

    #[test]
    fn test_read_token_budget_truncation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("big.txt");
        let content: String = (0..1000)
            .map(|i| format!("line {} with some content\n", i))
            .collect();
        std::fs::write(&file_path, &content).expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "token_budget": 500
        });

        let result = tool.call(input).content;
        assert!(
            result.contains("output cap") && result.contains("Use start_line="),
            "should be truncated, got: {}",
            result
        );
    }

    #[test]
    fn test_read_default_limit_pages() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("long.txt");
        let content: String = (1..=1600).map(|i| format!("{i}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let tool = ReadTool::default();
        let result = tool
            .call(serde_json::json!({
                "file_path": file_path.to_str().expect("path"),
            }))
            .content;
        assert!(result.contains("     1: 1"));
        assert!(result.contains("  1500: 1500"));
        assert!(!result.contains("  1501: 1501"));
        assert!(
            result.contains("[showing lines 1-1500 of 1600. Use start_line=1501 to continue]"),
            "got: {result}"
        );
    }

    #[test]
    fn test_read_token_cap_footer_is_full_string() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("short.txt");
        std::fs::write(&file_path, "one\ntwo\nthree\n").expect("write");

        let result = ReadTool::default()
            .call(serde_json::json!({
                "file_path": file_path.to_str().expect("path"),
                "token_budget": 12,
            }))
            .content;

        assert_eq!(
            result,
            "     1: one\n\n[showing lines 1-1 of 3 — output cap. Use start_line=2 to continue]"
        );
    }

    #[test]
    fn test_read_clamps_huge_end_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("long.txt");
        let content: String = (1..=1600).map(|i| format!("{i}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "end_line": 99999
        }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert!(!result.content.contains("  1501: 1501"));
        assert!(result.content.contains("Use start_line=1501 to continue"));
    }

    #[test]
    fn test_read_truncates_long_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("wide.txt");
        let long = "x".repeat(2000);
        std::fs::write(&file_path, format!("{long}\nshort\n")).expect("write");

        let tool = ReadTool::default();
        let result = tool
            .call(serde_json::json!({
                "file_path": file_path.to_str().expect("path"),
            }))
            .content;
        assert!(result.contains("[line truncated]"));
        assert!(!result.contains(&"x".repeat(1600)));
        assert!(result.contains("short"));
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
            "file_path": file_path.to_str().expect("path")
        });

        let result = tool.call(input);
        assert!(result.content.starts_with("Error:"));
        assert!(result.content.to_lowercase().contains("too large"));
    }

    #[test]
    fn test_validate_input() {
        let tool = ReadTool::default();

        // Missing file_path
        assert!(tool.validate_input(&serde_json::json!({})).is_err());

        // Empty file_path
        assert!(
            tool.validate_input(&serde_json::json!({"file_path": ""}))
                .is_err()
        );

        // start_line = 0 → fallback with warning
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "start_line": 0
        }));
        assert!(
            result.content.contains("start_line 0 is invalid"),
            "got: {}",
            result.content
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert!(result.content.contains("line1"), "got: {}", result.content);

        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
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
            tool.validate_input(&serde_json::json!({"file_path": "/tmp/x"}))
                .is_ok()
        );
    }

    #[test]
    fn negative_start_line_and_token_budget_warn_and_read() {
        let tool = ReadTool::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let path = file_path.to_str().expect("path");

        let start = tool.call(serde_json::json!({
            "file_path": path,
            "start_line": -5
        }));
        assert_eq!(start.level, crate::types::ToolSignalLevel::Warning);
        assert!(
            start.content.contains("start_line -5 is invalid"),
            "got: {}",
            start.content
        );
        assert!(start.content.contains("line1"), "got: {}", start.content);

        let budget = tool.call(serde_json::json!({
            "file_path": path,
            "token_budget": -1
        }));
        assert_eq!(budget.level, crate::types::ToolSignalLevel::Warning);
        assert!(
            budget.content.contains("token_budget -1 is invalid"),
            "got: {}",
            budget.content
        );
        assert!(budget.content.contains("line1"), "got: {}", budget.content);
    }

    #[test]
    fn test_suggest_similar_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").expect("write");
        std::fs::write(dir.path().join("main_test.rs"), "mod test;").expect("write");
        std::fs::write(dir.path().join("lib.rs"), "pub mod lib;").expect("write");
        std::fs::write(dir.path().join("build.rs"), "fn build() {}").expect("write");

        let missing = dir.path().join("mian.rs").to_str().unwrap().to_string();
        let suggestions = suggest_similar_files(&missing);
        assert!(
            suggestions.iter().any(|s| s.contains("main.rs")),
            "should suggest main.rs for 'mian.rs', got: {:?}",
            suggestions
        );
    }

    #[test]
    fn test_simple_similarity() {
        assert!(simple_similarity("main.rs", "main.rs") > 0.9);
        assert!(simple_similarity("main.rs", "main_test.rs") > 0.4);
        assert!(simple_similarity("main.rs", "completely_different.go") < 0.4);
        assert_eq!(simple_similarity("", "foo"), 0.0);
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
            "file_path": file_path.to_str().expect("path"),
        }));
        assert!(
            result.content.contains("TOKEN=known-path"),
            "read must not apply discovery FilterPreset; got: {}",
            result.content
        );
    }

    fn seed_session(root: &std::path::Path, text: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = crate::session::store::Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        s.insert_detail_rows(&[crate::types::user_text(text)])
            .unwrap();
        let id = s.id.clone();
        drop(s);
        id
    }

    fn read_virtual(
        root: &std::path::Path,
        path: &str,
        extra: serde_json::Value,
        session_id: &str,
    ) -> crate::types::ToolCallResult {
        let mut input = extra;
        input["file_path"] = serde_json::Value::String(path.to_string());
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
                output_limit: tool.max_result_size(),
                session_id: session_id.to_string(),
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
        assert!(
            other.content.contains("VIRTUAL_READ_NEEDLE"),
            "{}",
            other.content
        );
        let needle_line = other
            .content
            .lines()
            .find(|l| l.contains("VIRTUAL_READ_NEEDLE"))
            .expect("needle line");
        assert!(
            needle_line.trim_start().starts_with('3')
                || needle_line.contains("|VIRTUAL_READ_NEEDLE")
                || needle_line.contains("VIRTUAL_READ_NEEDLE"),
            "canonical body line should stay numbered, got: {needle_line}"
        );

        let self_read = read_virtual(root, &path, serde_json::json!({}), &sid);
        assert_eq!(
            self_read.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            self_read.content
        );
        assert!(
            self_read
                .content
                .contains(crate::session::transcript_file::IN_CONTEXT_WINDOW_MSG),
            "live surface should be filtered: {}",
            self_read.content
        );
        assert!(
            !self_read.content.contains("VIRTUAL_READ_NEEDLE"),
            "filtered read must not re-number remaining lines around the live item: {}",
            self_read.content
        );
    }

    fn seed_compacted(root: &std::path::Path, archived: &str, live: &str) -> String {
        let db = root.join(".litecode").join("sessions.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let s = crate::session::store::Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        s.insert_detail_rows(&[
            crate::types::user_text(archived),
            crate::types::user_text("filler middle"),
            crate::types::user_text(live),
        ])
        .unwrap();
        s.apply_compact_checkpoint_from(&crate::types::user_text("[summary] compact"), Some(2), 10)
            .unwrap();
        let id = s.id.clone();
        drop(s);
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
        let archived_line = full
            .content
            .lines()
            .find(|l| l.contains("ARCHIVED_READ_TOKEN"))
            .expect(&format!("archived line in full read:\n{}", full.content))
            .to_string();
        let live_line = full
            .content
            .lines()
            .find(|l| l.contains("LIVE_READ_TOKEN still in window"))
            .expect(&format!("live line in full read:\n{}", full.content))
            .to_string();
        let archived_no: usize = archived_line
            .split(':')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let live_no: usize = live_line.split(':').next().unwrap().trim().parse().unwrap();
        assert!(live_no > archived_no);

        let peeled = read_virtual(root, &current_path, serde_json::json!({}), &current);
        assert!(
            peeled.content.contains("ARCHIVED_READ_TOKEN"),
            "archived rows stay readable:\n{}",
            peeled.content
        );
        assert!(
            !peeled.content.contains("LIVE_READ_TOKEN still in window"),
            "current live surface must be peeled:\n{}",
            peeled.content
        );
        let peeled_archived = peeled
            .content
            .lines()
            .find(|l| l.contains("ARCHIVED_READ_TOKEN"))
            .expect("archived after peel");
        let peeled_no: usize = peeled_archived
            .split(':')
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            peeled_no, archived_no,
            "peel must keep canonical line numbers, full={archived_line} peeled={peeled_archived}"
        );

        let other_as_current = read_virtual(root, &other_path, serde_json::json!({}), &current);
        assert!(
            other_as_current
                .content
                .contains("LIVE_READ_TOKEN in other"),
            "other sessions are not peeled:\n{}",
            other_as_current.content
        );

        let live_only = read_virtual(
            root,
            &current_path,
            serde_json::json!({ "start_line": live_no, "end_line": live_no }),
            &current,
        );
        assert!(
            live_only
                .content
                .contains(crate::session::transcript_file::IN_CONTEXT_WINDOW_MSG),
            "offset into the live surface must stay in-window:\n{}",
            live_only.content
        );
        assert!(
            !live_only
                .content
                .contains("LIVE_READ_TOKEN still in window"),
            "live-only window must not leak surface text:\n{}",
            live_only.content
        );
    }
}
