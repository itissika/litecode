//! Agent `grep` tool — Zed-aligned narrow LexicalLane frontend.
//!
//! Schema is regex / include_pattern / path / offset / case_sensitive / no_ignore / expand.
//! Default view is matching lines grouped by file (glob order). `expand: true` uses
//! Markdown match cards: headings `crumb › L…` when tree-sitter resolves scopes.
//! Human workspace Search continues to use LexicalLane via the retrieval facade.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::engines::code_search::{
    LexicalMatch, LexicalQuery, enclosing_scopes, format_breadcrumb, lexical_search_with_preset,
    lines_slice, syntax_ancestor_snippet,
};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, Result, ToolCallResult};
use crate::workspace::ToolPathMode;
use crate::workspace::filter::FilterPreset;

/// Matches Zed `RESULTS_PER_PAGE`.
const RESULTS_PER_PAGE: usize = 20;
/// Fixed context lines around each hit when ancestor expansion is unavailable.
const CONTEXT_LINES: usize = 2;
/// Display cap per snippet line (chars).
const SNIPPET_LINE_MAX_CHARS: usize = 240;
pub struct GrepTool;

impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "regex": {
                    "type": "string",
                    "description": "Regular expression matched against file contents (not paths). Parsed as a Rust/ripgrep regex."
                },
                "include_pattern": {
                    "type": "string",
                    "description": "Optional path glob (e.g. **/*.rs), not content pattern"
                },
                "path": {
                    "type": "string",
                    "description": "Optional directory or single file to search (workspace-relative preferred; absolute paths outside the workspace only under All permission). For a directory, regex/include_pattern are matched relative to it."
                },
                "offset": {
                    "type": "integer",
                    "description": "0-based match index; 20 matches per page (default 0)"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether the regex is case-sensitive (default: false)."
                },
                "no_ignore": {
                    "type": "boolean",
                    "description": "When true, search without .gitignore / files.exclude / search.exclude (default: false)."
                },
                "expand": {
                    "type": "boolean",
                    "description": "If true, expand each hit into a code snippet (syntax ancestor up to 6 lines, otherwise ±2 lines) with headings. Default false: matching lines only. Prefer path/include_pattern to narrow before setting expand."
                }
            },
            "required": ["regex"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(std::future::ready(
            self.call_for_execution(input, execution),
        ))
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        // Unit-test helper only. Production Agent turns enter through execute().
        self.call_for_execution(
            input,
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: crate::config::workspace::workspace_root_lap(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: self.max_result_size(),
                session_id: String::new(),
            },
        )
    }

    fn description(&self, _ctx: &Context) -> String {
        "Search file contents with a regular expression; optional `path` scopes to a directory or single file (workspace-relative preferred)."
            .into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let regex = crate::tool::require_nonempty_string(input, "regex")?;
        let case_sensitive = input["case_sensitive"].as_bool().unwrap_or(false);
        if compile_regex_preview(regex, case_sensitive).is_err() {
            return Err("parameter 'regex' is not a valid regular expression".into());
        }
        Ok(())
    }

    fn max_result_size(&self) -> usize {
        // Page size is the primary budget; keep a generous char ceiling for snippets.
        50_000
    }
}

fn compile_regex_preview(
    pattern: &str,
    case_sensitive: bool,
) -> std::result::Result<(), regex::Error> {
    let mut re_str = String::new();
    if !case_sensitive {
        re_str.push_str("(?i)");
    }
    re_str.push_str(pattern);
    regex::Regex::new(&re_str).map(|_| ())
}

