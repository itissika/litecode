//! Adaptive workspace text index (Cox-style trigram accelerator).
//!
//! Own artifact under `.litecode/text-index/`, separate from semantic ANN/BM25.
//! Shares serve watcher notifications. Index narrows files; libripgrep verifies
//! with the query's current exclude preset. Falls back to a full walk only when
//! the index cannot be used (off, unindexable pattern, Unfiltered, candidate cap).
//! Ignore-rule / excludes changes reconcile the tracked path set (delta add/delete);
//! a full rebuild happens only on first build, open failure, or a huge delta.

mod literal;
mod meta;
mod policy;
mod store;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::engines::code_search::{LexicalQuery, LexicalSearchOutcome};
use crate::types::Result;
use crate::workspace::filter::FilterPreset;

pub use policy::{TextIndexMode, mode_from_env};

use meta::{INDEX_FORMAT, TextIndexMeta, load_meta, save_meta};
use policy::{
    HARD_FILE_CAP, corpus_delta, corpus_fingerprint, delta_prefers_rebuild,
    is_corpus_definition_rel, should_build, should_queue_text_path,
};
use store::{CandidateHits, TextIndexStore, count_search_files, list_search_paths};

const PENDING_DEBOUNCE: Duration = Duration::from_millis(300);

/// Process-wide registry so LexicalLane can find the active index by workspace root
/// without threading IdeBase through every grep call.
static REGISTRY: OnceLock<RwLock<Option<Registered>>> = OnceLock::new();

struct Registered {
    root: PathBuf,
    engine: Arc<TextIndexEngine>,
}

fn registry() -> &'static RwLock<Option<Registered>> {
    REGISTRY.get_or_init(|| RwLock::new(None))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIndexState {
    Off,
    Measuring,
    Building,
    Ready,
    Failed,
}

struct Inner {
    root: Option<PathBuf>,
    state: TextIndexState,
    store: Option<TextIndexStore>,
    pending: HashSet<(String, bool)>,
    file_count: u64,
    last_error: Option<String>,
    reconcile_requested: bool,
    build_gen: u64,
}

