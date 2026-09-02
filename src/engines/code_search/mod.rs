//! Workspace semantic + lexical code search.
//!
//! Layers (both ship in-process via Cargo — no PATH tools):
//! - L1 [`lexical_primitive`]: borrowable text search (LexicalLane)
//! - L2 [`semantic_engine`]: Code-corpus owner (dense ∥ BM25 CC α=0.8;
//!   RRF kept for eval only — not product default)
//! - [`embed`] / [`ort_embed`]: ORT CPU WOQ product embedder
//! - [`facade`]: L3 human grouped search `{ text, semantic? }`
//! - [`store`] / [`build`] / [`chunk`]: index artifacts under `.litecode/index/`
//!
//! Fusion knobs such as [`retrieve::CODE_SEMANTIC_CC_ALPHA`] apply only to
//! workspace Code semantic search. Session / Knowledge must not share them.

mod bm25;
mod build;
mod chunk;
pub mod code_tokenize;
mod embed;
/// AST enclosing breadcrumbs for text hits (used by agent `grep`).
pub mod enclosing;
mod facade;
mod fs_notify;
mod index_status;
mod lexical;
mod lexical_primitive;
mod meta;
mod ort_embed;
mod reconcile;
mod retrieve;
pub mod scan_policy;
mod semantic_engine;
mod store;

pub use build::{build_full_index, scannable_files};
pub use embed::{
    EMBEDDER_ID_GRANITE97Q, EMBEDDER_ID_HASH, EMBEDDER_ID_PASS, Embedder, EmbeddingModelStatus,
    HashEmbedder, open_production_embedder, probe_embedding_model, production_embedder_id,
};
pub use enclosing::{
    AncestorSnippet, MAX_ANCESTOR_LINES, ScopeSegment, enclosing_scopes, format_breadcrumb,
    lines_slice, syntax_ancestor_snippet,
};
pub use facade::{HumanSearchRequest, HumanSearchResponse, human_search};
pub use fs_notify::queue_fs_changes;
pub use index_status::{
    IndexPhase, IndexStatus, IndexingProgress, ResolvedIndexView, begin_building, begin_refreshing,
    clear_index_job, disk_index_status, mark_index_job_failed, read_pending_hint,
    resolve_index_view, should_full_rebuild, update_build_progress, write_pending_hint,
};
pub use lexical::{
    LexicalMatch, LexicalQuery, LexicalSearchOutcome, lexical_search, lexical_search_with_preset,
    type_to_include_globs,
};
pub(crate) use lexical::{compile_exclude_globs, path_glob_match_exclude};
pub use lexical_primitive::LexicalPrimitive;
pub use meta::{
    IndexMeta, index_dir, init_workspace_index, meta_path, needs_rebuild, read_meta, write_meta,
};
pub use reconcile::{queue_reconcile_dirty, sync_index_with_disk};
pub use retrieve::{CODE_SEMANTIC_CC_ALPHA, SEARCH_MODE_BM25_CC, SEARCH_MODE_BM25_RRF, SearchHit};
pub use scan_policy::{
    MAX_INDEX_FILE_BYTES, SKIP_DIRS, TEXT_EXTENSIONS, is_indexable_rel_path, is_scannable_rel_path,
    looks_binary,
};
pub use semantic_engine::SemanticEngine;
pub use store::{CodeSearchIndex, index_files_exist};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::types::{LitecodeError, Result};

pub const MODEL_ID: &str = "ibm-granite/granite-embedding-97m-multilingual-r2";
/// Bumped for ORT CPU WOQ product cutover (invalidates candle Pass indexes).
pub const PIPELINE_VERSION: u32 = 7;
pub const EMBED_DIM: usize = 384;
pub const CHUNK_LINES: usize = 60;
pub const CHUNK_OVERLAP: usize = 12;
pub const EMBED_MAX_LENGTH: usize = 512;
/// Batch size for full / incremental index embedding.
pub const EMBED_INDEX_BATCH: usize = 32;
/// Batch size for query embedding.
pub const EMBED_QUERY_BATCH: usize = 4;
pub const EMBED_BATCH: usize = EMBED_INDEX_BATCH;
pub const RETRIEVE_K: usize = 30;
pub const DEFAULT_TOP_K: usize = 8;
pub const MAX_TOP_K: usize = 20;
/// Product embedder id: ORT MatMulNBits Q8 + GatherBlockQuantized Q4 (Pareto).
pub const EMBEDDER_ID_ORT_Q8Q4: &str = "granite97-ort-q8q4";
/// How often the worker reconciles disk ↔ index (dirty signals + flush).
pub const INDEX_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
/// Default idle before dropping ORT Session (L1 OrtCold). Override: `LITECODE_EMBEDDER_COOL_SECS`.
pub const EMBEDDER_COOL_IDLE: Duration = Duration::from_secs(30);
/// Default idle before unloading RAM index (L2 IndexCold). Override: `LITECODE_INDEX_COOL_SECS`.
pub const INDEX_COOL_IDLE: Duration = Duration::from_secs(300);

