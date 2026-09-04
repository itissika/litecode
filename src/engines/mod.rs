//! Workspace-scoped infrastructure engines.
//!
//! Engines own long-lived workspace services. Tools are only consumers of
//! these services and never own their lifecycle. Lifecycle is driven solely by
//! `.litecode/engines.json` — never by the tool catalog.

pub mod code_search;
pub mod code_search_ipc;
pub mod session_search;
mod status_view;
pub mod text_index;
pub use status_view::EngineUsability;

mod code_search_engine;
pub use code_search_engine::CodeSearchEngine;

mod lsp_engine;
pub use lsp_engine::LspEngine;

pub use text_index::TextIndexEngine;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::resolved::ResolvedConfig;
use crate::engines::code_search_ipc::protocol::RefreshMode;
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

/// Gate for Agent/human semantic calls that would occupy the code_search worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSearchCallGate {
    Ready,
    Wait,
    Failed(String),
}

/// The single owner of workspace-scoped LSP and retrieval services.
#[derive(Clone)]
pub struct WorkspaceEngines {
    states: Arc<RwLock<HashMap<String, EngineState>>>,
    last_errors: Arc<RwLock<HashMap<String, String>>>,
    code_search: Arc<CodeSearchEngine>,
    lsp: Arc<LspEngine>,
    text_index: Arc<TextIndexEngine>,
    refresh_busy: Arc<AtomicBool>,
    session_reader: Arc<RwLock<Option<SessionDataReader>>>,
}