/// Adaptive text-search accelerator owned by [`crate::engines::WorkspaceEngines`].
pub struct TextIndexEngine {
    inner: Mutex<Inner>,
    cv: Condvar,
    stop: AtomicBool,
    /// Background measure/build + pending apply thread (at most one).
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Default for TextIndexEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextIndexEngine {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                root: None,
                state: TextIndexState::Off,
                store: None,
                pending: HashSet::new(),
                file_count: 0,
                last_error: None,
                reconcile_requested: false,
                build_gen: 0,
            }),
            cv: Condvar::new(),
            stop: AtomicBool::new(false),
            worker: Mutex::new(None),
        }
    }

    pub fn state(&self) -> TextIndexState {
        self.inner
            .lock()
            .map(|g| g.state)
            .unwrap_or(TextIndexState::Off)
    }

    /// Bind to a workspace root and schedule auto measure/build (non-blocking).
    pub fn attach_workspace(self: &Arc<Self>, root: &Path) {
        let root = crate::config::path::canon_abs_lossy(root);
        {
            let mut g = self.inner.lock().unwrap();
            g.root = Some(root.clone());
            g.state = TextIndexState::Measuring;
            g.last_error = None;
        }
        {
            let mut reg = registry().write().unwrap();
            *reg = Some(Registered {
                root: root.clone(),
                engine: Arc::clone(self),
            });
        }
        self.spawn_lifecycle(root);
    }

    pub fn detach(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.cv.notify_all();
        if let Ok(mut w) = self.worker.lock()
            && let Some(h) = w.take()
        {
            let _ = h.join();
        }
        self.stop.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.inner.lock() {
            g.store = None;
            g.state = TextIndexState::Off;
            g.root = None;
            g.pending.clear();
            g.reconcile_requested = false;
        }
        if let Ok(mut reg) = registry().write() {
            *reg = None;
        }
    }

    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) {
        let Ok(mut g) = self.inner.lock() else {
            return;
        };
        if matches!(g.state, TextIndexState::Off | TextIndexState::Failed) {
            return;
        }
        let root = g.root.clone();
        let mut wake = false;
        for p in paths {
            if is_corpus_definition_rel(p) {
                g.reconcile_requested = true;
                wake = true;
                continue;
            }
            let Some(root) = root.as_ref() else {
                continue;
            };
            if should_queue_text_path(root, p, deleted) {
                g.pending.insert((p.clone(), deleted));
                wake = true;
            }
        }
        if wake {
            self.cv.notify_one();
        }
    }

    pub fn request_reconcile(&self) {
        if let Ok(mut g) = self.inner.lock() {
            if matches!(g.state, TextIndexState::Off | TextIndexState::Failed) {
                return;
            }
            g.reconcile_requested = true;
            self.cv.notify_one();
        }
    }

    /// Try index-accelerated search. `None` means caller must use libripgrep.
    pub fn try_search(
        &self,
        query: &LexicalQuery,
        preset: FilterPreset,
    ) -> Option<Result<LexicalSearchOutcome>> {
        if preset == FilterPreset::Unfiltered {
            return None;
        }
        if mode_from_env() == TextIndexMode::Off {
            return None;
        }
        let _ = self.flush_pending();
        let (root, store) = {
            let g = self.inner.lock().ok()?;
            if g.state != TextIndexState::Ready {
                return None;
            }
            let root = g.root.as_ref()?.clone();
            let store = g.store.as_ref()?.clone_reader().ok()?;
            (root, store)
        };

        match store.search_candidates(query) {
            Ok(None) => None, // pattern not indexable
            Ok(Some(hits)) => {
                if !accelerator_window_complete(&hits) {
                    tracing::debug!("text_index candidate cap hit; falling back to ripgrep");
                    return None;
                }
                Some(verify_candidates(&root, query, preset, &hits.paths))
            }
            Err(e) => {
                tracing::warn!(error = %e, "text_index query failed; falling back to ripgrep");
                None
            }
        }
    }

    fn flush_pending(&self) -> Result<()> {
        let (root, updates, store) = {
            let mut g = self.inner.lock().unwrap();
            if g.reconcile_requested {
                g.reconcile_requested = false;
                let root = g.root.clone();
                drop(g);
                if let Some(root) = root {
                    self.reconcile_sync(&root)?;
                }
                return Ok(());
            }
            if g.pending.is_empty() || g.state != TextIndexState::Ready {
                return Ok(());
            }
            let root = g
                .root
                .clone()
                .ok_or_else(|| crate::types::LitecodeError::Config("text_index: no root".into()))?;
            let updates: Vec<_> = g.pending.drain().collect();
            let store = g.store.take();
            (root, updates, store)
        };
        let Some(mut store) = store else {
            return Ok(());
        };
        if let Err(e) = store.apply_updates(&root, &updates) {
            tracing::warn!(error = %e, "text_index incremental update failed");
            if let Ok(mut g) = self.inner.lock() {
                g.store = Some(store);
            }
            return Ok(());
        }
        self.persist_meta(&root, &store);
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
        }
        Ok(())
    }

    /// Walk current Search paths, diff against the tracked set, apply
    /// add/delete. Rebuild only when the store is missing or the delta is huge.
    fn reconcile_sync(&self, root: &Path) -> Result<()> {
        let ready = self
            .inner
            .lock()
            .ok()
            .map(|g| g.state == TextIndexState::Ready && g.store.is_some())
            .unwrap_or(false);
        if !ready {
            return self.rebuild_sync(root);
        }
        let want = list_search_paths(root, || self.stop.load(Ordering::SeqCst))?;
        if self.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let Some(mut store) = self.inner.lock().ok().and_then(|mut g| g.store.take()) else {
            return self.rebuild_sync(root);
        };
        let have = store.tracked.clone();
        let updates = corpus_delta(&want, &have);
        let adds = updates.iter().filter(|(_, deleted)| !*deleted).count();
        if delta_prefers_rebuild(adds) {
            tracing::info!(
                adds,
                deletes = updates.len() - adds,
                tracked = have.len(),
                "text_index path-set adds too large; rebuilding"
            );
            drop(store);
            return self.rebuild_sync(root);
        }
        if !updates.is_empty() {
            if let Err(e) = store.apply_updates(root, &updates) {
                tracing::warn!(error = %e, "text_index path-set reconcile failed; rebuilding");
                drop(store);
                return self.rebuild_sync(root);
            }
            tracing::info!(
                delta = updates.len(),
                tracked = store.tracked.len(),
                "text_index path set reconciled"
            );
        }
        self.persist_meta(root, &store);
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
        }
        self.apply_pending_only()
    }

    fn persist_meta(&self, root: &Path, store: &TextIndexStore) {
        let file_count = store.tracked.len() as u64;
        if let Ok(mut g) = self.inner.lock() {
            g.file_count = file_count;
        }
        let mut oversized: Vec<String> = store.oversized.iter().cloned().collect();
        oversized.sort();
        let mut tracked: Vec<String> = store.tracked.iter().cloned().collect();
        tracked.sort();
        let _ = save_meta(
            root,
            &TextIndexMeta {
                format: INDEX_FORMAT,
                workspace_root: root.to_string_lossy().into_owned(),
                file_count,
                built_unix_ms: unix_ms(),
                corpus_fingerprint: corpus_fingerprint(),
                oversized,
                tracked,
            },
        );
    }

    fn spawn_lifecycle(self: &Arc<Self>, root: PathBuf) {
        let mode = mode_from_env();
        if mode == TextIndexMode::Off {
            let mut g = self.inner.lock().unwrap();
            g.state = TextIndexState::Off;
            return;
        }
        let engine = Arc::clone(self);
        let handle = thread::Builder::new()
            .name("text-index".into())
            .spawn(move || {
                engine.measure_and_maybe_build(&root, mode);
                engine.maintain_loop();
            })
            .ok();
        if let Some(h) = handle {
            *self.worker.lock().unwrap() = Some(h);
        }
    }

    fn maintain_loop(&self) {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let mut g = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            while !self.stop.load(Ordering::SeqCst)
                && !g.reconcile_requested
                && g.pending.is_empty()
            {
                let (gg, _) = match self.cv.wait_timeout(g, Duration::from_millis(500)) {
                    Ok(v) => v,
                    Err(_) => return,
                };
                g = gg;
            }
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let has_work = g.reconcile_requested || !g.pending.is_empty();
            drop(g);
            if !has_work {
                continue;
            }
            thread::sleep(PENDING_DEBOUNCE);
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let _ = self.flush_pending();
        }
    }

    fn measure_and_maybe_build(&self, root: &Path, mode: TextIndexMode) {
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        let count = match count_search_files(root, HARD_FILE_CAP + 1) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "text_index file count failed");
                if let Ok(mut g) = self.inner.lock() {
                    g.state = TextIndexState::Failed;
                    g.last_error = Some(e.to_string());
                }
                return;
            }
        };
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        {
            let mut g = self.inner.lock().unwrap();
            g.file_count = count;
        }
        if count > HARD_FILE_CAP {
            tracing::warn!(
                count,
                cap = HARD_FILE_CAP,
                "text_index skipped: workspace exceeds hard file cap"
            );
            if let Ok(mut g) = self.inner.lock() {
                g.state = TextIndexState::Off;
            }
            return;
        }
        if !should_build(mode, count) {
            if let Ok(mut g) = self.inner.lock() {
                g.state = TextIndexState::Off;
            }
            return;
        }
        let fingerprint = corpus_fingerprint();
        if let Ok(Some(meta)) = load_meta(root)
            && meta.format == INDEX_FORMAT
            && meta.workspace_root == root.to_string_lossy()
            && let Ok(store) = TextIndexStore::open(root, meta.oversized, meta.tracked)
        {
            let fingerprint_mismatch = meta.corpus_fingerprint != fingerprint;
            if let Ok(mut g) = self.inner.lock() {
                g.file_count = store.tracked.len() as u64;
                g.store = Some(store);
                g.state = TextIndexState::Ready;
                if fingerprint_mismatch {
                    g.reconcile_requested = true;
                }
            }
            tracing::info!(
                count,
                fingerprint_mismatch,
                "text_index loaded from disk"
            );
            let _ = self.flush_pending();
            return;
        }
        if let Ok(mut g) = self.inner.lock() {
            g.state = TextIndexState::Building;
        }
        if let Err(e) = self.rebuild_sync(root) {
            tracing::warn!(error = %e, "text_index build failed");
            if let Ok(mut g) = self.inner.lock() {
                g.state = TextIndexState::Failed;
                g.last_error = Some(e.to_string());
                g.store = None;
            }
        }
    }

    fn rebuild_sync(&self, root: &Path) -> Result<()> {
        let started = Instant::now();
        tracing::info!(path = %root.display(), "text_index build starting");
        if let Ok(mut g) = self.inner.lock() {
            g.state = TextIndexState::Building;
            g.build_gen += 1;
        }
        let store = TextIndexStore::build(root, || self.stop.load(Ordering::SeqCst))?;
        if self.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.persist_meta(root, &store);
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
            g.state = TextIndexState::Ready;
        }
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "text_index build complete"
        );
        // Apply events that arrived during the build (do not drop them).
        self.apply_pending_only()
    }

    fn apply_pending_only(&self) -> Result<()> {
        let (root, updates, store) = {
            let mut g = self.inner.lock().unwrap();
            if g.pending.is_empty() || g.state != TextIndexState::Ready {
                return Ok(());
            }
            let root = g
                .root
                .clone()
                .ok_or_else(|| crate::types::LitecodeError::Config("text_index: no root".into()))?;
            let updates: Vec<_> = g.pending.drain().collect();
            let store = g.store.take();
            (root, updates, store)
        };
        let Some(mut store) = store else {
            return Ok(());
        };
        if let Err(e) = store.apply_updates(&root, &updates) {
            tracing::warn!(error = %e, "text_index incremental update failed");
            if let Ok(mut g) = self.inner.lock() {
                g.store = Some(store);
            }
            return Ok(());
        }
        self.persist_meta(&root, &store);
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
        }
        Ok(())
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn accelerator_window_complete(hits: &CandidateHits) -> bool {
    !hits.truncated
}

