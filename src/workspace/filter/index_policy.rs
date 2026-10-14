//! Index content policy for the code semantic index.
//!
//! Directory discovery uses [`FilterPreset::CodeIndex`] (`files_exclude` +
//! gitignore, not `search_exclude`). [`PRODUCT_INTERNAL_DIRS`] (`.litecode`)
//! and the dependency dirs in [`super::defaults::CODE_INDEX_SKIP_DIRS`] are
//! hard skips. Which files are embedded is the suffix allowlist
//! `code_index_extensions`. Binary / size stay physical embed gates.

use std::path::Path;

use super::binary::looks_binary;
use super::defaults::PRODUCT_INTERNAL_DIRS;
use super::dirs::path_has_product_internal_dir;
use super::exclude::path_excluded;
use super::gitignore::path_gitignored;
use super::preset::FilterPreset;
use super::workspace_excludes::active_workspace_excludes;

pub const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Alias of [`PRODUCT_INTERNAL_DIRS`] for older call sites / status JSON.
/// Not a discovery denylist — use [`FilterPreset`] excludes for `.git` / `node_modules`.
pub const SKIP_DIRS: &[&str] = PRODUCT_INTERNAL_DIRS;

/// Path-only gate: product-internal trees never enter the index (no disk I/O).
///
/// Does **not** re-implement VS Code / gitignore directory excludes — those are
/// applied by walk [`FilterPreset::CodeIndex`] or [`should_queue_index_update`].
pub fn is_scannable_rel_path(rel: &str) -> bool {
    !path_has_product_internal_dir(rel)
}

/// Last filename suffix (`types.d.ts` → `ts`). Extensionless and dotfiles
/// with an empty stem (`.gitignore`) are not allowlisted.
pub fn file_suffix(rel: &str) -> Option<&str> {
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext)
}

/// Whether `rel` is on the active code-index suffix allowlist.
/// An empty list allows nothing.
pub fn code_index_extension_allowed(rel: &str) -> bool {
    let Some(ext) = file_suffix(rel) else {
        return false;
    };
    active_workspace_excludes()
        .code_index_extensions
        .iter()
        .any(|e| e.eq_ignore_ascii_case(ext))
}

/// Full indexability check including the suffix allowlist, size, and binary sniff.
pub fn is_indexable_rel_path(rel: &str, workspace_root: &Path) -> bool {
    if !is_scannable_rel_path(rel) || !code_index_extension_allowed(rel) {
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

/// Whether a workspace-relative change should dirty the code-index queue.
///
/// Code-index face: `files_exclude` + gitignore + suffix allowlist + the
/// dependency-dir hard skip. `search_exclude` is not consulted. Excluded or
/// disallowed paths still queue **deletes** so stale chunks drop.
pub fn should_queue_index_update(workspace_root: &Path, rel: &str, deleted: bool) -> bool {
    if path_has_product_internal_dir(rel) {
        return false;
    }
    if path_excluded(rel, FilterPreset::CodeIndex)
        || path_gitignored(workspace_root, rel, FilterPreset::CodeIndex)
    {
        return deleted;
    }
    if !code_index_extension_allowed(rel) {
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
        std::fs::write(root.join("notes.md"), "# n\n").unwrap();
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            assert!(is_indexable_rel_path("ok.rs", root));
            assert!(!is_indexable_rel_path("LICENSE", root));
            assert!(!is_indexable_rel_path("notes.md", root));
            assert!(!is_indexable_rel_path("bad.bin", root));
        });
    }

    #[test]
    fn queue_gate_uses_code_index_face() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let dir = TempDir::new().unwrap();
            let root = dir.path();
            assert!(should_queue_index_update(root, "src/a.rs", true));
            assert!(!should_queue_index_update(root, ".litecode/index/x", true));
            assert!(!should_queue_index_update(root, "node_modules/x.js", false));
            // Excluded path: delete still queues so stale chunks drop.
            assert!(should_queue_index_update(root, "node_modules/x.js", true));
            // target is not a code-index directory skip — `.rs` queues when not gitignored.
            assert!(should_queue_index_update(root, "target/foo.rs", false));
            assert!(should_queue_index_update(root, "src/a.rs", false));
            assert!(!should_queue_index_update(root, "Cargo.lock", false));
            assert!(!should_queue_index_update(root, "LICENSE", false));
            assert!(should_queue_index_update(root, "Cargo.lock", true));
            assert!(should_queue_index_update(root, "LICENSE", true));
            assert!(is_product_internal_dir_name(".litecode"));
            assert!(!is_product_internal_dir_name(".data"));
        });
    }

    #[test]
    fn search_exclude_does_not_gate_code_index_queue() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let mut cfg = WorkspaceExcludesFile::builtin_defaults();
        cfg.search_exclude.push("**/*.rs".into());
        cfg.search_exclude.push("src/write_lock.rs".into());

        with_excludes_cache_for_test(cfg, || {
            assert!(should_queue_index_update(root, "src/write_lock.rs", false));
            assert!(!should_queue_index_update(root, "notes.md", false));
            assert!(should_queue_index_update(root, "notes.md", true));
            assert!(!should_queue_index_update(root, "data.json", false));
        });
    }

    #[test]
    fn extension_allowlist_matches_last_suffix_and_empty_blocks_all() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            assert!(code_index_extension_allowed("Button.tsx"));
            assert!(code_index_extension_allowed("types.d.ts"));
            assert!(code_index_extension_allowed("src/main.RS"));
            assert!(!code_index_extension_allowed("Makefile"));
            assert!(!code_index_extension_allowed("Dockerfile"));
            assert!(!code_index_extension_allowed(".gitignore"));
            assert!(!code_index_extension_allowed("notes.md"));
            assert!(!code_index_extension_allowed("Cargo.lock"));
        });

        let mut empty = WorkspaceExcludesFile::builtin_defaults();
        empty.code_index_extensions.clear();
        with_excludes_cache_for_test(empty, || {
            assert!(!code_index_extension_allowed("src/main.rs"));
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
                "code index queue must honor gitignore"
            );
            assert!(
                should_queue_index_update(root, "target/foo.rs", true),
                "gitignore-excluded delete still queues to drop stale index"
            );
            assert!(should_queue_index_update(root, "src.rs", false));
        });
    }
}
