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

mod atomic;
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
mod stamps;
mod store;

#[cfg(test)]
pub(crate) use atomic::WRITTEN_BYTES;
pub use atomic::{
    AtomicPart, append_durable, append_fsync, atomic_write_bytes, atomic_write_set,
    sweep_atomic_tmp, truncate_to_committed,
};
pub use build::scannable_files;
pub use embed::{
    EMBEDDER_ID_GRANITE97Q, EMBEDDER_ID_HASH, EMBEDDER_ID_PASS, Embedder, EmbeddingModelStatus,
    HashEmbedder, model_dir, open_production_embedder, probe_embedding_model,
    production_embedder_id,
};
pub use enclosing::{
    AncestorSnippet, MAX_ANCESTOR_LINES, ScopeEntry, ScopeMatch, ScopeSegment, enclosing_scopes,
    find_scope, format_breadcrumb, lines_slice, list_scopes, scope_at, syntax_ancestor_snippet,
};
pub use facade::{HumanSearchRequest, HumanSearchResponse, human_search};
pub use fs_notify::queue_fs_changes;
pub use index_status::{
    CodeEngineFacts, CodeQueue, IndexPhase, IndexRebuildReason, IndexStatus, IndexWork,
    IndexingProgress, ResolvedIndexView, begin_refreshing, clear_index_job, code_corpus_progress,
    decide_index_work, disk_index_status, index_work_from_disk, indexed_chunk_count,
    mark_index_job_failed, pending_work_counts, read_code_queue, reclaim_stale_index_job,
    resolve_index_view, should_full_rebuild, update_chunk_progress, write_code_queue,
    write_code_queue_kept,
};
pub use lexical::{
    LexicalMatch, LexicalQuery, LexicalSearchOutcome, lexical_search, lexical_search_for_agent,
    lexical_search_for_agent_cancellable, lexical_search_with_preset, type_to_include_globs,
};
pub use lexical_primitive::LexicalPrimitive;
pub use meta::{
    IndexMeta, index_dir, init_workspace_index, meta_path, needs_rebuild, read_meta, write_meta,
};
pub use reconcile::{queue_reconcile_dirty, queue_reconcile_from_stamps, sync_index_with_disk};
pub use retrieve::{CODE_SEMANTIC_CC_ALPHA, SEARCH_MODE_BM25_CC, SEARCH_MODE_BM25_RRF, SearchHit};
pub use scan_policy::{
    MAX_INDEX_FILE_BYTES, SKIP_DIRS, is_indexable_rel_path, is_scannable_rel_path, looks_binary,
};
pub use semantic_engine::SemanticEngine;
pub use store::{CodeSearchIndex, index_files_exist};
pub(crate) use store::{persist_usearch, restore_usearch};

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
    /// Nanoseconds since the Unix epoch (WP-141: ms could hide same-ms edits).
    pub mtime_ns: u64,
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
    /// Last live inference device (`cuda-ort` / `cpu-ort` / `hash`). Survives OrtCold.
    embed_device: Mutex<String>,
    /// File changes from serve watcher via IPC; consumed on refresh / agent.
    pub pending_updates: Mutex<HashSet<(String, bool)>>, // (relative_path, deleted)
    /// mtime/len after a successful index of a path (reconcile fast path).
    /// Persisted to `stamps.v1` after every index commit (WP-141).
    pub file_stamps: Mutex<HashMap<String, FileStamp>>,
    /// WP-141: set when a cooled index had no valid `stamps.v1` to scan
    /// against; the next index load runs a full reconcile. Internal only.
    full_reconcile_on_load: std::sync::atomic::AtomicBool,
    /// WP-143: paths whose stamp changed since the last `stamps.v1` write
    /// (appended per batch); `stamps_full` forces a full rewrite.
    stamps_dirty: Mutex<HashSet<String>>,
    stamps_full: std::sync::atomic::AtomicBool,
    stamps_appended: std::sync::atomic::AtomicUsize,
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
        let device = embedder
            .as_ref()
            .map(|emb| emb.inference_device().to_string())
            .unwrap_or_default();
        // WP-141: the persisted `stamps.v1` (validated against meta.json)
        // seeds the map. WP-143: a fresh index has none; files are stamped
        // batch by batch as they commit.
        let file_stamps: HashMap<String, FileStamp> = stamps::load(&workspace_root)
            .map(|entries| {
                entries
                    .into_iter()
                    .filter_map(|(p, s)| s.map(|s| (p, s)))
                    .collect()
            })
            .unwrap_or_default();
        let runtime = Self {
            workspace_root,
            session_reader: Mutex::new(session_reader),
            index: Mutex::new(Some(index)),
            session_index: Mutex::new(None),
            bm25: Mutex::new(None),
            embedder: Mutex::new(embedder),
            embed_device: Mutex::new(device),
            pending_updates: Mutex::new(HashSet::new()),
            file_stamps: Mutex::new(file_stamps),
            full_reconcile_on_load: std::sync::atomic::AtomicBool::new(false),
            stamps_dirty: Mutex::new(HashSet::new()),
            stamps_full: std::sync::atomic::AtomicBool::new(true),
            stamps_appended: std::sync::atomic::AtomicUsize::new(0),
            last_embed_at: Mutex::new(now),
            last_index_at: Mutex::new(now),
        };
        if let Err(e) = runtime.resync_bm25() {
            tracing::warn!(error = %e, "code_search BM25 sidecar build failed at warmup");
        }
        runtime
    }

    /// WP-141: write `stamps.v1` after an index commit. Keys = every path the
    /// index holds (unstamped ones as `?`) plus stamped paths that produced no
    /// chunks, so a cooled index still knows its full path set. Loaded index
    /// only; a cooled index has nothing new to commit.
    pub(crate) fn persist_stamps(&self) {
        use std::sync::atomic::Ordering;
        let Ok(guard) = self.index.lock() else {
            return;
        };
        let Some(index) = guard.as_ref() else {
            return;
        };
        // WP-143: append only the changed paths (bytes ≈ the batch). Full
        // rewrite on the first write, when the header no longer matches, or
        // once the appended lines outgrow the map (amortized compaction).
        let dirty: Vec<String> = self
            .stamps_dirty
            .lock()
            .map(|mut d| d.drain().collect())
            .unwrap_or_default();
        let full = self.stamps_full.swap(false, Ordering::SeqCst);
        let limit = self
            .file_stamps
            .lock()
            .map(|s| s.len())
            .unwrap_or(0)
            .max(1024);
        if !full && self.stamps_appended.load(Ordering::SeqCst) + dirty.len() <= limit {
            let changes: Vec<(String, stamps::StampChange)> = {
                let Ok(st) = self.file_stamps.lock() else {
                    return;
                };
                dirty
                    .into_iter()
                    .map(|p| {
                        let c = match st.get(&p) {
                            Some(s) => Some(Some(*s)),
                            None if index.has_path(&p) => Some(None),
                            None => None,
                        };
                        (p, c)
                    })
                    .collect()
            };
            if stamps::append(&self.workspace_root, &changes) {
                self.stamps_appended
                    .fetch_add(changes.len(), Ordering::SeqCst);
                return;
            }
        }
        self.stamps_appended.store(0, Ordering::SeqCst);
        let indexed = index.indexed_paths();
        drop(guard);
        let Ok(stamps) = self.file_stamps.lock() else {
            return;
        };
        let mut entries: stamps::StampEntries = indexed.into_iter().map(|p| (p, None)).collect();
        for (path, stamp) in stamps.iter() {
            entries.insert(path.clone(), Some(*stamp));
        }
        drop(stamps);
        stamps::save(&self.workspace_root, &entries);
    }

    pub(crate) fn request_full_reconcile_on_load(&self) {
        self.full_reconcile_on_load
            .store(true, std::sync::atomic::Ordering::SeqCst);
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

    /// Persist the worker queue with its exact surviving chunk count
    /// (`queue.json`, WP-131). Takes the index lock briefly.
    pub(crate) fn persist_queue(&self, pending: &HashSet<(String, bool)>) {
        // Loaded index only: never reload a cooled index under the queue lock.
        // Lock order pending -> index matches update paths (none take the
        // reverse). Falls back to the previous / on-disk value when cooled.
        let kept = self.index.lock().ok().and_then(|guard| {
            let index = guard.as_ref()?;
            let paths: HashSet<&str> = pending.iter().map(|(p, _)| p.as_str()).collect();
            let old = index
                .chunks()
                .values()
                .filter(|c| paths.contains(c.path.as_str()))
                .count() as u64;
            Some((index.chunks().len() as u64).saturating_sub(old))
        });
        match kept {
            Some(kept) => write_code_queue_kept(&self.workspace_root, pending, kept),
            None => write_code_queue(&self.workspace_root, pending),
        }
    }

    pub(crate) fn has_pending_updates(&self) -> bool {
        let code_dirty = self
            .pending_updates
            .lock()
            .map(|g| !g.is_empty())
            .unwrap_or(true);
        if code_dirty {
            return true;
        }
        crate::engines::session_search::read_session_queue(&self.workspace_root).queued > 0
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

    fn stamp_embed_device(&self, device: &str) {
        if let Ok(mut guard) = self.embed_device.lock() {
            *guard = device.to_string();
        }
    }

    /// Device captured when the ORT (or hash) session last opened.
    pub fn embed_device(&self) -> String {
        self.embed_device
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
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

    /// Load Session ANN from disk (or empty shell). Does not embed.
    pub fn ensure_session_index(&self) -> Result<()> {
        let reader = self.session_reader()?;
        {
            let guard = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
            if guard.is_some() {
                crate::engines::session_search::queue_session_dirty(&self.workspace_root, &reader);
                return Ok(());
            }
        }
        tracing::info!("session_search loading session ANN index");
        let loaded = crate::engines::session_search::load_session_index(&self.workspace_root)?;
        crate::engines::session_search::queue_session_dirty(&self.workspace_root, &reader);
        let mut guard = self
            .session_index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
        *guard = Some(loaded);
        self.touch_index_at();
        Ok(())
    }

    /// First-desired / refresh consume: embed session drift and save.
    pub fn consume_session_index(&self) -> Result<()> {
        let reader = match self.session_reader() {
            Ok(r) => r,
            Err(_) => return Ok(()),
        };
        self.ensure_session_index()?;
        self.with_embedder(|emb| {
            let mut guard = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
            let index = guard
                .as_mut()
                .ok_or_else(|| LitecodeError::Config("session_index missing after load".into()))?;
            crate::engines::session_search::consume_session_index(
                &self.workspace_root,
                &reader,
                emb,
                index,
            )?;
            Ok(())
        })?;
        self.touch_index_at();
        Ok(())
    }

    /// ANN search only. Load from disk if cold; does not digest `sessions.db`.
    pub fn search_sessions(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        if self.session_reader().is_err() {
            return Err(LitecodeError::Config(
                "session semantic index unavailable: SessionData reader not ready".into(),
            ));
        }
        {
            let loaded = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?
                .is_some();
            if !loaded {
                let loaded_idx =
                    crate::engines::session_search::load_session_index(&self.workspace_root)?;
                let mut guard = self
                    .session_index
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
                *guard = Some(loaded_idx);
            }
        }
        self.with_embedder(|emb| {
            let guard = self
                .session_index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("session_index lock: {e}")))?;
            let index = guard
                .as_ref()
                .ok_or_else(|| LitecodeError::Config("session_index missing after load".into()))?;
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
        let mut loaded = CodeSearchIndex::load(&self.workspace_root)?;
        // WP-143: the load repair dropped these files; forget their stamps
        // so the next reconcile re-reads them.
        if loaded.take_stamps_reset() {
            if let Ok(mut st) = self.file_stamps.lock() {
                st.clear();
            }
            self.force_full_stamps();
        }
        for path in loaded.take_repaired_paths() {
            self.clear_file_stamp(&path);
        }
        {
            let mut guard = self
                .index
                .lock()
                .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
            *guard = Some(loaded);
        }
        self.ensure_bm25_open()?;
        self.touch_index_at();
        // WP-141: the cooled scan could not trust its stamps; reconcile now
        // that the index (the real path set) is back. Stat + queue only.
        if self
            .full_reconcile_on_load
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            reconcile::queue_reconcile_dirty(self);
        }
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
            let emb = embed::open_production_embedder()?;
            self.stamp_embed_device(emb.inference_device());
            *guard = Some(emb);
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

    fn set_file_stamp(&self, rel_path: &str, stamp: FileStamp) {
        if let Ok(mut stamps) = self.file_stamps.lock() {
            stamps.insert(rel_path.to_string(), stamp);
        }
        self.mark_stamp_dirty(rel_path);
    }

    fn clear_file_stamp(&self, rel_path: &str) {
        if let Ok(mut stamps) = self.file_stamps.lock() {
            stamps.remove(rel_path);
        }
        self.mark_stamp_dirty(rel_path);
    }

    fn mark_stamp_dirty(&self, rel_path: &str) {
        if let Ok(mut d) = self.stamps_dirty.lock() {
            d.insert(rel_path.to_string());
        }
    }

    /// Next `persist_stamps` rewrites the whole file (fresh index, reset).
    fn force_full_stamps(&self) {
        self.stamps_full
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// WP-143: wipe the code index and start a fresh, empty, compatible one.
/// Clears vectors/log/chunks/checkpoint/meta/stamps/queue, then commits an
/// empty store with a new meta (pipeline/model/embedder of `embedder_id`).
/// Everything is then indexed through the incremental path ([`update_files`]),
/// batch by batch, so an interrupted first build resumes from its stamps.
pub fn start_fresh_index(workspace_root: &Path, embedder_id: &str) -> Result<CodeSearchIndex> {
    let dir = index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    stamps::discard(workspace_root);
    for p in [
        store::checkpoint_path(workspace_root),
        meta_path(workspace_root),
        store::vectors_log_path(workspace_root),
        store::vectors_path(workspace_root),
        store::chunks_path(workspace_root),
    ] {
        let _ = std::fs::remove_file(p);
    }
    write_code_queue(workspace_root, &Default::default());
    let mut index = CodeSearchIndex::new_empty()?;
    index.set_embedder_id(embedder_id);
    index.save(workspace_root)?;
    Ok(index)
}

/// Index `workspace_root` from scratch through the production path
/// ([`start_fresh_index`] + reconcile + [`update_files`], batch by batch) and
/// return the finished index. Blocking; for tests and offline tools.
pub fn index_workspace_blocking(
    workspace_root: &Path,
    embedder: Box<dyn embed::Embedder>,
) -> Result<CodeSearchIndex> {
    let fresh = start_fresh_index(workspace_root, embedder.embedder_id())?;
    let runtime = CodeSearchRuntime::new(workspace_root.to_path_buf(), fresh, Some(embedder), None);
    refresh_index_incremental(&runtime)?;
    runtime.persist_stamps();
    let index = runtime
        .index
        .lock()
        .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?
        .take()
        .ok_or_else(|| LitecodeError::Config("code_search index missing".into()))?;
    Ok(index)
}

/// Load the index for warmup. Caller owns embedder lifecycle.
///
/// Compatible on-disk vectors are **loaded** (a partial first build too; the
/// worker's reconcile queues what is missing). Absent / incompatible /
/// unloadable ⇒ [`start_fresh_index`]: an empty index, filled afterwards by
/// the incremental path. No separate full-build path (WP-143).
pub fn warmup_index(
    workspace_root: &Path,
    embedder: &mut dyn embed::Embedder,
) -> Result<CodeSearchIndex> {
    let dir = index_dir(workspace_root);
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let result = if should_full_rebuild(workspace_root) {
        start_fresh_index(workspace_root, embedder.embedder_id())
    } else {
        match CodeSearchIndex::load(workspace_root) {
            Ok(index) => Ok(index),
            Err(e) => {
                tracing::warn!(error = %e, "code_search index unloadable; starting fresh");
                start_fresh_index(workspace_root, embedder.embedder_id())
            }
        }
    };
    match &result {
        Ok(_) => clear_index_job(workspace_root),
        Err(e) => mark_index_job_failed(workspace_root, e.to_string()),
    }
    result
}

/// Rebuild (manual / incompatible) inside a live runtime: start fresh, queue
/// every scannable file, and run the incremental path. Search keeps working
/// on the partial index between batches (status refreshing).
pub fn rebuild_index_in_runtime(runtime: &CodeSearchRuntime) -> Result<()> {
    let root = &runtime.workspace_root;
    let embedder_id = runtime.with_embedder(|emb| Ok(emb.embedder_id()))?;
    let fresh = match start_fresh_index(root, embedder_id) {
        Ok(i) => i,
        Err(e) => {
            mark_index_job_failed(root, e.to_string());
            return Err(e);
        }
    };
    {
        let mut guard = runtime
            .index
            .lock()
            .map_err(|e| LitecodeError::Config(format!("index lock: {e}")))?;
        *guard = Some(fresh);
    }
    if let Ok(mut st) = runtime.file_stamps.lock() {
        st.clear();
    }
    if let Ok(mut d) = runtime.stamps_dirty.lock() {
        d.clear();
    }
    runtime.force_full_stamps();
    if let Err(e) = runtime.resync_bm25() {
        tracing::warn!(error = %e, "code_search BM25 reset for rebuild failed");
    }
    if let Ok(mut pending) = runtime.pending_updates.lock() {
        pending.clear();
    }
    refresh_index_incremental(runtime)
}

/// Incremental disk sync while Warm (refresh path when index is compatible).
pub fn refresh_index_incremental(runtime: &CodeSearchRuntime) -> Result<()> {
    // Reconcile first so `queue.json.kept_chunks` already covers every dirty
    // path when the job starts: done never drops between start and pass 1.
    reconcile::queue_reconcile_dirty(runtime);
    begin_refreshing(&runtime.workspace_root);
    flush_pending_updates(runtime);
    if let Ok(pending) = runtime.pending_updates.lock() {
        runtime.persist_queue(&pending); // WP-131 exact signal
    }
    clear_index_job(&runtime.workspace_root);
    Ok(())
}

/// Incremental update for changed relative paths (debounced watcher / reconcile).
///
/// WP-131 two-pass progress (denominator = whole index):
/// 1. scanning (`total = null`): classify paths and chunk every dirty file;
///    `done = kept` = index chunks minus old chunks of removed + re-indexed files.
/// 2. embedding: `total = kept + new`, `done` rises from `kept` per file and is
///    pinned to the real index size at the end (`done == total`).
///
/// Progress writes are no-ops when no refresh job is in flight.
pub fn update_files(runtime: &CodeSearchRuntime, updates: &[(String, bool)]) -> Result<()> {
    let root = &runtime.workspace_root;

    // ---- Pass 1: classify + chunk (scanning) ----
    // WP-141: reconcile only stats, so a stamp mismatch lands here; the
    // content comparison happens in this pass. Stamps are taken before each
    // read (a write racing the read leaves an older stamp ⇒ re-checked).
    // WP-143: chunk text is held up to PASS1_HOLD_BYTES, beyond that only the
    // count is kept and the file is re-chunked in pass 2.
    let mut removals: Vec<String> = Vec::new();
    let mut reindex: Vec<Reindex> = Vec::new();
    // Paths whose stamp is refreshed without any embedding.
    let mut stamp_only: Vec<(String, FileStamp)> = Vec::new();
    let mut held_bytes = 0usize;
    for (path, is_deleted) in updates {
        if *is_deleted || !is_indexable_rel_path(path, root) || !root.join(path).is_file() {
            // WP-141: a file that exists but is not indexable (binary, too
            // large) is accounted with a stamp so the stat scan skips it.
            if !*is_deleted
                && root.join(path).is_file()
                && let Some(stamp) = reconcile::read_stamp(&root.join(path))
            {
                stamp_only.push((path.clone(), stamp));
            }
            removals.push(path.clone());
            continue;
        }
        let stamp = reconcile::read_stamp(&root.join(path));
        let content = match std::fs::read_to_string(root.join(path)) {
            Ok(c) => c,
            Err(e) => {
                // Not text: account for it so it is not re-queued every pass.
                if e.kind() == std::io::ErrorKind::InvalidData
                    && let Some(stamp) = stamp
                {
                    stamp_only.push((path.clone(), stamp));
                }
                // Unreadable: keep its old chunks; excluded from the work set.
                tracing::warn!(path = %path, error = %e, "code_search incremental read failed");
                continue;
            }
        };
        // Ids are provisional here; re-assigned from the index at commit.
        let (chunks, _) = chunk::chunk_file(path, &content, 1);
        if chunks.is_empty() {
            removals.push(path.clone());
            if let Some(stamp) = stamp {
                stamp_only.push((path.clone(), stamp));
            }
            continue;
        }
        // Same chunks as indexed (touch, or a stamp lost to invalidation):
        // refresh the stamp only, never re-embed.
        let same = runtime
            .with_index(|index| {
                Ok(index.has_path(path)
                    && !index
                        .paths_with_identical_chunks(std::iter::once((
                            path.as_str(),
                            chunks.as_slice(),
                        )))
                        .is_empty())
            })
            .unwrap_or(false);
        if same {
            if let Some(stamp) = stamp {
                stamp_only.push((path.clone(), stamp));
            }
            continue;
        }
        let n = chunks.len();
        held_bytes += chunks.iter().map(|c| c.text.len()).sum::<usize>();
        let held = (held_bytes <= PASS1_HOLD_BYTES).then_some(chunks);
        reindex.push(Reindex {
            path: path.clone(),
            held,
            n,
            stamp,
        });
    }
    let touched: Vec<&str> = removals
        .iter()
        .map(String::as_str)
        .chain(reindex.iter().map(|r| r.path.as_str()))
        .collect();
    let (current, old_touched) = runtime
        .with_index(|index| {
            let old: usize = touched.iter().map(|p| index.file_chunk_count(p)).sum();
            Ok((index.chunks().len() as u64, old as u64))
        })
        .unwrap_or((0, 0));
    let kept = current.saturating_sub(old_touched);
    update_chunk_progress(root, kept, None);
    let new_total: u64 = reindex.iter().map(|r| r.n as u64).sum();
    let total = kept + new_total;

    let mut changed = false;
    if !removals.is_empty() {
        runtime.with_index_mut(|index| {
            for path in &removals {
                index.remove_file(path);
            }
            index.save(root)
        })?;
        for path in &removals {
            runtime.clear_file_stamp(path);
        }
        changed = true;
    }
    let stamps_touched = !stamp_only.is_empty();
    for (path, stamp) in stamp_only {
        runtime.set_file_stamp(&path, stamp);
    }
    if changed || stamps_touched {
        runtime.persist_stamps();
    }

    // ---- Pass 2: embed per EMBED_BATCH, commit per batch (WP-143) ----
    // A batch commit = the files whose chunks are all embedded: replace them
    // in the index, one appended save, then their stamps. A file spanning
    // batches is held in memory until its last chunk is embedded (R2), so a
    // kill re-embeds at most the uncommitted part; committed chunks never.
    let mut done = kept;
    update_chunk_progress(root, done, Some(total));
    let mut files: Vec<Option<FileAcc>> = Vec::with_capacity(reindex.len());
    let mut queue: std::collections::VecDeque<(usize, chunk::ChunkRecord)> =
        std::collections::VecDeque::new();
    let mut next_file = reindex.into_iter();
    loop {
        // Feed whole files until a batch is full (or the input ends).
        while queue.len() < EMBED_BATCH {
            let Some(r) = next_file.next() else { break };
            let chunks = match r.held {
                Some(c) => c,
                None => match std::fs::read_to_string(root.join(&r.path)) {
                    Ok(content) => chunk::chunk_file(&r.path, &content, 1).0,
                    Err(e) => {
                        tracing::warn!(path = %r.path, error = %e, "code_search re-read failed");
                        continue;
                    }
                },
            };
            if chunks.is_empty() {
                continue;
            }
            let idx = files.len();
            files.push(Some(FileAcc {
                path: r.path,
                stamp: r.stamp,
                want: chunks.len(),
                chunks: Vec::with_capacity(chunks.len()),
                vectors: Vec::with_capacity(chunks.len()),
            }));
            queue.extend(chunks.into_iter().map(|c| (idx, c)));
        }
        if queue.is_empty() {
            break;
        }
        let take = queue.len().min(EMBED_BATCH);
        let batch: Vec<(usize, chunk::ChunkRecord)> = queue.drain(..take).collect();
        let texts: Vec<String> = batch.iter().map(|(_, c)| c.text.clone()).collect();
        let vectors = match runtime.with_embedder(|emb| emb.embed_batch(&texts)) {
            Ok(v) if v.len() == batch.len() => v,
            Ok(_) | Err(_) => {
                // Drop every file touched by this batch; they keep their old
                // chunks and stamps and are retried on the next pass.
                let bad: HashSet<usize> = batch.iter().map(|(i, _)| *i).collect();
                for i in &bad {
                    if let Some(f) = files[*i].take() {
                        tracing::warn!(path = %f.path, "code_search incremental embed failed");
                    }
                }
                queue.retain(|(i, _)| !bad.contains(i));
                continue;
            }
        };
        let mut completed: Vec<FileAcc> = Vec::new();
        for ((i, c), v) in batch.into_iter().zip(vectors) {
            let Some(f) = files[i].as_mut() else { continue };
            f.chunks.push(c);
            f.vectors.push(v);
            if f.chunks.len() == f.want
                && let Some(f) = files[i].take()
            {
                completed.push(f);
            }
        }
        if completed.is_empty() {
            continue;
        }
        let commit = runtime.with_index_mut(|index| {
            for f in &mut completed {
                index.remove_file(&f.path);
                let base_id = index.next_id();
                for (k, c) in f.chunks.iter_mut().enumerate() {
                    c.id = base_id + k as u64;
                }
                index.set_next_id(base_id + f.chunks.len() as u64);
                index.add_chunks_with_vectors(
                    std::mem::take(&mut f.chunks),
                    std::mem::take(&mut f.vectors),
                )?;
            }
            index.save(root)
        });
        match commit {
            Ok(()) => {
                for f in &completed {
                    match f.stamp {
                        Some(stamp) => runtime.set_file_stamp(&f.path, stamp),
                        None => runtime.clear_file_stamp(&f.path),
                    }
                    done += f.want as u64;
                }
                // Stamps follow the batch commit (crash ⇒ extra reads only).
                runtime.persist_stamps();
                changed = true;
                update_chunk_progress(root, done.min(total), Some(total));
            }
            Err(e) => {
                tracing::warn!(error = %e, "code_search batch commit failed");
                return Err(e);
            }
        }
    }
    // Failed files keep old chunks; pin to the real index size so done == total.
    let actual = runtime
        .with_index(|index| Ok(index.chunks().len() as u64))
        .unwrap_or(done);
    update_chunk_progress(root, actual, Some(actual));

    if changed {
        if let Err(e) = runtime.resync_bm25() {
            tracing::warn!(error = %e, "code_search BM25 resync after incremental update failed");
        }
        runtime.note_index_activity();
    }
    Ok(())
}

/// Pass-1 in-memory budget for chunk text before falling back to re-chunking.
const PASS1_HOLD_BYTES: usize = 256 * 1024 * 1024;

struct Reindex {
    path: String,
    /// Chunks cut in pass 1, or `None` past the hold budget (re-chunked).
    held: Option<Vec<chunk::ChunkRecord>>,
    n: usize,
    stamp: Option<FileStamp>,
}

struct FileAcc {
    path: String,
    stamp: Option<FileStamp>,
    want: usize,
    chunks: Vec<chunk::ChunkRecord>,
    vectors: Vec<Vec<f32>>,
}

/// Drain `pending_updates` into [`update_files`] (shared by search + periodic sync).
pub fn flush_pending_updates(runtime: &CodeSearchRuntime) {
    let pending: Vec<(String, bool)> = match runtime.pending_updates.lock() {
        Ok(mut guard) => guard.drain().collect(),
        Err(_) => vec![],
    };
    if pending.is_empty() {
        write_code_queue(&runtime.workspace_root, &Default::default());
        return;
    }
    // Single merged pass: one index save + one BM25 resync for the whole batch.
    let _ = update_files(runtime, &pending);
    if let Ok(remaining) = runtime.pending_updates.lock() {
        runtime.persist_queue(&remaining); // WP-131 exact signal
    }
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

    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::index_workspace_blocking;
    use tempfile::TempDir;

    fn runtime_with_hash(root: &Path) -> CodeSearchRuntime {
        let emb = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
        index.save(root).unwrap();
        CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        )
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
}

#[cfg(test)]
mod wp143_tests {
    //! WP-143: first build / rebuild through the incremental path, per-batch
    //! appended commits, crash injection and resume, proven by counters.
    use super::*;
    use crate::engines::code_search::embed::HashEmbedder;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// Hash embedder that counts embedded chunks and can "kill" the process
    /// (panic) when asked for batch number `panic_at` (0-based).
    struct CountingEmbedder {
        chunks: Arc<AtomicUsize>,
        batches: Arc<AtomicUsize>,
        panic_at: Option<usize>,
    }

    impl embed::Embedder for CountingEmbedder {
        fn embedder_id(&self) -> &'static str {
            EMBEDDER_ID_HASH
        }
        fn inference_device(&self) -> &'static str {
            "hash"
        }
        fn embed_batch(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let b = self.batches.fetch_add(1, Ordering::SeqCst);
            if self.panic_at == Some(b) {
                panic!("injected kill at batch {b}");
            }
            self.chunks.fetch_add(texts.len(), Ordering::SeqCst);
            HashEmbedder.embed_batch(texts)
        }
    }

    fn counting(panic_at: Option<usize>) -> (Box<CountingEmbedder>, Arc<AtomicUsize>) {
        let chunks = Arc::new(AtomicUsize::new(0));
        let emb = Box::new(CountingEmbedder {
            chunks: chunks.clone(),
            batches: Arc::new(AtomicUsize::new(0)),
            panic_at,
        });
        (emb, chunks)
    }

    /// `n` one-chunk files plus `big` files of 150 lines (several chunks, so
    /// some files span batch boundaries).
    fn workspace(n: usize, big: usize) -> TempDir {
        let dir = TempDir::new().unwrap();
        for i in 0..n {
            std::fs::write(
                dir.path().join(format!("f{i:04}.rs")),
                format!("fn wp143_f{i}() {{}}\n"),
            )
            .unwrap();
        }
        for i in 0..big {
            let body: String = (0..150)
                .map(|l| format!("fn big{i}_{l}() {{}}\n"))
                .collect();
            std::fs::write(dir.path().join(format!("big{i}.rs")), body).unwrap();
        }
        dir
    }

    fn expected_chunks(root: &Path) -> usize {
        scannable_files(root)
            .unwrap()
            .iter()
            .map(|p| {
                let c = std::fs::read_to_string(root.join(p)).unwrap();
                chunk::chunk_file(p, &c, 1).0.len()
            })
            .sum()
    }

    /// Restart like the worker: warmup load + reconcile + drain.
    fn restart(root: &Path) -> (CodeSearchRuntime, Arc<AtomicUsize>) {
        let (emb, count) = counting(None);
        let mut hash = HashEmbedder;
        let index = warmup_index(root, &mut hash).unwrap();
        let runtime = CodeSearchRuntime::new(root.to_path_buf(), index, Some(emb), None);
        refresh_index_incremental(&runtime).unwrap();
        (runtime, count)
    }

    fn assert_store_consistent(root: &Path) {
        let index = CodeSearchIndex::load(root).unwrap();
        for (id, c) in index.chunks() {
            let v = index
                .get_vector(*id)
                .unwrap_or_else(|| panic!("chunk {id} ({}) has no vector", c.path));
            // The vector belongs to this chunk's text (hash embedder).
            let want = HashEmbedder.embed_one(&c.text).unwrap();
            let cos: f32 = v.iter().zip(&want).map(|(a, b)| a * b).sum();
            assert!(cos > 0.99, "chunk {id} vector does not match its text");
        }
        assert_eq!(index.vector_count(), index.chunks().len(), "orphan vectors");
    }

    /// R1 backstop: a crash in the middle of a compaction's renames leaves
    /// files from two generations; the intersection keeps chunks and vectors
    /// one-to-one and invalidates only the affected files' stamps.
    #[test]
    fn crash_between_compaction_renames_keeps_chunks_and_vectors_one_to_one() {
        for k in 0..5 {
            let dir = workspace(40, 1);
            let root = dir.path();
            let (runtime, _) = {
                let fresh = start_fresh_index(root, EMBEDDER_ID_HASH).unwrap();
                let rt = CodeSearchRuntime::new(
                    root.to_path_buf(),
                    fresh,
                    Some(Box::new(HashEmbedder)),
                    None,
                );
                refresh_index_incremental(&rt).unwrap();
                (rt, ())
            };
            // Change files, then compact with an injected crash after k renames.
            std::fs::write(root.join("f0001.rs"), "fn changed_one() {}\n").unwrap();
            std::fs::remove_file(root.join("f0002.rs")).unwrap();
            std::fs::write(root.join("new.rs"), "fn brand_new() {}\n").unwrap();
            // Enough new files that the compacted chunks file is LONGER than
            // the old checkpoint's chunks_len (the 34k-gap regression).
            let extra: Vec<String> = (0..30).map(|i| format!("more{i:02}.rs")).collect();
            for p in &extra {
                std::fs::write(
                    root.join(p),
                    format!("fn more_{p}() {{}}\n").replace('.', "_"),
                )
                .unwrap();
            }
            runtime
                .with_index_mut(|index| {
                    index.remove_file("f0001.rs");
                    index.remove_file("f0002.rs");
                    for p in std::iter::once("new.rs".to_string()).chain(extra.iter().cloned()) {
                        let body = std::fs::read_to_string(root.join(&p)).unwrap();
                        let (mut c, _) = chunk::chunk_file(&p, &body, 1);
                        let id = index.next_id();
                        c[0].id = id;
                        index.set_next_id(id + 1);
                        let v = HashEmbedder.embed_batch(&[c[0].text.clone()]).unwrap();
                        index.add_chunks_with_vectors(c, v)?;
                    }
                    atomic::FAIL_AFTER_RENAMES.with(|f| f.set(Some(k)));
                    let r = index.save_full(root);
                    atomic::FAIL_AFTER_RENAMES.with(|f| f.set(None));
                    assert!(r.is_err(), "k={k}");
                    Ok(())
                })
                .unwrap();
            drop(runtime);
            assert_store_consistent(root);
            // Resume converges: every scannable file indexed, no extras.
            let (runtime, _) = restart(root);
            let paths = runtime.with_index(|i| Ok(i.indexed_paths())).unwrap();
            assert!(
                paths.contains("new.rs") && paths.contains("f0001.rs"),
                "k={k}"
            );
            assert!(!paths.contains("f0002.rs"), "k={k}");
            assert!(
                paths.contains("more29.rs"),
                "k={k}: tail files kept or re-indexed"
            );
            let n = runtime.with_index(|i| Ok(i.chunks().len())).unwrap();
            assert_eq!(n, expected_chunks(root), "k={k}");
            drop(runtime);
            assert_store_consistent(root);
        }
    }

    #[test]
    fn deleted_file_disappears_from_index_and_reload() {
        let dir = workspace(4, 0);
        let root = dir.path();
        let index = start_fresh_index(root, EMBEDDER_ID_HASH).unwrap();
        let runtime = CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        );
        refresh_index_incremental(&runtime).unwrap();
        assert!(runtime.with_index(|i| Ok(i.has_path("f0001.rs"))).unwrap());

        std::fs::remove_file(root.join("f0001.rs")).unwrap();
        refresh_index_incremental(&runtime).unwrap();

        let (has, paths, chunks, vectors) = runtime
            .with_index(|i| {
                Ok((
                    i.has_path("f0001.rs"),
                    i.indexed_paths(),
                    i.chunks().len(),
                    i.vector_count(),
                ))
            })
            .unwrap();
        assert!(!has, "deleted path must leave the live index");
        assert!(!paths.contains("f0001.rs"));
        assert_eq!(chunks, vectors, "no orphan vectors after a delete");
        drop(runtime);

        // A reload must not resurrect chunks from the checkpoint.
        let reloaded = CodeSearchIndex::load(root).unwrap();
        assert!(!reloaded.has_path("f0001.rs"));
        assert_store_consistent(root);
    }

}