fn cool_idle_from_env(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
}

pub fn embedder_cool_idle() -> Duration {
    cool_idle_from_env("LITECODE_EMBEDDER_COOL_SECS", EMBEDDER_COOL_IDLE)
}

pub fn index_cool_idle() -> Duration {
    cool_idle_from_env("LITECODE_INDEX_COOL_SECS", INDEX_COOL_IDLE)
}

/// Lightweight on-disk identity for stamp fast-path during reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime_ms: u64,
    pub len: u64,
}

/// Shared runtime after warmup. ORT Session and RAM index may cool-drop; disk artifacts stay.
pub struct CodeSearchRuntime {
    pub workspace_root: PathBuf,
    session_reader: Mutex<Option<crate::session::SessionDataReader>>,
    /// Dense ANN + chunks; `None` while IndexCold (reload from `.litecode/index/`).
    pub index: Mutex<Option<CodeSearchIndex>>,
    /// Session corpus ANN (`.litecode/session-index/`); cools with L2 alongside Code index.
    pub session_index: Mutex<Option<crate::engines::session_search::SessionSemanticIndex>>,
    /// Code-corpus BM25 sidecar (Tantivy). Rebuilt when the dense index mutates.
    bm25: Mutex<Option<bm25::Bm25Index>>,
    embedder: Mutex<Option<Box<dyn embed::Embedder>>>,
    /// File changes from serve watcher via IPC; processed lazily on search / reconcile.
    pub pending_updates: Mutex<HashSet<(String, bool)>>, // (relative_path, deleted)
    /// mtime/len after a successful index of a path (reconcile fast path).
    pub file_stamps: Mutex<HashMap<String, FileStamp>>,
    /// Last successful embed (query or incremental). Drives L1 OrtCold.
    last_embed_at: Mutex<Instant>,
    /// Last search / incremental update / cold index load. Drives L2 IndexCold.
    last_index_at: Mutex<Instant>,
}

impl CodeSearchRuntime {
    pub fn new(
        workspace_root: PathBuf,
        index: CodeSearchIndex,
        embedder: Option<Box<dyn embed::Embedder>>,
        session_reader: Option<crate::session::SessionDataReader>,
    ) -> Self {
        let now = Instant::now();
        let runtime = Self {
            workspace_root,
            session_reader: Mutex::new(session_reader),
            index: Mutex::new(Some(index)),
            session_index: Mutex::new(None),
            bm25: Mutex::new(None),
            embedder: Mutex::new(embedder),
            pending_updates: Mutex::new(HashSet::new()),
            file_stamps: Mutex::new(HashMap::new()),
            last_embed_at: Mutex::new(now),
            last_index_at: Mutex::new(now),
        };
        if let Err(e) = runtime.resync_bm25() {
            tracing::warn!(error = %e, "code_search BM25 sidecar build failed at warmup");
        }
        runtime
    }

    fn touch_embed_at(&self) {
        if let Ok(mut t) = self.last_embed_at.lock() {
            *t = Instant::now();
        }
    }

    fn touch_index_at(&self) {
        if let Ok(mut t) = self.last_index_at.lock() {
            *t = Instant::now();
        }
    }

    /// Mark index as actively used (search / incremental update).
    pub(crate) fn note_index_activity(&self) {
        self.touch_index_at();
    }

    pub(crate) fn has_pending_updates(&self) -> bool {
        self.pending_updates
            .lock()
            .map(|g| !g.is_empty())
            .unwrap_or(true)
    }

