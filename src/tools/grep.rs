//! Agent `grep` tool — Zed-aligned narrow LexicalLane frontend.
//!
//! Schema is regex / include_pattern / path / offset / no_ignore.
//! Matching is always case-insensitive. Results automatically degrade from
//! syntax-aware snippets to nearby context, then matching lines.
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

/// Fixed context lines around each hit when ancestor expansion is unavailable.
const CONTEXT_LINES: usize = 2;
/// Display cap per snippet line (chars).
const SNIPPET_LINE_MAX_CHARS: usize = 240;
/// One grep response must leave most of the model context available to reason.
const GREP_TOKEN_BUDGET: usize = 6_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrepView {
    Expanded,
    Context,
    Matches,
}

impl GrepView {
    fn label(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Context => "context",
            Self::Matches => "matches",
        }
    }
}
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
                    "description": "Regular expression matched against file contents (not paths). Rust/ripgrep syntax; always case-insensitive."
                },
                "include_pattern": {
                    "type": "string",
                    "description": "Optional filename glob relative to `path` (or the workspace root), e.g. **/*.rs or **/*.{ts,tsx}. Filters which files are searched, not content. Do not repeat `path` in the glob — if path is src, use **/*.rs not src/**/*.rs."
                },
                "path": {
                    "type": "string",
                    "description": "Optional directory or single file to search (workspace-relative preferred; absolute paths outside the workspace only under All permission). Omit to search the workspace. A directory is the walk root; a file searches only that file, including large files."
                },
                "offset": {
                    "type": "integer",
                    "description": "0-based match index. Omit on the first call; when more matches remain, pass the returned next offset."
                },
                "no_ignore": {
                    "type": "boolean",
                    "description": "When true, search without .gitignore / files.exclude / search.exclude (default: false)."
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
        "Search file contents with a regular expression. Use freely to locate symbols, strings, and usages. `path` may be a directory or a single file, including large files — the tool handles the rest."
            .into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let regex = crate::tool::require_nonempty_string(input, "regex")?;
        if compile_regex_preview(regex).is_err() {
            return Err("parameter 'regex' is not a valid regular expression".into());
        }
        Ok(())
    }

    fn max_result_size(&self) -> usize {
        // grep enforces its own exact token budget before this outer executor cap.
        usize::MAX
    }
}

fn compile_regex_preview(pattern: &str) -> std::result::Result<(), regex::Error> {
    regex::Regex::new(&format!("(?i){pattern}")).map(|_| ())
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
    run_grep_with_token_budget(input, workspace_root, path_mode, GREP_TOKEN_BUDGET)
}