impl GrepTool {
    fn call_for_execution(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        match run_grep(&input, &execution.workspace_root, execution.path_mode) {
            Ok(output) => ToolCallResult::ok(output),
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }
}

fn run_grep(input: &Value, workspace_root: &Path, path_mode: ToolPathMode) -> Result<String> {
    let regex = crate::tool::require_nonempty_string(input, "regex")
        .map_err(LitecodeError::ToolExecution)?;

    let include_pattern = input["include_pattern"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let expand = input["expand"].as_bool().unwrap_or(false);
    let offset = input["offset"].as_u64().map(|o| o as usize).unwrap_or(0);
    let case_sensitive = input["case_sensitive"].as_bool().unwrap_or(false);
    let no_ignore = input["no_ignore"].as_bool().unwrap_or(false);
    let preset = if no_ignore {
        FilterPreset::Unfiltered
    } else {
        FilterPreset::AgentText
    };

    // Search root is the turn's workspace, or the `path` arg resolved under the
    // tool's permission mode: All admits absolute outside-workspace paths, Safe
    // rejects them here (and in the SAFE preset's explicit deny rule).
    // A file `path` is scoped via LexicalQuery.path under its parent directory so
    // match paths stay relative and format_zed_page can read sources.
    let resolved = match input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw_path) => crate::workspace::resolve_agent(workspace_root, raw_path, path_mode)
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?,
        None => crate::config::path::canon_abs_lossy(workspace_root),
    };
    let file_scoped = resolved.is_file();
    let (root, file_scope): (PathBuf, Option<PathBuf>) = if file_scoped {
        let parent = resolved
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "path has no parent directory: {}",
                    resolved.display()
                ))
            })?;
        (parent.to_path_buf(), Some(resolved))
    } else if resolved.is_dir() {
        (resolved, None)
    } else {
        return Err(LitecodeError::ToolExecution(format!(
            "path does not exist: {}",
            resolved.display()
        )));
    };
    // Fetch a large set, sort by glob path key + line, then page. Encounter
    // order from the walker is not the view order.
    let ctx_lines = if expand { CONTEXT_LINES } else { 0 };
    let outcome = lexical_search_with_preset(
        &LexicalQuery {
            pattern: regex.to_string(),
            root: root.clone(),
            path: file_scope,
            case_sensitive,
            whole_word: false,
            is_regex: true,
            include: include_pattern,
            exclude: None,
            multiline: false,
            max_matches: usize::MAX,
            before_context: ctx_lines,
            after_context: ctx_lines,
            search_hidden: false,
        },
        preset,
    )?;

    if outcome.matches.is_empty() {
        if let Some(ref pat) = input["include_pattern"].as_str().filter(|s| !s.is_empty())
            && outcome.files_searched == 0
        {
            return Ok(format!(
                "No files matched include_pattern '{pat}'. Use forward slashes; multi-ext like '**/*.ts,**/*.tsx'; or omit include_pattern."
            ));
        }
        if outcome.files_searched > 0 {
            return Ok(format!(
                "No matches found (searched {} files).",
                outcome.files_searched
            ));
        }
        if file_scoped {
            return Ok(
                "No matches found (the file was not searched — it may be treated as binary, excluded by ignore rules, or the search index skipped it)."
                    .to_string(),
            );
        }
        return Ok("No matches found".to_string());
    }

    let mut matches = outcome.matches;
    sort_grep_matches(&mut matches);

    let total = matches.len();
    if offset >= total {
        let last = (total.saturating_sub(1) / RESULTS_PER_PAGE) * RESULTS_PER_PAGE;
        return Ok(format!(
            "offset {offset} past end ({total} matches); try offset {last}"
        ));
    }

    if expand {
        Ok(format_zed_page(&root, &matches, offset))
    } else {
        Ok(format_compact_page(&matches, offset))
    }
}

fn sort_grep_matches(matches: &mut [LexicalMatch]) {
    matches.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.path)
            .cmp(&crate::workspace::glob_hit_key(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });
}

fn wrap_grep_page(body: &str, offset: usize, shown: usize, has_more: bool) -> String {
    if shown == 0 {
        return String::new();
    }
    if has_more {
        format!(
            "Showing matches {}-{} (there were more matches found; use offset: {} to see next page):\n{body}",
            offset + 1,
            offset + shown,
            offset + RESULTS_PER_PAGE,
        )
    } else {
        format!("Found {shown} matches:\n{body}")
    }
}

