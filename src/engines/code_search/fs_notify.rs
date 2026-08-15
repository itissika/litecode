//! FS change ingestion for the semantic index (no OS watcher).
//!
//! Serve owns the sole `notify` watcher; the worker receives paths via IPC
//! `notify_fs_changes` and queues them here.

use crate::workspace::filter::should_queue_index_update;

use super::{SharedRuntime, write_pending_hint};

/// Queue workspace-relative paths into `pending_updates` after Index gates.
pub fn queue_fs_changes(runtime: &SharedRuntime, paths: &[String], deleted: bool) {
    let filtered: Vec<String> = paths
        .iter()
        .filter(|p| should_queue_index_update(p, deleted))
        .cloned()
        .collect();
    if filtered.is_empty() {
        return;
    }
    if let Ok(guard) = runtime.read()
        && let Some(rt) = guard.as_ref()
        && let Ok(mut pending) = rt.pending_updates.lock()
    {
        for path in filtered {
            pending.insert((path, deleted));
        }
        write_pending_hint(&rt.workspace_root, pending.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::CodeSearchRuntime;
    use crate::engines::code_search::build::build_full_index;
    use crate::engines::code_search::embed::HashEmbedder;
    use std::sync::{Arc, RwLock};
    use tempfile::TempDir;

    fn runtime_at(root: &std::path::Path) -> SharedRuntime {
        let mut emb = HashEmbedder;
        let index = build_full_index(root, &mut emb).unwrap();
        Arc::new(RwLock::new(Some(CodeSearchRuntime::new(
            root.to_path_buf(),
            index,
            Some(Box::new(HashEmbedder)),
        ))))
    }

    #[test]
    fn queue_scannable_path() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        let runtime = runtime_at(root);

        queue_fs_changes(&runtime, &["b.rs".into()], false);
        let pending = runtime
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .pending_updates
            .lock()
            .unwrap()
            .clone();
        assert!(pending.contains(&("b.rs".into(), false)));
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
