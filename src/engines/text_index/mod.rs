//! Adaptive workspace text index (Instant Grep–style accelerator).
//!
//! Own artifact under `.litecode/text-index/`, separate from semantic ANN/BM25.
//! Shares serve watcher notifications only. Always falls back to libripgrep.

mod literal;
mod meta;
mod policy;
mod store;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::engines::code_search::{LexicalQuery, LexicalSearchOutcome};
use crate::types::Result;
use crate::workspace::filter::FilterPreset;

pub use policy::{TextIndexMode, mode_from_env};

use meta::{INDEX_FORMAT, TextIndexMeta, load_meta, save_meta, text_index_dir};
use policy::{BUILD_FILE_THRESHOLD, DROP_FILE_THRESHOLD, HARD_FILE_CAP, should_queue_text_path};
use store::{TextIndexStore, count_agent_text_files};

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
    stop: AtomicBool,
    /// Background measure/build thread (at most one).
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
        self.spawn_measure_and_maybe_build(root);
    }

    pub fn detach(&self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Ok(mut w) = self.worker.lock()
            && let Some(h) = w.take() {
                let _ = h.join();
            }
        self.stop.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.inner.lock() {
            g.store = None;
            g.state = TextIndexState::Off;
            g.root = None;
            g.pending.clear();
        }
        if let Ok(mut reg) = registry().write() {
            *reg = None;
        }
    }

    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) {
        let Ok(mut g) = self.inner.lock() else {
            return;
        };
        if !matches!(g.state, TextIndexState::Ready | TextIndexState::Building) {
            return;
        }
        for p in paths {
            if should_queue_text_path(p, deleted) {
                g.pending.insert((p.clone(), deleted));
            }
        }
    }

    pub fn request_reconcile(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.reconcile_requested = true;
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
        let (root, store) = {
            let g = self.inner.lock().ok()?;
            if g.state != TextIndexState::Ready {
                return None;
            }
            let root = g.root.as_ref()?.clone();
            let store = g.store.as_ref()?.clone_reader().ok()?;
            (root, store)
        };

        // Flush pending before query (best-effort).
        let _ = self.flush_pending();

        match store.search_candidates(query) {
            Ok(None) => None, // pattern not indexable
            Ok(Some(paths)) => Some(verify_candidates(&root, query, preset, &paths)),
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
                g.pending.clear();
                let root = g.root.clone();
                drop(g);
                if let Some(root) = root {
                    self.rebuild_sync(&root)?;
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
            // Put store back if still usable
            if let Ok(mut g) = self.inner.lock() {
                g.store = Some(store);
            }
            return Ok(());
        }
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
        }
        Ok(())
    }

    fn spawn_measure_and_maybe_build(self: &Arc<Self>, root: PathBuf) {
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
                if engine.stop.load(Ordering::SeqCst) {
                    return;
                }
                let count = match count_agent_text_files(&root, HARD_FILE_CAP + 1) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(error = %e, "text_index file count failed");
                        if let Ok(mut g) = engine.inner.lock() {
                            g.state = TextIndexState::Failed;
                            g.last_error = Some(e.to_string());
                        }
                        return;
                    }
                };
                if engine.stop.load(Ordering::SeqCst) {
                    return;
                }
                {
                    let mut g = engine.inner.lock().unwrap();
                    g.file_count = count;
                }
                let should_build = match mode {
                    TextIndexMode::On => count <= HARD_FILE_CAP,
                    TextIndexMode::Auto => (BUILD_FILE_THRESHOLD..=HARD_FILE_CAP).contains(&count),
                    TextIndexMode::Off => false,
                };
                if count > HARD_FILE_CAP {
                    tracing::warn!(
                        count,
                        cap = HARD_FILE_CAP,
                        "text_index skipped: workspace exceeds hard file cap"
                    );
                    if let Ok(mut g) = engine.inner.lock() {
                        g.state = TextIndexState::Off;
                    }
                    return;
                }
                if !should_build {
                    // Drop existing on-disk index if below hysteresis while in auto.
                    if mode == TextIndexMode::Auto && count < DROP_FILE_THRESHOLD {
                        let dir = text_index_dir(&root);
                        let _ = std::fs::remove_dir_all(&dir);
                    }
                    if let Ok(mut g) = engine.inner.lock() {
                        g.state = TextIndexState::Off;
                    }
                    tracing::debug!(count, "text_index not needed for workspace size");
                    return;
                }
                // Try load existing meta if compatible.
                if let Ok(Some(meta)) = load_meta(&root)
                    && meta.format == INDEX_FORMAT
                    && meta.workspace_root == root.to_string_lossy()
                    && let Ok(store) = TextIndexStore::open(&root)
                {
                    if let Ok(mut g) = engine.inner.lock() {
                        g.store = Some(store);
                        g.state = TextIndexState::Ready;
                    }
                    tracing::info!(count, "text_index loaded from disk");
                    return;
                }
                if let Ok(mut g) = engine.inner.lock() {
                    g.state = TextIndexState::Building;
                    g.build_gen += 1;
                }
                if let Err(e) = engine.rebuild_sync(&root) {
                    tracing::warn!(error = %e, "text_index build failed");
                    if let Ok(mut g) = engine.inner.lock() {
                        g.state = TextIndexState::Failed;
                        g.last_error = Some(e.to_string());
                        g.store = None;
                    }
                }
            })
            .ok();
        if let Some(h) = handle {
            *self.worker.lock().unwrap() = Some(h);
        }
    }

    fn rebuild_sync(&self, root: &Path) -> Result<()> {
        let started = Instant::now();
        tracing::info!(path = %root.display(), "text_index build starting");
        let store = TextIndexStore::build(root, || self.stop.load(Ordering::SeqCst))?;
        if self.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let count = {
            let g = self.inner.lock().unwrap();
            g.file_count
        };
        save_meta(
            root,
            &TextIndexMeta {
                format: INDEX_FORMAT,
                workspace_root: root.to_string_lossy().into_owned(),
                file_count: count,
                built_unix_ms: unix_ms(),
            },
        )?;
        if let Ok(mut g) = self.inner.lock() {
            g.store = Some(store);
            g.state = TextIndexState::Ready;
            g.pending.clear();
        }
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "text_index build complete"
        );
        Ok(())
    }
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
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
        // Preserve deeper path scope if already set.
        q.path = Some(match q.path.take() {
            Some(p) if p.is_absolute() => p,
            Some(p) => rel.join(p),
            None => rel,
        });
    }
    // Single-file grep must not go through the global TopDocs candidate window:
    // a known file can miss the 2000-hit cap (or never have been indexed) and
    // we'd return empty without falling back to libripgrep.
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
    match reg.engine.try_search(&q, preset) {
        Some(Ok(out)) if out.matches.is_empty() && out.files_searched == 0 && q.path.is_some() => {
            None
        }
        other => other,
    }
}

