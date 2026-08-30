use serde::Serialize;

use std::path::Path;

use super::filter::{
    ExcludeMatcher, FilterPreset, RelPathCtx, cheap_rel_under, compile_include_pattern,
    configure_walk_under, normalize_pattern, walk_builder,
};
use super::path_sort::glob_hit_key;
use super::sandbox::{Sandbox, SandboxError};
use ignore::WalkBuilder;

#[derive(Debug, Clone, Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error("not a directory: {0}")]
    NotDir(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("ignore walk error: {0}")]
    Walk(String),
    #[error("{0}")]
    InvalidPattern(String),
}

/// Filename / glob listing for the explorer search box (human, not Agent glob).
#[derive(Debug, Clone, Serialize)]
pub struct GlobListing {
    pub entries: Vec<TreeEntry>,
    pub truncated: bool,
}

const MAX_GLOB_RESULTS: usize = 1000;

pub fn list_tree(
    sandbox: &Sandbox,
    rel_path: &str,
    depth: usize,
) -> Result<Vec<TreeEntry>, TreeError> {
    if depth == 0 {
        return Ok(Vec::new());
    }

    let dir = sandbox.resolve(rel_path)?;
    if !dir.is_dir() {
        return Err(TreeError::NotDir(sandbox.rel_path(&dir)?));
    }

    let root = sandbox.root();
    // Explorer is lazy one-level listing (VS Code readdir). `depth > 1` is ignored.
    if FilterPreset::Explorer.layers().git_ignore {
        list_tree_gitignore_layer(root, &dir)
    } else {
        list_tree_read_dir(root, &dir)
    }
}

