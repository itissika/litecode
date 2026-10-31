use crate::engines::code_search::{
    DEFAULT_TOP_K, IndexStatus, MAX_TOP_K, code_tokenize, enclosing_scopes, format_breadcrumb,
    resolve_index_view, scope_at,
};
use crate::engines::{
    CodeSearchCallGate, EngineState, RetrievalCorpus, RetrievalFilters, RetrievalHit,
    RetrievalModality, RetrievalQuery, WorkspaceEngines,
};
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tools::file_path::coded_error;
use crate::types::{ToolCallResult, ToolErrorKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

const DEFAULT_MIN_SCORE: f64 = 0.80;
/// Hard cap for one hit's source excerpt. No caller-controlled override exists.
const SNIPPET_MAX_LINES: u32 = 24;
/// Guard against minified/generated lines blowing up one snippet.
const SNIPPET_MAX_LINE_CHARS: usize = 500;

const ENGINE_OFF_MESSAGE: &str = "Code search engine is off. Enable it in Settings.";
const WARMING_MESSAGE: &str = "Code search engine is warming; no results yet.";
const BUSY_MESSAGE: &str = "Code search is busy; try again shortly.";

pub struct CodeSearchTool {
    engines: WorkspaceEngines,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct CodeSearchData {
    hits: Vec<CodeSearchHit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct CodeSearchHit {
    path: String,
    anchor_line: u32,
    start_line: u32,
    end_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    /// Which retrieval leg(s) admitted this chunk: `lex`, `dense`, or `both`.
    via: String,
    /// Raw dense cosine similarity; 0.0 means the hit is lexical-only.
    dense_similarity: f64,
    /// The anchor line itself, cheap enough for a table cell.
    text: String,
    snippet: String,
}

impl CodeSearchData {
    fn success(hits: Vec<CodeSearchHit>, message: Option<String>) -> Self {
        Self { hits, message }
    }
}

impl CodeSearchTool {
    pub fn new(engines: WorkspaceEngines) -> Self {
        Self { engines }
    }

    /// Start update/rebuild when the disk plan says work is due. Returns true
    /// when this call kicked off consume. Never waits on the worker.
    fn maybe_consume_index(&self) -> bool {
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

    fn present(data: CodeSearchData, content: String) -> ToolCallResult {
        ToolOutcome::ok(data).into_call_result(content)
    }

    fn not_ready_result(&self) -> Option<ToolCallResult> {
        let root = self.engines.code_search().workspace_root();
        let desired = root.as_deref().is_some_and(|root| {
            crate::config::workspace::workspace_engine_desired(root, "code_search")
        });
        if !desired {
            return Some(coded_error("unavailable", ENGINE_OFF_MESSAGE));
        }

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
            let clean = detail.split_whitespace().collect::<Vec<_>>().join(" ");
            return Some(coded_error("unavailable", clean));
        }

        let searchable = matches!(self.engines.state("code_search"), Some(EngineState::Warm))
            && matches!(gate, CodeSearchCallGate::Ready);
        if !searchable {
            return Some(coded_error("unavailable", WARMING_MESSAGE));
        }

        None
    }

    fn search_from_input(
        &self,
        input: Value,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(query) => query,
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false)
                    .with_error_code("invalid_input");
            }
        };
        let glob = input["include_pattern"]
            .as_str()
            .filter(|glob| !glob.is_empty());
        let mut notes: Vec<String> = Vec::new();
        let (top_k, top_k_note) = match parse_top_k(&input) {
            Ok(parsed) => parsed,
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false)
                    .with_error_code("invalid_input");
            }
        };
        notes.extend(top_k_note);
        let (min_score, min_score_note) = match parse_min_score(&input) {
            Ok(parsed) => parsed,
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false)
                    .with_error_code("invalid_input");
            }
        };
        notes.extend(min_score_note);

        if let Some(result) = self.not_ready_result() {
            return result;
        }

        let _ = self.maybe_consume_index();
        let sync = crate::tools::sync_pct_for_workspace(&self.engines, None);

        #[cfg(test)]
        SEARCH_CALLS.with(|calls| calls.set(calls.get() + 1));

        let request = RetrievalQuery {
            query: query.to_string(),
            corpus: RetrievalCorpus::Code,
            modality: RetrievalModality::Semantic,
            filters: RetrievalFilters {
                glob: glob.map(str::to_string),
                ..Default::default()
            },
            top_k,
            min_score,
            offset: 0,
            workspace_root: None,
        };
        let result = match cancel {
            Some(cancel) => self.engines.search_with_cancel(request, Some(cancel)),
            None => self.engines.search(request),
        };

        match result {
            Ok(hits) => {
                let root = self.engines.code_search().workspace_root();
                let hits: Vec<CodeSearchHit> = hits
                    .iter()
                    .filter_map(|hit| code_hit(root.as_deref(), query, hit))
                    .collect();
                let mut message = hits.is_empty().then(|| {
                    let scope = glob
                        .map(|pattern| format!(" for include_pattern '{pattern}'"))
                        .unwrap_or_default();
                    format!("No matching code chunks found{scope}.")
                });
                if !notes.is_empty() {
                    let note = notes.join(" ");
                    message = Some(match message {
                        Some(message) => format!("{message} {note}"),
                        None => note,
                    });
                }
                let data = CodeSearchData::success(hits, message);
                Self::present(data, Self::success_progress_content(sync.as_ref()))
            }
            Err(_error) if cancel.is_some_and(|token| token.is_cancelled()) => {
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Cancelled,
                    crate::engines::SEARCH_CANCELLED,
                    false,
                )
                .with_error_code("cancelled")
            }
            Err(error) if crate::engines::is_search_worker_busy(&error) => {
                coded_error("unavailable", BUSY_MESSAGE)
            }
            Err(error) => coded_error("unavailable", error.to_string()),
        }
    }

    fn success_progress_content(sync: Option<&crate::engines::progress::CorpusProgress>) -> String {
        unfinished_sync_line(sync).unwrap_or_default()
    }
}

impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        "code_search"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    /// The worker client lock is held across refresh/embed and the underlying
    /// search is blocking. Queue in a cancellable blocking task so neither the
    /// async runtime nor a cancelled run_code turn is stalled.
    fn execute(
        &self,
        input: Value,
        execution: crate::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = CodeSearchTool::new(self.engines.clone());
        let cancel = execution.cancel.clone();
        Box::pin(async move {
            let join =
                tokio::task::spawn_blocking(move || tool.search_from_input(input, Some(&cancel)));
            match join.await {
                Ok(result) => result,
                Err(error) => ToolCallResult::error_with_kind(
                    ToolErrorKind::Internal,
                    format!("code_search worker failed: {error}"),
                    false,
                )
                .with_error_code("internal"),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        // Test/synchronous helper: no caller token, so do not queue behind a
        // refresh. Production always enters through `execute`.
        self.search_from_input(input, None)
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        // A queued search can legitimately outlive 30s. The enclosing run_code
        // budget owns the deadline and cancels this cancellable tool.
        None
    }

    fn is_cancellable(&self) -> bool {
        true
    }
}

fn parse_top_k(input: &Value) -> Result<(usize, Option<String>), String> {
    let clamped = crate::tool::clamp::clamp_int(input, "top_k", 1, MAX_TOP_K as u64, "hits")?;
    Ok(match clamped {
        None => (DEFAULT_TOP_K, None),
        Some(clamped) => (clamped.value as usize, clamped.note),
    })
}

fn parse_min_score(input: &Value) -> Result<(Option<f64>, Option<String>), String> {
    let clamped = crate::tool::clamp::clamp_float(input, "min_score", 0.0, 1.0, "")?;
    Ok(match clamped {
        None => (Some(DEFAULT_MIN_SCORE), None),
        Some(clamped) => ((clamped.value > 0.0).then_some(clamped.value), clamped.note),
    })
}

fn unfinished_sync_line(sync: Option<&crate::engines::progress::CorpusProgress>) -> Option<String> {
    if sync.and_then(crate::engines::progress::CorpusProgress::percent) == Some(100) {
        None
    } else {
        Some(crate::tools::format_sync_pct_line(sync))
    }
}

fn code_hit(root: Option<&Path>, query: &str, hit: &RetrievalHit) -> Option<CodeSearchHit> {
    let RetrievalHit::Code {
        path,
        start_line,
        end_line,
        summary,
        dense_similarity,
        lexical_match,
        ..
    } = hit
    else {
        return None;
    };
    let source = root.and_then(|root| std::fs::read_to_string(root.join(path)).ok());
    Some(result_for_source(
        path,
        source.as_deref(),
        query,
        *start_line,
        *end_line,
        summary,
        *dense_similarity,
        *lexical_match,
    ))
}

fn result_for_source(
    path: &str,
    source: Option<&str>,
    query: &str,
    chunk_start: u32,
    chunk_end: u32,
    summary: &str,
    dense_similarity: f64,
    lexical_match: bool,
) -> CodeSearchHit {
    let via = hit_via(dense_similarity, lexical_match);
    let Some(source) = source else {
        return CodeSearchHit {
            path: path.to_string(),
            anchor_line: chunk_start.max(1),
            start_line: chunk_start.max(1),
            end_line: chunk_end.max(chunk_start).max(1),
            symbol: None,
            via,
            dense_similarity,
            text: summary.to_string(),
            snippet: summary.to_string(),
        };
    };
    let line_count = source.lines().count() as u32;
    if line_count == 0 {
        return CodeSearchHit {
            path: path.to_string(),
            anchor_line: chunk_start.max(1),
            start_line: chunk_start.max(1),
            end_line: chunk_end.max(chunk_start).max(1),
            symbol: None,
            via,
            dense_similarity,
            text: summary.to_string(),
            snippet: summary.to_string(),
        };
    }
    let chunk_start = chunk_start.clamp(1, line_count);
    let chunk_end = chunk_end.max(chunk_start).min(line_count);

    let terms = query_terms_without_noise(query);
    let identifiers = code_tokenize::extract_identifiers(query);
    let term_anchor = best_query_anchor(source, chunk_start, chunk_end, &terms, &identifiers);

    let anchor_line = if let Some((line, _)) = term_anchor {
        line
    } else {
        let probe = first_informative_non_symbol_line(source, chunk_start, chunk_end)
            .unwrap_or(chunk_start);
        scope_at(path, source, probe, probe)
            .map(|scope| scope.start_line.clamp(chunk_start, chunk_end))
            .unwrap_or(probe)
    };

    let scope = scope_at(path, source, anchor_line, anchor_line);
    let symbol = scope
        .as_ref()
        .map(|scope| scope.chain.clone())
        .or_else(|| format_breadcrumb(&enclosing_scopes(path, source, anchor_line)));

    let (start_line, end_line) = if let Some(scope) = scope.as_ref() {
        let scope_start = scope.start_line.max(1).min(line_count);
        let scope_end = scope.end_line.max(scope_start).min(line_count);
        if scope_end.saturating_sub(scope_start) < SNIPPET_MAX_LINES {
            (scope_start, scope_end)
        } else {
            window_within(anchor_line, SNIPPET_MAX_LINES, scope_start, scope_end)
        }
    } else {
        window_within(anchor_line, SNIPPET_MAX_LINES, 1, line_count)
    };

    let snippet = bounded_snippet(source, start_line, end_line);
    let snippet = if snippet.is_empty() {
        summary.to_string()
    } else {
        snippet
    };
    let anchor_line = anchor_line.clamp(1, line_count);
    let text = bounded_snippet(source, anchor_line, anchor_line)
        .trim()
        .to_string();
    let text = if text.is_empty() {
        summary.to_string()
    } else {
        text
    };
    CodeSearchHit {
        path: path.to_string(),
        anchor_line,
        start_line,
        end_line,
        symbol,
        via,
        dense_similarity,
        text,
        snippet,
    }
}

fn hit_via(dense_similarity: f64, lexical_match: bool) -> String {
    match (lexical_match, dense_similarity > 0.0) {
        (true, true) => "both".to_string(),
        (true, false) => "lex".to_string(),
        (false, true) => "dense".to_string(),
        (false, false) => "dense".to_string(),
    }
}

fn bounded_snippet(source: &str, start_line: u32, end_line: u32) -> String {
    let mut out = String::new();
    for (index, line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        if line_no < start_line || line_no > end_line {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        let mut chars = line.chars();
        let kept: String = chars.by_ref().take(SNIPPET_MAX_LINE_CHARS).collect();
        out.push_str(&kept);
        if chars.next().is_some() {
            out.push_str("...");
        }
    }
    out
}

fn window_within(anchor: u32, max_lines: u32, low: u32, high: u32) -> (u32, u32) {
    let low = low.max(1).min(high.max(1));
    let high = high.max(low);
    let max_lines = max_lines.max(1).min(high - low + 1);
    let before = max_lines.saturating_sub(1) / 2;
    let start = anchor.saturating_sub(before).clamp(low, high);
    let end = start.saturating_add(max_lines - 1).min(high);
    let start = end.saturating_sub(max_lines - 1).max(low);
    (start, end)
}

fn query_terms_without_noise(query: &str) -> Vec<String> {
    code_tokenize::query_terms(query)
        .into_iter()
        .filter(|term| !is_query_noise(term))
        .collect()
}

fn is_query_noise(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "code"
            | "do"
            | "does"
            | "find"
            | "for"
            | "from"
            | "how"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "that"
            | "the"
            | "this"
            | "to"
            | "where"
            | "which"
            | "with"
            | "work"
            | "works"
    )
}

fn best_query_anchor(
    source: &str,
    chunk_start: u32,
    chunk_end: u32,
    terms: &[String],
    identifiers: &[String],
) -> Option<(u32, u32)> {
    if terms.is_empty() && identifiers.is_empty() {
        return None;
    }
    let mut best: Option<(u32, u32)> = None;
    for (index, line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        if line_no < chunk_start || line_no > chunk_end || !is_informative_line(line) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let line_terms: HashSet<String> = code_tokenize::query_terms(line).into_iter().collect();
        let mut score = 0u32;
        for term in terms {
            if line_terms.contains(term) {
                score = score.saturating_add(2);
            } else if term.len() >= 4 && lower.contains(term) {
                score = score.saturating_add(1);
            }
        }
        for identifier in identifiers {
            if line_terms.contains(identifier) || lower.contains(identifier) {
                score = score.saturating_add(8);
            }
        }
        if is_comment_line(line) {
            score /= 2;
        }
        if score > 0 && best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((line_no, score));
        }
    }
    best
}

fn first_informative_non_symbol_line(source: &str, start: u32, end: u32) -> Option<u32> {
    let mut first_informative = None;
    let mut first_comment = None;
    for (index, line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        if line_no < start || line_no > end || !is_informative_line(line) {
            continue;
        }
        first_informative.get_or_insert(line_no);
        if is_comment_line(line) {
            first_comment.get_or_insert(line_no);
            continue;
        }
        if !is_symbol_line(line) {
            return Some(line_no);
        }
    }
    first_comment.or(first_informative)
}

fn is_informative_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    !trimmed
        .chars()
        .all(|ch| ch.is_ascii_punctuation() || ch.is_whitespace())
}

