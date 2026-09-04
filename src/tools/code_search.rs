use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::context_pipeline::Context;
use crate::engines::code_search::{
    DEFAULT_TOP_K, IndexStatus, MAX_TOP_K, ResolvedIndexView, enclosing_scopes, format_breadcrumb,
    lines_slice, resolve_index_view, syntax_ancestor_snippet,
};
use crate::engines::{
    CodeSearchCallGate, EngineState, RetrievalCorpus, RetrievalFilters, RetrievalHit,
    RetrievalModality, RetrievalQuery, WorkspaceEngines,
};
use crate::tool::Tool;
use crate::types::ToolCallResult;

/// Same budget as grep `content`: one response must leave room to reason.
const CODE_SEARCH_TOKEN_BUDGET: usize = 6_000;
/// Display cap per snippet line (chars). Matches grep.
const SNIPPET_LINE_MAX_CHARS: usize = 240;
/// Max lines shown from an indexed chunk when AST ancestor is unavailable (Expanded).
const EXPANDED_CHUNK_LINES: u32 = 12;
/// Max lines shown from an indexed chunk in the compact view.
const CONTEXT_CHUNK_LINES: u32 = 6;

const CODE_SEARCH_WARM_WAIT: Duration = Duration::from_secs(60);
const CODE_SEARCH_WARM_POLL: Duration = Duration::from_millis(50);

pub struct CodeSearchTool {
    engines: WorkspaceEngines,
}

impl CodeSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self { engines }
    }
}

impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        "code_search"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language or keyword query to find relevant code"
                },
                "include_pattern": {
                    "type": "string",
                    "description": "Optional glob filter for file paths (e.g. '**/*.rs', '**/*.{ts,tsx}')"
                },
                "top_k": {
                    "type": "integer",
                    "description": format!("Number of results to return (default {DEFAULT_TOP_K}, max {MAX_TOP_K})")
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolCallResult::error(e),
        };

        let glob = input["include_pattern"].as_str().filter(|s| !s.is_empty());
        let top_k = input["top_k"]
            .as_u64()
            .map(|k| k as usize)
            .unwrap_or(DEFAULT_TOP_K)
            .clamp(1, MAX_TOP_K);

        match self.engines.state("code_search") {
            Some(EngineState::Failed) => {
                let detail = self
                    .engines
                    .last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into());
                return ToolCallResult::error(detail);
            }
            _ => {}
        }

        if !self.engines.is_warmed("code_search") {
            let started = Instant::now();
            while started.elapsed() < CODE_SEARCH_WARM_WAIT {
                if self.engines.is_warmed("code_search") {
                    break;
                }
                if matches!(self.engines.state("code_search"), Some(EngineState::Failed)) {
                    break;
                }
                std::thread::sleep(CODE_SEARCH_WARM_POLL);
            }
        }

        match self.engines.state("code_search") {
            Some(EngineState::Failed) => {
                let detail = self
                    .engines
                    .last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into());
                return ToolCallResult::error(detail);
            }
            Some(EngineState::Warm) => {}
            _ => {
                return ToolCallResult::ok(indexing_wait_message(&self.engines));
            }
        }

        match self.engines.code_search_call_gate() {
            CodeSearchCallGate::Failed(detail) => {
                return ToolCallResult::error(detail);
            }
            CodeSearchCallGate::Wait => {
                return ToolCallResult::ok(indexing_wait_message(&self.engines));
            }
            CodeSearchCallGate::Ready => {}
        }

        match self.engines.search(RetrievalQuery {
            query: query.to_string(),
            corpus: RetrievalCorpus::Code,
            modality: RetrievalModality::Semantic,
            filters: RetrievalFilters {
                glob: glob.map(str::to_string),
                ..Default::default()
            },
            top_k,
            workspace_root: None,
            offset: 0,
        }) {
            Ok(hits) => {
                if hits.is_empty() {
                    let scope = glob
                        .map(|p| format!(" for include_pattern '{p}'"))
                        .unwrap_or_default();
                    return ToolCallResult::ok(format!("No matching code chunks found{scope}."));
                }
                let root = self.engines.code_search().workspace_root();
                ToolCallResult::ok(format_code_search_hits(
                    root.as_deref(),
                    &hits,
                    CODE_SEARCH_TOKEN_BUDGET,
                ))
            }
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Semantic code search over the workspace index when the engine is Warm.".into()
    }

    fn timeout(&self) -> Option<u64> {
        Some(60)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeSearchView {
    Expanded,
    Context,
}

impl CodeSearchView {
    fn label(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Context => "context",
        }
    }

    fn chunk_cap(self) -> u32 {
        match self {
            Self::Expanded => EXPANDED_CHUNK_LINES,
            Self::Context => CONTEXT_CHUNK_LINES,
        }
    }
}

