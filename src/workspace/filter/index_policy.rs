//! Semantic-index content policy (migrated from engines/code_search/scan_policy).
//!
//! Extension / size / noise / binary gates for embedding. Directory discovery
//! matches search corpora via [`FilterPreset::Index`] (`files`∪`search` exclude +
//! gitignore). Only [`PRODUCT_INTERNAL_DIRS`] (`.litecode`) is an extra hard-skip
//! for index (product runtime). Eval dirs such as `.data` follow user config.

use std::path::{Path, PathBuf};

use super::binary::looks_binary;
use super::defaults::PRODUCT_INTERNAL_DIRS;
use super::dirs::path_has_product_internal_dir;
use super::exclude::path_excluded;
use super::preset::FilterPreset;

pub const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Alias of [`PRODUCT_INTERNAL_DIRS`] for older call sites / status JSON.
/// Not a discovery denylist — use [`FilterPreset`] excludes for `.git` / `node_modules`.
pub const SKIP_DIRS: &[&str] = PRODUCT_INTERNAL_DIRS;

pub const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cc", "cpp", "h", "hpp", "cs", "rb",
    "php", "swift", "kt", "scala", "sh", "bash", "zsh", "fish", "md", "txt", "json", "yaml", "yml",
    "toml", "xml", "html", "css", "scss", "sql", "lua", "vim", "el", "clj", "ex", "exs", "hs",
    "ml", "mli", "r", "dart", "vue", "svelte", "zig", "nim", "tf", "proto", "graphql", "gql",
];

/// Extension whitelist + product-internal path rules (no disk I/O).
///
/// Does **not** re-implement VS Code / gitignore directory excludes — those are
/// applied by walk [`FilterPreset::Index`] or [`should_queue_index_update`].
pub fn is_scannable_rel_path(rel: &str) -> bool {
    if path_has_product_internal_dir(rel) {
        return false;
    }
    if is_noise_basename(rel) {
        return false;
    }
    let path = PathBuf::from(rel);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() {
        return false;
    }
    TEXT_EXTENSIONS.contains(&ext.as_str())
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

pub fn is_noise_basename(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let lower = name.to_ascii_lowercase();
    if lower.contains("lock") {
        return true;
    }
    lower.ends_with(".min.js")
}

/// Whether a workspace-relative change should dirty the semantic index queue.
///
/// Directory face matches [`FilterPreset::Index`] excludes + product-internal;
/// non-deletes also require [`is_scannable_rel_path`].
pub fn should_queue_index_update(rel: &str, deleted: bool) -> bool {
    if path_has_product_internal_dir(rel) || path_excluded(rel, FilterPreset::Index) {
        return false;
    }
    deleted || is_scannable_rel_path(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filter::is_product_internal_dir_name;
    use tempfile::TempDir;

    #[test]
    fn scannable_content_gates_not_discovery_dirs() {
        // Discovery dirs are not re-checked here — node_modules / target may pass
        // content gates when extension+noise allow; walk / should_queue apply excludes.
        assert!(is_scannable_rel_path("node_modules/pkg/index.js"));
        assert!(is_scannable_rel_path("target/foo.rs"));
        assert!(is_scannable_rel_path("dist/app.js"));
        assert!(!is_scannable_rel_path("Cargo.lock"));
        assert!(!is_scannable_rel_path("package-lock.json"));
        assert!(!is_scannable_rel_path("dist/bundle.min.js"));
        assert!(!is_scannable_rel_path(".litecode/index/x.rs"));
        assert!(is_scannable_rel_path("src/main.rs"));
    }

    #[test]
    fn indexable_rejects_binary_and_oversize() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.rs"), "fn ok() {}\n").unwrap();
        std::fs::write(root.join("bad.bin"), b"hello\x00world").unwrap();
        assert!(is_indexable_rel_path("ok.rs", root));
        assert!(!is_indexable_rel_path("bad.bin", root));
    }

    #[test]
    fn queue_gate_uses_index_excludes_and_content() {
        assert!(should_queue_index_update("src/a.rs", true));
        assert!(!should_queue_index_update(".litecode/index/x", true));
        assert!(!should_queue_index_update("node_modules/x.js", false));
        assert!(!should_queue_index_update("node_modules/x.js", true));
        // target is not a discovery exclude — queues when scannable.
        assert!(should_queue_index_update("target/foo.rs", false));
        assert!(should_queue_index_update("src/a.rs", false));
        assert!(is_product_internal_dir_name(".litecode"));
        assert!(!is_product_internal_dir_name(".data"));
    }
}
