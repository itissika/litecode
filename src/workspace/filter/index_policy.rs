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
    
    use tempfile::TempDir;

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

}
