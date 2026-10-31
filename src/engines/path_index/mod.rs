//! In-memory Explorer path catalog for the human `/` mention menu.
//!
//! A background walk fills paths under the same [`FilterPreset::Explorer`]
//! rules as the file tree. Later watcher events insert or delete. The menu
//! reads this catalog and stats only the hits it is about to show.
//! Agent grep and lexical search do not read this catalog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;

use crate::config::path::canon_abs_lossy;
use crate::workspace::filter::{
    FilterPreset, RelPathCtx, cheap_rel_under, path_excluded, path_gitignored, walk_builder,
};
use crate::workspace::glob_hit_key;

const TICK: Duration = Duration::from_millis(500);
const FILL_BATCH: usize = 512;
const MENTION_LIMIT: usize = 8;
const VERIFY_BUDGET: usize = 32;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MentionPath {
    pub path: String,
    pub file: bool,
}

struct Gate {
    root: Option<PathBuf>,
    queue: Vec<(String, bool)>,
}

enum Op {
    Remove(String),
    EnsureDir(String),
    Upsert(String, bool),
}

/// Explorer path catalog. One worker fills it, then applies queued updates.
pub struct PathIndex {
    gate: Mutex<Gate>,
    cv: Condvar,
    paths: RwLock<HashMap<String, bool>>,
    stop: AtomicBool,
    /// Set by the serve loop after it drops filesystem events. The worker
    /// clears the catalog and walks again; the caller does not.
    rescan: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
    spawn_count: AtomicU64,
}

impl Default for PathIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl PathIndex {
    pub fn new() -> Self {
        Self {
            gate: Mutex::new(Gate {
                root: None,
                queue: Vec::new(),
            }),
            cv: Condvar::new(),
            paths: RwLock::new(HashMap::new()),
            stop: AtomicBool::new(false),
            rescan: AtomicBool::new(false),
            worker: Mutex::new(None),
            spawn_count: AtomicU64::new(0),
        }
    }

