//! Single-path gitignore matching for the Search (and Explorer) line.
//!
//! Walks use [`ignore::WalkBuilder`]; incremental queues must ask the same
//! question for one relative path without walking the tree.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use super::preset::FilterPreset;

/// Whether `rel` is ignored by git ignore files under `preset`'s git layers.
pub fn path_gitignored(workspace_root: &Path, rel: &str, preset: FilterPreset) -> bool {
    let layers = preset.layers();
    if !layers.git_ignore {
        return false;
    }
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    if rel.is_empty() {
        return false;
    }
    let abs = workspace_root.join(&rel);
    let is_dir = abs.is_dir();

    if layers.git_global {
        let (global, _) = Gitignore::global();
        if global.matched_path_or_any_parents(&abs, is_dir).is_ignore() {
            return true;
        }
    }

    let mut builder = GitignoreBuilder::new(workspace_root);
    if layers.git_exclude {
        let _ = builder.add(workspace_root.join(".git").join("info").join("exclude"));
    }
    let _ = builder.add(workspace_root.join(".gitignore"));
    let mut prefix = PathBuf::new();
    if let Some(parent) = Path::new(&rel).parent() {
        for c in parent.components() {
            prefix.push(c);
            let _ = builder.add(workspace_root.join(&prefix).join(".gitignore"));
        }
    }
    let Ok(gi) = builder.build() else {
        return false;
    };
    gi.matched_path_or_any_parents(&rel, is_dir).is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filter::WorkspaceExcludesFile;
    use crate::workspace::filter::with_excludes_cache_for_test;
    use tempfile::TempDir;

    #[test]
    fn search_honors_gitignore_when_switch_on() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/foo.rs"), "fn t() {}\n").unwrap();
        std::fs::write(root.join("src.rs"), "fn s() {}\n").unwrap();

        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            assert!(path_gitignored(root, "target/foo.rs", FilterPreset::Search));
            assert!(!path_gitignored(root, "src.rs", FilterPreset::Search));
            assert!(!path_gitignored(
                root,
                "target/foo.rs",
                FilterPreset::Watcher
            ));
        });
    }
}