    pub(crate) fn index_is_loaded(&self) -> bool {
        self.index.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub(crate) fn session_index_is_loaded(&self) -> bool {
        self.session_index
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    pub(crate) fn embedder_is_loaded(&self) -> bool {
        self.embedder.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn attach_session_reader(&self, reader: crate::session::SessionDataReader) {
        if let Ok(mut guard) = self.session_reader.lock() {
            *guard = Some(reader);
        }
    }

    fn session_reader(&self) -> Result<crate::session::SessionDataReader> {
        self.session_reader
            .lock()
            .map_err(|e| LitecodeError::Config(format!("session_reader lock: {e}")))?
            .clone()
            .ok_or_else(|| {
                LitecodeError::Config(
                    "session semantic index unavailable: SessionData reader not ready".into(),
                )
            })
    }

    /// Warm-time or IndexCold reload: build/load Session ANN from `sessions.db`.
    pub fn ensure_session_index(&self) -> Result<()> {
        let reader = self.session_reader()?;
        {
            let guard = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
            if guard.is_some() {
                return Ok(());
            }
        }
        tracing::info!("session_search loading session ANN index");
        let loaded = self.with_embedder(|emb| {
            crate::engines::session_search::ensure_session_index(&self.workspace_root, &reader, emb)
        })?;
        let mut guard = self
            .session_index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
        *guard = Some(loaded);
        self.touch_index_at();
        Ok(())
    }

    /// Light reconcile of Session ANN against `sessions.db`, then ANN search.
    pub fn search_sessions(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        self.ensure_session_index()?;
        self.with_embedder(|emb| {
            let mut guard = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
            let index = guard.as_mut().ok_or_else(|| {
                LitecodeError::Config("session_index missing after ensure".into())
            })?;
            let reader = self.session_reader()?;
            let _ = index.reconcile(&reader, &self.workspace_root, emb)?;
            let qv = emb.embed_one(query)?;
            let hits = index.search(&qv, top_k, session_id)?;
            self.touch_index_at();
            Ok(hits)
        })
    }

    /// Load dense index + BM25 from disk when IndexCold. Does not touch activity if already loaded.
    pub(crate) fn ensure_index(&self) -> Result<()> {
        {
            let guard = self
                .index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
            if guard.is_some() {
                drop(guard);
                self.ensure_bm25_open()?;
                return Ok(());
            }
        }

        tracing::info!("code_search loading index from disk (IndexCold → hot)");
        let loaded = CodeSearchIndex::load(&self.workspace_root)?;
        {
            let mut guard = self
                .index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
            *guard = Some(loaded);
        }
        self.ensure_bm25_open()?;
        self.touch_index_at();
        Ok(())
    }

    fn ensure_bm25_open(&self) -> Result<()> {
        {
            let guard = self
                .bm25
                .lock()
                .map_err(|e| LitecodeError::Config(format!("bm25 lock: {e}")))?;
            if guard.is_some() {
                return Ok(());
            }
        }
        match bm25::Bm25Index::open(&self.workspace_root) {
            Ok(opened) => {
                let mut guard = self
                    .bm25
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("bm25 lock: {e}")))?;
                *guard = Some(opened);
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "code_search BM25 open failed after index load; rebuilding"
                );
                self.resync_bm25()
            }
        }
    }