fn sort_tree_entries(entries: &mut [TreeEntry]) {
    entries.sort_by(|a, b| match (a.kind.as_str(), b.kind.as_str()) {
        ("dir", "file") => std::cmp::Ordering::Less,
        ("file", "dir") => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
}

fn tree_entry(name: String, rel: String, is_dir: bool) -> TreeEntry {
    TreeEntry {
        name,
        path: rel,
        // NOTE: contract with the frontend is `kind: "file" | "dir"`
        // (see web/src/api/workspace.ts). Do not change this to
        // "directory"/"file" — the file tree keys off "dir" to tell
        // folders apart from files.
        kind: if is_dir { "dir".into() } else { "file".into() },
        size: None,
    }
}

/// Default explorer path: `read_dir` + `files.exclude`. No canonicalize, no size stat.
fn list_tree_read_dir(root: &Path, dir: &Path) -> Result<Vec<TreeEntry>, TreeError> {
    let matcher = ExcludeMatcher::for_preset(FilterPreset::Explorer);
    let mut entries = Vec::new();
    for dent in std::fs::read_dir(dir)? {
        let dent = dent?;
        let path = dent.path();
        let name = dent.file_name().to_string_lossy().into_owned();
        let ft = dent.file_type()?;
        let is_dir = if ft.is_symlink() {
            std::fs::metadata(&path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
        } else {
            ft.is_dir()
        };
        let Some(rel) = cheap_rel_under(root, &path) else {
            continue;
        };
        if matcher.matches(&rel) {
            continue;
        }
        entries.push(tree_entry(name, rel, is_dir));
    }
    sort_tree_entries(&mut entries);
    Ok(entries)
}

/// `explorer_git_ignore=true`: WalkBuilder at depth 1 for gitignore layers.
/// Still cheap_rel (no canonicalize) and no size.
fn list_tree_gitignore_layer(root: &Path, dir: &Path) -> Result<Vec<TreeEntry>, TreeError> {
    let mut builder = WalkBuilder::new(dir);
    configure_walk_under(&mut builder, root, dir, FilterPreset::Explorer);
    let walker = builder.max_depth(Some(1)).build();
    let mut entries = Vec::new();
    for result in walker {
        let entry = result.map_err(|e| TreeError::Walk(e.to_string()))?;
        let path = entry.path();
        if path == dir {
            continue;
        }
        let Some(rel) = cheap_rel_under(root, path) else {
            continue;
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        entries.push(tree_entry(name, rel, is_dir));
    }
    sort_tree_entries(&mut entries);
    Ok(entries)
}

fn parent_rel(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((p, _)) if !p.is_empty() => p,
        _ => "",
    }
}

/// Directories that must be listed so `rel_path` is visible (root + ancestors, not the leaf).
fn reveal_dirs(rel_path: &str) -> Vec<String> {
    if rel_path.is_empty() {
        return Vec::new();
    }
    let mut dirs = vec![String::new()];
    let mut stack = Vec::new();
    let mut cur = parent_rel(rel_path).to_string();
    while !cur.is_empty() {
        stack.push(cur.clone());
        cur = parent_rel(&cur).to_string();
    }
    stack.reverse();
    dirs.extend(stack);
    dirs
}

/// One-shot ancestor listing for explorer reveal (VS Code `resolveTo`).
pub fn list_tree_reveal(
    sandbox: &Sandbox,
    rel_path: &str,
) -> Result<Vec<(String, Vec<TreeEntry>)>, TreeError> {
    let mut out = Vec::new();
    for dir in reveal_dirs(rel_path) {
        let entries = list_tree(sandbox, &dir, 1)?;
        out.push((dir, entries));
    }
    Ok(out)
}

/// Find files and folders by filename glob, using the same Explorer preset as
/// the tree so results match what browsing would show.
///
/// Plain text (no `* ? [ {`) is wrapped as `*{query}*` so typing `FileTree`
/// finds `FileTree.tsx`. Matching is case-insensitive. Unlike the Agent `glob`
/// tool, `*.ts` is recursive — humans expect the whole workspace.
pub fn list_glob(sandbox: &Sandbox, query: &str) -> Result<GlobListing, TreeError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(GlobListing {
            entries: Vec::new(),
            truncated: false,
        });
    }

    let pattern = human_filename_pattern(query);
    let matcher = compile_include_pattern(&pattern.to_lowercase()).map_err(|e| {
        let msg = e.to_string();
        let cleaned = msg.strip_prefix("tool execution error: ").unwrap_or(&msg);
        TreeError::InvalidPattern(cleaned.to_string())
    })?;

    let root = sandbox.root().to_path_buf();
    let rel_ctx = RelPathCtx::new(&root).unwrap_or_else(|_| RelPathCtx::new_lossy(&root));
    let walker = walk_builder(&root, FilterPreset::Explorer).build();

    let mut entries = Vec::new();
    for result in walker {
        let entry = result.map_err(|e| TreeError::Walk(e.to_string()))?;
        let path = entry.path();
        if path == rel_ctx.root_lap() || path == root.as_path() {
            continue;
        }

        let Some(rel) = cheap_rel_under(rel_ctx.root_lap(), path)
            .or_else(|| rel_ctx.rel(path))
            .or_else(|| sandbox.rel_path(path).ok())
        else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        if !matcher.matches(&rel.to_lowercase()) {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = if is_dir {
            None
        } else {
            entry.metadata().ok().map(|m| m.len())
        };
        entries.push(TreeEntry {
            name,
            path: rel,
            kind: if is_dir { "dir".into() } else { "file".into() },
            size,
        });
    }

    entries.sort_by(|a, b| glob_hit_key(&a.path).cmp(&glob_hit_key(&b.path)));
    let truncated = entries.len() > MAX_GLOB_RESULTS;
    if truncated {
        entries.truncate(MAX_GLOB_RESULTS);
    }
    Ok(GlobListing { entries, truncated })
}

fn human_filename_pattern(query: &str) -> String {
    let q = normalize_pattern(query);
    if has_glob_metachar(&q) {
        q
    } else {
        format!("*{q}*")
    }
}

fn has_glob_metachar(q: &str) -> bool {
    q.contains('*') || q.contains('?') || q.contains('[') || q.contains('{')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

    fn names(entries: &[TreeEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn by_name<'a>(entries: &'a [TreeEntry]) -> std::collections::HashMap<&'a str, &'a TreeEntry> {
        entries.iter().map(|e| (e.name.as_str(), e)).collect()
    }

    #[test]
    fn list_tree_is_one_level_and_reports_kinds_without_size() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/inner.txt"), "nested").unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let entries = list_tree(&sandbox, "", 2).unwrap();
        let map = by_name(&entries);

        assert_eq!(map["src"].kind, "dir");
        assert_eq!(map["src"].size, None);
        assert_eq!(map["a.txt"].kind, "file");
        assert_eq!(map["a.txt"].size, None);
        assert!(
            !names(&entries).contains(&"inner.txt"),
            "depth>1 must still list one level only; got {entries:?}"
        );

        let src_kids = list_tree(&sandbox, "src", 1).unwrap();
        assert_eq!(src_kids.len(), 1);
        assert_eq!(src_kids[0].path, "src/inner.txt");
        assert_eq!(src_kids[0].kind, "file");
        assert_eq!(src_kids[0].size, None);
    }

    #[test]
    fn list_tree_hides_git_shows_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config"), "").unwrap();
        std::fs::create_dir(root.join("node_modules")).unwrap();
        std::fs::write(root.join("node_modules/leftpad.js"), "").unwrap();
        std::fs::write(root.join("app.rs"), "").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let entries = list_tree(&sandbox, "", 1).unwrap();
            let listed = names(&entries);
            assert!(
                !listed.contains(&".git"),
                "files.exclude must hide .git; got {listed:?}"
            );
            assert!(
                listed.contains(&"node_modules"),
                "explorer must show node_modules (search.exclude only); got {listed:?}"
            );
            assert!(listed.contains(&"app.rs"));
        });
    }

    #[test]
    fn list_tree_honors_explorer_git_ignore_switch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(root.join("ignored.rs"), "fn ig() {}\n").unwrap();
        std::fs::write(root.join("visible.rs"), "fn vs() {}\n").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let entries = list_tree(&sandbox, "", 1).unwrap();
            let listed = names(&entries);
            assert!(
                listed.contains(&"ignored.rs"),
                "default explorer_git_ignore=false must show gitignored file; got {listed:?}"
            );
            assert!(listed.contains(&"visible.rs"));
        });

        with_excludes_cache_for_test(
            WorkspaceExcludesFile {
                explorer_git_ignore: true,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                let entries = list_tree(&sandbox, "", 1).unwrap();
                let listed = names(&entries);
                assert!(
                    !listed.contains(&"ignored.rs"),
                    "explorer_git_ignore=true must hide gitignored file; got {listed:?}"
                );
                assert!(listed.contains(&"visible.rs"));
            },
        );
    }

    #[test]
    fn list_tree_reveal_lists_root_and_ancestors_not_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/a")).unwrap();
        std::fs::write(root.join("src/a/b.ts"), "").unwrap();
        std::fs::write(root.join("README.md"), "").unwrap();
        std::fs::write(root.join("src/other.rs"), "").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let layers = list_tree_reveal(&sandbox, "src/a/b.ts").unwrap();
        let keys: Vec<&str> = layers.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["", "src", "src/a"]);
        assert!(
            !keys.contains(&"src/a/b.ts"),
            "must not treat the file as a directory; got {keys:?}"
        );

        let root_names = names(&layers[0].1);
        assert!(root_names.contains(&"src"));
        assert!(root_names.contains(&"README.md"));
        assert!(!root_names.contains(&"b.ts"));

        let src_names = names(&layers[1].1);
        assert!(src_names.contains(&"a"));
        assert!(src_names.contains(&"other.rs"));

        let a_names = names(&layers[2].1);
        assert_eq!(a_names, vec!["b.ts"]);
    }

    #[test]
    fn list_tree_reveal_empty_path_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        assert!(list_tree_reveal(&sandbox, "").unwrap().is_empty());
    }

    fn glob_paths(sandbox: &Sandbox, query: &str) -> Vec<String> {
        list_glob(sandbox, query)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    #[test]
    fn list_glob_plain_text_matches_filename_substring() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/components")).unwrap();
        std::fs::write(root.join("src/components/FileTree.tsx"), "").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "hi").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let hits = glob_paths(&sandbox, "FileTree");
        assert_eq!(hits, vec!["src/components/FileTree.tsx"]);
        assert!(glob_paths(&sandbox, "main").contains(&"src/main.rs".to_string()));
        assert!(glob_paths(&sandbox, "nope").is_empty());
    }

    #[test]
    fn list_glob_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Main.rs"), "").unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(glob_paths(&sandbox, "main"), vec!["Main.rs"]);
    }

    #[test]
    fn list_glob_star_pattern_is_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("top.ts"), "").unwrap();
        std::fs::write(root.join("src/nested/leaf.ts"), "").unwrap();
        std::fs::write(root.join("src/nested/skip.rs"), "").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let mut hits = glob_paths(&sandbox, "*.ts");
        hits.sort();
        assert_eq!(hits, vec!["src/nested/leaf.ts", "top.ts"]);
    }

    #[test]
    fn list_glob_includes_matching_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/stores")).unwrap();
        std::fs::write(dir.path().join("src/stores/treeStore.ts"), "").unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        let listing = list_glob(&sandbox, "store").unwrap();
        let paths: Vec<_> = listing
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind.as_str()))
            .collect();
        assert!(
            paths.contains(&("src/stores", "dir")),
            "expected stores dir, got {paths:?}"
        );
        assert!(
            paths.contains(&("src/stores/treeStore.ts", "file")),
            "expected treeStore.ts, got {paths:?}"
        );
    }

    #[test]
    fn list_glob_empty_query_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        let listing = list_glob(&sandbox, "  ").unwrap();
        assert!(listing.entries.is_empty());
        assert!(!listing.truncated);
    }

    #[test]
    fn list_glob_rejects_unclosed_bracket() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        assert!(matches!(
            list_glob(&sandbox, "file["),
            Err(TreeError::InvalidPattern(_))
        ));
    }

    #[test]
    fn list_glob_truncates_after_sort() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Shallower hits must survive truncation: z.md is depth 0, nested files
        // are depth 1. Walk order is not sort order.
        std::fs::write(root.join("z.md"), "").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        for i in 0..=MAX_GLOB_RESULTS {
            std::fs::write(root.join("src").join(format!("f{i}.txt")), "").unwrap();
        }
        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let listing = list_glob(&sandbox, "*.txt").unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries.len(), MAX_GLOB_RESULTS);
        assert!(
            listing.entries.iter().all(|e| e.path.starts_with("src/")),
            "plain *.txt must not match z.md; got {:?}",
            listing.entries.iter().map(|e| &e.path).collect::<Vec<_>>()
        );

        let listing = list_glob(&sandbox, "*").unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.entries[0].path, "src");
        assert_eq!(listing.entries[1].path, "z.md");
    }
}
