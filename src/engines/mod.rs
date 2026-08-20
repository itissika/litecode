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
    /// Override sessions DB path; default = `<workspace>/.litecode/sessions.db`.
    pub sessions_db: Option<PathBuf>,
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

/// Combined Session search: dual-lane pages for agents / human UI (no cross-lane fuse).
#[derive(Debug, Clone)]
pub struct SessionSearchBundle {
    pub text_hits: Vec<session_search::SessionTextHit>,
    pub text_has_more: bool,
    /// Present when semantic engine is Warm; empty vec if Warm but no hits.
    pub semantic_hits: Option<Vec<session_search::SessionTextHit>>,
    pub semantic_has_more: bool,
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

/// The single owner of workspace-scoped LSP and retrieval services.
#[derive(Clone)]
pub struct WorkspaceEngines {
    states: Arc<RwLock<HashMap<String, EngineState>>>,
    last_errors: Arc<RwLock<HashMap<String, String>>>,
    code_search: Arc<CodeSearchEngine>,
    lsp: Arc<LspEngine>,
    text_index: Arc<TextIndexEngine>,
    refresh_busy: Arc<AtomicBool>,
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
        }
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
                let hits = self.code_search.search(
                    &request.query,
                    request.filters.glob.as_deref(),
                    request.top_k,
                )?;
                Ok(hits.into_iter().map(RetrievalHit::from_code).collect())
            }
            (RetrievalCorpus::Session, RetrievalModality::Text) => {
                let db = resolve_sessions_db(&request)?;
                let page = session_search::search(
                    &db,
                    &session_search::SessionTextQuery {
                        query: request.query,
                        offset: request.offset,
                        include_session_id: request.filters.include_session_id,
                        exclude_session_ids: request.filters.exclude_session_ids,
                        project: request.filters.project,
                        exclude_context_window: request.filters.exclude_context_window,
                    },
                )?;
                Ok(page
                    .hits
                    .into_iter()
                    .map(RetrievalHit::from_session)
                    .collect())
            }
            (RetrievalCorpus::Session, RetrievalModality::Semantic) => {
                if !self.is_warmed("code_search") {
                    return Err(LitecodeError::Config(
                        "session semantic search requires code_search engine Warm".into(),
                    ));
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

    /// Session dual-lane search: lexical always + semantic when Warm.
    /// Does not start engines. Returns paginated columns (no char windows —
    /// the tool layer expands windows).
    pub fn search_sessions(
        &self,
        query: &str,
        offset: usize,
        filters: RetrievalFilters,
        workspace_root: Option<PathBuf>,
    ) -> Result<SessionSearchBundle> {
        let db = if let Some(p) = filters.sessions_db.clone() {
            p
        } else if let Some(root) = workspace_root.as_ref() {
            session_search::sessions_db_under(root)
        } else {
            return Err(LitecodeError::Config(
                "session search requires workspace_root or sessions_db".into(),
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
        let text_all = session_search::search_all(&db, &text_q)?;
        let text_page = session_search::paginate_hits(text_all, offset);

        let (semantic_hits, semantic_has_more) = if self.is_warmed("code_search") {
            match self.code_search.search_sessions(
                query,
                session_search::SEMANTIC_WINDOW,
                filters.include_session_id.as_deref(),
            ) {
                Ok(hits) => {
                    let gated = session_search::gate_semantic_hits(session_search::filter_hits(
                        hits, &text_q,
                    ));
                    let page = session_search::paginate_hits(gated, offset);
                    (Some(page.hits), page.has_more)
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "session semantic lane failed; returning text only"
                    );
                    (None, false)
                }
            }
        } else {
            (None, false)
        };

        Ok(SessionSearchBundle {
            text_hits: text_page.hits,
            text_has_more: text_page.has_more,
            semantic_hits,
            semantic_has_more,
            offset: text_page.offset,
        })
    }

    /// Grouped human search. `corpus=code` (default): LexicalLane text + optional semantic.
    /// `corpus=session`: fuzzy text page + optional session_semantic when Warm.
    pub fn human_search(
        &self,
        workspace_root: &Path,
        req: &crate::engines::code_search::HumanSearchRequest,
    ) -> Result<crate::engines::code_search::HumanSearchResponse> {
        let corpus = req.corpus.trim().to_ascii_lowercase();
        match corpus.as_str() {
            "" | "code" => {
                let semantic = if req.include_semantic && self.is_warmed("code_search") {
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
                Ok(crate::engines::code_search::HumanSearchResponse {
                    text: Vec::new(),
                    semantic: None,
                    session: Some(bundle.text_hits),
                    session_semantic: if req.include_semantic {
                        bundle.semantic_hits
                    } else {
                        None
                    },
                    session_has_more: Some(bundle.text_has_more),
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

    /// Test-only: force a workspace engine runtime state.
    #[cfg(test)]
    pub fn set_state_for_test(&self, id: &str, state: EngineState) {
        if let Ok(mut guard) = self.states.write() {
            guard.insert(id.to_string(), state);
        }
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
        crate::config::workspace::enable_code_search_engine(root)?;

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

        let mode = if code_search::should_full_rebuild(root) {
            // Surface building immediately for detail polling.
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

fn resolve_sessions_db(request: &RetrievalQuery) -> Result<PathBuf> {
    if let Some(p) = &request.filters.sessions_db {
        return Ok(p.clone());
    }
    let root = request.workspace_root.as_ref().ok_or_else(|| {
        LitecodeError::Config(
            "session search requires workspace_root or filters.sessions_db".into(),
        )
    })?;
    Ok(session_search::sessions_db_under(root))
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
    use crate::session::store::Session;
    use crate::types::user_text;
    use tempfile::TempDir;

    #[test]
    fn workspace_engines_routes_session_text() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        let session = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        session
            .insert_detail_rows(&[user_text("route me ROUTER_MARKER please")])
            .unwrap();
        let sid = session.id.clone();
        drop(session);

        let engines = WorkspaceEngines::new();
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
        let session = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        session
            .insert_detail_rows(&[user_text("bundle BUNDLE_MARKER text")])
            .unwrap();
        drop(session);

        let engines = WorkspaceEngines::new();
        assert!(!engines.is_warmed("code_search"));
        let bundle = engines
            .search_sessions(
                "BUNDLE_MARKER",
                0,
                RetrievalFilters::default(),
                Some(root.to_path_buf()),
            )
            .unwrap();
        assert_eq!(bundle.text_hits.len(), 1);
        assert!(bundle.semantic_hits.is_none());
    }

    #[test]
    fn human_search_session_corpus_returns_session_column() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let litecode = root.join(".litecode");
        std::fs::create_dir_all(&litecode).unwrap();
        let db = litecode.join("sessions.db");
        let session = Session::open(
            db.to_str().unwrap(),
            root.to_str().unwrap(),
            "default",
            None,
        )
        .unwrap();
        session
            .insert_detail_rows(&[user_text("human HUMAN_SESSION_HIT panel")])
            .unwrap();
        drop(session);

        let engines = WorkspaceEngines::new();
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
        assert!(resp.session.as_ref().is_some_and(|h| !h.is_empty()));
        assert!(resp.session_semantic.is_none()); // cold engine
        assert_eq!(resp.session_has_more, Some(false));
        assert!(resp.text.is_empty());
        assert!(resp.semantic.is_none());
        let session_hits = resp.session.expect("session column");
        assert_eq!(session_hits.len(), 1);
        assert!(session_hits[0].summary.contains("HUMAN_SESSION_HIT"));
    }
}
