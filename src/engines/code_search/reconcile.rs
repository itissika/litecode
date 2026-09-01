//! Disk ↔ index reconcile: only emits dirty signals into `pending_updates`.
//!
//! Digestion stays on the existing watcher path: [`super::flush_pending_updates`]
//! → [`super::update_files`].

use std::collections::HashSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::build::scannable_files;
use super::{CodeSearchRuntime, FileStamp};

/// Discover paths that diverge from the index and queue them as dirty signals.
///
/// - Missing on disk (or no longer indexable) → `(path, deleted=true)`
/// - New or content-changed → `(path, deleted=false)`
///
/// Uses in-memory mtime/len stamps as a fast path after the first successful
/// index of a file; cold stamps fall back to chunk-content comparison.
pub fn queue_reconcile_dirty(runtime: &CodeSearchRuntime) {
    // IndexCold: do not pull the index back into RAM just for reconcile.
    // Watcher pending remains; next search ensure_index + flush digests drift.
    if !runtime.index_is_loaded() {
        return;
    }

    let disk_files = match scannable_files(&runtime.workspace_root) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "code_search reconcile: scan failed");
            return;
        }
    };
    let disk_set: HashSet<String> = disk_files.iter().cloned().collect();

    let indexed = match runtime.with_index(|index| Ok(index.indexed_paths())) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "code_search reconcile: indexed_paths failed");
            return;
        }
    };

    let stamp_map: std::collections::HashMap<String, FileStamp> = match runtime.file_stamps.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => return,
    };

    let mut dirty: Vec<(String, bool)> = Vec::new();
    let mut matched_stamps: Vec<(String, FileStamp)> = Vec::new();

    for path in &indexed {
        if !disk_set.contains(path) {
            dirty.push((path.clone(), true));
        }
    }

    for path in &disk_files {
        let abs = runtime.workspace_root.join(path);
        let Some(stamp) = read_stamp(&abs) else {
            continue;
        };

        if !indexed.contains(path) {
            dirty.push((path.clone(), false));
            continue;
        }

        if stamp_map.get(path) == Some(&stamp) {
            continue;
        }

        let content = match fs::read_to_string(&abs) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search reconcile: read failed");
                continue;
            }
        };

        let matches = match runtime
            .with_index(|index| Ok(index.file_content_matches(path, &content)))
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "code_search reconcile: compare failed");
                continue;
            }
        };

        if matches {
            matched_stamps.push((path.clone(), stamp));
        } else {
            dirty.push((path.clone(), false));
        }
    }

    if dirty.is_empty() && matched_stamps.is_empty() {
        return;
    }

    if let Ok(mut stamps) = runtime.file_stamps.lock() {
        for (path, _) in dirty.iter().filter(|(_, deleted)| *deleted) {
            stamps.remove(path);
        }
        for (path, stamp) in matched_stamps {
            stamps.insert(path, stamp);
        }
    }

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
        drop(pending);
        super::write_pending_hint(&runtime.workspace_root, pending_len);
    }
}

/// Reconcile dirty signals then digest via the shared flush/update path.
pub fn sync_index_with_disk(runtime: &CodeSearchRuntime) {
    queue_reconcile_dirty(runtime);
    super::flush_pending_updates(runtime);
}

pub fn read_stamp(abs: &std::path::Path) -> Option<FileStamp> {
    let meta = fs::metadata(abs).ok()?;
    let modified = meta.modified().ok()?;
    let mtime_ms = modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
        .or_else(|| {
            SystemTime::now()
                .duration_since(modified)
                .ok()
                .map(|d| u64::MAX.saturating_sub(d.as_millis() as u64))
        })?;
    Some(FileStamp {
        mtime_ms,
        len: meta.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::build::build_full_index;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::{CodeSearchRuntime, flush_pending_updates};
    use tempfile::TempDir;

    #[test]
    fn reconcile_queues_new_modified_and_deleted() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        let runtime = CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        );
        // Seed stamps as if we had just indexed.
        sync_index_with_disk(&runtime);
        assert!(runtime.pending_updates.lock().unwrap().is_empty());

        std::fs::write(root.join("a.rs"), "fn a_changed() {}\n").unwrap();
        std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();
        std::fs::remove_file(root.join("b.rs")).unwrap();

        queue_reconcile_dirty(&runtime);
        let pending: HashSet<_> = runtime.pending_updates.lock().unwrap().clone();
        assert!(pending.contains(&("a.rs".into(), false)));
        assert!(pending.contains(&("c.rs".into(), false)));
        assert!(pending.contains(&("b.rs".into(), true)));

        flush_pending_updates(&runtime);
        assert!(runtime.pending_updates.lock().unwrap().is_empty());

        let paths = runtime.with_index(|idx| Ok(idx.indexed_paths())).unwrap();
        assert!(paths.contains("a.rs"));
        assert!(paths.contains("c.rs"));
        assert!(!paths.contains("b.rs"));
        let a_ok = runtime
            .with_index(|idx| {
                let content = std::fs::read_to_string(root.join("a.rs")).unwrap();
                Ok(idx.file_content_matches("a.rs", &content))
            })
            .unwrap();
        assert!(a_ok);
    }

    #[test]
    fn stamp_fast_path_skips_unchanged() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        let runtime = CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            None,
            None,
        );
        sync_index_with_disk(&runtime);

        queue_reconcile_dirty(&runtime);
        assert!(
            runtime.pending_updates.lock().unwrap().is_empty(),
            "unchanged files must not be re-queued when stamps match"
        );
    }

    #[test]
    fn index_cold_skips_reconcile_without_reload() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();

        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        index.save(root).unwrap();
        let runtime = CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        );
        runtime.drop_index_for_cool();
        assert!(!runtime.index_is_loaded());

        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        queue_reconcile_dirty(&runtime);
        assert!(
            !runtime.index_is_loaded(),
            "IndexCold reconcile must not reload RAM index"
        );
        assert!(
            runtime.pending_updates.lock().unwrap().is_empty(),
            "IndexCold reconcile must not invent pending from disk scan"
        );
    }
}
