use serde_json::Value;
use similar::TextDiff;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;
use crate::types::ToolCallResult;

/// Minimum similarity ratio (0-100) to suggest a fuzzy match.
const SIMILARITY_THRESHOLD: u8 = 80;

/// Minimum line-by-line similarity ratio (0-100) for block mode alignment.
const LINE_SIMILARITY_THRESHOLD: u8 = 80;

pub struct EditTool {
    ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

impl EditTool {
    pub fn new() -> Self {
        Self { ide: None }
    }

    pub fn with_ide(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide: Some(ide) }
    }

    async fn finish_ok(
        &self,
        path: &str,
        msg: String,
        execution: &ToolExecutionContext,
        raw_path: &str,
        resolved: &std::path::Path,
    ) -> ToolCallResult {
        let result = lsp_feedback::maybe_append_local_lsp_errors(
            self.ide.as_ref().map(|ide| ide.engines.as_ref()),
            path,
            ToolCallResult::ok(msg),
        )
        .await;
        crate::tools::file_path::with_path_risk_warning(
            result,
            &execution.workspace_root,
            raw_path,
            resolved,
            "editing",
        )
    }

    fn test_execution_context(&self) -> ToolExecutionContext {
        ToolExecutionContext {
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
        }
    }
}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact file text to find. Do not include read's line-number prefix (`    N: `). Must match one place unless replace_all is true; add surrounding lines if it appears more than once."
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text. Use an empty string to delete old_string."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace all occurrences (default: false)"
                },
                "mode": {
                    "type": "string",
                    "enum": ["exact", "block"],
                    "description": "Edit mode: 'exact' for literal replacement, 'block' for line-aligned fuzzy matching (default: exact). Both modes use old_string and new_string."
                }
            },
            "required": ["file_path", "old_string", "new_string"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "file_path")?;
        crate::tool::require_nonempty_string(input, "old_string")?;
        crate::tool::require_string(input, "new_string")?;
        if let Some(mode) = input.get("mode") {
            let mode = crate::tool::require_string_value(mode, "mode")?;
            if mode != "exact" && mode != "block" {
                return Err(crate::tool::must_be_one_of(
                    "mode",
                    &["exact", "block"],
                    mode,
                ));
            }
        }
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> bool {
        // Deletion edit: new_string is empty
        input["new_string"].as_str().is_some_and(|s| s.is_empty())
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        match input["file_path"].as_str() {
            Some(path) if !path.is_empty() => {
                let key = crate::workspace::resolve_agent(workspace_root, path, path_mode)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string());
                vec![ResourceKey::File(key)]
            }
            _ => vec![],
        }
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        // Unit-test helper only. Production Agent turns enter through execute().
        let execution = self.test_execution_context();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(self.call_for_execution(input, execution))
    }

    fn description(&self, _ctx: &Context) -> String {
        "Replace text in a file. Exact matching treats CRLF/LF and common typography (smart quotes, dashes) as equivalent; block mode uses line-aligned fuzzy matching.".into()
    }
}

