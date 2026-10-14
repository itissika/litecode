//! Workspace scans that feed the index. WP-143: there is no separate full
//! build; first build and rebuild go through the incremental path
//! ([`super::update_files`]).

use std::path::Path;

use crate::types::Result;
use crate::workspace::filter::{
    FilterPreset, MAX_INDEX_FILE_BYTES, RelPathCtx, WalkOptions, is_indexable_rel_path,
    is_scannable_rel_path, walk_builder, walk_builder_with,
};

/// WP-141 periodic scan: the same walk and path filter as
/// [`scannable_files`], but metadata only (stat for size + stamp). The binary
/// sniff, which opens the file, is left to `update_files` for paths whose
/// stamp changed, so an unchanged tree is never opened.
pub(crate) fn scan_stat_candidates(
    workspace_root: &Path,
) -> Result<Vec<(String, Option<super::FileStamp>)>> {
    let mut out = Vec::new();
    let rel_ctx =
        RelPathCtx::new(workspace_root).unwrap_or_else(|_| RelPathCtx::new_lossy(workspace_root));
    // No binary sniff in the walk either: that opens every file.
    let walker = walk_builder_with(
        workspace_root,
        FilterPreset::Search,
        WalkOptions {
            skip_binary: Some(false),
            ..WalkOptions::default()
        },
    );
    for result in walker.build() {
        let Ok(entry) = result else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel_str) = rel_ctx.rel(entry.path()) else {
            continue;
        };
        if !is_scannable_rel_path(&rel_str) {
            continue;
        }
        let meta = std::fs::metadata(entry.path()).ok();
        if meta
            .as_ref()
            .is_some_and(|m| !m.is_file() || m.len() > MAX_INDEX_FILE_BYTES)
        {
            continue;
        }
        let stamp = meta.as_ref().and_then(super::reconcile::stamp_from_meta);
        out.push((rel_str, stamp));
    }
    out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub fn scannable_files(workspace_root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let rel_ctx =
        RelPathCtx::new(workspace_root).unwrap_or_else(|_| RelPathCtx::new_lossy(workspace_root));
    let walker = walk_builder(workspace_root, FilterPreset::Search);
    for result in walker.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(rel_str) = rel_ctx.rel(path) else {
            continue;
        };
        if is_indexable_rel_path(&rel_str, workspace_root) {
            out.push(rel_str);
        }
    }
    out.sort_unstable();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::embed::HashEmbedder;
    use crate::engines::code_search::index_workspace_blocking;
    use crate::workspace::filter::is_scannable_rel_path;
    use tempfile::TempDir;

    #[test]
    fn build_index_skips_non_scannable() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("ok.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("skip.bin"), b"\0\0\0").unwrap();
        let emb = HashEmbedder;
        let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
        assert!(index.chunks().values().any(|c| c.path == "ok.rs"));
    }

    #[test]
    fn scannable_respects_policy() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.rs"), "x\n").unwrap();
        std::fs::write(root.join("LICENSE"), "MIT\n").unwrap();
        std::fs::write(root.join("serve.ps1"), "Write-Host hi\n").unwrap();
        let files = scannable_files(root).unwrap();
        assert!(files.iter().any(|f| f == "a.rs"));
        assert!(files.iter().any(|f| f == "LICENSE"));
        assert!(files.iter().any(|f| f == "serve.ps1"));
        assert!(is_scannable_rel_path("a.rs"));
    }

    #[test]
    fn embedding_corpus_honors_index_gitignore_switch() {
        // The real embedding input path: walk(Index) + content gates. Toggling
        // the search-side gitignore switch must add/drop gitignored files from
        // the corpus, without a full reindex fingerprint.
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // The ignore crate only honors .gitignore with a git repo marker present.
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "ref_vendor.rs\n").unwrap();
        std::fs::write(root.join("ref_vendor.rs"), "fn r() {}\n").unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

        // Default: git_ignore=true → the pulled reference repo stays out.
        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            let files = scannable_files(root).unwrap();
            assert!(files.iter().any(|f| f == "main.rs"), "{files:?}");
            assert!(
                !files.iter().any(|f| f == "ref_vendor.rs"),
                "gitignored file must not enter the embedding corpus: {files:?}"
            );
        });

        // git_ignore=false → the ignored file joins the corpus.
        with_excludes_cache_for_test(
            WorkspaceExcludesFile {
                git_ignore: false,
                ..WorkspaceExcludesFile::builtin_defaults()
            },
            || {
                let files = scannable_files(root).unwrap();
                assert!(
                    files.iter().any(|f| f == "ref_vendor.rs"),
                    "git_ignore=false must admit the file: {files:?}"
                );
            },
        );
    }
}