fn is_comment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with('#')
        || trimmed.starts_with("--")
}

fn is_symbol_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let mut words = trimmed.split_whitespace();
    let Some(mut first) = words.next() else {
        return false;
    };
    // Strip leading visibility / modifiers.
    while matches!(
        first,
        "pub"
            | "pub(crate)"
            | "public"
            | "private"
            | "protected"
            | "async"
            | "static"
            | "export"
            | "unsafe"
            | "const"
    ) {
        let Some(next) = words.next() else {
            return false;
        };
        first = next;
    }
    matches!(
        first,
        "fn" | "def"
            | "function"
            | "class"
            | "struct"
            | "enum"
            | "trait"
            | "impl"
            | "interface"
            | "namespace"
            | "module"
            | "mod"
            | "type"
            | "record"
            | "union"
            | "macro"
    )
}

#[cfg(test)]
thread_local! {
    /// Test-only: counts real retrieval calls issued by `search_from_input`.
    static SEARCH_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_queue(root: &std::path::Path, n: usize) {
        let pending: std::collections::HashSet<(String, bool)> =
            (0..n).map(|i| (format!("src/f{i}.rs"), false)).collect();
        crate::engines::code_search::write_code_queue(root, &pending);
    }

    fn ready_root(root: &Path) {
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let index_dir = crate::engines::code_search::index_dir(root);
        std::fs::create_dir_all(&index_dir).unwrap();
        std::fs::write(index_dir.join("chunks.jsonl"), "").unwrap();
        std::fs::write(index_dir.join("vectors.usearch"), "").unwrap();
        write_queue(root, 1);
    }

    fn assert_unavailable(result: &ToolCallResult) {
        assert_eq!(result.error_code.as_deref(), Some("unavailable"));
        assert!(result.data.is_none(), "{result:?}");
        assert!(result.error.is_some(), "{result:?}");
    }

    #[test]
    fn warming_returns_immediately_as_unavailable() {
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
        assert_unavailable(&result);
        assert!(result.content.contains("warming"));
    }

    #[test]
    fn off_and_failed_are_unavailable() {
        let off = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(off.path().join(".litecode")).unwrap();
        let engines = WorkspaceEngines::new();
        engines
            .code_search()
            .set_workspace(off.path().to_path_buf());
        let result = CodeSearchTool::new(engines).call_inner(serde_json::json!({ "query": "q" }));
        assert_unavailable(&result);

        let failed = tempfile::TempDir::new().unwrap();
        let root = failed.path();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        crate::engines::code_search::mark_index_job_failed(root, "embed exploded");
        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        let before = search_calls();
        let result = CodeSearchTool::new(engines).call_inner(serde_json::json!({ "query": "q" }));
        assert_eq!(search_calls(), before, "failed must not search");
        assert_unavailable(&result);
        assert!(
            result
                .error
                .as_ref()
                .is_some_and(|error| error.message.contains("embed exploded")),
            "{result:?}"
        );
    }

    #[test]
    fn query_anchor_uses_relevant_source_line_and_ast_symbol() {
        let mut source = String::from("pub fn outer() {\n");
        for i in 0..40 {
            source.push_str(&format!("    let filler_{i} = {i};\n"));
        }
        source.push_str("    let auth_token = compute_auth_token();\n");
        source.push_str("    consume(auth_token);\n}\n");
        let hit = result_for_source(
            "src/auth.rs",
            Some(&source),
            "compute_auth_token",
            1,
            44,
            "pub fn outer() {",
            0.0,
            true,
        );
        assert_eq!(hit.anchor_line, 42);
        assert_eq!(hit.text, "let auth_token = compute_auth_token();");
        assert_eq!(hit.via, "lex");
        assert!(hit.snippet.contains("compute_auth_token"), "{hit:?}");
        assert!((hit.end_line - hit.start_line + 1) <= SNIPPET_MAX_LINES);
        assert!(hit.start_line <= hit.anchor_line && hit.anchor_line <= hit.end_line);
        assert!(
            hit.symbol
                .as_deref()
                .unwrap_or_default()
                .contains("fn outer")
        );
    }

    #[test]
    fn long_ast_scope_is_centered_on_the_relevant_anchor_and_capped() {
        let mut source = String::from("pub fn wide() {\n");
        for i in 0..80 {
            source.push_str(&format!("    let filler_{i} = {i};\n"));
        }
        source.push_str("    let requested_anchor = auth_middleware();\n");
        source.push_str("}\n");
        let hit = result_for_source(
            "src/wide.rs",
            Some(&source),
            "auth_middleware",
            1,
            83,
            "pub fn wide() {",
            0.0,
            true,
        );
        assert_eq!(hit.anchor_line, 82);
        assert!((hit.end_line - hit.start_line + 1) <= SNIPPET_MAX_LINES);
        assert!(hit.start_line <= hit.anchor_line && hit.anchor_line <= hit.end_line);
        assert!(hit.snippet.contains("auth_middleware"));
        assert!(!hit.snippet.starts_with("pub fn wide() {"));
    }

    #[test]
    fn short_ast_scope_is_emitted_complete() {
        let mut source = String::new();
        for _ in 0..20 {
            source.push_str("// preface\n");
        }
        source.push_str("fn short_scope() {\n    let needle = 1;\n    needle + 1\n}\n");
        source.push_str("fn other() {\n    let other_needle = 2;\n    other_needle\n}\n");
        let hit = result_for_source(
            "src/short.rs",
            Some(&source),
            "needle",
            1,
            25,
            "fn short_scope() {",
            0.0,
            true,
        );
        assert_eq!(hit.anchor_line, 22);
        assert_eq!((hit.start_line, hit.end_line), (21, 24));
        assert_eq!(hit.symbol.as_deref(), Some("fn short_scope"));
        assert!(hit.snippet.starts_with("fn short_scope()"));
    }

    #[test]
    fn overlong_source_lines_are_capped_without_losing_the_anchor() {
        let source = format!("needle {}\n", "x".repeat(2_000));
        let hit = result_for_source("src/long.rs", Some(&source), "needle", 1, 1, "", 0.0, true);
        assert!(hit.snippet.starts_with("needle "));
        assert!(hit.snippet.ends_with("..."));
        assert!(hit.snippet.chars().count() <= SNIPPET_MAX_LINE_CHARS + 3);
    }

    #[test]
    fn via_distinguishes_lexical_dense_and_both() {
        assert_eq!(hit_via(0.0, true), "lex");
        assert_eq!(hit_via(0.0, false), "dense");
        assert_eq!(hit_via(0.42, true), "both");
        assert_eq!(hit_via(0.42, false), "dense");
    }

    #[test]
    fn missing_source_falls_back_to_summary_and_never_uses_first_line_as_snippet() {
        let hit = result_for_source(
            "gone.rs",
            None,
            "target",
            10,
            20,
            "summary fallback",
            0.0,
            true,
        );
        assert_eq!(hit.snippet, "summary fallback");
        assert_eq!(hit.text, "summary fallback");
        assert_eq!(hit.anchor_line, 10);
        assert_eq!((hit.start_line, hit.end_line), (10, 20));
    }

    #[test]
    fn no_term_and_no_ast_picks_first_informative_non_symbol_line() {
        let source = "\n{\n// path notes\nfn declared_but_not_ast() {}\nplain informational line\n";
        let hit = result_for_source(
            "README.txt",
            Some(source),
            "unrelated words",
            1,
            5,
            "",
            0.0,
            false,
        );
        assert_eq!(hit.anchor_line, 5);
        assert!(hit.snippet.contains("plain informational line"));
        assert!(hit.symbol.is_none());
    }

    #[test]
    fn success_at_one_hundred_percent_has_no_sync_noise() {
        let sync = crate::engines::progress::CorpusProgress::ready(7, 0);
        assert_eq!(super::unfinished_sync_line(Some(&sync)), None);
        let sync = crate::engines::progress::CorpusProgress::ready(7, 1);
        assert!(
            super::unfinished_sync_line(Some(&sync))
                .unwrap()
                .starts_with("sync_pct:")
        );
    }

    fn code_search_surface() -> crate::tool::code_contract::CodeSurface {
        let tool: std::sync::Arc<dyn crate::tool::Tool> =
            std::sync::Arc::new(CodeSearchTool::new(WorkspaceEngines::new()));
        crate::tool::code_contract::build_surface(&[tool]).expect("code_search SDK contract")
    }

    #[test]
    fn schema_is_single_query_without_hard_numeric_bounds_and_no_paging() {
        assert_eq!((DEFAULT_TOP_K, MAX_TOP_K), (8, 20));
        let surface = code_search_surface();
        let schema = &surface.tools["code_search"].input_schema;
        let properties = schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("query"));
        assert!(!properties.contains_key("offset"));
        assert!(!properties.contains_key("cursor"));
        assert!(!properties.contains_key("page"));
        assert!(properties["top_k"].get("maximum").is_none());
        assert!(properties["min_score"].get("minimum").is_none());
        assert!(properties["min_score"].get("maximum").is_none());
        assert_eq!(
            parse_min_score(&serde_json::json!({})),
            Ok((Some(0.8), None))
        );
        assert_eq!(
            parse_min_score(&serde_json::json!({"min_score": 0})),
            Ok((None, None))
        );
        let (score, note) = parse_min_score(&serde_json::json!({"min_score": 1.1})).unwrap();
        assert_eq!(score, Some(1.0));
        assert_eq!(
            note.as_deref(),
            Some("`min_score` was reduced from 1.1 to 1.")
        );
        let (top_k, note) = parse_top_k(&serde_json::json!({"top_k": 30})).unwrap();
        assert_eq!(top_k, MAX_TOP_K);
        assert_eq!(
            note.as_deref(),
            Some("`top_k` was reduced from 30 to 20 hits.")
        );
    }

    #[test]
    fn zero_hits_are_success_payload_not_a_soft_failure() {
        let surface = code_search_surface();
        let data =
            CodeSearchData::success(Vec::new(), Some("No matching code chunks found.".into()));
        let mut value = serde_json::to_value(&data).expect("serialize empty success");
        assert!(value.get("ok").is_none(), "{value}");
        assert!(value.get("status").is_none(), "{value}");
        value["ok"] = serde_json::json!(true);
        assert!(
            crate::tool::schema_validate(&surface.tools["code_search"].output_schema, &value)
                .is_ok(),
            "{value}"
        );
    }

    #[test]
    fn generated_output_schema_requires_envelope_and_hit_shapes() {
        let surface = code_search_surface();
        let schema = &surface.tools["code_search"].output_schema;
        assert_eq!(schema["required"], serde_json::json!(["ok", "hits"]));
        let hit = schema["properties"]["hits"]["items"].as_object().unwrap();
        assert_eq!(
            hit["required"],
            serde_json::json!([
                "path",
                "anchor_line",
                "start_line",
                "end_line",
                "via",
                "dense_similarity",
                "text",
                "snippet"
            ])
        );

        let success = CodeSearchData::success(
            vec![result_for_source(
                "src/a.rs",
                Some("fn a() {}\nlet needle = 1;\n"),
                "needle",
                1,
                2,
                "fn a() {}",
                0.0,
                true,
            )],
            None,
        );
        let mut value = serde_json::to_value(&success).expect("serialize success");
        value["ok"] = serde_json::json!(true);
        assert!(
            crate::tool::schema_validate(schema, &value).is_ok(),
            "success data must match the generated code contract"
        );
    }

    #[test]
    fn busy_worker_queue_waits_for_the_lock_then_returns_a_typed_outcome() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        ready_root(root);
        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);

        let holder_engine = engines.code_search();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let guard = holder_engine.hold_client_for_test();
            locked_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            drop(guard);
        });
        locked_rx.recv().unwrap();

        let tool = CodeSearchTool::new(engines);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let search = std::thread::spawn(move || {
            runtime.block_on(tool.execute(
                serde_json::json!({ "query": "auth" }),
                crate::tool::trait_::ToolExecutionContext {
                    path_mode: crate::workspace::ToolPathMode::All,
                    workspace_root: dir.path().to_path_buf(),
                    call_id: String::new(),
                    cancel: tokio_util::sync::CancellationToken::new(),
                    output_limit: usize::MAX,
                    session_id: String::new(),
                    session: None,
                },
            ))
        });
        std::thread::sleep(std::time::Duration::from_millis(120));
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let result = search.join().unwrap();
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(100),
            "the search should have queued rather than reported busy immediately"
        );
        assert_eq!(
            result.error_code.as_deref(),
            Some("unavailable"),
            "{result:?}"
        );
        assert!(result.data.is_none(), "{result:?}");
    }

    #[test]
    fn cancelled_worker_queue_wait_returns_quickly() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        ready_root(root);
        let engines = WorkspaceEngines::new();
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        let code_engine = engines.code_search();
        let guard = code_engine.hold_client_for_test();
        let tool = CodeSearchTool::new(engines.clone());
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_thread = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancel_thread.cancel();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let result = runtime.block_on(tool.execute(
            serde_json::json!({ "query": "auth" }),
            crate::tool::trait_::ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: String::new(),
                cancel,
                output_limit: usize::MAX,
                session_id: String::new(),
                session: None,
            },
        ));
        drop(guard);
        canceller.join().unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "cancelled queue wait must not park the runtime"
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert_eq!(
            result.error.as_ref().unwrap().kind,
            ToolErrorKind::Cancelled
        );
    }

    fn search_calls() -> usize {
        SEARCH_CALLS.with(|calls| calls.get())
    }
}
