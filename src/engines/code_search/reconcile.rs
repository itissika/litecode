//! Disk ↔ index reconcile: only emits dirty signals into `pending_updates`.
//!
//! Digestion is consume-only (`refresh_index_incremental` /
//! [`super::flush_pending_updates`] → [`super::update_files`]).
//!
//! WP-141: the periodic scan is the source of truth (the watcher only makes
//! changes show up sooner). It only stats: a path whose mtime/len match its
//! stamp is skipped without being read; any mismatch is queued and
//! `update_files` compares content (identical ⇒ stamp refresh, no embed).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::build::scan_stat_candidates;
use super::{CodeSearchRuntime, FileStamp, stamps};

/// What the scan compares the disk against.
struct Known {
    /// Every path the index has accounted for (indexed or stamped).
    paths: HashSet<String>,
    stamps: HashMap<String, FileStamp>,
}

fn known_set(runtime: &CodeSearchRuntime) -> Option<Known> {
    let mem: HashMap<String, FileStamp> = runtime.file_stamps.lock().ok()?.clone();
    if runtime.index_is_loaded() {
        let indexed = runtime
            .with_index(|index| Ok(index.indexed_paths()))
            .map_err(|e| tracing::warn!(error = %e, "code_search reconcile: indexed_paths failed"))
            .ok()?;
        let mut paths = indexed;
        paths.extend(mem.keys().cloned());
        return Some(Known { paths, stamps: mem });
    }
    // IndexCold: never pull the index back into RAM for a scan. The persisted
    // stamp keys are the index's path set; without them the scan cannot tell
    // deletions, so it defers to a full reconcile on the next load.
    let Some(disk) = stamps::load(&runtime.workspace_root) else {
        runtime.request_full_reconcile_on_load();
        return None;
    };
    let mut paths: HashSet<String> = disk.keys().cloned().collect();
    paths.extend(mem.keys().cloned());
    let mut merged: HashMap<String, FileStamp> = disk
        .into_iter()
        .filter_map(|(p, s)| s.map(|s| (p, s)))
        .collect();
    merged.extend(mem);
    Some(Known {
        paths,
        stamps: merged,
    })
}

/// Pure diff (stat results in, dirty signals out). `disk` = scannable paths
/// with their current stamp (`None` = stat failed ⇒ queued, update decides).
pub(crate) fn diff_against_stamps(
    disk: &[(String, Option<FileStamp>)],
    known_paths: &HashSet<String>,
    stamps: &HashMap<String, FileStamp>,
) -> Vec<(String, bool)> {
    let disk_set: HashSet<&str> = disk.iter().map(|(p, _)| p.as_str()).collect();
    let mut dirty: Vec<(String, bool)> = known_paths
        .iter()
        .filter(|p| !disk_set.contains(p.as_str()))
        .map(|p| (p.clone(), true))
        .collect();
    for (path, stamp) in disk {
        match (stamp, stamps.get(path)) {
            (Some(now), Some(was)) if now == was => {}
            _ => dirty.push((path.clone(), false)),
        }
    }
    dirty.sort();
    dirty
}

/// Discover paths that diverge from the index and queue them as dirty signals.
///
/// - Known to the index, gone from disk (or no longer indexable) → `(path, true)`
/// - New, or mtime/len differ from the stamp → `(path, false)`
///
/// Runs whether or not the index is loaded (WP-141): a cooled index scans
/// against `stamps.v1` and its changes wait in the queue for the next load.
pub fn queue_reconcile_dirty(runtime: &CodeSearchRuntime) {
    let disk = match scan_stat_candidates(&runtime.workspace_root) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "code_search reconcile: scan failed");
            return;
        }
    };
    let Some(known) = known_set(runtime) else {
        return;
    };
    let dirty = diff_against_stamps(&disk, &known.paths, &known.stamps);
    if dirty.is_empty() {
        return;
    }

    let queued_del = dirty.iter().filter(|(_, d)| *d).count();
    let queued_mod = dirty.len() - queued_del;
    if let Ok(mut pending) = runtime.pending_updates.lock() {
        for item in dirty {
            pending.insert(item);
        }
        let pending_len = pending.len();
        tracing::info!(
            deleted = queued_del,
            modified = queued_mod,
            pending = pending_len,
            "code_search reconcile queued dirty paths"
        );
        runtime.persist_queue(&pending); // WP-131 exact signal
    }
}

