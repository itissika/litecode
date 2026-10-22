//! Workspace-scoped infrastructure engines.
//!
//! Engines own long-lived workspace services. Tools are only consumers of
//! these services and never own their lifecycle. Lifecycle is driven solely by
//! `.litecode/engines.json` — never by the tool catalog.

pub mod code_search;
pub mod code_search_ipc;
pub mod path_index;
pub mod progress;
pub mod session_search;
mod status_view;
pub use status_view::{EngineUsability, retrieval_progress_for_workspace};

mod code_search_engine;
pub use code_search_engine::{CodeSearchEngine, is_search_worker_busy};

mod lsp_engine;
pub use lsp_engine::LspEngine;

pub use path_index::PathIndex;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::resolved::ResolvedConfig;
use crate::engines::code_search_ipc::protocol::{RefreshMode, RefreshScope};
use crate::session::SessionDataReader;
use crate::types::{LitecodeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalCorpus {
    Code,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalModality {
    Text,
    Semantic,
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalFilters {
    pub glob: Option<String>,
    /// Include-only session scope (resolved full id).
    pub include_session_id: Option<String>,
    pub exclude_session_ids: Vec<String>,
    pub project: Option<String>,
    /// Soft-exclude live model window (current surface seqs).
    pub exclude_context_window: Option<session_search::ContextWindowExclude>,
    /// Caller session id: its own hits and its child sessions' hits rank first.
    pub caller_session_id: Option<String>,
    /// Override sessions reader for tests; production injects via ServeState.
    pub session: Option<SessionDataReader>,
}

#[derive(Debug, Clone)]
pub struct RetrievalQuery {
    pub query: String,
    pub corpus: RetrievalCorpus,
    pub modality: RetrievalModality,
    pub filters: RetrievalFilters,
    pub top_k: usize,
    /// Text pagination offset (Session × Text / grep-style). Ignored for semantic.
    pub offset: usize,
    /// Workspace root used to resolve default sessions.db / code paths.
    pub workspace_root: Option<PathBuf>,
}

/// Combined Session search: one ranked stream (lexical, then unique semantic).
#[derive(Debug, Clone)]
pub struct SessionSearchBundle {
    pub ranked: Vec<session_search::SessionTextHit>,
    pub offset: usize,
}

/// Unified retrieval hit (corpus-tagged). Code tool formatting maps the Code arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "corpus", rename_all = "snake_case")]
pub enum RetrievalHit {
    Code {
        path: String,
        start_line: u32,
        end_line: u32,
        summary: String,
        score: f64,
    },
    Session {
        session_id: String,
        seq: i64,
        item_type: String,
        summary: String,
        score: f64,
    },
}

impl RetrievalHit {
    fn from_code(h: code_search::SearchHit) -> Self {
        Self::Code {
            path: h.path,
            start_line: h.start_line,
            end_line: h.end_line,
            summary: h.summary,
            score: h.score,
        }
    }

    fn from_session(h: session_search::SessionTextHit) -> Self {
        Self::Session {
            session_id: h.session_id,
            seq: h.seq,
            item_type: h.item_type,
            summary: h.summary,
            score: h.score,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    Idle,
    Warming,
    Warm,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    /// Persisted workspace intent; this is not the runtime warm state.
    pub desired: bool,
    pub state: Option<EngineState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshAcceptedMode {
    /// Engine was cold; start/warmup will build or load as needed.
    Starting,
    /// Refresh already running (warmup or prior refresh).
    InProgress,
    Rebuild,
    Incremental,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshAccepted {
    pub desired: bool,
    pub mode: RefreshAcceptedMode,
}

/// Result of one [`WorkspaceEngines::code_index_tick`] pass (WP-079).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexTick {
    /// Incremental refresh spawned.
    Started,
    /// Nothing done; reason attached.
    Skipped(CodeIndexTickSkip),
}

/// Why a code index tick did nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIndexTickSkip {
    NoWorkspace,
    NotDesired,
    NotWarm,
    /// A refresh is already running.
    Busy,
    WorkerDown,
    /// No pending code work.
    Clean,
    /// Only first build / incompatible warmup / manual refresh may rebuild.
    RebuildPending,
}

/// Gate for Agent/human semantic calls that would occupy the code_search worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSearchCallGate {
    Ready,
    Wait,
    Failed(String),
}

pub fn merge_index_work(
    code: code_search::IndexWork,
    session: code_search::IndexWork,
) -> code_search::IndexWork {
    use code_search::{IndexRebuildReason, IndexWork};
    fn rebuild_rank(reason: IndexRebuildReason) -> u8 {
        match reason {
            IndexRebuildReason::FirstDesired => 2,
            IndexRebuildReason::Incompatible => 1,
        }
    }
    match (code, session) {
        (IndexWork::Rebuild { reason: a }, IndexWork::Rebuild { reason: b }) => {
            IndexWork::Rebuild {
                reason: if rebuild_rank(a) >= rebuild_rank(b) {
                    a
                } else {
                    b
                },
            }
        }
        (IndexWork::Rebuild { reason }, _) | (_, IndexWork::Rebuild { reason }) => {
            IndexWork::Rebuild { reason }
        }
        (IndexWork::Update { dirty: a }, IndexWork::Update { dirty: b }) => IndexWork::Update {
            dirty: a.saturating_add(b),
        },
        (IndexWork::Update { dirty }, IndexWork::None)
        | (IndexWork::None, IndexWork::Update { dirty }) => IndexWork::Update { dirty },
        (IndexWork::None, IndexWork::None) => IndexWork::None,
    }
}

/// Union plan for Human Refresh. Agent tools use corpus-specific work instead.
pub fn engine_index_work(workspace_root: &Path) -> code_search::IndexWork {
    merge_index_work(
        code_search::index_work_from_disk(workspace_root),
        session_search::session_work_from_disk(workspace_root),
    )
}

/// Stops the path-index thread when the last [`WorkspaceEngines`] owner goes away.
///
/// The worker holds its own `Arc<PathIndex>`, so `PathIndex`'s destructor never
/// runs while that thread is alive. This guard is what `detach`s it.
struct PathIndexKeep {
    index: Arc<PathIndex>,
}

impl Drop for PathIndexKeep {
    fn drop(&mut self) {
        self.index.detach();
    }
}

/// The single owner of workspace-scoped LSP and retrieval services.
#[derive(Clone)]
pub struct WorkspaceEngines {
    states: Arc<RwLock<HashMap<String, EngineState>>>,
    last_errors: Arc<RwLock<HashMap<String, String>>>,
    code_search: Arc<CodeSearchEngine>,
    lsp: Arc<LspEngine>,
    path_index: Arc<PathIndexKeep>,
    refresh_busy: Arc<AtomicBool>,
    session_refresh_busy: Arc<AtomicBool>,
    session_reader: Arc<RwLock<Option<SessionDataReader>>>,
}

impl WorkspaceEngines {
    pub fn new() -> Self {
        let states = Arc::new(RwLock::new(HashMap::new()));
        let last_errors = Arc::new(RwLock::new(HashMap::new()));
        let code_search = Arc::new(CodeSearchEngine::new());
        let lsp = Arc::new(LspEngine::new());
        let path_index = Arc::new(PathIndexKeep {
            index: Arc::new(PathIndex::new()),
        });

        let state_ref = Arc::clone(&states);
        let error_ref = Arc::clone(&last_errors);
        code_search.set_worker_failed_handler(Arc::new(move || {
            if let Ok(mut guard) = state_ref.write() {
                guard.insert("code_search".into(), EngineState::Idle);
            }
            if let Ok(mut guard) = error_ref.write() {
                guard.insert("code_search".into(), "code_search worker exited".into());
            }
        }));

        // WP-167: the LSP availability probe runs on first need. Its failure
        // records the same `Failed` + error text the eager warmup produced.
        let state_ref = Arc::clone(&states);
        let error_ref = Arc::clone(&last_errors);
        lsp.hub()
            .set_probe_failure_sink(Arc::new(move |error: String| {
                if let Ok(mut guard) = state_ref.write() {
                    if matches!(guard.get("lsp"), Some(EngineState::Stopped) | None) {
                        return;
                    }
                    guard.insert("lsp".into(), EngineState::Failed);
                }
                if let Ok(mut guard) = error_ref.write() {
                    guard.insert(
                        "lsp".into(),
                        crate::types::LitecodeError::Config(error).to_string(),
                    );
                }
            }));

        Self {
            states,
            last_errors,
            code_search,
            lsp,
            path_index,
            refresh_busy: Arc::new(AtomicBool::new(false)),
            session_refresh_busy: Arc::new(AtomicBool::new(false)),
            session_reader: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_session_reader(&self, reader: SessionDataReader) {
        // Inject into a live worker; do not restart warmup for SessionData nod.
        self.code_search.set_session_reader(reader.clone());
        if let Ok(mut guard) = self.session_reader.write() {
            *guard = Some(reader);
        }
    }

    pub fn session_reader(&self) -> Option<SessionDataReader> {
        self.session_reader.read().ok().and_then(|g| g.clone())
    }

    pub fn code_search(&self) -> Arc<CodeSearchEngine> {
        Arc::clone(&self.code_search)
    }

    pub fn path_index(&self) -> Arc<PathIndex> {
        Arc::clone(&self.path_index.index)
    }

    /// Unified retrieval surface: corpus × modality. Unsupported pairs fail closed.
    pub fn search(&self, request: RetrievalQuery) -> Result<Vec<RetrievalHit>> {
        match (request.corpus, request.modality) {
            (RetrievalCorpus::Code, RetrievalModality::Semantic) => {
                match self.code_search_call_gate() {
                    CodeSearchCallGate::Failed(detail) => {
                        return Err(LitecodeError::ToolExecution(detail));
                    }
                    CodeSearchCallGate::Wait => {
                        return Err(LitecodeError::ToolExecution(
                            "code_search index is updating; try again shortly".into(),
                        ));
                    }
                    CodeSearchCallGate::Ready => {}
                }
                let hits = self.code_search.search(
                    &request.query,
                    request.filters.glob.as_deref(),
                    request.top_k,
                )?;
                Ok(hits.into_iter().map(RetrievalHit::from_code).collect())
            }
            (RetrievalCorpus::Session, RetrievalModality::Text) => {
                let reader = request
                    .filters
                    .session
                    .clone()
                    .or_else(|| self.session_reader())
                    .ok_or_else(|| {
                        LitecodeError::Config(
                            "session search requires an injected SessionDataReader".into(),
                        )
                    })?;
                let ranked = session_search::search_all(
                    &reader,
                    &session_search::SessionTextQuery {
                        query: request.query,
                        offset: 0,
                        include_session_id: request.filters.include_session_id,
                        exclude_session_ids: request.filters.exclude_session_ids,
                        project: request.filters.project,
                        exclude_context_window: request.filters.exclude_context_window,
                    },
                )?;
                let start = request.offset.min(ranked.len());
                let end = start.saturating_add(request.top_k).min(ranked.len());
                Ok(ranked[start..end]
                    .iter()
                    .cloned()
                    .map(RetrievalHit::from_session)
                    .collect())
            }
            (RetrievalCorpus::Session, RetrievalModality::Semantic) => {
                if !self.is_warmed("code_search") {
                    return Err(LitecodeError::Config(
                        "session semantic search requires code_search engine Warm".into(),
                    ));
                }
                match self.retrieval_job_gate() {
                    CodeSearchCallGate::Failed(detail) => {
                        return Err(LitecodeError::ToolExecution(detail));
                    }
                    CodeSearchCallGate::Wait => {
                        return Err(LitecodeError::ToolExecution(
                            "code_search index is updating; try again shortly".into(),
                        ));
                    }
                    CodeSearchCallGate::Ready => {}
                }
                let top_k = request.top_k.clamp(1, session_search::SEMANTIC_WINDOW);
                let hits = self.code_search.search_sessions(
                    &request.query,
                    top_k,
                    request.filters.include_session_id.as_deref(),
                )?;
                let text_q = session_search::SessionTextQuery {
                    query: String::new(),
                    include_session_id: request.filters.include_session_id,
                    exclude_session_ids: request.filters.exclude_session_ids,
                    project: request.filters.project,
                    exclude_context_window: request.filters.exclude_context_window,
                    ..Default::default()
                };
                Ok(session_search::filter_hits(hits, &text_q)
                    .into_iter()
                    .map(RetrievalHit::from_session)
                    .collect())
            }
            (corpus, modality) => Err(LitecodeError::Config(format!(
                "unsupported retrieval combination: {corpus:?} × {modality:?}"
            ))),
        }
    }

    /// Session search: lexical always + semantic when Warm, fused into one ranked stream.
    /// Does not start engines. Pagination / grouping happens in `session_search::build_search_page`.
    pub fn search_sessions(
        &self,
        query: &str,
        offset: usize,
        filters: RetrievalFilters,
        workspace_root: Option<PathBuf>,
    ) -> Result<SessionSearchBundle> {
        let reader = if let Some(r) = filters.session.clone() {
            r
        } else if let Some(r) = self.session_reader() {
            r
        } else {
            return Err(LitecodeError::Config(
                "session search requires an injected SessionDataReader".into(),
            ));
        };

        let text_q = session_search::SessionTextQuery {
            query: query.to_string(),
            offset: 0,
            include_session_id: filters.include_session_id.clone(),
            exclude_session_ids: filters.exclude_session_ids.clone(),
            project: filters.project.clone(),
            exclude_context_window: filters.exclude_context_window.clone(),
        };
        let lexical = session_search::search_all(&reader, &text_q)?;

        let semantic = if matches!(self.session_semantic_gate(), CodeSearchCallGate::Ready) {
            match self.code_search.search_sessions(
                query,
                session_search::SEMANTIC_WINDOW,
                filters.include_session_id.as_deref(),
            ) {
                Ok(hits) => {
                    session_search::gate_semantic_hits(session_search::filter_hits(hits, &text_q))
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "session semantic lane failed; returning text only"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let mut ranked = session_search::fuse_session_layers(lexical, semantic);
        if !ranked.is_empty() {
            let mut session_ids: Vec<String> =
                ranked.iter().map(|h| h.session_id.clone()).collect();
            session_ids.sort();
            session_ids.dedup();
            let updated_at: HashMap<String, i64> =
                session_search::load_session_meta(&reader, &session_ids)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, ts)| (id, ts.updated_at))
                    .collect();
            let mut prefer: Vec<String> = Vec::new();
            if let Some(caller) = filters
                .caller_session_id
                .as_deref()
                .filter(|s| !s.is_empty())
            {
                prefer.push(caller.to_string());
                if let Ok(children) = reader.list_child_ids_blocking(caller) {
                    prefer.extend(children);
                }
            }
            session_search::sort_hits_for_agent(&mut ranked, &prefer, &updated_at);
        }
        Ok(SessionSearchBundle { ranked, offset })
    }

    /// Grouped human search. `corpus=code` (default): LexicalLane text + optional semantic.
    /// `corpus=session`: lexical-then-semantic grouped token page (semantic only when Warm).
    pub fn human_search(
        &self,
        workspace_root: &Path,
        req: &crate::engines::code_search::HumanSearchRequest,
    ) -> Result<crate::engines::code_search::HumanSearchResponse> {
        let corpus = req.corpus.trim().to_ascii_lowercase();
        match corpus.as_str() {
            "" | "code" => {
                let semantic = if req.include_semantic
                    && matches!(self.code_search_call_gate(), CodeSearchCallGate::Ready)
                {
                    let top_k = req
                        .top_k
                        .unwrap_or(crate::engines::code_search::DEFAULT_TOP_K)
                        .clamp(1, crate::engines::code_search::MAX_TOP_K);
                    match self.search(RetrievalQuery {
                        query: req.query.clone(),
                        corpus: RetrievalCorpus::Code,
                        modality: RetrievalModality::Semantic,
                        filters: RetrievalFilters {
                            glob: req.include.clone(),
                            ..Default::default()
                        },
                        top_k,
                        offset: 0,
                        workspace_root: Some(workspace_root.to_path_buf()),
                    }) {
                        Ok(hits) => Some(
                            hits.into_iter()
                                .filter_map(|h| match h {
                                    RetrievalHit::Code {
                                        path,
                                        start_line,
                                        end_line,
                                        summary,
                                        score,
                                    } => Some(code_search::SearchHit {
                                        path,
                                        start_line,
                                        end_line,
                                        summary,
                                        score,
                                    }),
                                    _ => None,
                                })
                                .collect(),
                        ),
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "semantic lane search failed; returning text only"
                            );
                            None
                        }
                    }
                } else {
                    None
                };
                crate::engines::code_search::human_search(workspace_root, req, semantic)
            }
            "session" => {
                let bundle = self.search_sessions(
                    &req.query,
                    req.offset.unwrap_or(0),
                    RetrievalFilters {
                        project: req.project.clone(),
                        ..Default::default()
                    },
                    Some(workspace_root.to_path_buf()),
                )?;
                let ranked = if req.include_semantic {
                    bundle.ranked
                } else {
                    bundle
                        .ranked
                        .into_iter()
                        .filter(|h| h.lane == session_search::SessionHitLane::Text)
                        .collect()
                };
                let page = session_search::build_search_page(
                    &self.session_reader().ok_or_else(|| {
                        LitecodeError::Config(
                            "session search requires an injected SessionDataReader".into(),
                        )
                    })?,
                    &ranked,
                    bundle.offset,
                )?;
                Ok(crate::engines::code_search::HumanSearchResponse {
                    text: Vec::new(),
                    semantic: None,
                    session_page: Some(page),
                })
            }
            other => Err(LitecodeError::Config(format!(
                "unsupported retrieval corpus: {other}"
            ))),
        }
    }

    pub fn lsp_hub(&self) -> crate::lsp::SharedLspHub {
        self.lsp.hub()
    }

    /// WP-167: run the deferred LSP availability probe if it has not run for
    /// this activation. A failure flips `state("lsp")` to `Failed` with the
    /// original error. No-op unless the engine is `Warm`.
    pub async fn ensure_lsp_probed(&self) -> std::result::Result<(), String> {
        if self.state("lsp") != Some(EngineState::Warm) {
            return Ok(());
        }
        self.lsp.hub().ensure_probed().await
    }

    /// Blocking form of [`Self::ensure_lsp_probed`] (for `spawn_blocking` callers).
    pub fn ensure_lsp_probed_blocking(&self) -> std::result::Result<(), String> {
        if self.state("lsp") != Some(EngineState::Warm) {
            return Ok(());
        }
        self.lsp.hub().ensure_probed_blocking()
    }

    pub fn memory_sample(&self) -> crate::telemetry::MemorySample {
        let embed_pids = self
            .code_search
            .worker_pid()
            .into_iter()
            .collect::<Vec<_>>();
        let lsp_pids = self.lsp.hub().language_server_pids();
        crate::telemetry::sample_memory(&embed_pids, &lsp_pids)
    }

    pub fn state(&self, id: &str) -> Option<EngineState> {
        self.states.read().ok()?.get(id).copied()
    }

    pub fn last_error(&self, id: &str) -> Option<String> {
        self.last_errors
            .read()
            .ok()
            .and_then(|m| m.get(id).cloned())
    }

    pub fn is_warmed(&self, id: &str) -> bool {
        self.state(id) == Some(EngineState::Warm)
    }

    /// WP-057: searchable when Warm (up and able to answer). Warming / Failed /
    /// Stopped / Idle are not searchable. Tools WP-061 hide tools when this is false
    /// (or desired=false). Silent refresh does **not** clear searchability.
    pub fn is_searchable(&self, id: &str) -> bool {
        if id != "code_search" {
            return self.is_warmed(id);
        }
        matches!(self.state("code_search"), Some(EngineState::Warm))
            && matches!(self.code_search_call_gate(), CodeSearchCallGate::Ready)
    }

    pub fn is_refresh_busy(&self) -> bool {
        self.refresh_busy.load(Ordering::SeqCst)
    }

    /// Whether a retrieval *job* blocks search.
    ///
    /// WP-057: silent incremental refresh (`Refreshing` / `refresh_busy`) must
    /// **not** block search while Warm. Only Building (first/incompatible) or
    /// Failed block; Warming itself is handled via `!is_warmed`.
    pub fn retrieval_job_gate(&self) -> CodeSearchCallGate {
        if matches!(self.state("code_search"), Some(EngineState::Failed)) {
            return CodeSearchCallGate::Failed(
                self.last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into()),
            );
        }
        if let Some(root) = self.code_search.workspace_root() {
            let view = code_search::resolve_index_view(&root, self.state("code_search"));
            match view.status {
                code_search::IndexStatus::Building => {
                    return CodeSearchCallGate::Wait;
                }
                code_search::IndexStatus::Failed => {
                    return CodeSearchCallGate::Failed(
                        view.job_error
                            .or_else(|| self.last_error("code_search"))
                            .unwrap_or_else(|| "code_search index failed".into()),
                    );
                }
                _ => {}
            }
        }
        if self.is_warmed("code_search") {
            CodeSearchCallGate::Ready
        } else {
            CodeSearchCallGate::Wait
        }
    }

    /// Whether the session semantic lane may be read now.
    ///
    /// Deliberately narrower than [`Self::retrieval_job_gate`]: that one answers
    /// "is the *code* corpus ready", and the session ANN shares nothing with it
    /// but the worker process. Gating the session lane on the code corpus going
    /// Stale/Building is how a code rebuild used to take session search with it.
    /// A session read is allowed whenever the worker is warm; whether the ANN is
    /// current is not this gate's question — staleness costs recall, not truth.
    pub fn session_semantic_gate(&self) -> CodeSearchCallGate {
        // `worker_pid` is an atomic load, not the client lock: asking this on the
        // search path must never wait behind a refresh that holds that lock. A
        // dead worker fails the call and degrades to text; a worker that is not
        // there at all is not a search to attempt.
        if self.is_warmed("code_search") && self.code_search.worker_pid().is_some() {
            CodeSearchCallGate::Ready
        } else {
            CodeSearchCallGate::Wait
        }
    }

    /// Whether Agent semantic search may hit the worker now.
    ///
    /// WP-057: Warm ⇒ searchable. Stale / Refreshing do **not** Wait (silent
    /// background refresh). Building / Absent / NeedsRebuild Wait (Warming bucket
    /// / not yet searchable). Failed stays Failed.
    pub fn code_search_call_gate(&self) -> CodeSearchCallGate {
        match self.retrieval_job_gate() {
            CodeSearchCallGate::Ready => {}
            other => return other,
        }
        if let Some(root) = self.code_search.workspace_root() {
            let view = code_search::resolve_index_view(&root, self.state("code_search"));
            match view.status {
                code_search::IndexStatus::NeedsRebuild | code_search::IndexStatus::Absent => {
                    // Should normally still be Warming; keep closed if mis-labeled Warm.
                    return CodeSearchCallGate::Wait;
                }
                _ => {}
            }
        }
        CodeSearchCallGate::Ready
    }

    /// Test-only: force a workspace engine runtime state.
    #[cfg(test)]
    pub fn set_state_for_test(&self, id: &str, state: EngineState) {
        if let Ok(mut guard) = self.states.write() {
            guard.insert(id.to_string(), state);
        }
    }

    #[cfg(test)]
    pub fn set_refresh_busy_for_test(&self, busy: bool) {
        self.refresh_busy.store(busy, Ordering::SeqCst);
    }

    /// Test-only: force a workspace engine last error string.
    #[cfg(test)]
    pub fn set_last_error_for_test(&self, id: &str, error: &str) {
        if let Ok(mut guard) = self.last_errors.write() {
            guard.insert(id.to_string(), error.to_string());
        }
    }

    pub fn workspace_engine_statuses(
        &self,
        workspace_root: &std::path::Path,
    ) -> HashMap<String, EngineStatus> {
        ["code_search", "lsp"]
            .into_iter()
            .map(|id| {
                (
                    id.to_string(),
                    EngineStatus {
                        desired: crate::config::workspace::workspace_engine_desired(
                            workspace_root,
                            id,
                        ),
                        state: self.state(id),
                        error: self
                            .last_errors
                            .read()
                            .ok()
                            .and_then(|m| m.get(id).cloned()),
                    },
                )
            })
            .collect()
    }

    pub async fn wait_until_warmed(&self, id: &str, max_wait: Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            if self.is_warmed(id) {
                return true;
            }
            if start.elapsed() >= max_wait {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Reconcile runtime engines from `.litecode/engines.json` only.
    pub fn reconcile(&self, resolved: &ResolvedConfig) {
        let root = resolved.workspace_root();
        for id in ["code_search", "lsp"] {
            if crate::config::workspace::workspace_engine_desired(root, id) {
                self.start(id, resolved);
            } else {
                self.stop(id);
            }
        }
    }

    pub fn stop_all(&self) {
        self.stop("code_search");
        self.stop("lsp");
        self.path_index.index.detach();
    }

    /// Single index refresh: auto-starts engine if needed; Warm path rebuilds or syncs.
    pub fn request_refresh(&self, resolved: &ResolvedConfig) -> Result<RefreshAccepted> {
        let root = resolved.workspace_root();
        if !crate::config::workspace::workspace_engine_desired(root, "code_search") {
            return Err(crate::types::LitecodeError::Config(
                "code search engine is off; enable it in Settings → Engines".into(),
            ));
        }

        let state = self.state("code_search");
        if matches!(state, Some(EngineState::Warming)) || self.refresh_busy.load(Ordering::SeqCst) {
            return Ok(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::InProgress,
            });
        }

        if !matches!(state, Some(EngineState::Warm)) {
            self.reconcile(resolved);
            return Ok(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::Starting,
            });
        }

        self.consume_index_work()
    }

    /// Run pending index update/rebuild for **both** corpora. Human Refresh.
    ///
    /// WP-079: the two corpora are planned and executed **separately**. A
    /// session-side rebuild never drags the code index into building/Warming
    /// (and vice versa); only the code corpus' own rebuild may cool it.
    /// [`engine_index_work`] (merged view) is for status display only.
    ///
    /// Does not start the engine; caller must already be Warm (or use
    /// [`Self::request_refresh`] which reconciles first).
    pub fn consume_index_work(&self) -> Result<RefreshAccepted> {
        let code = self.consume_code_index_work()?;
        let session = self.consume_session_index_work()?;
        let code_idle = matches!(code.mode, RefreshAcceptedMode::Incremental)
            && matches!(
                self.code_search
                    .workspace_root()
                    .map(|root| code_search::index_work_from_disk(&root)),
                Some(code_search::IndexWork::None) | None
            )
            && !self.refresh_busy.load(Ordering::SeqCst);
        Ok(if code_idle { session } else { code })
    }

    /// Code corpus only. Used by agent `code_search`. Does not consume session.
    pub fn consume_code_index_work(&self) -> Result<RefreshAccepted> {
        match self.consume_preflight(&self.refresh_busy)? {
            ConsumePreflight::ShortCircuit(accepted) => Ok(accepted),
            ConsumePreflight::Ready(root) => match code_search::index_work_from_disk(&root) {
                code_search::IndexWork::None => Ok(RefreshAccepted {
                    desired: true,
                    mode: RefreshAcceptedMode::Incremental,
                }),
                work => self.spawn_scoped_refresh(
                    &root,
                    work,
                    RefreshScope::Code,
                    &self.refresh_busy,
                    true,
                ),
            },
        }
    }

    /// Session corpus only. Used by agent `session_search`.
    ///
    /// Does not write code `job.json` or set [`Self::is_refresh_busy`], so
    /// `code_search` is not gated as refreshing.
    pub fn consume_session_index_work(&self) -> Result<RefreshAccepted> {
        match self.consume_preflight(&self.session_refresh_busy)? {
            ConsumePreflight::ShortCircuit(accepted) => Ok(accepted),
            ConsumePreflight::Ready(root) => {
                // Read the store when we can see it: the on-disk hint is only a
                // leftover from the last load and cannot know about rows written
                // since (`session_work_now`).
                let work = match self.session_reader() {
                    Some(reader) => session_search::session_work_now(&root, &reader),
                    None => session_search::session_work_from_disk(&root),
                };
                match work {
                    code_search::IndexWork::None => Ok(RefreshAccepted {
                        desired: true,
                        mode: RefreshAcceptedMode::Incremental,
                    }),
                    work => {
                        if !self.code_search.worker_alive() {
                            return Ok(RefreshAccepted {
                                desired: true,
                                mode: RefreshAcceptedMode::Starting,
                            });
                        }
                        self.spawn_scoped_refresh(
                            &root,
                            work,
                            RefreshScope::Session,
                            &self.session_refresh_busy,
                            false,
                        )
                    }
                }
            }
        }
    }

    fn consume_preflight(&self, busy: &AtomicBool) -> Result<ConsumePreflight> {
        let Some(root) = self.code_search.workspace_root() else {
            return Err(crate::types::LitecodeError::Config(
                "code_search: workspace not set".into(),
            ));
        };
        if !crate::config::workspace::workspace_engine_desired(&root, "code_search") {
            return Err(crate::types::LitecodeError::Config(
                "code search engine is off; enable it in Settings → Engines".into(),
            ));
        }
        let state = self.state("code_search");
        if matches!(state, Some(EngineState::Warming)) || busy.load(Ordering::SeqCst) {
            return Ok(ConsumePreflight::ShortCircuit(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::InProgress,
            }));
        }
        if !matches!(state, Some(EngineState::Warm)) {
            return Ok(ConsumePreflight::ShortCircuit(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::Starting,
            }));
        }
        Ok(ConsumePreflight::Ready(root))
    }

    /// One silent incremental pass for the code corpus (WP-079). Sync: runs the
    /// WP-141 stat-only disk scan (blocks for that scan), then decides and,
    /// when Warm + due, spawns the refresh and returns.
    /// Never rebuilds and never enters Warming; safe every tick, during turns.
    /// NotWarm / no worker: still scans and writes `queue.json`, but never
    /// digests (choice a).
    pub fn code_index_tick(&self) -> CodeIndexTick {
        use CodeIndexTickSkip as Skip;
        let Some(root) = self.code_search.workspace_root() else {
            return CodeIndexTick::Skipped(Skip::NoWorkspace);
        };
        if !crate::config::workspace::workspace_engine_desired(&root, "code_search") {
            return CodeIndexTick::Skipped(Skip::NotDesired);
        }
        // WP-141: the periodic stat-only scan rides this tick (no timer of its
        // own). Worker up → IPC stat scan (works with a cooled RAM index).
        // Otherwise the serve process stats `stamps.v1` into queue.json.
        if self.code_search.worker_alive() {
            self.code_search.request_reconcile();
        } else {
            code_search::queue_reconcile_from_stamps(&root);
        }
        if !matches!(self.state("code_search"), Some(EngineState::Warm)) {
            return CodeIndexTick::Skipped(Skip::NotWarm);
        }
        if self.refresh_busy.load(Ordering::SeqCst) {
            return CodeIndexTick::Skipped(Skip::Busy);
        }
        let work = code_search::index_work_from_disk(&root);
        match work {
            code_search::IndexWork::None => return CodeIndexTick::Skipped(Skip::Clean),
            code_search::IndexWork::Rebuild { .. } => {
                return CodeIndexTick::Skipped(Skip::RebuildPending);
            }
            code_search::IndexWork::Update { .. } => {}
        }
        if !self.code_search.worker_alive() {
            return CodeIndexTick::Skipped(Skip::WorkerDown);
        }
        match self.spawn_scoped_refresh(&root, work, RefreshScope::Code, &self.refresh_busy, true) {
            Ok(RefreshAccepted {
                mode: RefreshAcceptedMode::Incremental,
                ..
            }) => CodeIndexTick::Started,
            Ok(_) => CodeIndexTick::Skipped(Skip::Busy),
            Err(e) => {
                tracing::debug!(error = %e, "code index tick refresh not started");
                CodeIndexTick::Skipped(Skip::Busy)
            }
        }
    }

    /// Reconcile dirty paths on a Warm worker. Does not embed or enable retrieval.
    pub fn request_index_sync(&self, workspace_root: &Path) {
        if !crate::config::workspace::workspace_engine_desired(workspace_root, "code_search") {
            return;
        }
        let state = self.state("code_search");
        if self.refresh_busy.load(Ordering::SeqCst)
            || matches!(state, Some(EngineState::Warming))
            || !matches!(state, Some(EngineState::Warm))
        {
            return;
        }
        self.code_search.request_reconcile();
    }

    fn spawn_scoped_refresh(
        &self,
        root: &Path,
        work: code_search::IndexWork,
        scope: RefreshScope,
        busy: &Arc<AtomicBool>,
        touch_code_job: bool,
    ) -> Result<RefreshAccepted> {
        let mode = if touch_code_job {
            match work {
                code_search::IndexWork::Rebuild { .. } => {
                    // WP-143: a rebuild is the incremental path from empty.
                    code_search::begin_refreshing(root);
                    RefreshAcceptedMode::Rebuild
                }
                _ => {
                    code_search::begin_refreshing(root);
                    RefreshAcceptedMode::Incremental
                }
            }
        } else {
            match work {
                code_search::IndexWork::Rebuild { .. } => RefreshAcceptedMode::Rebuild,
                _ => RefreshAcceptedMode::Incremental,
            }
        };

        if busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::InProgress,
            });
        }

        // WP-057 Warming bucket: incompatible/first rebuild ⇒ not searchable.
        let rebuild_cooling = touch_code_job && matches!(mode, RefreshAcceptedMode::Rebuild);
        if rebuild_cooling {
            if let Ok(mut states) = self.states.write() {
                states.insert("code_search".into(), EngineState::Warming);
            }
        }

        let engine = Arc::clone(&self.code_search);
        let busy = Arc::clone(busy);
        let errors = Arc::clone(&self.last_errors);
        let states = Arc::clone(&self.states);
        let root_owned = root.to_path_buf();

        let finish = move |result: Result<RefreshMode>| {
            busy.store(false, Ordering::SeqCst);
            match result {
                Ok(_) => {
                    if touch_code_job && let Ok(mut guard) = errors.write() {
                        guard.remove("code_search");
                    }
                    if rebuild_cooling {
                        if let Ok(mut guard) = states.write() {
                            if !matches!(guard.get("code_search"), Some(EngineState::Stopped)) {
                                guard.insert("code_search".into(), EngineState::Warm);
                            }
                        }
                    }
                }
                Err(error) => {
                    if touch_code_job {
                        code_search::mark_index_job_failed(&root_owned, error.to_string());
                        if let Ok(mut guard) = errors.write() {
                            guard.insert("code_search".into(), error.to_string());
                        }
                    } else {
                        tracing::warn!(error = %error, "session index consume failed");
                    }
                    if rebuild_cooling {
                        if let Ok(mut guard) = states.write() {
                            if !matches!(guard.get("code_search"), Some(EngineState::Stopped)) {
                                guard.insert("code_search".into(), EngineState::Failed);
                            }
                        }
                    }
                }
            }
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    engine.refresh_scope(scope).map(|r| r.mode)
                })
                .await;
                finish(match result {
                    Ok(Ok(mode)) => Ok(mode),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(LitecodeError::Config(format!(
                        "code_search refresh task failed: {e}"
                    ))),
                });
            });
        } else {
            std::thread::spawn(move || finish(engine.refresh_scope(scope).map(|r| r.mode)));
        }

        Ok(RefreshAccepted {
            desired: true,
            mode,
        })
    }

    fn start(&self, id: &str, resolved: &ResolvedConfig) {
        let already_running = self
            .state(id)
            .is_some_and(|state| matches!(state, EngineState::Warm | EngineState::Warming));
        if already_running {
            return;
        }

        let root = crate::config::path::canon_abs_lossy(resolved.workspace_root());
        if id == "code_search" {
            self.code_search.set_workspace(root.clone());
        } else {
            self.lsp.set_workspace(root);
        }
        if let Ok(mut states) = self.states.write() {
            states.insert(id.to_string(), EngineState::Warming);
        }

        let engine = if id == "code_search" {
            let engine = Arc::clone(&self.code_search);
            EngineCall::Retrieval(engine)
        } else {
            EngineCall::Lsp(Arc::clone(&self.lsp))
        };
        let states = Arc::clone(&self.states);
        let errors = Arc::clone(&self.last_errors);
        let id_owned = id.to_string();

        let finish = move |result: Result<()>| {
            if let Ok(mut guard) = states.write() {
                if matches!(guard.get(&id_owned), Some(EngineState::Stopped)) {
                    return;
                }
                guard.insert(
                    id_owned.clone(),
                    if result.is_ok() {
                        EngineState::Warm
                    } else {
                        EngineState::Failed
                    },
                );
            }
            if let Ok(mut guard) = errors.write() {
                match result {
                    Ok(()) => {
                        guard.remove(&id_owned);
                    }
                    Err(error) => {
                        guard.insert(id_owned, error.to_string());
                    }
                }
            }
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || engine.warmup()).await;
                finish(match result {
                    Ok(result) => result,
                    Err(error) => Err(crate::types::LitecodeError::Config(format!(
                        "engine warmup task failed: {error}"
                    ))),
                });
            });
        } else {
            // Startup reconcile runs before the tokio runtime exists — never block
            // `litecode serve` listen on a full ORT rebuild.
            std::thread::spawn(move || finish(engine.warmup()));
        }
    }

    fn stop(&self, id: &str) {
        match id {
            "code_search" => {
                // L3: clear only this side's busy flag. The session consume
                // runs on its own flag and clears it when its task ends, so a
                // stop + immediate restart cannot dispatch a second session refresh.
                self.refresh_busy.store(false, Ordering::SeqCst);
                self.code_search.stop();
            }
            "lsp" => {
                self.lsp.stop();
            }
            _ => return,
        }
        if let Ok(mut states) = self.states.write() {
            states.insert(id.to_string(), EngineState::Stopped);
        }
        if let Ok(mut errors) = self.last_errors.write() {
            errors.remove(id);
        }
    }
}

enum ConsumePreflight {
    ShortCircuit(RefreshAccepted),
    Ready(PathBuf),
}

enum EngineCall {
    Retrieval(Arc<CodeSearchEngine>),
    Lsp(Arc<LspEngine>),
}

impl EngineCall {
    fn warmup(&self) -> Result<()> {
        match self {
            Self::Retrieval(engine) => engine.warmup(),
            Self::Lsp(engine) => engine.warmup(),
        }
    }
}

impl Default for WorkspaceEngines {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    
    
    
    

    // ---- WP-167: deferred LSP availability probe ----

}
