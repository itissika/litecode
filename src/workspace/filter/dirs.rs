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

/// True when `name` is a discovery segment or product-internal dir (LSP / shallow walks).
pub fn is_discovery_or_product_dir_name(name: &str) -> bool {
    is_product_internal_dir_name(name) || discovery_exclude_dir_basenames().contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_includes_git_and_node_modules() {
        let d = discovery_exclude_dir_basenames();
        assert!(d.contains(".git"));
        assert!(d.contains("node_modules"));
        assert!(d.contains("bower_components"));
        assert!(!d.contains("target"));
        assert!(!d.contains(".litecode"));
    }

    #[test]
    fn snapshot_composes_discovery_product_and_only() {
        let s = snapshot_exclude_dir_basenames();
        assert!(s.iter().any(|x| x == ".git"));
        assert!(s.iter().any(|x| x == "node_modules"));
        assert!(s.iter().any(|x| x == ".litecode"));
        assert!(s.iter().any(|x| x == "Library"));
        assert!(s.iter().any(|x| x == "target"));
    }

    #[test]
    fn product_internal_helpers() {
        assert!(is_product_internal_dir_name(".litecode"));
        assert!(path_has_product_internal_dir(".litecode/index/x"));
        assert!(is_discovery_or_product_dir_name("node_modules"));
        assert!(is_discovery_or_product_dir_name(".data"));
        assert!(!is_discovery_or_product_dir_name("target"));
    }
}
