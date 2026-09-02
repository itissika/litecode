//! Thresholds and path gates for the adaptive text index.

use std::path::Path;

use crate::workspace::filter::{
    FilterPreset, active_workspace_excludes, is_workspace_excludes_rel, path_excluded,
    path_gitignored, path_has_product_internal_dir, should_queue_index_update,
};

/// Refuse to build above this (protect against $HOME-sized roots).
pub const HARD_FILE_CAP: u64 = 200_000;
/// Skip putting individual files larger than this into Tantivy; keep the path
/// as an oversized sidecar so grep still verifies them (superset, never a miss).
pub const MAX_INDEX_FILE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextIndexMode {
    Auto,
    On,
    Off,
}

/// `LITECODE_TEXT_INDEX=auto|on|off` (default auto).
pub fn mode_from_env() -> TextIndexMode {
    match std::env::var("LITECODE_TEXT_INDEX")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "on" | "1" | "true" => TextIndexMode::On,
        "off" | "0" | "false" => TextIndexMode::Off,
        _ => TextIndexMode::Auto,
    }
}

/// Auto and On both build whenever the Search corpus fits under the cap.
pub fn should_build(mode: TextIndexMode, file_count: u64) -> bool {
    match mode {
        TextIndexMode::Off => false,
        TextIndexMode::On | TextIndexMode::Auto => file_count <= HARD_FILE_CAP,
    }
}

/// Fingerprint of the Search corpus definition. A mismatch means the
/// tracked path set must be reconciled (delta add/delete, or rebuild if huge).
pub fn corpus_fingerprint() -> String {
    let cfg = active_workspace_excludes();
    let mut files = cfg.files_exclude.clone();
    files.sort();
    let mut search = cfg.search_exclude.clone();
    search.sort();
    format!(
        "v1\nfiles:{}\nsearch:{}\ngit_ignore:{}",
        files.join("\x1f"),
        search.join("\x1f"),
        cfg.git_ignore
    )
}

/// Ignore-rule / excludes files define the corpus; a change must reconcile the
/// tracked path set, not reindex the ignore file as content.
pub fn is_corpus_definition_rel(rel: &str) -> bool {
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    if is_workspace_excludes_rel(&rel) {
        return true;
    }
    if rel == ".git/info/exclude" || rel.ends_with("/.git/info/exclude") {
        return true;
    }
    let name = rel.rsplit('/').next().unwrap_or(rel.as_str());
    matches!(name, ".gitignore" | ".ignore" | ".fdignore" | ".rgignore")
}

/// Rebuild instead of incremental apply when this many paths must be *added*
/// (each add reads the file body). Deletes are cheap `delete_term`s and never
/// prefer a rewrite — rebuild would re-read the files that are staying.
pub const RECONCILE_ADD_ABS: usize = 1_000;

/// `true` when the add side of a path-set diff is large enough that rewriting
/// the index is the simpler bound on foreground work.
pub fn delta_prefers_rebuild(adds: usize) -> bool {
    adds >= RECONCILE_ADD_ABS
}

/// Paths to delete (`true`) or add (`false`) so `have` becomes `want`.
pub fn corpus_delta(
    want: &std::collections::HashSet<String>,
    have: &std::collections::HashSet<String>,
) -> Vec<(String, bool)> {
    let mut out = Vec::with_capacity(want.len().abs_diff(have.len()).max(4));
    for p in have.difference(want) {
        out.push((p.clone(), true));
    }
    for p in want.difference(have) {
        out.push((p.clone(), false));
    }
    out
}

/// Queue gate for text-index incremental updates (same Search face as semantic).
pub fn should_queue_text_path(workspace_root: &Path, rel: &str, deleted: bool) -> bool {
    should_queue_index_update(workspace_root, rel, deleted)
}