impl EditTool {
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
        let old_string = match crate::tool::require_nonempty_string(&input, "old_string") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };
        let new_string = match crate::tool::require_string(&input, "new_string") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };
        let path_display = path.to_string_lossy().into_owned();
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&path).ok());
        if let (Some(ide), true) = (&self.ide, workspace_relative.is_some()) {
            ide.sync_document_if_ready(&path).await;
        }
        let raw_bytes = match (&self.ide, &workspace_relative) {
            (Some(ide), Some(relative)) => match ide.workspace.read_file_bytes(relative) {
                Ok((_, bytes)) => bytes,
                Err(error) => return ToolCallResult::error(error.to_string()),
            },
            _ => match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return ToolCallResult::error(format!("read {path_display}: {error}"));
                }
            },
        };
        let decoded = match crate::workspace::text_codec::decode_utf8_bytes(&raw_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                return ToolCallResult::error(crate::workspace::text_codec::decode_error_for_path(
                    error,
                    &path_display,
                ));
            }
        };
        let content = decoded.text;
        if old_string == new_string {
            return ToolCallResult::error(format!(
                "No change: old_string and new_string are identical in {path_display}. This is a no-op; change new_string or skip the edit. Do not retry the same call."
            ));
        }
        let mode = input["mode"].as_str().unwrap_or("exact");
        let edited = match mode {
            "block" => match apply_block_edit(&path_display, &content, old_string, new_string) {
                Ok(content) => (content, format!("Edited {path_display}")),
                Err(msg) => return ToolCallResult::error(msg),
            },
            _ if input["replace_all"].as_bool().unwrap_or(false) => {
                match apply_exact_replace(&path_display, &content, old_string, new_string, true) {
                    Ok((edited_content, count)) => (
                        edited_content,
                        format!("Replaced {count} occurrences in {path_display}"),
                    ),
                    Err(msg) => return ToolCallResult::error(msg),
                }
            }
            _ => {
                match apply_exact_replace(&path_display, &content, old_string, new_string, false) {
                    Ok((edited_content, _)) => (edited_content, format!("Edited {path_display}")),
                    Err(msg) => return ToolCallResult::error(msg),
                }
            }
        };
        let to_write = crate::workspace::text_codec::reattach_utf8_bom(decoded.has_bom, &edited.0);
        match (&self.ide, workspace_relative) {
            (Some(ide), Some(relative)) => match ide.workspace.write_file(&relative, &to_write) {
                Ok(_) => {
                    ide.sync_document_if_ready(&path).await;
                    self.finish_ok(&path_display, edited.1, &execution, raw_path, &path)
                        .await
                }
                Err(error) => ToolCallResult::error(error.to_string()),
            },
            _ => match crate::workspace::file_ops::atomic_write(&path, &to_write) {
                Ok(()) => {
                    self.finish_ok(&path_display, edited.1, &execution, raw_path, &path)
                        .await
                }
                Err(error) => ToolCallResult::error(error.to_string()),
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Exact-mode helpers
// ---------------------------------------------------------------------------

fn not_found_message(path: &str, content: &str, old_string: &str, block_mode: bool) -> String {
    let mode_hint = if block_mode {
        "verify with read, or try exact mode"
    } else {
        "verify with read, or try block mode"
    };
    let mut parts = vec![format!("old_string not found in {path}.")];

    let mut cited_line: Option<usize> = None;
    if looks_like_read_line_prefixes(old_string) {
        let stripped = strip_read_line_prefixes(old_string);
        let stripped_lines =
            crate::workspace::text_codec::edit_match_line_numbers(content, &stripped);
        if stripped_lines.len() == 1 {
            cited_line = Some(stripped_lines[0]);
            parts.push(format!(
                "old_string includes read line-number prefixes (`    N: `), which are not in the file. Without those prefixes it matches once at line {}.",
                stripped_lines[0]
            ));
        } else if stripped_lines.len() > 1 {
            parts.push(format!(
                "old_string includes read line-number prefixes (`    N: `), which are not in the file. Without those prefixes it matches {} times (lines {}). Drop the prefixes and add unique surrounding lines, or set replace_all.",
                stripped_lines.len(),
                format_line_list(&stripped_lines)
            ));
        } else {
            parts.push(
                "old_string includes read line-number prefixes (`    N: `), which are not in the file. Copy only the text after the colon."
                    .into(),
            );
        }
    }

    if let Some((line, kind)) = unique_relaxed_line_match(content, old_string) {
        cited_line = Some(line);
        let file_line = content.lines().nth(line - 1).unwrap_or("");
        parts.push(format!(
            "Line {line} matches old_string except for {kind}:\n{}",
            truncate_hint(file_line, 160)
        ));
    }

    let similar = find_similar_text(content, old_string);
    if let Some((line, text)) = similar.as_ref() {
        cited_line = Some(*line);
        parts.push(format!(
            "Did you mean line {line}:\n{}",
            truncate_hint(text, 400)
        ));
    }

    if let Some(nearest) = nearest_match_hint(content, old_string) {
        let skip = cited_line
            .is_some_and(|line| nearest.starts_with(&format!("Nearest match: line {line}:")));
        if !skip {
            parts.push(nearest);
        }
    }

    if let Some(confusable) =
        crate::workspace::text_codec::confusable_miss_hint(content, old_string)
    {
        parts.push(confusable);
    }

    if content.is_empty() {
        parts.push("The file is empty.".into());
    }

    parts.push(mode_hint.into());
    parts.push(RETRY_GUIDANCE.into());
    parts.join("\n\n")
}

const RETRY_GUIDANCE: &str = "Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix.";

fn multiple_matches_message(path: &str, content: &str, old: &str) -> String {
    let lines = crate::workspace::text_codec::edit_match_line_numbers(content, old);
    if lines.is_empty() {
        return format!(
            "Found multiple matches for old_string in {path}. Use replace_all or add surrounding unique lines to old_string.\n\n{RETRY_GUIDANCE}"
        );
    }
    format!(
        "Found {} matches for old_string in {path} (lines {}). Use replace_all to change every occurrence, or add surrounding unique lines so only one place matches.\n\n{RETRY_GUIDANCE}",
        lines.len(),
        format_line_list(&lines)
    )
}

fn apply_exact_replace(
    path: &str,
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    match crate::workspace::text_codec::edit_preserving_replace(content, old, new, replace_all) {
        crate::workspace::text_codec::EditReplace::Applied { count, .. }
            if count > 1 && !replace_all =>
        {
            Err(multiple_matches_message(path, content, old))
        }
        crate::workspace::text_codec::EditReplace::Applied { content, count } => {
            Ok((content, count))
        }
        crate::workspace::text_codec::EditReplace::NotFound => {
            Err(not_found_message(path, content, old, false))
        }
        crate::workspace::text_codec::EditReplace::Ambiguous => {
            Err(ambiguous_confusable_message(path, content, old))
        }
    }
}

fn ambiguous_confusable_message(path: &str, _content: &str, _old: &str) -> String {
    format!(
        "old_string matched via Unicode typography normalization in {path}, but the match is ambiguous (partial or overlapping). Use a more specific old_string anchored on nearby ASCII (do not match a single '-' or '.' inside a dash or ellipsis).\n\n{RETRY_GUIDANCE}"
    )
}

fn looks_like_read_line_prefixes(old: &str) -> bool {
    let lines: Vec<&str> = old.lines().collect();
    if lines.is_empty() {
        return false;
    }
    let prefixed = lines
        .iter()
        .filter(|line| strip_one_read_prefix(line).is_some())
        .count();
    prefixed * 2 >= lines.len()
}

fn strip_read_line_prefixes(old: &str) -> String {
    old.lines()
        .map(|line| strip_one_read_prefix(line).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_one_read_prefix(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after = &rest[digits..];
    if let Some(stripped) = after.strip_prefix(": ") {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix(':') {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix("| ") {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix('|') {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix('→') {
        return Some(stripped.strip_prefix(' ').unwrap_or(stripped));
    }
    None
}

fn unique_relaxed_line_match(content: &str, old: &str) -> Option<(usize, &'static str)> {
    if old.lines().count() != 1 {
        return None;
    }
    let needle = old.trim();
    if needle.is_empty() {
        return None;
    }
    let collapsed = collapse_ws(needle);
    let mut trim_hits = Vec::new();
    let mut space_hits = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line == needle || line == old {
            continue;
        }
        if line.trim() == needle {
            trim_hits.push(i + 1);
        } else if collapse_ws(line) == collapsed {
            space_hits.push(i + 1);
        }
    }
    if trim_hits.len() == 1 {
        return Some((trim_hits[0], "leading/trailing whitespace"));
    }
    if space_hits.len() == 1 {
        return Some((space_hits[0], "internal spacing (tabs vs spaces)"));
    }
    None
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn nearest_match_hint(content: &str, old_string: &str) -> Option<String> {
    let keyword = old_string
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .filter(|w| w.chars().count() >= 4 && w.chars().any(|c| c.is_ascii_alphanumeric()))
        .max_by_key(|w| w.chars().count())
        .unwrap_or("");
    if keyword.is_empty() {
        return None;
    }
    content.lines().enumerate().find_map(|(i, line)| {
        line.contains(keyword).then(|| {
            format!(
                "Nearest match: line {}: {}",
                i + 1,
                truncate_hint(line.trim_end(), 160)
            )
        })
    })
}

fn format_line_list(lines: &[usize]) -> String {
    const MAX: usize = 8;
    if lines.len() <= MAX {
        return lines
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let shown: Vec<String> = lines[..MAX].iter().map(|n| n.to_string()).collect();
    format!("{} (and {} more)", shown.join(", "), lines.len() - MAX)
}

fn truncate_hint(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}…")
}

/// Search the file content for chunks similar to `old_string`.
/// Returns a suggested match if similarity >= SIMILARITY_THRESHOLD.
fn find_similar_text(content: &str, old_string: &str) -> Option<(usize, String)> {
    // Normalize CRLF so the similarity check compares against the same
    // LF-normalized candidate windows below.
    let normalized;
    let old_string = if old_string.contains("\r\n") {
        normalized = old_string.replace("\r\n", "\n");
        normalized.as_str()
    } else {
        old_string
    };

    if old_string.len() < 5 || content.len() < old_string.len() {
        return None;
    }

    let line_count = old_string.lines().count().max(1);
    let content_lines: Vec<&str> = content.lines().collect();
    let mut best_match: Option<(u8, usize, String)> = None;

    for (start, window) in content_lines.windows(line_count).enumerate() {
        let candidate = window.join("\n");
        let sim = compute_similarity(&candidate, old_string);
        if sim >= SIMILARITY_THRESHOLD {
            let current_best = best_match.as_ref().map(|(s, _, _)| *s).unwrap_or(0);
            if sim > current_best {
                best_match = Some((sim, start + 1, candidate));
            }
        }
    }

    best_match.map(|(_, line, text)| (line, text))
}

/// Compute similarity ratio (0-100) between two strings using `similar`.
fn compute_similarity(a: &str, b: &str) -> u8 {
    let diff = TextDiff::from_lines(a, b);
    let ratio = diff.ratio();
    (ratio * 100.0) as u8
}

// ---------------------------------------------------------------------------
// Block-mode helpers — line-aligned fuzzy matching
// ---------------------------------------------------------------------------

/// Apply a block edit with line alignment.
///
/// 1. Try EOL-agnostic exact match, then typography-confusable fallback.
/// 2. If that fails, try line-number alignment via sliding window.
fn apply_block_edit(
    path: &str,
    content: &str,
    old: &str,
    new: &str,
) -> std::result::Result<String, String> {
    match crate::workspace::text_codec::edit_preserving_replace(content, old, new, false) {
        crate::workspace::text_codec::EditReplace::Applied { count, .. } if count > 1 => {
            return Err(multiple_matches_message(path, content, old));
        }
        crate::workspace::text_codec::EditReplace::Applied { content, count: 1 } => {
            return Ok(content);
        }
        crate::workspace::text_codec::EditReplace::Ambiguous => {
            return Err(ambiguous_confusable_message(path, content, old));
        }
        crate::workspace::text_codec::EditReplace::NotFound
        | crate::workspace::text_codec::EditReplace::Applied { .. } => {}
    }

    let (bodies, eols) = crate::workspace::text_codec::split_keep_eol(content);
    let search_lines: Vec<&str> = old.lines().collect();

    if search_lines.is_empty() {
        return Err("empty old_string in block mode".into());
    }

    let match_result = find_best_line_match(&bodies, &search_lines);

    match match_result {
        Some((start, end)) => {
            let new_lines: Vec<&str> = new.lines().collect();
            let region_eol = eols
                .get(start)
                .copied()
                .filter(|eol| !eol.is_empty())
                .or_else(|| eols.iter().copied().find(|eol| !eol.is_empty()))
                .unwrap_or("\n");
            let last_replaced_eol = eols.get(end.saturating_sub(1)).copied().unwrap_or("");

            let mut out_bodies: Vec<&str> = Vec::new();
            let mut out_eols: Vec<&str> = Vec::new();
            out_bodies.extend_from_slice(&bodies[..start]);
            out_eols.extend_from_slice(&eols[..start]);
            for (j, line) in new_lines.iter().enumerate() {
                out_bodies.push(line);
                if j + 1 < new_lines.len() {
                    out_eols.push(region_eol);
                } else {
                    out_eols.push(last_replaced_eol);
                }
            }
            out_bodies.extend_from_slice(&bodies[end..]);
            out_eols.extend_from_slice(&eols[end..]);
            Ok(crate::workspace::text_codec::join_keep_eol(
                &out_bodies,
                &out_eols,
            ))
        }
        None => Err(not_found_message(path, content, old, true)),
    }
}

/// Find the best matching line range in `content_lines` for `search_lines`
/// using a sliding window and line-by-line similarity.
///
/// Returns `Some((start, end))` where `end` is exclusive, or `None` if no
/// match exceeds `LINE_SIMILARITY_THRESHOLD`.
fn find_best_line_match(content_lines: &[&str], search_lines: &[&str]) -> Option<(usize, usize)> {
    let search_len = search_lines.len();
    if search_len == 0 || content_lines.len() < search_len {
        return None;
    }

    let mut best_start: Option<usize> = None;
    let mut best_similarity: u8 = 0;

    // Sliding window of size search_len
    for start in 0..=(content_lines.len() - search_len) {
        let end = start + search_len;
        let window = &content_lines[start..end];

        let sim = compute_line_similarity(window, search_lines);
        if sim >= LINE_SIMILARITY_THRESHOLD && sim > best_similarity {
            best_similarity = sim;
            best_start = Some(start);
        }
    }

    // Also try slightly larger windows (content may have extra lines due to edits)
    for extra in 1..=3 {
        if content_lines.len() < search_len + extra {
            break;
        }
        for start in 0..=(content_lines.len() - search_len - extra) {
            let end = start + search_len + extra;
            let window = &content_lines[start..end];

            let sim = compute_line_similarity(window, search_lines);
            // Require slightly higher threshold for expanded windows
            let threshold = LINE_SIMILARITY_THRESHOLD.saturating_add(extra as u8 * 2);
            if sim >= threshold && sim > best_similarity {
                best_similarity = sim;
                best_start = Some(start);
            }
        }
    }

    best_start.map(|start| (start, start + search_len))
}

/// Compute similarity ratio (0-100) between two slices of lines by comparing
/// each line pair using character-level similarity.
fn compute_line_similarity(a_lines: &[&str], b_lines: &[&str]) -> u8 {
    if a_lines.is_empty() && b_lines.is_empty() {
        return 100;
    }
    if a_lines.is_empty() || b_lines.is_empty() {
        return 0;
    }

    // Use the longer line count to iterate; compare what we can
    let max_len = a_lines.len().max(b_lines.len());
    let mut total_similarity: f64 = 0.0;

    for i in 0..max_len {
        let line_sim = match (a_lines.get(i), b_lines.get(i)) {
            (Some(a), Some(b)) => {
                if a == b {
                    100.0
                } else {
                    compute_similarity(a, b) as f64
                }
            }
            (None, _) | (_, None) => 0.0,
        };
        total_similarity += line_sim;
    }

    (total_similarity / max_len as f64) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_fuzzy_edit_preserves_crlf_and_trailing_newline() {
        // "a\nb" is not a raw substring of CRLF content, so this exercises the
        // line-aligned path that previously joined with "\n".
        let content = "a\r\nb\r\nc\r\n";
        let result = apply_block_edit("t", content, "a\nb", "X").unwrap();
        assert_eq!(result, "X\r\nc\r\n");
    }

    #[test]
    fn block_fuzzy_edit_preserves_lf_without_trailing_newline() {
        // "b\nc\n" is not a raw substring (content has no trailing newline),
        // so this exercises the line-aligned path on an LF file.
        let content = "a\nb\nc";
        let result = apply_block_edit("t", content, "b\nc\n", "X").unwrap();
        assert_eq!(result, "a\nX");
    }

    #[test]
    fn exact_replace_matches_lf_old_in_crlf_file() {
        let content = "a\r\nb\r\n";
        let (result, count) =
            crate::workspace::text_codec::eol_preserving_replace(content, "a\nb", "X", false);
        assert_eq!(count, 1);
        assert_eq!(result, "X\r\n");
    }

    #[test]
    fn exact_replace_keeps_lf_file_as_lf() {
        let content = "a\nb\n";
        let (result, count) =
            crate::workspace::text_codec::eol_preserving_replace(content, "a\nb", "X", false);
        assert_eq!(count, 1);
        assert_eq!(result, "X\n");
    }

    #[test]
    fn mixed_eol_exact_replace_keeps_unmatched() {
        let content = "a\r\nb\nc\n";
        let (result, count) =
            crate::workspace::text_codec::eol_preserving_replace(content, "b", "B", false);
        assert_eq!(count, 1);
        assert_eq!(result, "a\r\nB\nc\n");
    }

    #[test]
    fn edit_tool_roundtrips_bom_and_matches_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.rs");
        std::fs::write(&path, "\u{feff}fn main() {}\n").unwrap();
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": "fn main() {}",
            "new_string": "fn start() {}"
        }));
        assert!(
            !result.content.starts_with("Error:"),
            "edit should match without BOM in old_string, got: {}",
            result.content
        );
        let on_disk = std::fs::read(&path).unwrap();
        assert!(
            on_disk.starts_with(&[0xEF, 0xBB, 0xBF]),
            "BOM must be written back"
        );
        assert_eq!(
            String::from_utf8(on_disk).unwrap(),
            "\u{feff}fn start() {}\n"
        );
    }

    #[test]
    fn edit_tool_rejects_nul_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin.txt");
        std::fs::write(&path, b"ok\x00still-utf8").unwrap();
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": "ok",
            "new_string": "no"
        }));
        assert!(result.content.to_lowercase().contains("binary"));
        assert_eq!(std::fs::read(&path).unwrap(), b"ok\x00still-utf8");
    }

    #[test]
    fn edit_tool_rejects_utf16() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u16.txt");
        std::fs::write(&path, [0xFF, 0xFE, b'A', 0x00]).unwrap();
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": "A",
            "new_string": "B"
        }));
        assert!(result.content.contains("UTF-16"));
    }

    #[test]
    fn block_fuzzy_keeps_mixed_eol_unmatched() {
        let content = "a\r\nbb\nc\n";
        let result = apply_block_edit("t", content, "zz", "X");
        assert!(result.is_err());
        let result = apply_block_edit("t", content, "bb", "BB").unwrap();
        assert_eq!(result, "a\r\nBB\nc\n");
    }

    #[test]
    fn exact_replace_smart_quotes_in_file() {
        let content = "msg = \u{201C}hello\u{201D}\n";
        let (_, count) = crate::workspace::text_codec::eol_preserving_replace(
            content,
            "\"hello\"",
            "\"hi\"",
            false,
        );
        assert_eq!(count, 0, "byte-exact must miss smart quotes");
        match crate::workspace::text_codec::edit_preserving_replace(
            content,
            "\"hello\"",
            "\"hi\"",
            false,
        ) {
            crate::workspace::text_codec::EditReplace::Applied { content, count } => {
                assert_eq!(count, 1);
                assert_eq!(content, "msg = \"hi\"\n");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn edit_tool_replaces_smart_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(&path, "say \u{201C}hello\u{201D}\n").unwrap();
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": "\"hello\"",
            "new_string": "\"hi\""
        }));
        assert!(
            !result.content.starts_with("Error:"),
            "got: {}",
            result.content
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "say \"hi\"\n");
    }

    #[test]
    fn edit_tool_rejects_partial_em_dash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dash.txt");
        std::fs::write(&path, "foo\u{2014}bar\n").unwrap();
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": "-",
            "new_string": "="
        }));
        assert!(
            result.content.contains("ambiguous"),
            "got: {}",
            result.content
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo\u{2014}bar\n");
    }

    #[test]
    fn not_found_read_prefix_view() {
        let content = "fn start() {}\nfn main() {}\n";
        let msg = not_found_message("t.rs", content, "     2: fn main() {}", false);
        assert_eq!(
            msg,
            "old_string not found in t.rs.\n\n\
old_string includes read line-number prefixes (`    N: `), which are not in the file. Without those prefixes it matches once at line 2.\n\n\
verify with read, or try block mode\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }

    #[test]
    fn not_found_nearest_keyword_view() {
        let content = "alpha\nunique_identifier_xyz = 1\nomega\n";
        let msg = not_found_message("t.rs", content, "unique_identifier_xyz = 2", false);
        assert_eq!(
            msg,
            "old_string not found in t.rs.\n\n\
Nearest match: line 2: unique_identifier_xyz = 1\n\n\
verify with read, or try block mode\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }

    #[test]
    fn multiple_matches_view() {
        let msg = multiple_matches_message("t.rs", "foo\nbar\nfoo\n", "foo");
        assert_eq!(
            msg,
            "Found 2 matches for old_string in t.rs (lines 1, 3). Use replace_all to change every occurrence, or add surrounding unique lines so only one place matches.\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }

    fn assert_tool_wire_error(path: &std::path::Path, old: &str, new: &str, body: &str) {
        let tool = EditTool::new();
        let result = tool.call(serde_json::json!({
            "file_path": path.to_str().unwrap(),
            "old_string": old,
            "new_string": new
        }));
        assert_eq!(result.content, format!("Error: {body}"));
    }

    #[test]
    fn edit_tool_wire_view_prefixes_error_and_matches_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        let content = "fn main() {}\n";
        std::fs::write(&path, content).unwrap();
        let display = path.to_string_lossy();
        let body = not_found_message(&display, content, "     1: fn main() {}", false);
        assert_tool_wire_error(&path, "     1: fn main() {}", "fn start() {}", &body);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn edit_tool_wire_view_multiple_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        let content = "foo\nbar\nfoo\n";
        std::fs::write(&path, content).unwrap();
        let display = path.to_string_lossy();
        let body = multiple_matches_message(&display, content, "foo");
        assert_tool_wire_error(&path, "foo", "FOO", &body);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn edit_tool_wire_view_identical_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let display = path.to_string_lossy();
        assert_tool_wire_error(
            &path,
            "fn main() {}",
            "fn main() {}",
            &format!(
                "No change: old_string and new_string are identical in {display}. This is a no-op; change new_string or skip the edit. Do not retry the same call."
            ),
        );
    }

    #[test]
    fn edit_tool_wire_view_ambiguous_em_dash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dash.txt");
        let content = "foo\u{2014}bar\n";
        std::fs::write(&path, content).unwrap();
        let display = path.to_string_lossy();
        let body = ambiguous_confusable_message(&display, content, "-");
        assert_tool_wire_error(&path, "-", "=", &body);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
    }

    #[test]
    fn not_found_indent_mismatch_view() {
        let msg = not_found_message("t.rs", "    foo()\n", "foo()", false);
        assert_eq!(
            msg,
            "old_string not found in t.rs.\n\n\
Line 1 matches old_string except for leading/trailing whitespace:\n    foo()\n\n\
verify with read, or try block mode\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }

    #[test]
    fn not_found_empty_file_view() {
        let msg = not_found_message("t.rs", "", "hello", false);
        assert_eq!(
            msg,
            "old_string not found in t.rs.\n\n\
The file is empty.\n\n\
verify with read, or try block mode\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }

    #[test]
    fn ambiguous_em_dash_view() {
        let msg = ambiguous_confusable_message("t.rs", "foo\u{2014}bar\n", "-");
        assert_eq!(
            msg,
            "old_string matched via Unicode typography normalization in t.rs, but the match is ambiguous (partial or overlapping). Use a more specific old_string anchored on nearby ASCII (do not match a single '-' or '.' inside a dash or ellipsis).\n\n\
Do not retry the same old_string; re-read the file and copy the exact bytes after read's line-number prefix."
        );
    }
}