    pub(crate) fn with_embedder<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut dyn embed::Embedder) -> Result<T>,
    {
        let mut guard = self
            .embedder
            .lock()
            .map_err(|e| LitecodeError::Config(format!("embedder lock: {e}")))?;
        if guard.is_none() {
            // Lazy open after OrtCold (or tests that omitted a preloaded embedder).
            *guard = Some(embed::open_production_embedder()?);
        }
        let out = f(guard.as_mut().expect("embedder slot").as_mut());
        if out.is_ok() {
            self.touch_embed_at();
        }
        out
    }

    pub(crate) fn with_index_mut<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut CodeSearchIndex) -> Result<T>,
    {
        self.ensure_index()?;
        let mut guard = self
            .index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
        let index = guard.as_mut().ok_or_else(|| {
            LitecodeError::Config("code_search index missing after ensure".into())
        })?;
        f(index)
    }

    pub(crate) fn with_index<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&CodeSearchIndex) -> Result<T>,
    {
        self.ensure_index()?;
        let guard = self
            .index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
        let index = guard.as_ref().ok_or_else(|| {
            LitecodeError::Config("code_search index missing after ensure".into())
        })?;
        f(index)
    }

    /// Dense index + Code BM25 together (Code semantic CC path).
    pub(crate) fn with_index_and_bm25<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&CodeSearchIndex, &bm25::Bm25Index) -> Result<T>,
    {
        self.ensure_index()?;
        let index = self
            .index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
        let bm25 = self
            .bm25
            .lock()
            .map_err(|e| LitecodeError::Config(format!("bm25 lock: {e}")))?;
        let index = index.as_ref().ok_or_else(|| {
            LitecodeError::Config("code_search index missing after ensure".into())
        })?;
        let Some(bm25) = bm25.as_ref() else {
            return Err(LitecodeError::Config(
                "code_search BM25 sidecar unavailable".into(),
            ));
        };
        f(index, bm25)
    }

    /// Drop any open BM25 handle, rebuild from current chunks, reopen.
    /// Must close the handle before `bm25::rebuild` wipes the on-disk dir.
    pub(crate) fn resync_bm25(&self) -> Result<()> {
        if let Ok(mut guard) = self.bm25.lock() {
            *guard = None;
        }
        // Index must already be in RAM (caller ensured); rebuild from chunks.
        {
            let guard = self
                .index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
            let index = guard.as_ref().ok_or_else(|| {
                LitecodeError::Config("code_search cannot rebuild BM25 without index".into())
            })?;
            bm25::rebuild(&self.workspace_root, index.chunks())?;
        }
        let opened = bm25::Bm25Index::open(&self.workspace_root)?;
        let mut guard = self
            .bm25
            .lock()
            .map_err(|e| LitecodeError::Config(format!("bm25 lock: {e}")))?;
        *guard = Some(opened);
        Ok(())
    }

    /// L1 OrtCold: drop ORT Session when idle and no pending file updates.
    pub fn drop_embedder_for_cool(&self) {
        if self.has_pending_updates() {
            return;
        }
        let mut guard = match self.embedder.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if guard.is_none() {
            return;
        }
        tracing::info!("code_search OrtCold: dropping embedder session");
        *guard = None;
        drop(guard);
        crate::telemetry::release_heap_to_os();
    }

    /// L2 IndexCold: unload RAM Code + Session indexes + BM25 (disk artifacts retained).
    pub fn drop_index_for_cool(&self) {
        if self.has_pending_updates() {
            return;
        }
        {
            let mut bm25 = match self.bm25.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            *bm25 = None;
        }
        {
            let mut index = match self.index.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if index.is_some() {
                tracing::info!("code_search IndexCold: unloading RAM code index");
                *index = None;
            }
        }
        {
            let mut session = match self.session_index.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if session.is_some() {
                tracing::info!("code_search IndexCold: unloading RAM session index");
                *session = None;
            }
        }
        crate::telemetry::release_heap_to_os();
    }

    /// Periodic two-tier cool (worker poll). Pending updates block both tiers.
    pub fn maybe_cool_memory(&self) {
        self.maybe_cool_memory_with(embedder_cool_idle(), index_cool_idle());
    }

    /// Test / env-override entry: cool using explicit idle thresholds.
    pub fn maybe_cool_memory_with(&self, embedder_idle: Duration, index_idle: Duration) {
        if self.has_pending_updates() {
            return;
        }
        if self.embedder_is_loaded() {
            let idle = self
                .last_embed_at
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or(Duration::ZERO);
            if idle >= embedder_idle {
                self.drop_embedder_for_cool();
            }
        }
        if self.index_is_loaded() || self.session_index_is_loaded() {
            let idle = self
                .last_index_at
                .lock()
                .map(|t| t.elapsed())
                .unwrap_or(Duration::ZERO);
            if idle >= index_idle {
                self.drop_index_for_cool();
            }
        }
    }

    fn note_file_stamp(&self, rel_path: &str) {
        let abs = self.workspace_root.join(rel_path);
        if let Some(stamp) = reconcile::read_stamp(&abs)
            && let Ok(mut stamps) = self.file_stamps.lock()
        {
            stamps.insert(rel_path.to_string(), stamp);
        }
    }

    fn clear_file_stamp(&self, rel_path: &str) {
        if let Ok(mut stamps) = self.file_stamps.lock() {
            stamps.remove(rel_path);
        }
    }
}