fn format_compact_page(matches: &[LexicalMatch], offset: usize) -> String {
    if matches.is_empty() {
        return String::new();
    }
    let page_end = (offset + RESULTS_PER_PAGE).min(matches.len());
    let page = &matches[offset..page_end];
    let has_more = matches.len() > page_end;

    let mut body = String::new();
    let mut current: Option<&str> = None;
    for m in page {
        if current != Some(m.path.as_str()) {
            body.push_str(&m.path);
            body.push('\n');
            current = Some(&m.path);
        }
        let text = truncate_snippet_lines(m.line_text.trim_end_matches(['\n', '\r']));
        body.push_str(&format!("  {:>6}:{text}\n", m.start_line));
    }
    wrap_grep_page(&body, offset, page.len(), has_more)
}

/// Zed grep-panel style: page grouped by file (`## Matches in {path}`) with
/// AST-breadcrumb headings per hit. File order is glob_hit_key (same as compact).
fn format_zed_page(root: &Path, matches: &[LexicalMatch], offset: usize) -> String {
    if matches.is_empty() {
        return String::new();
    }

    let page_end = (offset + RESULTS_PER_PAGE).min(matches.len());
    let page = &matches[offset..page_end];
    let has_more = matches.len() > page_end;

    let mut file_order: Vec<String> = Vec::new();
    let mut by_file: BTreeMap<String, Vec<&LexicalMatch>> = BTreeMap::new();
    for m in page {
        if !by_file.contains_key(&m.path) {
            file_order.push(m.path.clone());
        }
        by_file.entry(m.path.clone()).or_default().push(m);
    }

    // Per-file source cache for AST breadcrumbs + ancestor snippets.
    let mut file_sources: HashMap<String, Option<String>> = HashMap::new();

    let mut body = String::new();
    for path in file_order {
        let Some(file_matches) = by_file.get(&path) else {
            continue;
        };
        body.push_str(&format!("\n## Matches in {path}\n"));

        let source = file_sources
            .entry(path.clone())
            .or_insert_with(|| std::fs::read_to_string(root.join(&path)).ok())
            .as_deref();

        let ranges = merge_snippet_ranges(file_matches, source, &path);
        for range in ranges {
            let line_label = if range.start_line == range.end_line {
                format!("L{}", range.start_line)
            } else {
                format!("L{}-{}", range.start_line, range.end_line)
            };
            let heading = match source
                .and_then(|src| format_breadcrumb(&enclosing_scopes(&path, src, range.hit_line)))
            {
                Some(crumb) => format!("\n### {crumb} › {line_label}"),
                None => format!("\n### {line_label}"),
            };
            body.push_str(&heading);
            body.push('\n');
            body.push_str(&fence_block(&range.text));
            if range.remaining_lines > 0 {
                body.push_str(&format!(
                    "\n{} lines remaining in ancestor node. Read the file to see all.\n",
                    range.remaining_lines
                ));
            }
        }
    }

    let shown = page.len();
    wrap_grep_page(&body, offset, shown, has_more)
}

struct SnippetRange {
    start_line: u32,
    end_line: u32,
    /// Primary match line used for enclosing-scope lookup.
    hit_line: u32,
    text: String,
    remaining_lines: u32,
}

fn merge_snippet_ranges(
    file_matches: &[&LexicalMatch],
    source: Option<&str>,
    path: &str,
) -> Vec<SnippetRange> {
    let mut ranges: Vec<SnippetRange> = Vec::new();
    for m in file_matches {
        let built = snippet_for_match(m, source, path);
        if let Some(last) = ranges.last_mut()
            && built.start_line <= last.end_line.saturating_add(1)
        {
            if built.end_line > last.end_line {
                last.end_line = built.end_line;
                last.text = if let Some(src) = source {
                    lines_slice(src, last.start_line, last.end_line)
                } else {
                    merge_snippet_text(&last.text, &built.text)
                };
            }
            // Keep the larger remaining hint if either side was truncated.
            last.remaining_lines = last.remaining_lines.max(built.remaining_lines);
            continue;
        }
        ranges.push(built);
    }
    ranges
}

