use serde_json::Value;
use similar::TextDiff;
use std::path::Path;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::session::media::MAX_MEDIA_BLOB_SIZE;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{ToolCallResult, ToolOutputPart};

/// Default / max line window. Omitted `limit` no longer means “read to EOF”.
const DEFAULT_LINE_LIMIT: usize = 1500;
/// Soft output cap so a page stays under the 32KB spill threshold.
const DEFAULT_CHAR_BUDGET: usize = 24_000;
/// Per-line display cap (chars, not bytes).
const MAX_LINE_CHARS: usize = 1500;
/// When `limit` is 0 or negative, return this many lines and attach a Warning.
const INVALID_LIMIT_FALLBACK: usize = 50;

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
                    "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                },
                "offset": {
                    "type": "integer",
                    "description": "Start line (1-indexed, default 1)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max lines after offset (default and max 1500). Use offset to page."
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
            && let Some(ide) = &self.ide {
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

        let (offset, offset_warning) = resolve_offset(input);
        let (limit, limit_warning) = resolve_limit(input);
        let (token_budget, budget_warning) = resolve_token_budget(input);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        if offset > total_lines && total_lines > 0 {
            return ToolCallResult::error(format!(
                "offset {} exceeds file length ({} lines)",
                offset, total_lines
            ));
        }

        let start = offset - 1;
        let end = (start + limit).min(total_lines);

        let mut result = String::new();
        let mut char_count = 0usize;
        let mut lines_included = 0usize;
        let mut hit_char_cap = false;

        for (i, line) in lines[start..end].iter().enumerate() {
            let formatted = format_read_line(start + i + 1, line);
            let next = char_count + formatted.len();
            if lines_included > 0 && next > token_budget {
                hit_char_cap = true;
                break;
            }
            result.push_str(&formatted);
            char_count = next;
            lines_included += 1;
            if char_count > token_budget {
                hit_char_cap = true;
                break;
            }
        }

        if result.is_empty() {
            return apply_read_warnings(
                ToolCallResult::ok("(empty file)"),
                [&offset_warning, &limit_warning, &budget_warning],
            );
        }

        if let Some(footer) =
            pagination_footer(start + 1, start + lines_included, total_lines, hit_char_cap)
        {
            result.push('\n');
            result.push_str(&footer);
        }

        apply_read_warnings(
            ToolCallResult::ok(result.trim_end_matches(['\n', '\r']).to_string()),
            [&offset_warning, &limit_warning, &budget_warning],
        )
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

fn resolve_offset(input: &Value) -> (usize, Option<String>) {
    if input.get("offset").is_none() {
        return (1, None);
    }
    if let Some(n) = input["offset"].as_i64() {
        if n < 1 {
            return (
                1,
                Some(format!(
                    "offset {n} is invalid (must be >= 1); starting at line 1"
                )),
            );
        }
        return (n as usize, None);
    }
    if let Some(n) = input["offset"].as_u64() {
        if n < 1 {
            return (
                1,
                Some("offset 0 is invalid (must be >= 1); starting at line 1".into()),
            );
        }
        return (n as usize, None);
    }
    (
        1,
        Some("offset must be a positive integer; starting at line 1".into()),
    )
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

fn resolve_limit(input: &Value) -> (usize, Option<String>) {
    if input.get("limit").is_none() {
        return (DEFAULT_LINE_LIMIT, None);
    }
    if let Some(n) = input["limit"].as_i64() {
        if n <= 0 {
            return (
                INVALID_LIMIT_FALLBACK,
                Some(format!(
                    "limit {n} is invalid (must be >= 1); showing first {INVALID_LIMIT_FALLBACK} lines instead"
                )),
            );
        }
        return clamp_line_limit(n as usize);
    }
    if let Some(n) = input["limit"].as_u64() {
        return clamp_line_limit(n as usize);
    }
    (
        INVALID_LIMIT_FALLBACK,
        Some(format!(
            "limit must be a positive integer; showing first {INVALID_LIMIT_FALLBACK} lines instead"
        )),
    )
}

fn clamp_line_limit(n: usize) -> (usize, Option<String>) {
    if n > DEFAULT_LINE_LIMIT {
        (
            DEFAULT_LINE_LIMIT,
            Some(format!(
                "limit {n} exceeds max {DEFAULT_LINE_LIMIT}; showing {DEFAULT_LINE_LIMIT} lines. Use offset to continue"
            )),
        )
    } else {
        (n, None)
    }
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
) -> Option<String> {
    if last_shown == 0 {
        return None;
    }
    if last_shown >= total && !hit_char_cap {
        return None;
    }
    let next = last_shown + 1;
    if hit_char_cap {
        Some(format!(
            "[showing lines {start_1}-{last_shown} of {total} — output cap. Use offset={next} to continue]"
        ))
    } else {
        Some(format!(
            "[showing lines {start_1}-{last_shown} of {total}. Use offset={next} to continue]"
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
    fn test_read_with_offset_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\nline4\nline5\n").expect("write");

        let tool = ReadTool::default();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "offset": 2,
            "limit": 2
        });

        let result = tool.call(input).content;
        assert!(result.contains("2: line2"));
        assert!(result.contains("3: line3"));
        assert!(!result.contains("line1"));
        assert!(!result.contains("line4"));
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
            result.contains("output cap") && result.contains("Use offset="),
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
            result.contains("[showing lines 1-1500 of 1600. Use offset=1501 to continue]"),
            "got: {result}"
        );
    }

    #[test]
    fn test_read_clamps_huge_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("long.txt");
        let content: String = (1..=1600).map(|i| format!("{i}\n")).collect();
        std::fs::write(&file_path, &content).expect("write");

        let tool = ReadTool::default();
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "limit": 99999
        }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert!(!result.content.contains("  1501: 1501"));
        assert!(result.content.contains("Use offset=1501 to continue"));
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

        // offset = 0 → fallback with warning (same family as limit = 0)
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "offset": 0
        }));
        assert!(
            result.content.contains("offset 0 is invalid"),
            "got: {}",
            result.content
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
        assert!(result.content.contains("line1"), "got: {}", result.content);

        // limit = 0 → fallback with warning (not a hard validation error)
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let result = tool.call(serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "limit": 0
        }));
        assert!(
            result.content.contains("invalid"),
            "got: {}",
            result.content
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);

        // Valid
        assert!(
            tool.validate_input(&serde_json::json!({"file_path": "/tmp/x"}))
                .is_ok()
        );
    }

    #[test]
    fn negative_offset_and_token_budget_warn_and_read() {
        let tool = ReadTool::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("lines.txt");
        std::fs::write(&file_path, "line1\nline2\nline3\n").expect("write");
        let path = file_path.to_str().expect("path");

        let offset = tool.call(serde_json::json!({
            "file_path": path,
            "offset": -5
        }));
        assert_eq!(offset.level, crate::types::ToolSignalLevel::Warning);
        assert!(
            offset.content.contains("offset -5 is invalid"),
            "got: {}",
            offset.content
        );
        assert!(offset.content.contains("line1"), "got: {}", offset.content);

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
}