/// Serve-process scan when no worker is alive (WP-141 NotWarm / WorkerDown).
/// Pure-stat against `stamps.v1`; writes `queue.json` only. Does not load the
/// index or embed. Missing/invalid stamps ⇒ no-op (full reconcile waits for
/// the next load). An empty dirty set clears a stale count-only queue.
pub fn queue_reconcile_from_stamps(workspace_root: &Path) {
    let Some(entries) = stamps::load(workspace_root) else {
        return;
    };
    let disk = match scan_stat_candidates(workspace_root) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "code_search serve reconcile: scan failed");
            return;
        }
    };
    let known_paths: HashSet<String> = entries.keys().cloned().collect();
    let stamp_map: HashMap<String, FileStamp> = entries
        .into_iter()
        .filter_map(|(p, s)| s.map(|s| (p, s)))
        .collect();
    let dirty = diff_against_stamps(&disk, &known_paths, &stamp_map);
    let pending: HashSet<(String, bool)> = dirty.into_iter().collect();
    if pending.is_empty() {
        // Tree matches stamps: drop a stale count-only queue left by a dead worker.
        super::write_code_queue(workspace_root, &HashSet::new());
        return;
    }
    let queued_del = pending.iter().filter(|(_, d)| *d).count();
    tracing::info!(
        deleted = queued_del,
        modified = pending.len() - queued_del,
        pending = pending.len(),
        "code_search serve reconcile queued dirty paths"
    );
    super::write_code_queue(workspace_root, &pending);
}

/// Reconcile dirty signals then digest via the shared flush/update path.
pub fn sync_index_with_disk(runtime: &CodeSearchRuntime) {
    queue_reconcile_dirty(runtime);
    super::flush_pending_updates(runtime);
}

pub fn read_stamp(abs: &std::path::Path) -> Option<FileStamp> {
    stamp_from_meta(&fs::metadata(abs).ok()?)
}

pub(crate) fn stamp_from_meta(meta: &fs::Metadata) -> Option<FileStamp> {
    let modified = meta.modified().ok()?;
    let mtime_ns = modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as u64)
        .or_else(|| {
            SystemTime::now()
                .duration_since(modified)
                .ok()
                .map(|d| u64::MAX.saturating_sub(d.as_nanos() as u64))
        })?;
    Some(FileStamp {
        mtime_ns,
        len: meta.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::index_workspace_blocking;
    use crate::engines::code_search::{CodeSearchRuntime, flush_pending_updates};
    use tempfile::TempDir;

    fn saved_runtime(
        root: &std::path::Path,
        emb: Box<dyn crate::engines::code_search::Embedder>,
    ) -> CodeSearchRuntime {
        let e = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(e)).unwrap();
        index.save(root).unwrap();
        CodeSearchRuntime::new(root.to_path_buf(), index, Some(emb), None)
    }

    fn reopen(
        root: &std::path::Path,
        emb: Box<dyn crate::engines::code_search::Embedder>,
    ) -> CodeSearchRuntime {
        let index = crate::engines::code_search::CodeSearchIndex::load(root).unwrap();
        CodeSearchRuntime::new(root.to_path_buf(), index, Some(emb), None)
    }

    fn pending(rt: &CodeSearchRuntime) -> HashSet<(String, bool)> {
        rt.pending_updates.lock().unwrap().clone()
    }

    /// Counts embed calls (proves "touch ⇒ no re-embed").
    struct CountingEmbedder(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl crate::engines::code_search::Embedder for CountingEmbedder {
        fn embedder_id(&self) -> &'static str {
            HashEmbedder.embedder_id()
        }
        fn inference_device(&self) -> &'static str {
            "hash"
        }
        fn embed_batch(&mut self, texts: &[String]) -> crate::types::Result<Vec<Vec<f32>>> {
            self.0
                .fetch_add(texts.len(), std::sync::atomic::Ordering::SeqCst);
            HashEmbedder.embed_batch(texts)
        }
    }

    // Crash between index commit and stamp write: meta.json moved on, stamps
    // did not, so stamps are discarded and every path is re-checked by
    // content; only the really changed file is embedded (more reads, never a
    // missed change).
    #[test]
    fn crash_between_index_commit_and_stamps_rereads_never_misses() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for f in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.join(f), format!("fn {}() {{}}\n", &f[..1])).unwrap();
        }
        drop(saved_runtime(root, Box::new(HashEmbedder)));
        std::fs::write(root.join("b.rs"), "fn b_after_crash() {}\n").unwrap();
        let mut meta = crate::engines::code_search::read_meta(root)
            .unwrap()
            .unwrap();
        meta.created_at = "2099-01-01T00:00:00Z".into();
        crate::engines::code_search::write_meta(root, &meta).unwrap();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let rt = reopen(root, Box::new(CountingEmbedder(calls.clone())));
        assert!(
            !stamps::stamps_path(root).exists(),
            "mismatched stamps discarded"
        );
        queue_reconcile_dirty(&rt);
        assert_eq!(pending(&rt).len(), 3, "no stamps: every path re-checked");
        flush_pending_updates(&rt);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "only b.rs (1 chunk) re-embedded"
        );
        assert!(
            stamps::stamps_path(root).is_file(),
            "stamps rewritten after commit"
        );
        queue_reconcile_dirty(&rt);
        assert!(pending(&rt).is_empty());
    }
}