fn run_grep_with_token_budget(
    input: &Value,
    workspace_root: &Path,
    path_mode: ToolPathMode,
    token_budget: usize,
) -> Result<String> {
    let regex = crate::tool::require_nonempty_string(input, "regex")
        .map_err(LitecodeError::ToolExecution)?;

    let include_pattern = input["include_pattern"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let offset = input["offset"].as_u64().map(|o| o as usize).unwrap_or(0);
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
    // match paths stay relative and snippet rendering can read sources.
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
    let outcome = lexical_search_with_preset(
        &LexicalQuery {
            pattern: regex.to_string(),
            root: root.clone(),
            path: file_scope,
            case_sensitive: false,
            whole_word: false,
            is_regex: true,
            include: include_pattern,
            exclude: None,
            multiline: false,
            max_matches: usize::MAX,
            before_context: CONTEXT_LINES,
            after_context: CONTEXT_LINES,
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
        return Ok(format!(
            "offset {offset} past end ({total} matches); try offset 0"
        ));
    }

    Ok(render_grep_page(&root, &matches, offset, token_budget))
}

fn sort_grep_matches(matches: &mut [LexicalMatch]) {
    matches.sort_by(|a, b| {
        crate::workspace::glob_hit_key(&a.path)
            .cmp(&crate::workspace::glob_hit_key(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });
}

fn wrap_grep_page(body: &str, offset: usize, shown: usize, total: usize, view: GrepView) -> String {
    if shown == 0 {
        return String::new();
    }
    if shown < total.saturating_sub(offset) {
        format!(
            "Showing matches {}-{} of {total} (view: {}; use offset: {} to continue):\n{body}",
            offset + 1,
            offset + shown,
            view.label(),
            offset + shown,
        )
    } else if offset > 0 {
        format!(
            "Showing matches {}-{} of {total} (view: {}):\n{body}",
            offset + 1,
            offset + shown,
            view.label(),
        )
    } else {
        format!("Found {shown} matches (view: {}):\n{body}", view.label())
    }
}

fn render_grep_page(
    root: &Path,
    matches: &[LexicalMatch],
    offset: usize,
    token_budget: usize,
) -> String {
    let remaining = &matches[offset..];

    let view = select_grep_view(root, matches, token_budget);
    if view != GrepView::Matches {
        let body = format_grep_body(root, remaining, view);
        return wrap_grep_page(&body, offset, remaining.len(), matches.len(), view);
    }

    let all_compact_body = format_grep_body(root, matches, GrepView::Matches);
    let all_compact = wrap_grep_page(
        &all_compact_body,
        0,
        matches.len(),
        matches.len(),
        GrepView::Matches,
    );
    if crate::session::count_text_tokens(&all_compact) <= token_budget {
        let body = format_grep_body(root, remaining, GrepView::Matches);
        return wrap_grep_page(
            &body,
            offset,
            remaining.len(),
            matches.len(),
            GrepView::Matches,
        );
    }

    // Matching lines are the final representation. Once it needs pagination,
    // every offset uses it so the result-set's information density is stable.
    let mut low = 0usize;
    let mut high = remaining.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let body = format_grep_body(root, &remaining[..mid], GrepView::Matches);
        let output = wrap_grep_page(&body, offset, mid, matches.len(), GrepView::Matches);
        if crate::session::count_text_tokens(&output) <= token_budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    if low == 0 {
        // A single already line-capped hit is still more actionable than an
        // empty successful search result, and guarantees the next offset moves.
        low = 1;
    }
    let body = format_grep_body(root, &remaining[..low], GrepView::Matches);
    wrap_grep_page(&body, offset, low, matches.len(), GrepView::Matches)
}

fn select_grep_view(root: &Path, matches: &[LexicalMatch], token_budget: usize) -> GrepView {
    for view in [GrepView::Expanded, GrepView::Context, GrepView::Matches] {
        let body = format_grep_body(root, matches, view);
        let output = wrap_grep_page(&body, 0, matches.len(), matches.len(), view);
        if crate::session::count_text_tokens(&output) <= token_budget {
            return view;
        }
    }
    GrepView::Matches
}

fn format_grep_body(root: &Path, matches: &[LexicalMatch], view: GrepView) -> String {
    match view {
        GrepView::Expanded | GrepView::Context => format_snippet_body(root, matches, view),
        GrepView::Matches => format_compact_body(matches),
    }
}

fn format_compact_body(matches: &[LexicalMatch]) -> String {
    let mut body = String::new();
    let mut current: Option<&str> = None;
    for m in matches {
        if current != Some(m.path.as_str()) {
            body.push_str(&m.path);
            body.push('\n');
            current = Some(&m.path);
        }
        let text = truncate_snippet_lines(m.line_text.trim_end_matches(['\n', '\r']));
        body.push_str(&format!("  {:>6}:{text}\n", m.start_line));
    }
    body
}

/// Zed grep-panel style: page grouped by file (`## Matches in {path}`) with
/// AST-breadcrumb headings per hit. File order is glob_hit_key (same as compact).
fn format_snippet_body(root: &Path, matches: &[LexicalMatch], view: GrepView) -> String {
    let mut file_order: Vec<String> = Vec::new();
    let mut by_file: BTreeMap<String, Vec<&LexicalMatch>> = BTreeMap::new();
    for m in matches {
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

        let ranges = merge_snippet_ranges(file_matches, source, &path, view == GrepView::Expanded);
        for range in ranges {
            let line_label = if range.start_line == range.end_line {
                format!("L{}", range.start_line)
            } else {
                format!("L{}-{}", range.start_line, range.end_line)
            };
            let heading = match (view == GrepView::Expanded)
                .then(|| {
                    source.and_then(|src| {
                        format_breadcrumb(&enclosing_scopes(&path, src, range.hit_line))
                    })
                })
                .flatten()
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

    body
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
    use_ancestor: bool,
) -> Vec<SnippetRange> {
    let mut ranges: Vec<SnippetRange> = Vec::new();
    for m in file_matches {
        let built = snippet_for_match(m, source, path, use_ancestor);
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

fn snippet_for_match(
    m: &LexicalMatch,
    source: Option<&str>,
    path: &str,
    use_ancestor: bool,
) -> SnippetRange {
    let match_end = m.end_line.max(m.start_line);

    if use_ancestor
        && let Some(src) = source
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

    fn call_in_with_budget(dir: &std::path::Path, input: Value, token_budget: usize) -> String {
        run_grep_with_token_budget(
            &input,
            dir,
            crate::workspace::ToolPathMode::Safe,
            token_budget,
        )
        .expect("grep succeeds")
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

        let result = call_in(dir.path(), serde_json::json!({ "regex": "quick brown" }));
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
    fn test_grep_default_uses_expanded_view() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("hello.txt"),
            "the quick brown fox\njumps over the lazy dog\nquick brown again\n",
        )
        .unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "quick brown" }));
        assert!(result.contains("view: expanded"), "got: {result}");
        assert!(result.contains("## Matches in hello.txt"), "got: {result}");
        assert!(result.contains("quick brown again"), "got: {result}");
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
        let a = result.find("## Matches in a.md").expect("a.md heading");
        let z = result.find("## Matches in z.md").expect("z.md heading");
        let nested = result
            .find("## Matches in src/a.rs")
            .expect("src/a.rs heading");
        assert!(a < z && z < nested, "expected glob order, got: {result}");
    }

    #[test]
    fn test_grep_expanded_uses_glob_file_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("z.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("src/a.txt"), "needle\n").unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "needle" }));
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
    fn test_grep_is_always_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("g.txt"), "Hello World\n").unwrap();

        let result = call_in(dir.path(), serde_json::json!({ "regex": "hello world" }));
        assert!(
            result.contains("Hello World"),
            "grep is case-insensitive, got: {result}"
        );

        // Obsolete field must not restore case-sensitive matching.
        let ignored = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "hello world",
                "case_sensitive": true
            }),
        );
        assert!(
            ignored.contains("Hello World"),
            "case_sensitive is ignored, got: {ignored}"
        );
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
    fn test_grep_compact_paginates_by_real_token_budget() {
        let dir = tempfile::tempdir().unwrap();
        let content: String = (0..200)
            .map(|i| format!("item{i:03} {}\n", "x".repeat(40)))
            .collect();
        std::fs::write(dir.path().join("many.txt"), &content).unwrap();

        let page1 =
            call_in_with_budget(dir.path(), serde_json::json!({ "regex": "item\\d+" }), 300);
        assert!(page1.contains("view: matches"), "got: {page1}");
        assert!(
            crate::session::count_text_tokens(&page1) <= 300,
            "page exceeded budget: {} tokens\n{page1}",
            crate::session::count_text_tokens(&page1)
        );
        assert!(!page1.contains("... [truncated]"), "got: {page1}");
        let next = page1
            .split("use offset: ")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse::<usize>().ok())
            .expect("next offset in page footer");
        assert!(next > 0 && next < 200, "got next={next}: {page1}");

        let page2 = call_in_with_budget(
            dir.path(),
            serde_json::json!({ "regex": "item\\d+", "offset": next }),
            300,
        );
        assert!(
            page2.contains(&format!("Showing matches {}-", next + 1)),
            "got: {page2}"
        );
        assert!(!page2.contains("item000"), "got: {page2}");
        assert!(
            crate::session::count_text_tokens(&page2) <= 300,
            "page exceeded budget: {} tokens\n{page2}",
            crate::session::count_text_tokens(&page2)
        );
        assert!(!page2.contains("... [truncated]"), "got: {page2}");

        // Even a late offset that could independently fit as a snippet page
        // must preserve the compact view selected for the whole result set.
        let final_page = call_in_with_budget(
            dir.path(),
            serde_json::json!({ "regex": "item\\d+", "offset": 190 }),
            300,
        );
        assert!(final_page.contains("view: matches"), "got: {final_page}");
        assert!(!final_page.contains("## Matches in"), "got: {final_page}");
    }

    #[test]
    fn test_grep_degrades_whole_page_through_each_view() {
        let dir = tempfile::tempdir().unwrap();

        let mut rich_source = String::from("fn rich() {\n");
        for _ in 0..4 {
            rich_source.push_str(&format!("    let padding = {:?};\n", "x".repeat(220)));
        }
        rich_source.push_str("    let needle = true;\n}\n");
        std::fs::write(dir.path().join("rich.rs"), &rich_source).unwrap();
        let rich_match = LexicalMatch {
            path: "rich.rs".into(),
            start_line: 6,
            end_line: 6,
            line_text: "    let needle = true;\n".into(),
            context_before: vec![],
            context_after: vec![],
        };
        let expanded = wrap_grep_page(
            &format_grep_body(dir.path(), &[rich_match.clone()], GrepView::Expanded),
            0,
            1,
            1,
            GrepView::Expanded,
        );
        let context = wrap_grep_page(
            &format_grep_body(dir.path(), &[rich_match.clone()], GrepView::Context),
            0,
            1,
            1,
            GrepView::Context,
        );
        let context_cap = crate::session::count_text_tokens(&context);
        assert!(
            crate::session::count_text_tokens(&expanded) > context_cap,
            "fixture must make ancestor view larger"
        );
        let degraded = render_grep_page(dir.path(), &[rich_match], 0, context_cap);
        assert!(degraded.contains("view: context"), "got: {degraded}");
        assert!(
            crate::session::count_text_tokens(&degraded) <= context_cap,
            "context view exceeded cap"
        );

        let mut many_source = String::new();
        let mut many_matches = Vec::new();
        for i in 0..120u32 {
            let hit_line = i * 7 + 4;
            many_source.push_str(&format!(
                "fn f{i}() {{\n    let a = {i};\n    let b = {i};\n    let needle_{i} = true;\n    let c = {i};\n}}\n\n"
            ));
            many_matches.push(LexicalMatch {
                path: "many.rs".into(),
                start_line: hit_line,
                end_line: hit_line,
                line_text: format!("    let needle_{i} = true;\n"),
                context_before: vec![],
                context_after: vec![],
            });
        }
        std::fs::write(dir.path().join("many.rs"), many_source).unwrap();
        let context_all = wrap_grep_page(
            &format_grep_body(dir.path(), &many_matches, GrepView::Context),
            0,
            many_matches.len(),
            many_matches.len(),
            GrepView::Context,
        );
        let compact_all = wrap_grep_page(
            &format_grep_body(dir.path(), &many_matches, GrepView::Matches),
            0,
            many_matches.len(),
            many_matches.len(),
            GrepView::Matches,
        );
        let compact_cap = crate::session::count_text_tokens(&compact_all);
        assert!(
            crate::session::count_text_tokens(&context_all) > compact_cap,
            "fixture must make context view larger"
        );
        let compact = render_grep_page(dir.path(), &many_matches, 0, compact_cap);
        assert!(compact.contains("view: matches"), "got: {compact}");
        assert!(compact.contains("needle_119"), "got: {compact}");
        assert!(
            crate::session::count_text_tokens(&compact) <= compact_cap,
            "compact view exceeded cap"
        );
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
        let result = call_in(dir.path(), serde_json::json!({ "regex": "println" }));
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
        assert!(result.contains("view: expanded"), "got: {result}");
        assert!(result.contains("你好世界"), "got: {result}");
    }

    #[test]
    fn test_grep_fence_survives_inner_backticks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("f.md"),
            "before\n```\nNEEDLE inside fence\n```\nafter\n",
        )
        .unwrap();
        let result = call_in(dir.path(), serde_json::json!({ "regex": "NEEDLE" }));
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
        let result = call_in(dir.path(), serde_json::json!({ "regex": "hit" }));
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
        let result = call_in(dir.path(), serde_json::json!({ "regex": "needle" }));
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
            serde_json::json!({ "regex": "Inside if block" }),
        );
        assert!(
            if_hit.contains("if condition") && if_hit.contains("Inside if block"),
            "if ancestor, got: {if_hit}"
        );
        assert!(
            if_hit.contains("### impl MyStruct › fn method_with_block › L"),
            "got: {if_hit}"
        );

        let mid = call_in(dir.path(), serde_json::json!({ "regex": "Line 5" }));
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

        let result = call_in(dir.path(), serde_json::json!({ "regex": "needle_" }));
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
    fn test_schema_hides_expand_and_ignores_obsolete_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "zed_only\n").unwrap();
        let props = GrepTool.schema().get("properties").unwrap().clone();
        assert!(
            props.get("expand").is_none(),
            "expand must not be advertised"
        );
        assert!(
            props.get("case_sensitive").is_none(),
            "case_sensitive must not be advertised"
        );
        let include = props["include_pattern"]["description"].as_str().unwrap();
        assert!(
            include.contains("**/*.rs") && include.contains("`path`"),
            "include_pattern must explain glob vs path, got: {include}"
        );
        let path = props["path"]["description"].as_str().unwrap();
        assert!(
            path.contains("directory or single file") && path.contains("large files"),
            "path must explain dir, file, and large files, got: {path}"
        );
        assert_eq!(GrepTool.max_result_size(), usize::MAX);
        // Passing obsolete fields must not change behavior if regex is present.
        let result = call_in(
            dir.path(),
            serde_json::json!({
                "regex": "zed_only",
                "pattern": "wrong",
                "output_mode": "count",
                "multiline": true,
                "max_matches": 1,
                "case_sensitive": true
            }),
        );
        assert!(result.contains("zed_only"), "got: {result}");
        assert!(result.contains("view: expanded"), "got: {result}");
    }

    #[test]
    fn test_description_encourages_use_without_budget() {
        let ctx = crate::context_pipeline::Context {
            cwd: Path::new("/tmp").to_path_buf(),
            workspace_paths: crate::config::resolved::WorkspacePaths::for_legacy_root(Path::new(
                "/tmp",
            )),
            agents_md: None,
            claude_md: None,
        };
        let d = GrepTool.description(&ctx);
        let lower = d.to_ascii_lowercase();
        assert!(d.contains("Use freely"), "got: {d}");
        assert!(
            d.contains("directory or a single file"),
            "path must cover dir and file, got: {d}"
        );
        assert!(lower.contains("large file"), "got: {d}");
        assert!(
            lower.contains("handles the rest"),
            "tool should absorb result size, got: {d}"
        );
        assert!(
            !lower.contains("bash") && !lower.contains("token") && !lower.contains("budget"),
            "must not mention bash or imply a budget, got: {d}"
        );
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
        assert!(file.contains("view: expanded"), "got: {file}");
        assert!(file.contains("single_needle"), "got: {file}");
    }
}
