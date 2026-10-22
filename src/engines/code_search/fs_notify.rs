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
mod tests {
    use super::*;
    use crate::engines::code_search::CodeSearchRuntime;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::index_workspace_blocking;
    use std::sync::{Arc, RwLock};
    use tempfile::TempDir;

    fn runtime_at(root: &std::path::Path) -> SharedRuntime {
        let emb = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
        Arc::new(RwLock::new(Some(CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
            None,
        ))))
    }

    #[test]
    fn queue_skips_litecode_and_skip_dirs() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let runtime = runtime_at(root);

        queue_fs_changes(
            &runtime,
            &[
                ".litecode/index/x".into(),
                "node_modules/x.js".into(),
                "src/ok.rs".into(),
            ],
            false,
        );
        let pending: Vec<_> = runtime
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .pending_updates
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "src/ok.rs");
    }
}