fn verify_candidates(
    workspace_root: &Path,
    query: &LexicalQuery,
    preset: FilterPreset,
    candidates: &[String],
) -> Result<LexicalSearchOutcome> {
    // Reuse libripgrep verify on the candidate set by temporarily scoping —
    // walk only listed files via a synthetic path list search.
    store::verify_with_ripgrep(workspace_root, query, preset, candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::LexicalQuery;
    use crate::workspace::filter::FilterPreset;
    use literal::indexable_literal;
    use std::sync::Arc;
    use store::{TextIndexStore, verify_with_ripgrep};
    use tempfile::TempDir;

    #[test]
    fn mode_env_defaults_auto() {
        let _ = mode_from_env();
    }

    #[test]
    fn small_workspace_stays_off_in_auto() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() { hello_world(); }\n").unwrap();
        assert!(BUILD_FILE_THRESHOLD > 1);
        let count = count_agent_text_files(root, 100).unwrap();
        assert_eq!(count, 1);
        assert!(count < BUILD_FILE_THRESHOLD);
    }

    #[test]
    fn force_on_builds_and_finds_literal() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() { unique_needle_xyz(); }\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();

        // Build synchronously without env race.
        let store = TextIndexStore::build(root, || false).unwrap();
        let q = LexicalQuery {
            pattern: "unique_needle_xyz".into(),
            root: root.to_path_buf(),
            path: None,
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 10,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        };
        let paths = store.search_candidates(&q).unwrap().expect("indexable");
        assert!(paths.iter().any(|p| p == "a.rs"), "{paths:?}");
        let out = verify_with_ripgrep(root, &q, FilterPreset::AgentText, &paths).unwrap();
        assert_eq!(out.matches.len(), 1);
        assert_eq!(out.matches[0].path, "a.rs");
    }

    #[test]
    fn file_scoped_search_falls_back_when_index_does_not_contain_file() {
        use crate::engines::code_search::lexical_search_with_preset;

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("indexed.rs"), "ApplyAppearance in index\n").unwrap();
        let store = TextIndexStore::build(root, || false).unwrap();
        std::fs::write(root.join("target.rs"), "ApplyAppearance in target\n").unwrap();

        let engine = Arc::new(TextIndexEngine::new());
        {
            let mut g = engine.inner.lock().unwrap();
            g.root = Some(crate::config::path::canon_abs_lossy(root));
            g.store = Some(store);
            g.state = TextIndexState::Ready;
            g.file_count = 1;
        }
        {
            let mut reg = registry().write().unwrap();
            *reg = Some(Registered {
                root: crate::config::path::canon_abs_lossy(root),
                engine: Arc::clone(&engine),
            });
        }

        let q = LexicalQuery {
            pattern: "ApplyAppearance|Addressables|async|LoadAsset".into(),
            root: root.to_path_buf(),
            path: Some(root.join("target.rs")),
            case_sensitive: false,
            whole_word: false,
            is_regex: true,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 10,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        };
        let out = lexical_search_with_preset(&q, FilterPreset::AgentText).unwrap();
        {
            let mut reg = registry().write().unwrap();
            *reg = None;
        }
        assert!(
            out.matches.iter().any(|m| m.path.ends_with("target.rs")),
            "file-scoped grep must hit the file even when the text index never saw it: {out:?}"
        );
    }

    #[test]
    fn unfiltered_preset_skips_accelerator() {
        let engine = Arc::new(TextIndexEngine::new());
        let dir = TempDir::new().unwrap();
        let q = LexicalQuery {
            pattern: "abcdef".into(),
            root: dir.path().to_path_buf(),
            path: None,
            case_sensitive: false,
            whole_word: false,
            is_regex: true,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 10,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        };
        assert!(engine.try_search(&q, FilterPreset::Unfiltered).is_none());
    }

    #[test]
    fn pure_wildcard_regex_not_indexable() {
        assert!(indexable_literal(".*", true).is_none());
    }

    /// Manual workspace bench: `cargo test --lib bench_litecode_workspace_text_index -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_litecode_workspace_text_index() {
        use crate::engines::code_search::{LexicalQuery, lexical_search_with_preset};
        use std::time::Instant;

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let count = count_agent_text_files(&root, HARD_FILE_CAP + 1).unwrap();
        eprintln!("agent_text_file_count={count}");

        let t0 = Instant::now();
        let store = TextIndexStore::build(&root, || false).unwrap();
        eprintln!("build_ms={}", t0.elapsed().as_millis());

        // Register a Ready engine so lexical_search_with_preset can hit the accelerator.
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

        let pattern = "lexical_search_with_preset"; // exists in this repo
        let mk = || LexicalQuery {
            pattern: pattern.into(),
            root: root.clone(),
            path: None,
            case_sensitive: false,
            whole_word: false,
            is_regex: false,
            include: None,
            exclude: None,
            multiline: false,
            max_matches: 50,
            before_context: 0,
            after_context: 0,
            search_hidden: false,
        };

        // Warm + measure ON (index path)
        let _ = lexical_search_with_preset(&mk(), FilterPreset::AgentText).unwrap();
        let t1 = Instant::now();
        let on = lexical_search_with_preset(&mk(), FilterPreset::AgentText).unwrap();
        let on_ms = t1.elapsed().as_millis();

        // OFF: detach registry → pure ripgrep
        engine.detach();
        let _ = lexical_search_with_preset(&mk(), FilterPreset::AgentText).unwrap();
        let t2 = Instant::now();
        let off = lexical_search_with_preset(&mk(), FilterPreset::AgentText).unwrap();
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
        assert_eq!(on.matches.len(), off.matches.len());
    }
}