impl WorkspaceEngines {
    pub fn new() -> Self {
        let states = Arc::new(RwLock::new(HashMap::new()));
        let last_errors = Arc::new(RwLock::new(HashMap::new()));
        let code_search = Arc::new(CodeSearchEngine::new());
        let lsp = Arc::new(LspEngine::new());
        let text_index = Arc::new(TextIndexEngine::new());

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

        Self {
            states,
            last_errors,
            code_search,
            lsp,
            text_index,
            refresh_busy: Arc::new(AtomicBool::new(false)),
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

    pub fn text_index(&self) -> Arc<TextIndexEngine> {
        Arc::clone(&self.text_index)
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

        let semantic = if matches!(self.code_search_call_gate(), CodeSearchCallGate::Ready) {
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

        Ok(SessionSearchBundle {
            ranked: session_search::merge_session_hits(lexical, semantic),
            offset,
        })
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

    pub fn is_refresh_busy(&self) -> bool {
        self.refresh_busy.load(Ordering::SeqCst)
    }

    /// Whether Agent semantic search may hit the worker now.
    ///
    /// Wait = index is building/refreshing (return "try again", do not search).
    /// Failed = job failed; do not return hits from a possibly stale corpus.
    pub fn code_search_call_gate(&self) -> CodeSearchCallGate {
        if matches!(self.state("code_search"), Some(EngineState::Failed)) {
            return CodeSearchCallGate::Failed(
                self.last_error("code_search")
                    .unwrap_or_else(|| "code_search engine failed".into()),
            );
        }
        if self.is_refresh_busy() {
            return CodeSearchCallGate::Wait;
        }
        if let Some(root) = self.code_search.workspace_root() {
            let view = code_search::resolve_index_view(&root, self.state("code_search"));
            match view.status {
                code_search::IndexStatus::Building | code_search::IndexStatus::Refreshing => {
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
        // Adaptive text index: independent of retrieval.desired.
        self.text_index.attach_workspace(root);
    }

    pub fn stop_all(&self) {
        self.stop("code_search");
        self.stop("lsp");
        self.text_index.detach();
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

        self.spawn_index_refresh(root)
    }

    /// Align the running code index to current discovery config. Does not enable retrieval.
    ///
    /// No-op when the engine is off, still warming, or a refresh is already in flight.
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
        let _ = self.spawn_index_refresh(workspace_root);
    }

    fn spawn_index_refresh(&self, root: &Path) -> Result<RefreshAccepted> {
        let mode = if code_search::should_full_rebuild(root) {
            code_search::begin_building(root);
            RefreshAcceptedMode::Rebuild
        } else {
            code_search::begin_refreshing(root);
            RefreshAcceptedMode::Incremental
        };

        if self
            .refresh_busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Ok(RefreshAccepted {
                desired: true,
                mode: RefreshAcceptedMode::InProgress,
            });
        }

        let engine = Arc::clone(&self.code_search);
        let busy = Arc::clone(&self.refresh_busy);
        let errors = Arc::clone(&self.last_errors);
        let root_owned = root.to_path_buf();

        let finish = move |result: Result<RefreshMode>| {
            busy.store(false, Ordering::SeqCst);
            match result {
                Ok(_) => {
                    if let Ok(mut guard) = errors.write() {
                        guard.remove("code_search");
                    }
                }
                Err(error) => {
                    code_search::mark_index_job_failed(&root_owned, error.to_string());
                    if let Ok(mut guard) = errors.write() {
                        guard.insert("code_search".into(), error.to_string());
                    }
                }
            }
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result =
                    tokio::task::spawn_blocking(move || engine.refresh().map(|r| r.mode)).await;
                finish(match result {
                    Ok(Ok(mode)) => Ok(mode),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(LitecodeError::Config(format!(
                        "code_search refresh task failed: {e}"
                    ))),
                });
            });
        } else {
            std::thread::spawn(move || finish(engine.refresh().map(|r| r.mode)));
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
    use super::*;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;
    use tempfile::TempDir;

    #[test]
    fn workspace_engines_routes_session_text() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(&litecode).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data
                .create_session(root.to_str().unwrap(), "default", None)
                .unwrap();
            data.insert_items(&id, &[user_text("route me ROUTER_MARKER please")])
                .unwrap();
            id
        };

        let engines = WorkspaceEngines::new();
        engines.set_session_reader(SessionDataReader::open(&db));
        // Must not require code_search Warm.
        assert!(!engines.is_warmed("code_search"));

        let hits = engines
            .search(RetrievalQuery {
                query: "ROUTER_MARKER".into(),
                corpus: RetrievalCorpus::Session,
                modality: RetrievalModality::Text,
                filters: RetrievalFilters::default(),
                top_k: 8,
                offset: 0,
                workspace_root: Some(root.to_path_buf()),
            })
            .unwrap();

        assert_eq!(hits.len(), 1);
        match &hits[0] {
            RetrievalHit::Session {
                session_id,
                summary,
                ..
            } => {
                assert_eq!(session_id, &sid);
                assert!(summary.contains("ROUTER_MARKER"));
            }
            other => panic!("expected Session hit, got {other:?}"),
        }
    }

    #[test]
    fn search_sessions_without_reader_is_explicit_error() {
        let engines = WorkspaceEngines::new();
        let err = engines
            .search_sessions("q", 0, RetrievalFilters::default(), None)
            .unwrap_err();
        assert!(err.to_string().contains("SessionDataReader"), "got: {err}");
        assert_ne!(engines.state("code_search"), Some(EngineState::Failed));
    }

    #[test]
    fn session_semantic_requires_warm() {
        let engines = WorkspaceEngines::new();
        let err = engines
            .search(RetrievalQuery {
                query: "x".into(),
                corpus: RetrievalCorpus::Session,
                modality: RetrievalModality::Semantic,
                filters: RetrievalFilters::default(),
                top_k: 4,
                offset: 0,
                workspace_root: Some(PathBuf::from("/tmp")),
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("Warm") || err.to_string().contains("warm"),
            "got: {err}"
        );
    }

    #[test]
    fn search_sessions_skips_semantic_when_cold() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        {
            let lease = WorkspaceWriteLease::acquire(&litecode).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data
                .create_session(root.to_str().unwrap(), "default", None)
                .unwrap();
            data.insert_items(&id, &[user_text("bundle BUNDLE_MARKER text")])
                .unwrap();
        }

        let engines = WorkspaceEngines::new();
        engines.set_session_reader(SessionDataReader::open(&db));
        assert!(!engines.is_warmed("code_search"));
        let bundle = engines
            .search_sessions(
                "BUNDLE_MARKER",
                0,
                RetrievalFilters::default(),
                Some(root.to_path_buf()),
            )
            .unwrap();
        assert_eq!(bundle.ranked.len(), 1);
        assert!(
            bundle
                .ranked
                .iter()
                .all(|h| h.lane == session_search::SessionHitLane::Text)
        );
    }

    fn session_db_with_marker(
        root: &std::path::Path,
        marker: &str,
    ) -> (String, std::path::PathBuf) {
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        let sid = {
            let lease = WorkspaceWriteLease::acquire(&litecode).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data
                .create_session(root.to_str().unwrap(), "default", None)
                .unwrap();
            data.insert_items(&id, &[user_text(marker)]).unwrap();
            id
        };
        (sid, db)
    }

    #[test]
    fn search_sessions_lexical_survives_code_index_refresh() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_sid, db) = session_db_with_marker(root, "bundle REFRESH_MARKER text");

        let engines = WorkspaceEngines::new();
        engines.set_session_reader(SessionDataReader::open(&db));
        engines.code_search().set_workspace(root.to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);

        let bundle = engines
            .search_sessions(
                "REFRESH_MARKER",
                0,
                RetrievalFilters::default(),
                Some(root.to_path_buf()),
            )
            .unwrap();
        assert_eq!(bundle.ranked.len(), 1);
        assert!(
            bundle
                .ranked
                .iter()
                .all(|h| h.lane == session_search::SessionHitLane::Text)
        );
        assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
        assert!(engines.last_error("code_search").is_none());
    }

    #[test]
    fn code_semantic_search_fails_fast_while_index_updating() {
        let dir = TempDir::new().unwrap();
        let engines = WorkspaceEngines::new();
        engines
            .code_search()
            .set_workspace(dir.path().to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);

        let err = engines
            .search(RetrievalQuery {
                query: "anything".into(),
                corpus: RetrievalCorpus::Code,
                modality: RetrievalModality::Semantic,
                filters: RetrievalFilters::default(),
                top_k: 4,
                offset: 0,
                workspace_root: Some(dir.path().to_path_buf()),
            })
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("updating") || msg.contains("try again"),
            "must fail closed without waiting on the worker: {msg}"
        );
        assert!(
            !msg.contains("worker"),
            "must not occupy IPC / report a dead worker: {msg}"
        );
        assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
    }