    /// Start the silent background fill. A second call for the same root
    /// leaves a live worker in place. A worker that has already exited is
    /// started again.
    pub fn attach(self: &Arc<Self>, root: &Path) {
        let root = canon_abs_lossy(root);
        {
            let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            let running = self
                .worker
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .is_some_and(|handle| !handle.is_finished());
            if gate.root.as_ref() == Some(&root) && running {
                return;
            }
        }
        self.detach();
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            gate.root = Some(root.clone());
            gate.queue.clear();
        }
        self.paths
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.stop.store(false, Ordering::SeqCst);
        let engine = Arc::clone(self);
        let handle = thread::Builder::new()
            .name("path-index".into())
            .spawn(move || engine.run())
            .ok();
        if let Some(handle) = handle {
            self.spawn_count.fetch_add(1, Ordering::SeqCst);
            *self.worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        }
    }

    pub fn detach(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.cv.notify_all();
        if let Some(handle) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
        self.stop.store(false, Ordering::SeqCst);
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            gate.root = None;
            gate.queue.clear();
        }
        self.paths
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        if gate.root.is_none() {
            return;
        }
        for path in paths {
            gate.queue.push((path.clone(), deleted));
        }
        self.cv.notify_one();
    }

    /// Ask the worker to drop its catalog and walk the tree again.
    ///
    /// Returns immediately. A caller that lost filesystem events uses this
    /// instead of walking on its own thread.
    pub fn request_rescan(&self) {
        {
            let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if gate.root.is_none() {
                return;
            }
        }
        self.rescan.store(true, Ordering::SeqCst);
        self.cv.notify_one();
    }

    /// Match `**/{query}*` the way the mention menu used to, then stat at most
    /// [`VERIFY_BUDGET`] sorted hits and return up to [`MENTION_LIMIT`].
    /// Missing or newly excluded hits are dropped. Nothing here walks the tree.
    pub fn mention_paths(&self, query: &str) -> Vec<MentionPath> {
        let query = clean_query(query);
        if query.is_empty() || has_dotdot(&query) {
            return Vec::new();
        }
        let root = {
            let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            gate.root.clone()
        };
        let Some(root) = root else {
            return Vec::new();
        };
        let mut hits: Vec<MentionPath> = {
            let paths = self.paths.read().unwrap_or_else(|e| e.into_inner());
            paths
                .iter()
                .filter(|(path, _)| matches_mention(path, &query))
                .map(|(path, file)| MentionPath {
                    path: path.clone(),
                    file: *file,
                })
                .collect()
        };
        hits.sort_by(|a, b| glob_hit_key(&a.path).cmp(&glob_hit_key(&b.path)));
        let mut out = Vec::new();
        let mut checked = 0usize;
        for hit in hits {
            if checked >= VERIFY_BUDGET {
                break;
            }
            checked += 1;
            if let Some(file) = verify_candidate(&root, &hit.path) {
                out.push(MentionPath {
                    path: hit.path,
                    file,
                });
                if out.len() >= MENTION_LIMIT {
                    break;
                }
            }
        }
        out
    }

    fn run(&self) {
        let root = {
            let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            gate.root.clone()
        };
        let Some(root) = root else {
            return;
        };
        self.fill(&root);
        if self.stop.load(Ordering::SeqCst) {
            return;
        }
        // WP-167: one-time migration cleanup, off the startup path. `.litecode`
        // is excluded from the explorer corpus, so running it after the first
        // fill does not change the index.
        remove_legacy_text_index(&root);
        self.flush_pending();
        while !self.stop.load(Ordering::SeqCst) {
            let updates = self.wait_updates();
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            if self.rescan.swap(false, Ordering::SeqCst) {
                self.paths
                    .write()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
                let root = {
                    let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
                    gate.root.clone()
                };
                if let Some(root) = root {
                    self.fill(&root);
                }
            }
            if updates.is_empty() {
                continue;
            }
            let root = {
                let gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
                gate.root.clone()
            };
            if let Some(root) = root {
                self.apply_updates(&root, updates);
            }
        }
    }

    fn fill(&self, root: &Path) {
        let ctx = RelPathCtx::new(root).unwrap_or_else(|_| RelPathCtx::new_lossy(root));
        let walker = walk_builder(root, FilterPreset::Explorer).build();
        let mut batch = Vec::with_capacity(FILL_BATCH);
        let mut logged = false;
        for result in walker {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    if !logged {
                        tracing::warn!(error = %error, "path_index walk skipped an entry");
                        logged = true;
                    }
                    continue;
                }
            };
            let Some(rel) =
                cheap_rel_under(ctx.root_lap(), entry.path()).or_else(|| ctx.rel(entry.path()))
            else {
                continue;
            };
            if rel.is_empty() {
                continue;
            }
            let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
            batch.push((rel, !is_dir));
            if batch.len() >= FILL_BATCH {
                self.insert_batch(&batch);
                batch.clear();
            }
        }
        if !batch.is_empty() {
            self.insert_batch(&batch);
        }
        tracing::debug!("path_index explorer fill finished");
    }

    fn insert_batch(&self, batch: &[(String, bool)]) {
        let mut paths = self.paths.write().unwrap_or_else(|e| e.into_inner());
        for (rel, file) in batch {
            paths.insert(rel.clone(), *file);
        }
    }

    fn wait_updates(&self) -> Vec<(String, bool)> {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        while !self.stop.load(Ordering::SeqCst)
            && gate.queue.is_empty()
            && !self.rescan.load(Ordering::SeqCst)
        {
            let (next, _) = self
                .cv
                .wait_timeout(gate, TICK)
                .unwrap_or_else(|e| e.into_inner());
            gate = next;
        }
        if self.stop.load(Ordering::SeqCst) {
            return Vec::new();
        }
        std::mem::take(&mut gate.queue)
    }

    fn flush_pending(&self) {
        let (root, updates) = {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            let root = gate.root.clone();
            let updates = std::mem::take(&mut gate.queue);
            (root, updates)
        };
        let Some(root) = root else {
            return;
        };
        if updates.is_empty() {
            return;
        }
        self.apply_updates(&root, updates);
    }

    fn apply_updates(&self, root: &Path, updates: Vec<(String, bool)>) {
        let mut ops = Vec::new();
        for (rel, deleted) in updates {
            let rel = normalize_rel(&rel);
            if rel.is_empty() || has_dotdot(&rel) {
                continue;
            }
            if deleted {
                ops.push(Op::Remove(rel));
                continue;
            }
            let Some(file) = classify_visible(root, &rel) else {
                continue;
            };
            let mut ancestors = Vec::new();
            let mut parent = parent_rel(&rel);
            while !parent.is_empty() {
                ancestors.push(parent.to_string());
                parent = parent_rel(parent);
            }
            ancestors.reverse();
            for ancestor in ancestors {
                if explorer_visible(root, &ancestor) {
                    ops.push(Op::EnsureDir(ancestor));
                }
            }
            ops.push(Op::Upsert(rel, file));
        }
        if ops.is_empty() {
            return;
        }
        let mut paths = self.paths.write().unwrap_or_else(|e| e.into_inner());
        for op in ops {
            match op {
                Op::Remove(rel) => remove_prefix(&mut paths, &rel),
                Op::EnsureDir(rel) => {
                    paths.entry(rel).or_insert(false);
                }
                Op::Upsert(rel, file) => {
                    paths.insert(rel, file);
                }
            }
        }
    }
}

