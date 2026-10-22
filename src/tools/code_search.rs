use crate::context_pipeline::Context;
use crate::engines::code_search::{
    DEFAULT_TOP_K, IndexStatus, MAX_TOP_K, enclosing_scopes, format_breadcrumb, lines_slice,
    resolve_index_view, syntax_ancestor_snippet,
};
use crate::engines::{
    CodeSearchCallGate, EngineState, RetrievalCorpus, RetrievalFilters, RetrievalHit,
    RetrievalModality, RetrievalQuery, WorkspaceEngines,
};
use crate::tool::Tool;
use crate::types::ToolCallResult;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Same budget as grep `content`: one response must leave room to reason.
const CODE_SEARCH_TOKEN_BUDGET: usize = 6_000;
/// Max lines shown from an indexed chunk when AST ancestor is unavailable (Expanded).
const EXPANDED_CHUNK_LINES: u32 = 12;
/// Max lines shown from an indexed chunk in the compact view.
const CONTEXT_CHUNK_LINES: u32 = 6;

pub struct CodeSearchTool {
    engines: WorkspaceEngines,
}

impl CodeSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self { engines }
    }

    /// Start update/rebuild when the disk plan says work is due. Returns true
    /// when this call kicked off consume. Never waits on the worker: an embed
    /// holds the client lock, and `worker_alive` takes that lock.
    fn maybe_consume_index(&self) -> bool {
        // `worker_pid` is an atomic. `worker_alive` blocks on the client lock.
        if self.engines.is_refresh_busy() || self.engines.code_search().worker_pid().is_none() {
            return false;
        }
        let Some(root) = self.engines.code_search().workspace_root() else {
            return false;
        };
        let view = resolve_index_view(&root, self.engines.state("code_search"));
        if matches!(view.status, IndexStatus::Building | IndexStatus::Refreshing) {
            return false;
        }
        match crate::engines::code_search::index_work_from_disk(&root) {
            crate::engines::code_search::IndexWork::None => false,
            _ => self.engines.consume_code_index_work().is_ok(),
        }
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

        // WP-148: visibility is bind-only; runtime state lands here as an ok result.
        if let Some(result) = self.not_ready_result() {
            return result;
        }

        // Kick index work if due, but never block the turn waiting for it.
        let _ = self.maybe_consume_index();

        let sync = crate::tools::sync_pct_for_workspace(&self.engines, None);
        #[cfg(test)]
        SEARCH_CALLS.with(|c| c.set(c.get() + 1));
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
                    return ToolCallResult::ok(crate::tools::prepend_sync_pct(
                        sync.as_ref(),
                        &format!("No matching code chunks found{scope}."),
                    ));
                }
                let root = self.engines.code_search().workspace_root();
                ToolCallResult::ok(crate::tools::prepend_sync_pct(
                    sync.as_ref(),
                    &format_code_search_hits(root.as_deref(), &hits, CODE_SEARCH_TOKEN_BUDGET),
                ))
            }
            Err(e) if crate::engines::is_search_worker_busy(&e) => warming_result(sync.as_ref()),
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/code_search.md"))
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(30))
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
            let line_label = crate::tool::format_line_label(snippet.start_line, snippet.end_line);
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
            body.push_str(&crate::tool::snippet::fence_block(&snippet.text));
            if snippet.remaining_lines > 0 {
                let reason = if snippet.from_ancestor {
                    "ancestor node"
                } else {
                    "this chunk"
                };
                body.push_str(&format!(
                    "\n{} lines remaining in {reason}.\n",
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

const ENGINE_OFF_BODY: &str = "代码引擎未开启，无结果。可在设置中开启代码检索引擎。";
const WARMING_BODY: &str = "引擎预热中，暂无结果，请稍后重试。";

#[cfg(test)]
thread_local! {
    /// Test-only: counts real retrieval calls issued by `call_inner`.
    static SEARCH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl CodeSearchTool {
    /// Contract v1 §C, decision order: engine off → failed → warming → (None = ready).
    fn not_ready_result(&self) -> Option<ToolCallResult> {
        use crate::tools::{SyncLineLabel, format_sync_pct_line_labeled};
        let root = self.engines.code_search().workspace_root();
        let desired = root
            .as_deref()
            .is_some_and(|r| crate::config::workspace::workspace_engine_desired(r, "code_search"));
        if !desired {
            let line = format_sync_pct_line_labeled(None, Some(SyncLineLabel::EngineOff));
            return Some(ToolCallResult::ok(format!("{line}\n{ENGINE_OFF_BODY}")));
        }
        let sync = crate::tools::sync_pct_for_workspace(&self.engines, root.as_deref());
        let gate = self.engines.code_search_call_gate();
        let failed_detail = match (&gate, self.engines.state("code_search")) {
            (_, Some(EngineState::Failed)) => Some(
                self.engines
                    .last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into()),
            ),
            (CodeSearchCallGate::Failed(detail), _) => Some(detail.clone()),
            _ => None,
        };
        if let Some(detail) = failed_detail {
            let line =
                format_sync_pct_line_labeled(sync.as_ref(), Some(SyncLineLabel::Failed(&detail)));
            let body = detail.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some(ToolCallResult::ok(format!("{line}\n代码引擎失败：{body}")));
        }
        let searchable = matches!(self.engines.state("code_search"), Some(EngineState::Warm))
            && matches!(gate, CodeSearchCallGate::Ready);
        if !searchable {
            return Some(warming_result(sync.as_ref()));
        }
        None
    }
}

fn warming_result(sync: Option<&crate::engines::progress::CorpusProgress>) -> ToolCallResult {
    use crate::tools::{SyncLineLabel, format_sync_pct_line_labeled};
    let line = format_sync_pct_line_labeled(sync, Some(SyncLineLabel::Warming));
    ToolCallResult::ok(format!("{line}\n{WARMING_BODY}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Queue `n` modified (non-deleted) files via the engines queue record (WP-134).
    fn write_queue(root: &std::path::Path, n: usize) {
        let pending: std::collections::HashSet<(String, bool)> =
            (0..n).map(|i| (format!("src/f{i}.rs"), false)).collect();
        crate::engines::code_search::write_code_queue(root, &pending);
    }

    #[test]
    fn warming_returns_immediately_without_sleeping() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warming);
        let tool = CodeSearchTool::new(engines);
        let started = std::time::Instant::now();
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "Warming path must not block"
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        let first = result.content.lines().next().unwrap();
        assert_v2(first);
        assert!(first.contains("(warming)"), "{first}");
        assert!(result.content.contains(WARMING_BODY), "{}", result.content);
    }

    #[test]
    fn indexing_wait_message_reads_job_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        // WP-143: first build / rebuild run as a refresh job (no Building job).
        crate::engines::code_search::begin_refreshing(root);
        crate::engines::code_search::update_chunk_progress(root, 4, Some(8));

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warming);

        let result = CodeSearchTool::new(engines).call_inner(serde_json::json!({ "query": "q" }));
        let first = result.content.lines().next().unwrap().to_string();
        assert_v2(&first);
        // sync_pct contract v2: the exact chunk counts ride the warming line.
        assert!(first.ends_with("% (warming) chunks 4/8"), "{first}");
        assert!(
            !result.content.contains("No matching code"),
            "must not fake hits while warming: {}",
            result.content
        );
    }

    #[test]
    fn warm_failed_index_job_returns_error_not_hits() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        crate::engines::code_search::mark_index_job_failed(root, "embed exploded");

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let tool = CodeSearchTool::new(engines);
        let before = search_calls();
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert_eq!(search_calls(), before, "failed must not search");
        assert_eq!(result.level, crate::types::ToolSignalLevel::Ok);
        let first = result.content.lines().next().unwrap();
        assert_v2(first);
        assert!(first.starts_with("sync_pct: failed ("), "{first}");
        assert!(
            result.content.contains("embed exploded"),
            "{}",
            result.content
        );
    }

    const V2: &str = r"^sync_pct: (?:(\d{1,3})%|(scanning|failed|unknown))(?: \(([^()\n]+)\))?(?: chunks (\d+)\/(\d+))?$";

    fn assert_v2(line: &str) {
        assert!(
            regex::Regex::new(V2).unwrap().is_match(line),
            "v2: {line:?}"
        );
    }

    fn search_calls() -> usize {
        SEARCH_CALLS.with(|c| c.get())
    }

    /// Ready-ish index (gate Ready when Warm) with desired=true.
    fn ready_root(root: &Path) {
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let index_dir = crate::engines::code_search::index_dir(root);
        std::fs::create_dir_all(&index_dir).unwrap();
        std::fs::write(index_dir.join("chunks.jsonl"), "").unwrap();
        std::fs::write(index_dir.join("vectors.usearch"), "").unwrap();
        write_queue(root, 1);
    }

    #[test]
    fn not_ready_issues_no_search() {
        let off = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(off.path().join(".litecode")).unwrap();
        let on = tempfile::TempDir::new().unwrap();
        crate::config::workspace::enable_code_search_engine(on.path()).unwrap();
        for (root, state) in [
            (off.path(), EngineState::Warm),
            (on.path(), EngineState::Warming),
            (on.path(), EngineState::Failed),
        ] {
            let e = WorkspaceEngines::new();
            e.code_search().set_workspace(root.to_path_buf());
            e.set_state_for_test("code_search", state);
            let before = search_calls();
            let r = CodeSearchTool::new(e).call_inner(serde_json::json!({ "query": "q" }));
            assert_eq!(search_calls(), before, "{state:?} must not search");
            assert_eq!(r.level, crate::types::ToolSignalLevel::Ok);
            assert_eq!(r.content.lines().count(), 2, "empty result: {}", r.content);
        }
        // Control: ready path does issue exactly one search.
        let ready = tempfile::TempDir::new().unwrap();
        ready_root(ready.path());
        let e = WorkspaceEngines::new();
        e.code_search().set_workspace(ready.path().to_path_buf());
        e.set_state_for_test("code_search", EngineState::Warm);
        let before = search_calls();
        let _ = CodeSearchTool::new(e).call_inner(serde_json::json!({ "query": "q" }));
        assert_eq!(search_calls(), before + 1);
    }

    #[test]
    fn warm_stale_is_searchable_without_warm_wait() {
        // WP-057/060: Warm + Stale ⇒ gate Ready (silent refresh). Tools visibility /
        // result-head sync% remain WP-061; this test only tracks the engine gate.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        crate::engines::code_search::init_workspace_index(root).unwrap();
        let mut meta = crate::engines::code_search::IndexMeta::shell();
        meta.indexed_files = 1;
        meta.indexed_chunks = 1;
        crate::engines::code_search::write_meta(root, &meta).unwrap();
        let index_dir = crate::engines::code_search::index_dir(root);
        std::fs::write(index_dir.join("chunks.jsonl"), "").unwrap();
        std::fs::write(index_dir.join("vectors.usearch"), "").unwrap();
        write_queue(root, 2);

        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        assert_eq!(
            engines.code_search_call_gate(),
            crate::engines::CodeSearchCallGate::Ready
        );
        assert!(engines.is_searchable("code_search"));

        let started = std::time::Instant::now();
        let tool = CodeSearchTool::new(engines);
        let result = tool.call_inner(serde_json::json!({ "query": "auth" }));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "stale Warm must not burn the 30s warm-wait budget"
        );
        // No live worker → search errors; must not be the old Stale-Wait warning path.
        assert!(
            result.level != crate::types::ToolSignalLevel::Warning
                || !result.content.contains("Try again shortly"),
            "must not Wait on Stale: {}",
            result.content
        );
    }
}