/// Load or build index for warmup. Caller owns embedder lifecycle.
///
/// Compatible on-disk vectors are **loaded**. Stale/pending file drift is not a
/// startup full rebuild (worker `sync_index_with_disk` catches up). Full build
/// only when the library is absent, a shell with no vectors, or unloadable
/// (pipeline/embedder mismatch).
pub fn warmup_index(
    workspace_root: &Path,
    embedder: &mut dyn embed::Embedder,
) -> Result<CodeSearchIndex> {
    let dir = index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

    let rebuild = should_full_rebuild(workspace_root);

    let result: Result<CodeSearchIndex> = if rebuild {
        let index = build::build_full_index(workspace_root, embedder)?;
        index.save(workspace_root)?;
        Ok(index)
    } else if let Ok(index) = CodeSearchIndex::load(workspace_root) {
        Ok(index)
    } else {
        let index = build::build_full_index(workspace_root, embedder)?;
        index.save(workspace_root)?;
        Ok(index)
    };
    match &result {
        Ok(_) => {
            clear_index_job(workspace_root);
            write_pending_hint(workspace_root, 0);
        }
        Err(e) => mark_index_job_failed(workspace_root, e.to_string()),
    }
    result
}

/// Force full rebuild of the in-memory + on-disk index (Warm worker path).
pub fn rebuild_index_in_runtime(runtime: &CodeSearchRuntime) -> Result<()> {
    begin_building(&runtime.workspace_root);
    let built = runtime.with_embedder(|emb| {
        let index = build::build_full_index(&runtime.workspace_root, emb)?;
        index.save(&runtime.workspace_root)?;
        Ok(index)
    });
    match built {
        Ok(index) => {
            {
                let mut guard = runtime
                    .index
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
                *guard = Some(index);
            }
            if let Err(e) = runtime.resync_bm25() {
                tracing::warn!(error = %e, "code_search BM25 resync after rebuild failed");
            }
            runtime.note_index_activity();
            clear_index_job(&runtime.workspace_root);
            write_pending_hint(&runtime.workspace_root, 0);
            if let Ok(mut pending) = runtime.pending_updates.lock() {
                pending.clear();
            }
            Ok(())
        }
        Err(e) => {
            mark_index_job_failed(&runtime.workspace_root, e.to_string());
            Err(e)
        }
    }
}

/// Incremental disk sync while Warm (refresh path when index is compatible).
pub fn refresh_index_incremental(runtime: &CodeSearchRuntime) -> Result<()> {
    begin_refreshing(&runtime.workspace_root);
    sync_index_with_disk(runtime);
    let pending = runtime.pending_updates.lock().map(|g| g.len()).unwrap_or(0);
    write_pending_hint(&runtime.workspace_root, pending);
    clear_index_job(&runtime.workspace_root);
    Ok(())
}

/// Incremental update for changed relative paths (debounced watcher / reconcile).
pub fn update_files(runtime: &CodeSearchRuntime, updates: &[(String, bool)]) -> Result<()> {
    let mut changed = false;

    // One pass for deletions: a single index save + stamp clear for the batch.
    let deleted: Vec<&str> = updates
        .iter()
        .filter(|(_, d)| *d)
        .map(|(p, _)| p.as_str())
        .collect();
    if !deleted.is_empty() {
        runtime.with_index_mut(|index| {
            for path in &deleted {
                index.remove_file(path);
            }
            index.save(&runtime.workspace_root)?;
            meta::write_meta(&runtime.workspace_root, &index.meta_snapshot())?;
            Ok(())
        })?;
        for path in &deleted {
            runtime.clear_file_stamp(path);
        }
        changed = true;
    }

    for (path, is_deleted) in updates {
        if *is_deleted {
            continue;
        }
        if !is_indexable_rel_path(path, &runtime.workspace_root) {
            if let Err(e) = runtime.with_index_mut(|index| {
                index.remove_file(path);
                Ok(())
            }) {
                tracing::warn!(path = %path, error = %e, "code_search remove non-indexable path failed");
            }
            runtime.clear_file_stamp(path);
            changed = true;
            continue;
        }
        let abs = runtime.workspace_root.join(path);
        if !abs.is_file() {
            if let Err(e) = runtime.with_index_mut(|index| {
                index.remove_file(path);
                Ok(())
            }) {
                tracing::warn!(path = %path, error = %e, "code_search remove missing file failed");
            }
            runtime.clear_file_stamp(path);
            changed = true;
            continue;
        }
        let content = match std::fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search incremental read failed");
                continue;
            }
        };
        let next_id = match runtime.with_index(|index| Ok(index.next_id())) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search incremental index read failed");
                continue;
            }
        };
        let (chunks, new_id) = chunk::chunk_file(path, &content, next_id);
        if chunks.is_empty() {
            continue;
        }
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let vectors = match runtime.with_embedder(|emb| emb.embed_batch(&texts)) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search incremental embed failed");
                continue;
            }
        };
        match runtime.with_index_mut(|index| {
            index.remove_file(path);
            index.set_next_id(new_id);
            index.add_chunks_with_vectors(chunks, vectors)?;
            index.save(&runtime.workspace_root)?;
            meta::write_meta(&runtime.workspace_root, &index.meta_snapshot())?;
            Ok(())
        }) {
            Ok(()) => {
                runtime.note_file_stamp(path);
                changed = true;
            }
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search incremental index update failed");
            }
        }
    }
    if changed {
        if let Err(e) = runtime.resync_bm25() {
            tracing::warn!(error = %e, "code_search BM25 resync after incremental update failed");
        }
        runtime.note_index_activity();
    }
    Ok(())
}