/// Registry lookup used by LexicalLane.
pub fn try_accelerated_search(
    query: &LexicalQuery,
    preset: FilterPreset,
) -> Option<Result<LexicalSearchOutcome>> {
    let reg = registry().read().ok()?;
    let reg = reg.as_ref()?;
    let qroot = crate::config::path::canon_abs_lossy(&query.root);
    if qroot != reg.root && !qroot.starts_with(&reg.root) {
        return None;
    }
    let mut q = query.clone();
    if qroot != reg.root {
        let rel = qroot.strip_prefix(&reg.root).ok()?.to_path_buf();
        q.root = reg.root.clone();
        q.path = Some(match q.path.take() {
            Some(p) if p.is_absolute() => p,
            Some(p) => rel.join(p),
            None => rel,
        });
    }
    // Known-path file grep opens the file directly; skip the global posting set.
    if let Some(p) = q.path.as_ref() {
        let abs = if p.is_absolute() {
            p.clone()
        } else {
            q.root.join(p)
        };
        if abs.is_file() {
            return None;
        }
    }
    reg.engine.try_search(&q, preset)
}

fn verify_candidates(
    workspace_root: &Path,
    query: &LexicalQuery,
    preset: FilterPreset,
    candidates: &[String],
) -> Result<LexicalSearchOutcome> {
    store::verify_with_ripgrep(workspace_root, query, preset, candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::{LexicalQuery, lexical_search_with_preset};
    use crate::workspace::filter::FilterPreset;
    use literal::indexable_literal;
    use std::sync::Arc;
    use store::{TextIndexStore, verify_with_ripgrep};
    use tempfile::TempDir;

    fn sample_query(root: &Path, pattern: &str) -> LexicalQuery {
        LexicalQuery {
            pattern: pattern.into(),
            root: root.to_path_buf(),
            path: None,
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 200,
            before_context: 0,
            after_context: 0,
        }
    }

    fn match_paths(out: &LexicalSearchOutcome) -> Vec<String> {
        let mut p: Vec<String> = out.matches.iter().map(|m| m.path.clone()).collect();
        p.sort();
        p.dedup();
        p
    }

    #[test]
    fn mode_env_defaults_auto() {
        let _ = mode_from_env();
    }

    #[test]
    fn small_workspace_is_eligible_in_auto() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() { hello_world(); }\n").unwrap();
        let count = count_search_files(root, 100).unwrap();
        assert_eq!(count, 1);
        assert!(should_build(TextIndexMode::Auto, count));
    }

    #[test]
    fn force_on_builds_and_finds_literal() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() { unique_needle_xyz(); }\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();

        let store = TextIndexStore::build(root, || false).unwrap();
        let q = sample_query(root, "unique_needle_xyz");
        let hits = store.search_candidates(&q).unwrap().expect("indexable");
        assert!(
            hits.paths.iter().any(|p| p == "a.rs"),
            "{paths:?}",
            paths = hits.paths
        );
        assert!(!hits.truncated);
        let out = verify_with_ripgrep(root, &q, FilterPreset::Search, &hits.paths).unwrap();
        assert_eq!(out.matches.len(), 1);
        assert_eq!(out.matches[0].path, "a.rs");
    }

    fn lock_text_index_registry_for_test() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register_ready_engine(
        root: &Path,
        store: TextIndexStore,
        file_count: u64,
    ) -> (std::sync::MutexGuard<'static, ()>, Arc<TextIndexEngine>) {
        let lock = lock_text_index_registry_for_test();
        let engine = Arc::new(TextIndexEngine::new());
        {
            let mut g = engine.inner.lock().unwrap();
            g.root = Some(crate::config::path::canon_abs_lossy(root));
            g.store = Some(store);
            g.state = TextIndexState::Ready;
            g.file_count = file_count;
        }
        {
            let mut reg = registry().write().unwrap();
            *reg = Some(Registered {
                root: crate::config::path::canon_abs_lossy(root),
                engine: Arc::clone(&engine),
            });
        }
        (lock, engine)
    }

    fn unregister_engine() {
        let mut reg = registry().write().unwrap();
        *reg = None;
    }

    #[test]
    fn indexed_verify_respects_exclude_pattern() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("tests")).unwrap();
        std::fs::write(root.join("main.rs"), "unique_needle_xyz\n").unwrap();
        std::fs::write(root.join("tests/ignored.rs"), "unique_needle_xyz\n").unwrap();

        let store = TextIndexStore::build(root, || false).unwrap();
        let mut q = sample_query(root, "unique_needle_xyz");
        q.exclude = Some("**/tests/**".into());
        q.max_matches = 10;
        let hits = store.search_candidates(&q).unwrap().expect("indexable");
        let out = verify_with_ripgrep(root, &q, FilterPreset::Search, &hits.paths).unwrap();
        assert_eq!(out.matches.len(), 1, "{out:?}");
        assert_eq!(out.matches[0].path, "main.rs");
    }

    #[test]
    fn file_scoped_search_falls_back_when_index_does_not_contain_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("indexed.rs"), "ApplyAppearance in index\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        std::fs::write(root.join("target.rs"), "ApplyAppearance in target\n").unwrap();

        let (_reg, engine) = register_ready_engine(root, store, 1);
        let mut q = sample_query(root, "ApplyAppearance|Addressables|async|LoadAsset");
        q.path = Some(root.join("target.rs"));
        q.is_regex = true;
        q.max_matches = 10;
        let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        drop(engine);
        assert!(
            out.matches.iter().any(|m| m.path.ends_with("target.rs")),
            "file-scoped grep must hit the file even when the text index never saw it: {out:?}"
        );
    }

    #[test]
    fn incremental_notify_finds_file_added_after_build() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("indexed.rs"), "shared_needle_xyz\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        std::fs::write(root.join("late.rs"), "shared_needle_xyz\n").unwrap();

        let (_reg, engine) = register_ready_engine(root, store, 1);
        engine.notify_fs_changes(&["late.rs".into()], false);
        let q = sample_query(root, "shared_needle_xyz");
        let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        drop(engine);
        let paths = match_paths(&out);
        assert!(
            paths.iter().any(|p| p.ends_with("late.rs")),
            "pending flush before search must index the new file: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("indexed.rs")),
            "must keep the already-indexed hit: {paths:?}"
        );
    }

    #[test]
    fn events_during_build_are_applied() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("indexed.rs"), "shared_needle_xyz\n").unwrap();
        let _reg = lock_text_index_registry_for_test();
        let engine = Arc::new(TextIndexEngine::new());
        {
            let mut g = engine.inner.lock().unwrap();
            g.root = Some(crate::config::path::canon_abs_lossy(root));
            g.state = TextIndexState::Measuring;
        }
        std::fs::write(root.join("late.rs"), "shared_needle_xyz\n").unwrap();
        engine.notify_fs_changes(&["late.rs".into()], false);
        engine.rebuild_sync(root).unwrap();
        {
            let mut reg = registry().write().unwrap();
            *reg = Some(Registered {
                root: crate::config::path::canon_abs_lossy(root),
                engine: Arc::clone(&engine),
            });
        }
        let q = sample_query(root, "shared_needle_xyz");
        let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        let paths = match_paths(&out);
        assert!(
            paths.iter().any(|p| p.ends_with("late.rs")),
            "files created during Measuring must survive rebuild: {paths:?}"
        );
    }

    #[test]
    fn posting_intersect_returns_all_files_beyond_old_topk() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // Old collector was TopDocs(2000); 80 files would not catch a regression.
        const N: usize = 2_100;
        for i in 0..N {
            std::fs::write(
                root.join(format!("f{i:04}.rs")),
                "shared_trigram_needle_abc\n",
            )
            .unwrap();
        }
        let store = TextIndexStore::build(root, || false).unwrap();
        let q = sample_query(root, "shared_trigram_needle_abc");
        let capped = store
            .search_candidates_with_limit(&q, 2_000)
            .unwrap()
            .expect("indexable");
        assert!(
            capped.truncated,
            "2_000 of {N} must still look like the old Top-K cap: {capped:?}"
        );
        let hits = store.search_candidates(&q).unwrap().expect("indexable");
        assert!(!hits.truncated);
        assert_eq!(hits.paths.len(), N, "{} paths", hits.paths.len());
        let mut q = q;
        q.max_matches = N;
        let out = verify_with_ripgrep(root, &q, FilterPreset::Search, &hits.paths).unwrap();
        assert_eq!(out.matches.len(), N);
    }

    #[test]
    fn truncated_candidate_window_is_flagged() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for i in 0..6 {
            std::fs::write(root.join(format!("f{i}.rs")), "shared_trigram_needle_abc\n").unwrap();
        }
        let store = TextIndexStore::build(root, || false).unwrap();
        let q = sample_query(root, "shared_trigram_needle_abc");
        let hits = store
            .search_candidates_with_limit(&q, 3)
            .unwrap()
            .expect("indexable");
        assert!(
            hits.truncated,
            "limit 3 of 6 files must flag truncation: {hits:?}"
        );
        assert!(hits.paths.len() <= 3);
        assert!(!accelerator_window_complete(&hits));
    }

    #[test]
    fn accelerator_matches_scan_including_hidden() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("vis.rs"), "parity_needle_xyz\n").unwrap();
        std::fs::write(root.join(".env"), "parity_needle_xyz\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        let (_reg, engine) = register_ready_engine(root, store, 2);
        let q = sample_query(root, "parity_needle_xyz");

        let acc = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        engine.detach();
        let scan = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        assert_eq!(match_paths(&acc), match_paths(&scan));
        assert!(
            match_paths(&acc).iter().any(|p| p.ends_with(".env")),
            "Search must include un-ignored hidden files: {:?}",
            match_paths(&acc)
        );
        unregister_engine();
    }

    #[test]
    fn oversized_file_is_still_verified() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("small.rs"), "oversize_needle_xyz in small\n").unwrap();
        let big = "oversize_needle_xyz\n".repeat(200_000);
        assert!(big.len() as u64 > policy::MAX_INDEX_FILE_BYTES);
        std::fs::write(root.join("huge.rs"), &big).unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        assert!(
            store.oversized.iter().any(|p| p.ends_with("huge.rs")),
            "oversized sidecar: {:?}",
            store.oversized
        );
        let (_reg, engine) = register_ready_engine(root, store, 2);
        let q = sample_query(root, "oversize_needle_xyz");
        let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        drop(engine);
        let paths = match_paths(&out);
        assert!(paths.iter().any(|p| p.ends_with("huge.rs")), "{paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("small.rs")), "{paths:?}");
    }

    #[test]
    fn unfiltered_preset_skips_accelerator() {
        let engine = Arc::new(TextIndexEngine::new());
        let dir = TempDir::new().unwrap();
        let mut q = sample_query(dir.path(), "abcdef");
        q.is_regex = true;
        q.max_matches = 10;
        assert!(engine.try_search(&q, FilterPreset::Unfiltered).is_none());
    }

    #[test]
    fn notify_gitignore_requests_reconcile_not_file_update() {
        let engine = Arc::new(TextIndexEngine::new());
        {
            let mut g = engine.inner.lock().unwrap();
            g.state = TextIndexState::Ready;
        }
        engine.notify_fs_changes(&[".gitignore".into()], false);
        let g = engine.inner.lock().unwrap();
        assert!(g.reconcile_requested);
        assert!(g.pending.is_empty());
    }

    fn git_root_with_two_needles(root: &Path) {
        // ignore crate only honors .gitignore when a git marker exists.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join("visible.rs"), "gitignore_needle_xyz\n").unwrap();
        std::fs::write(root.join("skip_me.rs"), "gitignore_needle_xyz\n").unwrap();
    }

    #[test]
    fn accelerator_gitignore_tighten_matches_scan() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        git_root_with_two_needles(root);
        std::fs::write(root.join(".gitignore"), "\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        assert!(store.tracked.iter().any(|p| p.ends_with("skip_me.rs")));

        let (_reg, engine) = register_ready_engine(root, store, 2);
        std::fs::write(root.join(".gitignore"), "skip_me.rs\n").unwrap();
        engine.notify_fs_changes(&[".gitignore".into()], false);
        let q = sample_query(root, "gitignore_needle_xyz");
        let acc = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        engine.detach();
        let scan = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        assert_eq!(match_paths(&acc), match_paths(&scan));
        let paths = match_paths(&acc);
        assert!(paths.iter().any(|p| p.ends_with("visible.rs")), "{paths:?}");
        assert!(
            !paths.iter().any(|p| p.ends_with("skip_me.rs")),
            "tightened gitignore must drop skip_me from accelerator and scan: {paths:?}"
        );
    }

    #[test]
    fn accelerator_gitignore_loosen_matches_scan() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        git_root_with_two_needles(root);
        std::fs::write(root.join(".gitignore"), "skip_me.rs\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        assert!(
            !store.tracked.iter().any(|p| p.ends_with("skip_me.rs")),
            "build must omit gitignored skip_me: {:?}",
            store.tracked
        );

        let (_reg, engine) = register_ready_engine(root, store, 1);
        std::fs::write(root.join(".gitignore"), "\n").unwrap();
        engine.notify_fs_changes(&[".gitignore".into()], false);
        let q = sample_query(root, "gitignore_needle_xyz");
        let acc = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        engine.detach();
        let scan = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        assert_eq!(match_paths(&acc), match_paths(&scan));
        let paths = match_paths(&acc);
        assert!(paths.iter().any(|p| p.ends_with("visible.rs")), "{paths:?}");
        assert!(
            paths.iter().any(|p| p.ends_with("skip_me.rs")),
            "loosened gitignore must add skip_me via reconcile: {paths:?}"
        );
    }

    #[test]
    fn unnotified_new_file_misses_until_notify() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("indexed.rs"), "shared_needle_xyz\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        std::fs::write(root.join("late.rs"), "shared_needle_xyz\n").unwrap();

        let (_reg, engine) = register_ready_engine(root, store, 1);
        let q = sample_query(root, "shared_needle_xyz");
        let stale = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        let stale_paths = match_paths(&stale);
        assert!(
            stale_paths.iter().any(|p| p.ends_with("indexed.rs")),
            "{stale_paths:?}"
        );
        assert!(
            !stale_paths.iter().any(|p| p.ends_with("late.rs")),
            "without notify, accelerator must not invent a scan fallback: {stale_paths:?}"
        );

        engine.notify_fs_changes(&["late.rs".into()], false);
        let flushed = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        engine.detach();
        let scan = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
        unregister_engine();
        assert_eq!(match_paths(&flushed), match_paths(&scan));
        assert!(
            match_paths(&flushed)
                .iter()
                .any(|p| p.ends_with("late.rs")),
            "notify then flush must restore scan parity: {:?}",
            match_paths(&flushed)
        );
    }

    #[test]
    fn list_search_paths_honors_search_exclude() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("src.rs"), "x\n").unwrap();
        std::fs::write(root.join("vendor/lib.rs"), "x\n").unwrap();

        crate::workspace::filter::with_excludes_cache_for_test(
            crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
            || {
                let all = list_search_paths(root, || false).unwrap();
                assert!(all.iter().any(|p| p.ends_with("src.rs")), "{all:?}");
                assert!(all.iter().any(|p| p.contains("vendor")), "{all:?}");
            },
        );

        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("**/vendor".into());
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            let tight = list_search_paths(root, || false).unwrap();
            assert!(tight.iter().any(|p| p.ends_with("src.rs")), "{tight:?}");
            assert!(
                !tight.iter().any(|p| p.contains("vendor")),
                "tightened search_exclude must drop vendor from path listing: {tight:?}"
            );
        });
    }

    #[test]
    fn corpus_reconcile_tightens_without_full_rebuild() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("src.rs"), "cfg_needle_xyz\n").unwrap();
        std::fs::write(root.join("vendor/lib.rs"), "cfg_needle_xyz\n").unwrap();
        let store = {
            let mut built = None;
            crate::workspace::filter::with_excludes_cache_for_test(
                crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
                || {
                    built = Some(TextIndexStore::build(root, || false).unwrap());
                },
            );
            built.unwrap()
        };
        assert!(store.tracked.iter().any(|p| p.contains("vendor")));

        let (_reg, engine) = register_ready_engine(root, store, 2);
        let gen_before = engine.inner.lock().unwrap().build_gen;
        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("**/vendor".into());
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            engine.request_reconcile();
            let q = sample_query(root, "cfg_needle_xyz");
            let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
            let paths = match_paths(&out);
            assert!(paths.iter().any(|p| p.ends_with("src.rs")), "{paths:?}");
            assert!(
                !paths.iter().any(|p| p.contains("vendor")),
                "reconcile must drop vendor from the corpus: {paths:?}"
            );
        });
        {
            let g = engine.inner.lock().unwrap();
            assert_eq!(g.build_gen, gen_before, "small delta must not rebuild");
            let tracked = &g.store.as_ref().unwrap().tracked;
            assert!(tracked.iter().any(|p| p.ends_with("src.rs")), "{tracked:?}");
            assert!(
                !tracked.iter().any(|p| p.contains("vendor")),
                "tracked set must drop vendor: {tracked:?}"
            );
        }
        unregister_engine();
        drop(engine);
    }

    #[test]
    fn corpus_reconcile_loosens_adds_newly_visible_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("src.rs"), "cfg_needle_xyz\n").unwrap();
        std::fs::write(root.join("vendor/lib.rs"), "cfg_needle_xyz\n").unwrap();

        let store = {
            let mut built = None;
            let mut tight = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
            tight.search_exclude.push("**/vendor".into());
            crate::workspace::filter::with_excludes_cache_for_test(tight, || {
                built = Some(TextIndexStore::build(root, || false).unwrap());
            });
            built.unwrap()
        };
        assert!(
            !store.tracked.iter().any(|p| p.contains("vendor")),
            "build under tight excludes must omit vendor: {:?}",
            store.tracked
        );

        let (_reg, engine) = register_ready_engine(root, store, 1);
        let gen_before = engine.inner.lock().unwrap().build_gen;
        crate::workspace::filter::with_excludes_cache_for_test(
            crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
            || {
                engine.request_reconcile();
                let q = sample_query(root, "cfg_needle_xyz");
                let out = lexical_search_with_preset(&q, FilterPreset::Search).unwrap();
                let paths = match_paths(&out);
                assert!(paths.iter().any(|p| p.ends_with("src.rs")), "{paths:?}");
                assert!(
                    paths.iter().any(|p| p.contains("vendor")),
                    "loosened excludes must add vendor via delta, not miss it: {paths:?}"
                );
            },
        );
        {
            let g = engine.inner.lock().unwrap();
            assert_eq!(g.build_gen, gen_before, "small delta must not rebuild");
            let tracked = &g.store.as_ref().unwrap().tracked;
            assert!(
                tracked.iter().any(|p| p.contains("vendor")),
                "tracked set must gain vendor: {tracked:?}"
            );
        }
        unregister_engine();
        drop(engine);
    }

    #[test]
    fn verify_honors_tightened_search_exclude() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("vendor")).unwrap();
        std::fs::write(root.join("src.rs"), "cfg_needle_xyz\n").unwrap();
        std::fs::write(root.join("vendor/lib.rs"), "cfg_needle_xyz\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        let q = sample_query(root, "cfg_needle_xyz");
        let hits = store.search_candidates(&q).unwrap().expect("indexable");
        assert!(hits.paths.iter().any(|p| p.contains("vendor")));

        let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.push("**/vendor".into());
        crate::workspace::filter::with_excludes_cache_for_test(file, || {
            let out = verify_with_ripgrep(root, &q, FilterPreset::Search, &hits.paths).unwrap();
            let paths = match_paths(&out);
            assert!(paths.iter().any(|p| p.ends_with("src.rs")), "{paths:?}");
            assert!(
                !paths.iter().any(|p| p.contains("vendor")),
                "live search_exclude must drop vendor: {paths:?}"
            );
        });
    }

    #[test]
    fn pure_wildcard_regex_not_indexable() {
        assert!(indexable_literal(".*", true).is_none());
    }

    /// Manual workspace bench: `cargo test --lib bench_litecode_workspace_text_index -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_litecode_workspace_text_index() {
        use std::time::Instant;

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let count = count_search_files(&root, HARD_FILE_CAP + 1).unwrap();
        eprintln!("agent_text_file_count={count}");

        let t0 = Instant::now();
        let store = TextIndexStore::build(&root, || false).unwrap();
        eprintln!("build_ms={}", t0.elapsed().as_millis());

        let engine = Arc::new(TextIndexEngine::new());
        {
            let mut g = engine.inner.lock().unwrap();
            g.root = Some(crate::config::path::canon_abs_lossy(&root));
            g.store = Some(store);
            g.state = TextIndexState::Ready;
            g.file_count = count;
        }
        {
            let mut reg = registry().write().unwrap();
            *reg = Some(Registered {
                root: crate::config::path::canon_abs_lossy(&root),
                engine: Arc::clone(&engine),
            });
        }

        let pattern = "lexical_search_with_preset";
        let mk = || sample_query(&root, pattern);

        let _ = lexical_search_with_preset(&mk(), FilterPreset::Search).unwrap();
        let t1 = Instant::now();
        let on = lexical_search_with_preset(&mk(), FilterPreset::Search).unwrap();
        let on_ms = t1.elapsed().as_millis();

        engine.detach();
        let _ = lexical_search_with_preset(&mk(), FilterPreset::Search).unwrap();
        let t2 = Instant::now();
        let off = lexical_search_with_preset(&mk(), FilterPreset::Search).unwrap();
        let off_ms = t2.elapsed().as_millis();

        eprintln!(
            "grep_on_ms={on_ms} hits={} files_searched={}",
            on.matches.len(),
            on.files_searched
        );
        eprintln!(
            "grep_off_ms={off_ms} hits={} files_searched={}",
            off.matches.len(),
            off.files_searched
        );
        assert_eq!(match_paths(&on), match_paths(&off));
    }
}