    #[test]
    fn session_text_search_ignores_code_index_refresh() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let (_sid, db) = session_db_with_marker(root, "route me SESSION_TEXT_MARKER please");

        let engines = WorkspaceEngines::new();
        engines.set_session_reader(SessionDataReader::open(&db));
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);

        let hits = engines
            .search(RetrievalQuery {
                query: "SESSION_TEXT_MARKER".into(),
                corpus: RetrievalCorpus::Session,
                modality: RetrievalModality::Text,
                filters: RetrievalFilters::default(),
                top_k: 8,
                offset: 0,
                workspace_root: Some(root.to_path_buf()),
            })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
    }

    #[test]
    fn session_semantic_fails_fast_while_code_index_updating() {
        let engines = WorkspaceEngines::new();
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.set_refresh_busy_for_test(true);
        let err = engines
            .search(RetrievalQuery {
                query: "x".into(),
                corpus: RetrievalCorpus::Session,
                modality: RetrievalModality::Semantic,
                filters: RetrievalFilters::default(),
                top_k: 4,
                offset: 0,
                workspace_root: Some(PathBuf::from("/tmp")),
            })
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("updating") || msg.contains("try again"),
            "got: {msg}"
        );
        assert!(!msg.contains("worker"), "got: {msg}");
        assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
    }

    #[test]
    fn human_search_session_corpus_returns_session_column() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        {
            let lease = WorkspaceWriteLease::acquire(&litecode).unwrap();
            let data = SessionData::open(&lease, &db).unwrap();
            let id = data
                .create_session(root.to_str().unwrap(), "default", None)
                .unwrap();
            data.insert_items(&id, &[user_text("human HUMAN_SESSION_HIT panel")])
                .unwrap();
        }

        let engines = WorkspaceEngines::new();
        engines.set_session_reader(SessionDataReader::open(&db));
        let resp = engines
            .human_search(
                root,
                &crate::engines::code_search::HumanSearchRequest {
                    query: "HUMAN_SESSION_HIT".into(),
                    corpus: "session".into(),
                    case_sensitive: true,
                    whole_word: false,
                    is_regex: false,
                    include: None,
                    exclude: None,
                    top_k: Some(5),
                    offset: None,
                    project: None,
                    include_semantic: true,
                },
            )
            .unwrap();
        assert!(
            resp.session_page
                .as_ref()
                .is_some_and(|p| !p.groups.is_empty())
        );
        assert!(resp.text.is_empty());
        assert!(resp.semantic.is_none());
        let page = resp.session_page.expect("session page");
        assert_eq!(page.groups.len(), 1);
        assert_eq!(page.groups[0].hits.len(), 1);
        assert!(page.groups[0].hits[0].summary.contains("HUMAN_SESSION_HIT"));
        assert!(!page.has_more);
    }

    #[test]
    fn request_index_sync_does_not_enable_retrieval() {
        let dir = TempDir::new().unwrap();
        let engines = WorkspaceEngines::new();
        engines.request_index_sync(dir.path());
        assert!(!crate::config::workspace::workspace_engine_desired(
            dir.path(),
            "code_search"
        ));
        assert!(!engines.is_refresh_busy());
        assert!(!dir.path().join(".litecode").join("engines.json").is_file());
    }

    #[test]
    fn request_index_sync_noops_when_desired_but_not_warm() {
        let dir = TempDir::new().unwrap();
        crate::engines::code_search::init_workspace_index(dir.path()).unwrap();
        crate::config::workspace::enable_code_search_engine(dir.path()).unwrap();
        let engines = WorkspaceEngines::new();
        engines
            .code_search()
            .set_workspace(dir.path().to_path_buf());
        engines.request_index_sync(dir.path());
        assert!(!engines.is_refresh_busy());
        let view = crate::engines::code_search::resolve_index_view(dir.path(), None);
        assert_ne!(
            view.status,
            crate::engines::code_search::IndexStatus::Refreshing
        );
    }

    #[test]
    fn request_index_sync_starts_refresh_when_warm_and_desired() {
        let dir = TempDir::new().unwrap();
        crate::engines::code_search::init_workspace_index(dir.path()).unwrap();
        crate::config::workspace::enable_code_search_engine(dir.path()).unwrap();
        let engines = WorkspaceEngines::new();
        engines
            .code_search()
            .set_workspace(dir.path().to_path_buf());
        engines.set_state_for_test("code_search", EngineState::Warm);
        engines.request_index_sync(dir.path());
        assert!(engines.is_refresh_busy());
        let view =
            crate::engines::code_search::resolve_index_view(dir.path(), Some(EngineState::Warm));
        assert!(
            matches!(
                view.status,
                crate::engines::code_search::IndexStatus::Refreshing
                    | crate::engines::code_search::IndexStatus::Building
            ),
            "expected in-flight index job, got {:?}",
            view.status
        );
    }
}
