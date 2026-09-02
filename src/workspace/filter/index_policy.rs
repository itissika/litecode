//! Index content policy (semantic + shared Search-queue gates).
//!
//! Directory discovery matches search corpora via [`FilterPreset::Search`]
//! (`files`∪`search` exclude + gitignore). Only [`PRODUCT_INTERNAL_DIRS`]
//! (`.litecode`) is an extra hard-skip. Binary / size are physical embed
//! gates, not a second exclude list — lockfiles and generated text belong in
//! `search.exclude` if you do not want grep or semantic to see them.

use std::path::Path;

use super::binary::looks_binary;
use super::defaults::PRODUCT_INTERNAL_DIRS;
use super::dirs::path_has_product_internal_dir;
use super::exclude::path_excluded;
use super::gitignore::path_gitignored;
use super::preset::FilterPreset;

pub const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Alias of [`PRODUCT_INTERNAL_DIRS`] for older call sites / status JSON.
/// Not a discovery denylist — use [`FilterPreset`] excludes for `.git` / `node_modules`.
pub const SKIP_DIRS: &[&str] = PRODUCT_INTERNAL_DIRS;

/// Path-only gate: product-internal trees never enter the index (no disk I/O).
///
/// Does **not** re-implement VS Code / gitignore directory excludes — those are
/// applied by walk [`FilterPreset::Search`] or [`should_queue_index_update`].
pub fn is_scannable_rel_path(rel: &str) -> bool {
    !path_has_product_internal_dir(rel)
}

/// Full indexability check including size and binary sniff (disk I/O).
pub fn is_indexable_rel_path(rel: &str, workspace_root: &Path) -> bool {
    if !is_scannable_rel_path(rel) {
        return false;
    }
    let abs = workspace_root.join(rel);
    if !abs.is_file() {
        return false;
    }
    let Ok(meta) = std::fs::metadata(&abs) else {
        return false;
    };
    if meta.len() > MAX_INDEX_FILE_BYTES {
        return false;
    }
    !looks_binary(&abs)
}

/// True when any path segment is product-internal (legacy name kept for callers).
pub fn path_has_skipped_dir(rel: &str) -> bool {
    path_has_product_internal_dir(rel)
}

/// Whether a workspace-relative change should dirty the Search-corpus index queue.
///
/// Same face as the text index: glob + gitignore + product-internal. Excluded
/// paths still queue **deletes** so stale chunks / postings drop.
pub fn should_queue_index_update(workspace_root: &Path, rel: &str, deleted: bool) -> bool {
    if path_has_product_internal_dir(rel) {
        return false;
    }
    if path_excluded(rel, FilterPreset::Search)
        || path_gitignored(workspace_root, rel, FilterPreset::Search)
    {
        return deleted;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filter::is_product_internal_dir_name;
    use tempfile::TempDir;

    #[test]
    fn scannable_is_not_a_language_list() {
        // Discovery dirs are not re-checked here — walk / should_queue apply excludes.
        assert!(is_scannable_rel_path("node_modules/pkg/index.js"));
        assert!(is_scannable_rel_path("target/foo.rs"));
        assert!(is_scannable_rel_path("dist/app.js"));
        assert!(is_scannable_rel_path("Cargo.lock"));
        assert!(is_scannable_rel_path("package-lock.json"));
        assert!(is_scannable_rel_path("dist/bundle.min.js"));
        assert!(is_scannable_rel_path("LICENSE"));
        assert!(is_scannable_rel_path("scripts/serve_win.ps1"));
        assert!(!is_scannable_rel_path(".litecode/index/x.rs"));
        assert!(is_scannable_rel_path("src/main.rs"));
    }

    #[test]
    fn indexable_rejects_binary_and_oversize() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.rs"), "fn ok() {}\n").unwrap();
        std::fs::write(root.join("bad.bin"), b"hello\x00world").unwrap();
        std::fs::write(root.join("LICENSE"), "MIT\n").unwrap();
        assert!(is_indexable_rel_path("ok.rs", root));
        assert!(is_indexable_rel_path("LICENSE", root));
        assert!(!is_indexable_rel_path("bad.bin", root));
    }

    #[test]
    fn queue_gate_uses_search_excludes() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        assert!(should_queue_index_update(root, "src/a.rs", true));
        assert!(!should_queue_index_update(root, ".litecode/index/x", true));
        assert!(!should_queue_index_update(root, "node_modules/x.js", false));
        // Excluded path: delete still queues so stale chunks drop.
        assert!(should_queue_index_update(root, "node_modules/x.js", true));
        // target is not a Search glob exclude — queues when not gitignored.
        assert!(should_queue_index_update(root, "target/foo.rs", false));
        assert!(should_queue_index_update(root, "src/a.rs", false));
        assert!(should_queue_index_update(root, "Cargo.lock", false));
        assert!(should_queue_index_update(root, "LICENSE", false));
        assert!(is_product_internal_dir_name(".litecode"));
        assert!(!is_product_internal_dir_name(".data"));
    }

    #[test]
    fn search_exclude_drops_lockfile_from_queue() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut cfg = WorkspaceExcludesFile::builtin_defaults();
        cfg.search_exclude.push("Cargo.lock".into());
        cfg.search_exclude.push("package-lock.json".into());

        with_excludes_cache_for_test(cfg, || {
            assert!(!should_queue_index_update(root, "Cargo.lock", false));
            assert!(should_queue_index_update(root, "Cargo.lock", true));
            assert!(!should_queue_index_update(
                root,
                "web/package-lock.json",
                false
            ));
            assert!(should_queue_index_update(root, "src/write_lock.rs", false));
        });
    }

    #[test]
    fn gitignore_target_does_not_queue_create() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/foo.rs"), "fn t() {}\n").unwrap();
        std::fs::write(root.join("src.rs"), "fn s() {}\n").unwrap();

        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            assert!(
                !should_queue_index_update(root, "target/foo.rs", false),
                "Search queue must honor gitignore"
            );
            assert!(
                should_queue_index_update(root, "target/foo.rs", true),
                "gitignore-excluded delete still queues to drop stale index"
            );
            assert!(should_queue_index_update(root, "src.rs", false));
        });
    }
}
