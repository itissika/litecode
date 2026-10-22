//! Composed directory basename sets — single ownership for shared consumers.
//!
//! Discovery segments come from VS Code files∪search globs; product internal and
//! snapshot-only lists are declared in [`super::defaults`]. Callers compose via
//! these helpers instead of hand-copying denylists.

use std::collections::HashSet;
use std::sync::LazyLock;

use super::defaults::{FILES_EXCLUDE, PRODUCT_INTERNAL_DIRS, SEARCH_EXCLUDE, SNAPSHOT_ONLY_DIRS};
use super::exclude::segment_name_from_exclude_glob;
use super::workspace_excludes::active_workspace_excludes;

fn discovery_basenames_from_static() -> HashSet<String> {
    let mut out = HashSet::new();
    for g in FILES_EXCLUDE.iter().chain(SEARCH_EXCLUDE.iter()) {
        if let Some(name) = segment_name_from_exclude_glob(g) {
            out.insert(name);
        }
    }
    out
}

static SNAPSHOT_DIR_BASENAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut set = discovery_basenames_from_static();
    for d in PRODUCT_INTERNAL_DIRS {
        set.insert((*d).to_string());
    }
    for d in SNAPSHOT_ONLY_DIRS {
        set.insert((*d).to_string());
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort_unstable();
    v
});

/// Segment-class directory basenames from the active workspace files∪search lists.
pub fn discovery_exclude_dir_basenames() -> HashSet<String> {
    let cfg = active_workspace_excludes();
    let mut out = HashSet::new();
    for g in cfg.files_exclude.iter().chain(cfg.search_exclude.iter()) {
        if let Some(name) = segment_name_from_exclude_glob(g) {
            out.insert(name);
        }
    }
    out
}

/// Sorted basenames for snapshot `$GIT_DIR/info/exclude` and path filters:
/// discovery ∪ [`PRODUCT_INTERNAL_DIRS`] ∪ [`SNAPSHOT_ONLY_DIRS`].
pub fn snapshot_exclude_dir_basenames() -> &'static [String] {
    &SNAPSHOT_DIR_BASENAMES
}

pub fn is_product_internal_dir_name(name: &str) -> bool {
    PRODUCT_INTERNAL_DIRS.contains(&name)
}

pub fn path_has_product_internal_dir(rel: &str) -> bool {
    rel.split('/').any(is_product_internal_dir_name)
}

/// Agent `grep` / `glob` may read this workspace-relative path.
/// It is `.litecode`, `.litecode/knowledge`, or a file under that folder.
/// `foo/.litecode` is not this path.
pub fn is_agent_knowledge_rel(rel: &str) -> bool {
    let rel = rel.trim_matches('/');
    rel == ".litecode" || rel == ".litecode/knowledge" || rel.starts_with(".litecode/knowledge/")
}

/// True when `name` is a discovery segment or product-internal dir (LSP / shallow walks).
pub fn is_discovery_or_product_dir_name(name: &str) -> bool {
    is_product_internal_dir_name(name) || discovery_exclude_dir_basenames().contains(name)
}