fn snippet_for_match(m: &LexicalMatch, source: Option<&str>, path: &str) -> SnippetRange {
    let match_end = m.end_line.max(m.start_line);

    if let Some(src) = source
        && let Some(ancestor) = syntax_ancestor_snippet(path, src, m.start_line, match_end)
    {
        return SnippetRange {
            start_line: ancestor.start_line,
            end_line: ancestor.end_line,
            hit_line: m.start_line,
            text: lines_slice(src, ancestor.start_line, ancestor.end_line),
            remaining_lines: ancestor.remaining_lines,
        };
    }

    // Fallback: ±CONTEXT_LINES from lexical hit (and optional source rebuild).
    if let Some(src) = source {
        let line_count = src.lines().count() as u32;
        let start = m.start_line.saturating_sub(CONTEXT_LINES as u32).max(1);
        let end = match_end
            .saturating_add(CONTEXT_LINES as u32)
            .min(line_count.max(1));
        return SnippetRange {
            start_line: start,
            end_line: end,
            hit_line: m.start_line,
            text: lines_slice(src, start, end),
            remaining_lines: 0,
        };
    }

    let (start, end, text) = snippet_from_lexical_fields(m);
    SnippetRange {
        start_line: start,
        end_line: end,
        hit_line: m.start_line,
        text,
        remaining_lines: 0,
    }
}

fn snippet_from_lexical_fields(m: &LexicalMatch) -> (u32, u32, String) {
    let mut lines: BTreeMap<u32, String> = BTreeMap::new();
    let first = m.start_line;
    let last = m.end_line.max(m.start_line);

    for (i, line) in m.context_before.iter().enumerate() {
        let line_no = first.saturating_sub((m.context_before.len() - i) as u32);
        if line_no >= 1 {
            lines.insert(line_no, line.clone());
        }
    }

    let match_text = m.line_text.trim_end_matches('\n');
    if match_text.contains('\n') {
        for (i, line) in match_text.split('\n').enumerate() {
            lines.insert(first + i as u32, line.to_string());
        }
    } else {
        lines.insert(first, match_text.to_string());
    }

    for (i, line) in m.context_after.iter().enumerate() {
        lines.insert(last + 1 + i as u32, line.clone());
    }

    if lines.is_empty() {
        return (first, last, String::new());
    }
    let start = *lines.keys().next().unwrap();
    let end = *lines.keys().next_back().unwrap();
    let mut text = String::new();
    for (i, (_ln, line)) in lines.iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        text.push_str(line);
    }
    (start, end, text)
}

fn merge_snippet_text(a: &str, b: &str) -> String {
    // Naive union by lines: keep unique lines in order of first appearance.
    let mut out: Vec<&str> = Vec::new();
    for line in a.split('\n').chain(b.split('\n')) {
        if !out.contains(&line) {
            out.push(line);
        }
    }
    out.join("\n")
}

fn fence_block(snippet: &str) -> String {
    let snippet = truncate_snippet_lines(snippet);
    // Fence longer than any backtick run inside the snippet (Zed pattern).
    let mut longest = 2usize;
    let mut run = 0usize;
    for ch in snippet.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let ticks = "`".repeat(longest + 1);
    format!("{ticks}\n{snippet}\n{ticks}\n")
}

