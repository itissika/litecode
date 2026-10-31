//! FS change ingestion for the semantic index (no OS watcher).
//!
//! Serve owns the sole `notify` watcher; the worker receives paths via IPC
//! `notify_fs_changes` and queues them here.

use crate::workspace::filter::should_queue_index_update;

use super::SharedRuntime;

/// Queue workspace-relative paths into `pending_updates` after Search gates.
pub fn queue_fs_changes(runtime: &SharedRuntime, paths: &[String], deleted: bool) {
    let Ok(guard) = runtime.read() else {
        return;
    };
    let Some(rt) = guard.as_ref() else {
        return;
    };
    let root = rt.workspace_root.clone();
    let filtered: Vec<String> = paths
        .iter()
        .filter(|p| should_queue_index_update(&root, p, deleted))
        .cloned()
        .collect();
    if filtered.is_empty() {
        return;
    }
    if let Ok(mut pending) = rt.pending_updates.lock() {
        for path in filtered {
            pending.insert((path, deleted));
        }
        rt.persist_queue(&pending); // WP-131 exact signal
    }
}

#[cfg(test)]
mod tests {}