/// Whether this relative path is skipped by `preset` at query time.
pub fn path_skipped_by_preset(workspace_root: &Path, rel: &str, preset: FilterPreset) -> bool {
    if path_has_product_internal_dir(rel) {
        return true;
    }
    if path_excluded(rel, preset) {
        return true;
    }
    path_gitignored(workspace_root, rel, preset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_skips_node_modules_and_litecode() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        assert!(!should_queue_text_path(root, "node_modules/x.js", false));
        assert!(!should_queue_text_path(root, ".litecode/text-index/x", false));
        assert!(!should_queue_text_path(root, ".git/config", false));
        assert!(should_queue_text_path(root, "src/main.rs", false));
        assert!(should_queue_text_path(root, "src/main.rs", true));
        // target/ is NOT a Search glob exclude; only gitignore drops it.
        assert!(should_queue_text_path(root, "target/foo.rs", false));
        assert!(should_queue_text_path(root, ".data/foo.rs", false));
        // Excluded path: delete still queues so the posting can be removed.
        assert!(should_queue_text_path(root, "node_modules/x.js", true));
    }

    #[test]
    fn queue_gate_matches_search_exclude_matcher() {
        use crate::workspace::filter::path_excluded;
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        for rel in [
            "node_modules/pkg/index.js",
            "bower_components/x",
            ".git/HEAD",
            ".svn/entries",
            ".DS_Store",
        ] {
            assert!(
                path_excluded(rel, FilterPreset::Search),
                "{rel} should be Search-excluded"
            );
            assert!(
                !should_queue_text_path(root, rel, false),
                "{rel} must not enter text-index queue on create"
            );
        }
    }

    #[test]
    fn gitignore_target_does_not_queue_create() {
        use crate::workspace::filter::{WorkspaceExcludesFile, with_excludes_cache_for_test};

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/foo.rs"), "fn t() {}\n").unwrap();

        with_excludes_cache_for_test(WorkspaceExcludesFile::builtin_defaults(), || {
            assert!(!should_queue_text_path(root, "target/foo.rs", false));
            assert!(should_queue_text_path(root, "target/foo.rs", true));
            assert!(should_queue_text_path(root, "src/main.rs", false));
        });
    }

    #[test]
    fn corpus_definition_paths() {
        assert!(is_corpus_definition_rel(".gitignore"));
        assert!(is_corpus_definition_rel("nested/.gitignore"));
        assert!(is_corpus_definition_rel(".litecode/excludes.json"));
        assert!(!is_corpus_definition_rel("src/main.rs"));
    }

    #[test]
    fn auto_builds_small_workspaces() {
        assert!(should_build(TextIndexMode::Auto, 1));
        assert!(should_build(TextIndexMode::Auto, HARD_FILE_CAP));
        assert!(!should_build(TextIndexMode::Auto, HARD_FILE_CAP + 1));
        assert!(!should_build(TextIndexMode::Off, 10));
    }

    #[test]
    fn delta_rebuild_threshold() {
        assert!(!delta_prefers_rebuild(0));
        assert!(!delta_prefers_rebuild(50));
        assert!(!delta_prefers_rebuild(RECONCILE_ADD_ABS - 1));
        assert!(delta_prefers_rebuild(RECONCILE_ADD_ABS));
        assert!(delta_prefers_rebuild(4_000));
    }

    #[test]
    fn corpus_delta_add_and_delete() {
        let have: std::collections::HashSet<_> = ["a.rs", "vendor/x.rs"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let want: std::collections::HashSet<_> =
            ["a.rs", "b.rs"].into_iter().map(str::to_string).collect();
        let mut d = corpus_delta(&want, &have);
        d.sort();
        assert_eq!(
            d,
            vec![("b.rs".into(), false), ("vendor/x.rs".into(), true)]
        );
    }

    #[test]
    fn search_does_not_skip_unignored_hidden() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        assert!(!path_skipped_by_preset(root, ".env", FilterPreset::Search));
        assert!(!path_skipped_by_preset(
            root,
            "src/.secret/a.rs",
            FilterPreset::Search
        ));
        assert!(path_skipped_by_preset(
            root,
            "node_modules/x.js",
            FilterPreset::Search
        ));
    }
}