struct CodeHitRef<'a> {
    path: &'a str,
    start_line: u32,
    end_line: u32,
    summary: &'a str,
}

fn code_hits(hits: &[RetrievalHit]) -> Vec<CodeHitRef<'_>> {
    hits.iter()
        .filter_map(|h| match h {
            RetrievalHit::Code {
                path,
                start_line,
                end_line,
                summary,
                score: _,
            } => Some(CodeHitRef {
                path,
                start_line: *start_line,
                end_line: *end_line,
                summary,
            }),
            _ => None,
        })
        .collect()
}

/// Grep `content`-style evidence for ranked semantic hits. No extra knobs.
fn format_code_search_hits(
    root: Option<&Path>,
    hits: &[RetrievalHit],
    token_budget: usize,
) -> String {
    let hits = code_hits(hits);
    if hits.is_empty() {
        return "No matching code chunks found.".into();
    }
    let view = select_code_search_view(root, &hits, token_budget);
    if page_fits(root, &hits, view, token_budget) {
        return wrap_code_search_page(
            &format_snippet_body(root, &hits, view),
            hits.len(),
            hits.len(),
            view,
        );
    }

    let mut low = 0usize;
    let mut high = hits.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let output = wrap_code_search_page(
            &format_snippet_body(root, &hits[..mid], CodeSearchView::Context),
            mid,
            hits.len(),
            CodeSearchView::Context,
        );
        if crate::session::count_text_tokens(&output) <= token_budget {
            low = mid;
        } else {
            high = mid.saturating_sub(1);
        }
    }
    let shown = low.max(1);
    wrap_code_search_page(
        &format_snippet_body(root, &hits[..shown], CodeSearchView::Context),
        shown,
        hits.len(),
        CodeSearchView::Context,
    )
}