fn truncate_snippet_lines(snippet: &str) -> String {
    snippet
        .lines()
        .map(|line| {
            if line.chars().count() <= SNIPPET_LINE_MAX_CHARS {
                line.to_string()
            } else {
                let kept: String = line.chars().take(SNIPPET_LINE_MAX_CHARS).collect();
                format!("{kept}… (line truncated)")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn with_cwd<R>(dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
        let _guard = CWD_LOCK.lock().expect("cwd lock");
        let prev = std::env::current_dir().expect("prev cwd");
        std::env::set_current_dir(dir).expect("set cwd");
        let out = f();
        let _ = std::env::set_current_dir(prev);
        out
    }

    fn call_in(dir: &std::path::Path, input: Value) -> String {
        call_in_mode(dir, input, crate::workspace::ToolPathMode::Safe)
    }

    fn call_in_mode(
        dir: &std::path::Path,
        input: Value,
        path_mode: crate::workspace::ToolPathMode,
    ) -> String {
        // Prefer explicit execution context (production path) over cwd coupling.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(GrepTool.execute(
            input,
            ToolExecutionContext {
                path_mode,
                workspace_root: dir.to_path_buf(),
                call_id: String::new(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: GrepTool.max_result_size(),
                session_id: String::new(),
            },
        ))
        .content
    }

    #[test]
    fn test_grep_basic_zed_shape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hello.txt"),
            "the quick brown fox\njumps over the lazy dog\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("other.txt"), "nothing interesting\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "quick brown", "expand": true }),
        );
        assert!(result.contains("## Matches in hello.txt"), "got: {result}");
        assert!(result.contains("### L1"), "got: {result}");
        assert!(result.contains("quick brown"), "got: {result}");
        assert!(!result.contains("other.txt"), "got: {result}");
        assert!(
            !result.contains('›'),
            "unsupported .txt must fall back to line-only, got: {result}"
        );
        assert!(
            !result.lines().any(|l| l.trim_start().starts_with('>')),
            "old > contract must be gone, got: {result}"
        );
        assert!(result.starts_with("Found "), "got: {result}");
        // Zed blank line between file header and match heading.
        assert!(
            result.contains("## Matches in hello.txt\n\n### "),
            "expected blank line before ###, got: {result}"
        );
    }

    #[test]
    fn test_grep_default_groups_path_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hello.txt"),
            "the quick brown fox\njumps over the lazy dog\nquick brown again\n",
        )
        .unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "quick brown" }));
        assert_eq!(
            result,
            "Found 2 matches:\nhello.txt\n       1:the quick brown fox\n       3:quick brown again\n"
        );
        assert!(!result.contains("## Matches in"));
        assert!(!result.contains("###"));
    }

    #[test]
    fn test_grep_default_glob_file_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/tools")).unwrap();
        std::fs::write(dir.path().join("src/tools/read.rs"), "needle here\n").unwrap();
        std::fs::write(dir.path().join("z.md"), "needle here\n").unwrap();
        std::fs::write(dir.path().join("a.md"), "needle here\nneedle two\n").unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "needle here\n").unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "needle" }));
        assert_eq!(
            result,
            concat!(
                "Found 5 matches:\n",
                "a.md\n",
                "       1:needle here\n",
                "       2:needle two\n",
                "z.md\n",
                "       1:needle here\n",
                "src/a.rs\n",
                "       1:needle here\n",
                "src/tools/read.rs\n",
                "       1:needle here\n",
            )
        );
    }

    #[test]
    fn test_grep_expanded_uses_glob_file_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("z.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("src/a.txt"), "needle\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "needle", "expand": true }),
        );
        let a = result.find("## Matches in a.txt").expect("a.txt heading");
        let z = result.find("## Matches in z.txt").expect("z.txt heading");
        let nested = result
            .find("## Matches in src/a.txt")
            .expect("src/a.txt heading");
        assert!(
            a < z && z < nested,
            "expected glob file order, got: {result}"
        );
    }

    #[test]
    fn test_grep_case_sensitive_default_false() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("g.txt"), "Hello World\n").unwrap();

        let insensitive = call_in(dir.path(), serde_json::json!({ "regex": "hello world" }));
        assert!(
            insensitive.contains("Hello World"),
            "default case_sensitive=false, got: {insensitive}"
        );

        let sensitive = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "hello world",
                "case_sensitive": true
            }),
        );
        assert_eq!(sensitive, "No matches found (searched 1 files).");
    }

    #[test]
    fn test_grep_include_pattern_braces() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("web/src")).unwrap();
        std::fs::write(dir.path().join("web/src/a.ts"), "notificationStore\n").unwrap();
        std::fs::write(dir.path().join("web/src/b.tsx"), "notificationStore\n").unwrap();
        std::fs::write(dir.path().join("web/src/c.rs"), "notificationStore\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "notificationStore",
                "include_pattern": "**/*.{ts,tsx}"
            }),
        );
        assert!(result.contains("a.ts"), "got: {result}");
        assert!(result.contains("b.tsx"), "got: {result}");
        assert!(!result.contains("c.rs"), "got: {result}");
    }

    #[test]
    fn test_grep_include_pattern_empty_scope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.rs"), "needle\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "needle",
                "include_pattern": "**/*.{ts,tsx}"
            }),
        );
        assert!(
            result.contains("No files matched include_pattern"),
            "got: {result}"
        );
    }

    #[test]
    fn test_grep_include_pattern() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "fn main() {}").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "main",
                "include_pattern": "*.rs"
            }),
        );
        assert!(result.contains("code.rs"), "got: {result}");
        assert!(!result.contains("notes.txt"), "got: {result}");
    }

    #[test]
    fn test_validate_input() {
        let tool = GrepTool;
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
        assert!(
            tool.validate_input(&serde_json::json!({"regex": ""}))
                .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"regex": "[invalid"}))
                .is_err()
        );
        assert!(
            tool.validate_input(&serde_json::json!({"regex": "["}))
                .unwrap_err()
                .contains("parameter 'regex' is not a valid regular expression")
        );
        assert!(
            tool.validate_input(&serde_json::json!({"regex": "hello"}))
                .is_ok()
        );
        // Old field name is not accepted as the required key.
        assert!(
            tool.validate_input(&serde_json::json!({"pattern": "hello"}))
                .is_err()
        );
    }

    #[test]
    fn test_no_matches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("t.txt"), "hello\n").unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "nonexistent_xyz" }),
        );
        assert_eq!(result, "No matches found (searched 1 files).");
    }

    #[test]
    fn test_grep_pagination_twenty_per_page() {
        let dir = tempfile::tempdir().unwrap();
        let content: String = (0..45).map(|i| format!("item{i:02}\n")).collect();
        std::fs::write(dir.path().join("many.txt"), &content).unwrap();

        let page1 = call_in(dir.path(), serde_json::json!({ "regex": "item\\d+" }));
        let expected_page1_prefix = concat!(
            "Showing matches 1-20 (there were more matches found; use offset: 20 to see next page):\n",
            "many.txt\n",
            "       1:item00\n",
            "       2:item01\n",
        );
        assert!(page1.starts_with(expected_page1_prefix), "got: {page1}");
        assert!(page1.contains("      20:item19\n"), "got: {page1}");
        assert!(!page1.contains("item20"), "got: {page1}");
        assert!(!page1.contains("## Matches in"), "got: {page1}");

        let page2 = call_in(
            dir.path(),
            serde_json::json!({ "regex": "item\\d+", "offset": 20 }),
        );
        assert!(
            page2.starts_with(
                "Showing matches 21-40 (there were more matches found; use offset: 40 to see next page):\nmany.txt\n      21:item20\n"
            ),
            "got: {page2}"
        );
        assert!(page2.contains("      40:item39\n"), "got: {page2}");
        assert!(!page2.contains("item00"), "got: {page2}");
    }

    #[test]
    fn test_offset_past_end() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("t.txt"), "foo\n").unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "foo", "offset": 9999 }),
        );
        assert!(result.contains("offset 9999 past end (1 matches)"));
        assert!(result.contains("try offset 0"));
    }

    #[test]
    fn test_grep_skips_gitignored_by_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "secret.txt\n").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "gitignore_needle\n").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "gitignore_needle\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "gitignore_needle" }),
        );
        assert!(result.contains("visible.txt"), "got: {result}");
        assert!(
            !result.contains("secret.txt"),
            "AgentText must respect .gitignore by default, got: {result}"
        );

        let raw = call_in(
            dir.path(),
            serde_json::json!({ "regex": "gitignore_needle", "no_ignore": true }),
        );
        assert!(raw.contains("secret.txt"), "got: {raw}");
    }

    #[test]
    fn test_grep_includes_hidden_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET_TOKEN=1\n").unwrap();
        std::fs::write(dir.path().join("open.txt"), "SECRET_TOKEN=1\n").unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "SECRET_TOKEN" }));
        assert!(result.contains("open.txt"), "got: {result}");
        assert!(
            result.contains(".env"),
            "AgentText must not skip un-ignored dotfiles, got: {result}"
        );
    }

    #[test]
    fn test_grep_context_in_snippet() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("c.rs"),
            "fn foo() {\n    let x = 1;\n    println!(\"hello\");\n    let y = 2;\n}\n",
        )
        .unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "println", "expand": true }),
        );
        // Ancestor expands to the whole fn when it fits in the cap.
        assert!(
            result.contains("fn foo()"),
            "ancestor/fn window, got: {result}"
        );
        assert!(result.contains("let x = 1"), "got: {result}");
        assert!(result.contains("let y = 2"), "got: {result}");
        assert!(result.contains("println!"), "got: {result}");
        assert!(
            result.contains("### fn foo › L"),
            "heading should be crumb › L…, got: {result}"
        );
    }

    #[test]
    fn test_grep_utf8() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("zh.txt"), "你好世界\n").unwrap();
        let result = call_in(dir.path(), serde_json::json!({ "regex": "世界" }));
        assert_eq!(result, "Found 1 matches:\nzh.txt\n       1:你好世界\n");
    }

    #[test]
    fn test_grep_fence_survives_inner_backticks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("f.md"),
            "before\n```\nNEEDLE inside fence\n```\nafter\n",
        )
        .unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "NEEDLE", "expand": true }),
        );
        assert!(result.contains("NEEDLE inside fence"), "got: {result}");
        // Outer fence must be longer than ```
        assert!(
            result.contains("````") || result.matches("```").count() >= 4,
            "need safe fence, got: {result}"
        );
    }

    #[test]
    fn test_grep_ast_breadcrumb_on_rust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("store.rs"),
            "impl Store {\n    fn save(&self) {\n        let hit = 1;\n    }\n}\n",
        )
        .unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "hit", "expand": true }),
        );
        assert!(
            result.contains("### impl Store › fn save › L"),
            "expected crumb › L heading, got: {result}"
        );
        assert!(result.contains('›'), "got: {result}");
        // Ancestor should prefer a useful window (fn body / impl), not only ±2.
        assert!(
            result.contains("fn save") || result.contains("let hit"),
            "got: {result}"
        );
    }

    #[test]
    fn test_grep_breadcrumb_falls_back_for_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "fn needle() {}\n").unwrap();
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "needle", "expand": true }),
        );
        assert!(result.contains("### L1"), "got: {result}");
        assert!(!result.contains('›'), "got: {result}");
    }

    #[test]
    fn test_grep_ancestor_if_block_and_remaining() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = String::from(
            "impl MyStruct {\n    fn method_with_block() {\n        let condition = true;\n        if condition {\n            println!(\"Inside if block\");\n        }\n    }\n\n    fn long_function() {\n",
        );
        for i in 1..=12 {
            src.push_str(&format!("        println!(\"Line {i}\");\n"));
        }
        src.push_str("    }\n}\n");
        std::fs::write(dir.path().join("test_syntax.rs"), &src).unwrap();

        let if_hit = call_in(
            dir.path(),
            serde_json::json!({ "regex": "Inside if block", "expand": true }),
        );
        assert!(
            if_hit.contains("if condition") && if_hit.contains("Inside if block"),
            "if ancestor, got: {if_hit}"
        );
        assert!(
            if_hit.contains("### impl MyStruct › fn method_with_block › L"),
            "got: {if_hit}"
        );

        let mid = call_in(
            dir.path(),
            serde_json::json!({ "regex": "Line 5", "expand": true }),
        );
        assert!(
            mid.contains("fn long_function"),
            "long fn should expand from start, got: {mid}"
        );
        assert!(
            mid.contains("lines remaining in ancestor node"),
            "expected remaining note, got: {mid}"
        );
        assert!(!mid.contains("Line 12"), "cap should drop tail, got: {mid}");
    }

    #[test]
    fn test_grep_truncates_long_snippet_line() {
        let dir = tempfile::tempdir().unwrap();
        let long = "needle_".to_string() + &"x".repeat(400);
        std::fs::write(dir.path().join("wide.txt"), format!("{long}\n")).unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "needle_", "expand": true }),
        );
        assert!(result.contains("(line truncated)"), "got: {result}");
        assert!(!result.contains(&"x".repeat(300)), "got: {result}");
    }

    #[test]
    fn test_grep_independent_of_repo_cwd_leak() {
        let dir = tempfile::tempdir().unwrap();
        // Empty dir: must not find litecode source via wrong root.
        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "fn lexical_search" }),
        );
        assert_eq!(result, "No matches found");
    }

    #[test]
    fn test_grep_execute_uses_context_root_not_process_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only_here.txt"), "context_root_hit\n").unwrap();
        let other = tempfile::tempdir().unwrap();
        // Process cwd is elsewhere; execute context points at `dir`.
        let result = with_cwd(other.path(), || {
            call_in(
                dir.path(),
                serde_json::json!({ "regex": "context_root_hit" }),
            )
        });
        assert!(result.contains("context_root_hit"), "got: {result}");
        assert!(result.contains("only_here.txt"), "got: {result}");
    }

    #[test]
    fn test_grep_works_with_empty_path_env() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("t.txt"), "path_free_hit\n").unwrap();
        let prev = std::env::var_os("PATH");
        let result = {
            unsafe {
                std::env::set_var("PATH", "");
            }
            let out = call_in(dir.path(), serde_json::json!({ "regex": "path_free_hit" }));
            match &prev {
                Some(p) => unsafe { std::env::set_var("PATH", p) },
                None => unsafe { std::env::remove_var("PATH") },
            }
            out
        };
        assert!(result.contains("path_free_hit"), "got: {result}");
    }

    #[test]
    fn test_old_schema_fields_are_ignored_not_required() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "zed_only\n").unwrap();
        // Passing obsolete fields must not change behavior if regex is present.
        let result = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "zed_only",
                "pattern": "wrong",
                "output_mode": "count",
                "multiline": true,
                "max_matches": 1,
                "expand": true
            }),
        );
        assert!(result.contains("zed_only"), "got: {result}");
        assert!(result.contains("## Matches in"), "got: {result}");
    }

    #[test]
    fn test_grep_path_workspace_subdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/inner.txt"), "subdir_needle\n").unwrap();
        std::fs::write(dir.path().join("top.txt"), "subdir_needle\n").unwrap();

        let result = call_in(
            dir.path(),
            serde_json::json!({ "regex": "subdir_needle", "path": "sub" }),
        );
        assert!(result.contains("inner.txt"), "got: {result}");
        assert!(!result.contains("top.txt"), "got: {result}");
    }

    #[test]
    fn test_grep_path_outside_workspace_all_mode() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("external.txt"), "external_needle_hit\n").unwrap();
        std::fs::write(workspace.path().join("local.txt"), "no needle\n").unwrap();

        let result = call_in_mode(
            workspace.path(),
            serde_json::json!({
                "regex": "external_needle_hit",
                "path": outside.path().to_string_lossy()
            }),
            crate::workspace::ToolPathMode::All,
        );
        assert!(result.contains("external.txt"), "got: {result}");
        assert!(!result.contains("local.txt"), "got: {result}");
        assert!(!result.starts_with("Error"), "got: {result}");
    }

    #[test]
    fn test_grep_path_outside_workspace_safe_denied() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("external.txt"), "external_needle_hit\n").unwrap();

        let result = call_in_mode(
            workspace.path(),
            serde_json::json!({
                "regex": "external_needle_hit",
                "path": outside.path().to_string_lossy()
            }),
            crate::workspace::ToolPathMode::Safe,
        );
        assert!(
            result.contains("SAFE mode only permits paths under the workspace"),
            "got: {result}"
        );
    }

    #[test]
    fn test_grep_path_missing() {
        let dir = tempfile::tempdir().unwrap();

        let missing = call_in(
            dir.path(),
            serde_json::json!({ "regex": "x", "path": "does-not-exist" }),
        );
        assert!(missing.contains("path does not exist"), "got: {missing}");
    }

    #[test]
    fn test_grep_path_single_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("single.txt"), "single_needle\n").unwrap();
        std::fs::write(dir.path().join("other.txt"), "single_needle\n").unwrap();

        let file = call_in(
            dir.path(),
            serde_json::json!({ "regex": "single_needle", "path": "single.txt" }),
        );
        assert_eq!(file, "Found 1 matches:\nsingle.txt\n       1:single_needle\n");
    }
}