/// Drain `pending_updates` into [`update_files`] (shared by search + periodic sync).
pub fn flush_pending_updates(runtime: &CodeSearchRuntime) {
    let pending: Vec<(String, bool)> = match runtime.pending_updates.lock() {
        Ok(mut guard) => guard.drain().collect(),
        Err(_) => vec![],
    };
    if pending.is_empty() {
        write_pending_hint(&runtime.workspace_root, 0);
        return;
    }
    // Single merged pass: one index save + one BM25 resync for the whole batch.
    let _ = update_files(runtime, &pending);
    let remaining = runtime.pending_updates.lock().map(|g| g.len()).unwrap_or(0);
    write_pending_hint(&runtime.workspace_root, remaining);
}

/// Semantic search entry — delegates to [`SemanticEngine`] (L2 sole owner).
pub fn search(
    runtime: &CodeSearchRuntime,
    query: &str,
    glob_filter: Option<&str>,
    top_k: usize,
) -> Result<Vec<SearchHit>> {
    SemanticEngine::search(runtime, query, glob_filter, top_k)
}

/// Handle held by the engine and tool for query / incremental updates.
pub type SharedRuntime = Arc<RwLock<Option<CodeSearchRuntime>>>;

#[cfg(test)]
mod cool_tests {
    use super::*;
    use crate::engines::code_search::build::build_full_index;
    use crate::engines::code_search::embed::HashEmbedder;
    use tempfile::TempDir;

    fn runtime_with_hash(root: &Path) -> CodeSearchRuntime {
        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        index.save(root).unwrap();
        CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        )
    }

    #[test]
    fn drop_embedder_and_index_reload_for_search() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("main.rs"), "fn cool_reload_target() {}\n").unwrap();
        let runtime = runtime_with_hash(root);

        assert!(runtime.embedder_is_loaded());
        assert!(runtime.index_is_loaded());

        runtime.drop_embedder_for_cool();
        assert!(!runtime.embedder_is_loaded());
        assert!(runtime.index_is_loaded());

        runtime.drop_index_for_cool();
        assert!(!runtime.index_is_loaded());

        // Hash embedder path: with_embedder opens production ORT — avoid that in unit test.
        // Re-install hash embedder and ensure_index from disk.
        {
            let mut emb = runtime.embedder.lock().unwrap();
            *emb = Some(Box::new(HashEmbedder));
        }
        runtime.ensure_index().unwrap();
        assert!(runtime.index_is_loaded());

        let hits = SemanticEngine::search(&runtime, "cool_reload_target", None, 5).unwrap();
        assert!(!hits.is_empty());
        assert!(runtime.embedder_is_loaded());
        assert!(runtime.index_is_loaded());
    }

    #[test]
    fn pending_blocks_cool_drop() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let runtime = runtime_with_hash(root);

        runtime
            .pending_updates
            .lock()
            .unwrap()
            .insert(("a.rs".into(), false));

        runtime.drop_embedder_for_cool();
        assert!(runtime.embedder_is_loaded(), "pending must block OrtCold");

        runtime.drop_index_for_cool();
        assert!(runtime.index_is_loaded(), "pending must block IndexCold");
    }

    #[test]
    fn maybe_cool_respects_zero_idle() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let runtime = runtime_with_hash(root);

        runtime.note_index_activity();
        runtime.maybe_cool_memory_with(Duration::ZERO, Duration::ZERO);

        assert!(!runtime.embedder_is_loaded());
        assert!(!runtime.index_is_loaded());
    }
}