fn remove_legacy_text_index(root: &Path) {
    let dir = root.join(".litecode").join("text-index");
    if !dir.is_dir() {
        return;
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => tracing::info!(path = %dir.display(), "removed legacy text-index"),
        Err(error) => {
            tracing::warn!(error = %error, "path_index could not remove legacy text-index");
        }
    }
}

fn explorer_visible(root: &Path, rel: &str) -> bool {
    if rel.is_empty() || has_dotdot(rel) {
        return false;
    }
    if path_excluded(rel, FilterPreset::Explorer) {
        return false;
    }
    !path_gitignored(root, rel, FilterPreset::Explorer)
}

fn classify_visible(root: &Path, rel: &str) -> Option<bool> {
    if !explorer_visible(root, rel) {
        return None;
    }
    let meta = std::fs::metadata(root.join(rel)).ok()?;
    if meta.is_dir() {
        Some(false)
    } else if meta.is_file() {
        Some(true)
    } else {
        None
    }
}

fn verify_candidate(root: &Path, rel: &str) -> Option<bool> {
    let meta = std::fs::metadata(root.join(rel)).ok()?;
    let file = if meta.is_dir() {
        false
    } else if meta.is_file() {
        true
    } else {
        return None;
    };
    if path_excluded(rel, FilterPreset::Explorer) {
        return None;
    }
    if path_gitignored(root, rel, FilterPreset::Explorer) {
        return None;
    }
    Some(file)
}

fn clean_query(query: &str) -> String {
    query
        .trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .chars()
        .filter(|c| !matches!(c, '*' | '?' | '[' | ']'))
        .collect()
}

fn normalize_rel(rel: &str) -> String {
    rel.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

fn has_dotdot(rel: &str) -> bool {
    rel.split('/').any(|part| part == "..")
}

fn parent_rel(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => parent,
        _ => "",
    }
}

fn remove_prefix(paths: &mut HashMap<String, bool>, rel: &str) {
    paths.remove(rel);
    let prefix = format!("{rel}/");
    paths.retain(|path, _| !path.starts_with(&prefix));
}

/// Case-insensitive `**/{query}*` : the query matches at a path boundary and
/// the remainder of the path contains no slash.
fn matches_mention(path: &str, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    if path.is_ascii() && query.is_ascii() {
        return matches_mention_bytes(path.as_bytes(), query.as_bytes());
    }
    let path = path.to_lowercase();
    let query = query.to_lowercase();
    let mut rest = path.as_str();
    loop {
        if let Some(after) = rest.strip_prefix(query.as_str())
            && !after.contains('/')
        {
            return true;
        }
        let Some((_, next)) = rest.split_once('/') else {
            return false;
        };
        if next.is_empty() {
            return false;
        }
        rest = next;
    }
}

fn matches_mention_bytes(path: &[u8], query: &[u8]) -> bool {
    if query.is_empty() {
        return false;
    }
    let mut i = 0usize;
    loop {
        if i + query.len() <= path.len()
            && path[i..i + query.len()].eq_ignore_ascii_case(query)
            && !path[i + query.len()..].contains(&b'/')
        {
            return true;
        }
        let Some(rel) = path[i..].iter().position(|byte| *byte == b'/') else {
            return false;
        };
        i += rel + 1;
        if i >= path.len() {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {}