fn select_code_search_view(
    root: Option<&Path>,
    hits: &[CodeHitRef<'_>],
    token_budget: usize,
) -> CodeSearchView {
    for view in [CodeSearchView::Expanded, CodeSearchView::Context] {
        if page_fits(root, hits, view, token_budget) {
            return view;
        }
    }
    CodeSearchView::Context
}

fn page_fits(
    root: Option<&Path>,
    hits: &[CodeHitRef<'_>],
    view: CodeSearchView,
    token_budget: usize,
) -> bool {
    let output = wrap_code_search_page(
        &format_snippet_body(root, hits, view),
        hits.len(),
        hits.len(),
        view,
    );
    crate::session::count_text_tokens(&output) <= token_budget
}

fn wrap_code_search_page(body: &str, shown: usize, total: usize, view: CodeSearchView) -> String {
    if shown < total {
        format!(
            "Showing {shown} of {total} chunks (view: {}):\n{body}",
            view.label()
        )
    } else {
        format!("Found {shown} chunks (view: {}):\n{body}", view.label())
    }
}

fn format_snippet_body(
    root: Option<&Path>,
    hits: &[CodeHitRef<'_>],
    view: CodeSearchView,
) -> String {
    let mut file_order: Vec<&str> = Vec::new();
    let mut by_file: HashMap<&str, Vec<&CodeHitRef<'_>>> = HashMap::new();
    for hit in hits {
        if !by_file.contains_key(hit.path) {
            file_order.push(hit.path);
        }
        by_file.entry(hit.path).or_default().push(hit);
    }

    let mut file_sources: HashMap<&str, Option<String>> = HashMap::new();
    let mut body = String::new();
    for path in file_order {
        let Some(file_hits) = by_file.get(path) else {
            continue;
        };
        body.push_str(&format!("\n## Matches in {path}\n"));
        let source = file_sources
            .entry(path)
            .or_insert_with(|| root.and_then(|root| std::fs::read_to_string(root.join(path)).ok()))
            .as_deref();
        for hit in file_hits {
            let snippet = snippet_for_hit(path, source, hit, view);
            let line_label = if snippet.start_line == snippet.end_line {
                format!("L{}", snippet.start_line)
            } else {
                format!("L{}-{}", snippet.start_line, snippet.end_line)
            };
            let heading = match (view == CodeSearchView::Expanded)
                .then(|| {
                    source.and_then(|src| {
                        format_breadcrumb(&enclosing_scopes(path, src, snippet.hit_line))
                    })
                })
                .flatten()
            {
                Some(crumb) => format!("\n### {crumb} › {line_label}"),
                None => format!("\n### {line_label}"),
            };
            body.push_str(&heading);
            body.push('\n');
            body.push_str(&fence_block(&snippet.text));
            if snippet.remaining_lines > 0 {
                let reason = if snippet.from_ancestor {
                    "ancestor node"
                } else {
                    "this chunk"
                };
                body.push_str(&format!(
                    "\n{} lines remaining in {reason}. Read the file to see all.\n",
                    snippet.remaining_lines
                ));
            }
        }
    }
    body
}

struct SnippetRange {
    start_line: u32,
    end_line: u32,
    hit_line: u32,
    text: String,
    remaining_lines: u32,
    from_ancestor: bool,
}

fn snippet_for_hit(
    path: &str,
    source: Option<&str>,
    hit: &CodeHitRef<'_>,
    view: CodeSearchView,
) -> SnippetRange {
    let chunk_end = hit.end_line.max(hit.start_line);
    // Prefer a line inside the chunk so breadcrumbs/ancestors resolve like grep
    // (signature-only start lines often sit on the node boundary).
    let focal = hit.start_line.saturating_add(1).min(chunk_end);
    if view == CodeSearchView::Expanded
        && let Some(src) = source
        && let Some(ancestor) = syntax_ancestor_snippet(path, src, focal, focal)
    {
        return SnippetRange {
            start_line: ancestor.start_line,
            end_line: ancestor.end_line,
            hit_line: focal,
            text: lines_slice(src, ancestor.start_line, ancestor.end_line),
            remaining_lines: ancestor.remaining_lines,
            from_ancestor: true,
        };
    }

    if let Some(src) = source {
        let line_count = src.lines().count() as u32;
        let start = hit.start_line.max(1);
        let cap_end = start.saturating_add(view.chunk_cap().saturating_sub(1));
        let end = cap_end.min(chunk_end).min(line_count.max(1));
        return SnippetRange {
            start_line: start,
            end_line: end,
            hit_line: focal,
            text: lines_slice(src, start, end),
            remaining_lines: chunk_end.saturating_sub(end),
            from_ancestor: false,
        };
    }

    SnippetRange {
        start_line: hit.start_line,
        end_line: chunk_end,
        hit_line: focal,
        text: hit.summary.to_string(),
        remaining_lines: 0,
        from_ancestor: false,
    }
}

fn fence_block(snippet: &str) -> String {
    let snippet = truncate_snippet_lines(snippet);
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

fn indexing_wait_message(engines: &WorkspaceEngines) -> String {
    let root = engines.code_search().workspace_root();
    let state = engines.state("code_search");
    if let Some(root) = root {
        let view = resolve_index_view(&root, state);
        if matches!(view.status, IndexStatus::Building | IndexStatus::Refreshing) {
            return format_index_progress(&view);
        }
    }
    if engines.is_refresh_busy() {
        return "code_search index is refreshing. Try again shortly.".into();
    }
    "code_search engine is still starting. Try again shortly.".into()
}

fn format_index_progress(view: &ResolvedIndexView) -> String {
    let kind = match view.status {
        IndexStatus::Building => "building",
        IndexStatus::Refreshing => "refreshing",
        other => {
            return format!("code_search index status is {other:?}. Try again shortly.");
        }
    };
    if let Some(p) = &view.progress {
        let eta = if p.files_total > 0 && p.files_done > 0 {
            format!(
                " (~{}% files)",
                (p.files_done.saturating_mul(100)) / p.files_total
            )
        } else {
            String::new()
        };
        format!(
            "code_search index is {kind} ({}): {}/{} files, {} chunks done{eta}. Try again shortly.",
            format!("{:?}", p.phase).to_lowercase(),
            p.files_done,
            p.files_total,
            p.chunks_done,
        )
    } else {
        format!("code_search index is {kind}. Try again shortly.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_up_to_sixty_seconds_then_searches_not_loading() {
        assert_eq!(CODE_SEARCH_WARM_WAIT, Duration::from_secs(60));
        assert_eq!(CODE_SEARCH_WARM_POLL, Duration::from_millis(50));
    }

    #[test]
    fn indexing_wait_message_mentions_progress() {
        let view = ResolvedIndexView {
            status: IndexStatus::Building,
            progress: Some(crate::engines::code_search::IndexingProgress {
                phase: crate::engines::code_search::IndexPhase::Embedding,
                files_done: 3,
                files_total: 10,
                chunks_done: 12,
            }),
            job_error: None,
        };
        let msg = format_index_progress(&view);
        assert!(msg.contains("building"), "{msg}");
        assert!(msg.contains("3/10"), "{msg}");
    }

    #[test]
    fn indexing_wait_message_reads_job_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::begin_building(root);
        crate::engines::code_search::update_build_progress(
            root,
            crate::engines::code_search::IndexingProgress {
                phase: crate::engines::code_search::IndexPhase::Embedding,
                files_done: 2,
                files_total: 8,
                chunks_done: 4,
            },
        );

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warming);

        let msg = indexing_wait_message(&engines);
        assert!(msg.contains("building"), "{msg}");
        assert!(msg.contains("2/8"), "{msg}");
        assert!(
            !msg.contains("No matching code"),
            "must not fake empty hits while indexing: {msg}"
        );
    }

    #[test]
    fn failed_engine_returns_last_error() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", EngineState::Failed);
        engines.set_last_error_for_test("code_search", "embedder missing");
        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(
            result.content.contains("embedder missing"),
            "{}",
            result.content
        );
    }

    #[test]
    fn warm_refreshing_returns_wait_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::begin_refreshing(root);

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert!(result.content.contains("refreshing"), "{}", result.content);
        assert!(
            !result.content.contains("No matching code"),
            "must not search stale corpus while refreshing: {}",
            result.content
        );
    }

    #[test]
    fn warm_refresh_busy_returns_wait_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        assert!(
            result.content.contains("refreshing") || result.content.contains("Try again shortly"),
            "{}",
            result.content
        );
        assert!(
            !result.content.contains("No matching code"),
            "{}",
            result.content
        );
    }

    #[test]
    fn warm_failed_index_job_returns_error_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        crate::engines::code_search::mark_index_job_failed(root, "embed exploded");

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(
            result.content.contains("embed exploded"),
            "{}",
            result.content
        );
    }

    fn code_hit(path: &str, start: u32, end: u32, summary: &str) -> RetrievalHit {
        RetrievalHit::Code {
            path: path.into(),
            start_line: start,
            end_line: end,
            summary: summary.into(),
            score: 0.99,
        }
    }

    fn render(root: &Path, hits: &[RetrievalHit]) -> String {
        format_code_search_hits(Some(root), hits, CODE_SEARCH_TOKEN_BUDGET)
    }

    #[test]
    fn agent_view_groups_by_file_with_breadcrumb_and_fence() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("store.rs"),
            "impl Store {\n    fn save(&self) {\n        let hit = 1;\n    }\n}\n",
        )
        .unwrap();
        let out = render(
            dir.path(),
            &[code_hit("store.rs", 2, 4, "fn save(&self) {")],
        );
        assert!(
            out.starts_with("Found 1 chunks (view: expanded):"),
            "header, got: {out}"
        );
        assert!(
            out.contains("## Matches in store.rs"),
            "file grouping, got: {out}"
        );
        assert!(
            out.contains("### impl Store › fn save › L"),
            "breadcrumb heading like grep content, got: {out}"
        );
        assert!(out.contains("```"), "fenced snippet, got: {out}");
        assert!(
            out.contains("fn save") || out.contains("let hit"),
            "source evidence, got: {out}"
        );
        assert!(
            !out.contains("score"),
            "score is not agent evidence, got: {out}"
        );
        assert!(
            !out.contains("fn save(&self) { (score"),
            "must not keep the one-line summary format, got: {out}"
        );
    }

    #[test]
    fn agent_view_txt_has_no_breadcrumb() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("notes.txt"), "restore theme preferences\n").unwrap();
        let out = render(
            dir.path(),
            &[code_hit("notes.txt", 1, 1, "restore theme preferences")],
        );
        assert!(out.contains("## Matches in notes.txt"), "got: {out}");
        assert!(out.contains("### L1"), "got: {out}");
        assert!(!out.contains('›'), "plain text has no crumb, got: {out}");
        assert!(
            out.contains("restore theme preferences"),
            "snippet text, got: {out}"
        );
    }

    #[test]
    fn agent_view_preserves_rank_order_across_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn second() {}\n").unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn first() {}\n").unwrap();
        let out = render(
            dir.path(),
            &[
                code_hit("b.rs", 1, 1, "fn second() {}"),
                code_hit("a.rs", 1, 1, "fn first() {}"),
            ],
        );
        let b = out.find("## Matches in b.rs").expect(&out);
        let a = out.find("## Matches in a.rs").expect(&out);
        assert!(b < a, "rank order, not path sort, got: {out}");
        assert!(
            out.starts_with("Found 2 chunks (view: expanded):"),
            "got: {out}"
        );
    }

    #[test]
    fn agent_view_does_not_dump_long_chunk() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut src = String::from("fn long_function() {\n");
        for i in 1..=50 {
            src.push_str(&format!("    println!(\"Line {i}\");\n"));
        }
        src.push_str("}\n");
        std::fs::write(dir.path().join("long.rs"), &src).unwrap();

        let out = render(
            dir.path(),
            &[code_hit("long.rs", 1, 52, "fn long_function() {")],
        );
        assert!(
            out.contains("### ") && out.contains("fn long_function"),
            "got: {out}"
        );
        assert!(
            out.contains("lines remaining"),
            "must cap the 60-line-class chunk, got: {out}"
        );
        assert!(
            !out.contains("Line 40"),
            "must not dump the full indexed window, got: {out}"
        );
        let shown_lines = out.matches("println!").count();
        assert!(
            shown_lines <= EXPANDED_CHUNK_LINES as usize,
            "too many body lines ({shown_lines}), got: {out}"
        );
    }

    #[test]
    fn agent_view_missing_file_falls_back_to_summary_without_score() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = render(
            dir.path(),
            &[code_hit("gone.rs", 10, 69, "pub fn save_theme() {")],
        );
        assert!(out.contains("## Matches in gone.rs"), "got: {out}");
        assert!(out.contains("### L10-69"), "got: {out}");
        assert!(out.contains("pub fn save_theme() {"), "got: {out}");
        assert!(!out.contains("(score"), "got: {out}");
    }

    #[test]
    fn agent_view_tight_budget_drops_to_context_without_breadcrumb() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("store.rs"),
            "impl Store {\n    fn save(&self) {\n        let hit = 1;\n    }\n}\n",
        )
        .unwrap();
        let hits = [code_hit("store.rs", 2, 4, "fn save(&self) {")];
        let expanded = format_code_search_hits(Some(dir.path()), &hits, CODE_SEARCH_TOKEN_BUDGET);
        assert!(
            expanded.contains("(view: expanded)") && expanded.contains("impl Store › fn save"),
            "full budget should be expanded, got: {expanded}"
        );
        let budget = crate::session::count_text_tokens(&expanded).saturating_sub(1);
        let out = format_code_search_hits(Some(dir.path()), &hits, budget.max(1));
        assert!(
            out.contains("(view: context)"),
            "just-under-expanded budget must select context, got: {out}"
        );
        assert!(
            !out.contains("impl Store › fn save"),
            "context omits breadcrumb, got: {out}"
        );
        assert!(
            out.contains("### L"),
            "still has a line heading, got: {out}"
        );
        assert!(out.contains("```"), "still fenced, got: {out}");
    }

    #[test]
    fn agent_view_tiny_budget_truncates_hit_count() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut hits = Vec::new();
        for i in 0..8 {
            let name = format!("f{i}.rs");
            std::fs::write(
                dir.path().join(&name),
                format!("fn item_{i}() {{ {} }}\n", "x".repeat(80)),
            )
            .unwrap();
            hits.push(code_hit(&name, 1, 1, "fn item"));
        }
        let all = format_code_search_hits(Some(dir.path()), &hits, CODE_SEARCH_TOKEN_BUDGET);
        let all_tokens = crate::session::count_text_tokens(&all);
        assert!(
            all.contains("Found 8 chunks"),
            "full budget keeps every hit, got: {all}"
        );
        let out = format_code_search_hits(Some(dir.path()), &hits, (all_tokens / 4).max(1));
        assert!(
            out.contains("Showing ") && out.contains(" of 8 chunks (view: context):"),
            "truncated page, got: {out}"
        );
        assert!(
            !out.contains("use offset"),
            "no pagination knob, got: {out}"
        );
    }

    #[test]
    fn fence_escapes_backticks_in_source() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.md"), "see ```inside``` fence\n").unwrap();
        let out = render(
            dir.path(),
            &[code_hit("a.md", 1, 1, "see ```inside``` fence")],
        );
        assert!(
            out.contains("````") || out.matches("```").count() >= 4,
            "safe fence, got: {out}"
        );
        assert!(out.contains("see ```inside``` fence"), "got: {out}");
    }
}
