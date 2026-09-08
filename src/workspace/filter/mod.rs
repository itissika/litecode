//! Unified workspace path filtering: orthogonal layers + named presets.
//!
//! Exclude globs are workspace-owned (`.litecode/excludes.json`), seeded from
//! VS Code (`files.exclude` / `search.exclude` / `files.watcherExclude`).
//! Three faces: Explorer (tree), Search (human/agent/index), Watcher (hard cut).
//! Walks compose ripgrep gitignore / binary. Index content gates (binary /
//! size only) stay in [`index_policy`]. Product-internal dirs compose via
//! [`dirs`].

use std::path::Path;

mod binary;
mod defaults;
mod dirs;
mod exclude;
mod gitignore;
mod index_policy;
mod layers;
mod path;
mod path_glob;
mod preset;
mod walk;
mod workspace_excludes;

pub use binary::looks_binary;
pub use defaults::{
    FILES_EXCLUDE, PRODUCT_INTERNAL_DIRS, SEARCH_EXCLUDE, SNAPSHOT_ONLY_DIRS, WATCHER_EXCLUDE,
};
pub use dirs::{
    discovery_exclude_dir_basenames, is_discovery_or_product_dir_name,
    is_product_internal_dir_name, path_has_product_internal_dir, snapshot_exclude_dir_basenames,
};
pub use exclude::{ExcludeMatcher, path_excluded};
pub use gitignore::path_gitignored;
pub use index_policy::{
    MAX_INDEX_FILE_BYTES, SKIP_DIRS, is_indexable_rel_path, is_scannable_rel_path,
    path_has_skipped_dir, should_queue_index_update,
};
pub use layers::FilterLayers;
pub use path::{RelPathCtx, cheap_rel_under, rel_path_under};
pub use path_glob::{
    PathGlobMatcher, compile_include_pattern, compile_include_patterns, normalize_pattern,
    path_matches_include, split_glob_include_exclude,
};
pub use preset::{FilterPreset, exclude_globs, search_and_files_exclude_globs};

/// Footer when grep/glob searched zero files under default filters.
pub fn empty_discovery_hint() -> &'static str {
    "Default filters may hide paths (.gitignore, files_exclude, search_exclude). Workspace exclude lists are in .litecode/excludes.json."
}

const CORPUS_LIST_CAP: usize = 8;

/// Live Search-corpus boundary for agent grep empty states.
///
/// `None` when `no_ignore` (the walk did not apply these lists). Does not
/// search excluded trees; it only names the active config.
pub fn search_corpus_note(no_ignore: bool) -> Option<String> {
    if no_ignore {
        return None;
    }
    let cfg = active_workspace_excludes();
    let mut parts = vec![
        format!(
            "search_exclude: {}",
            format_corpus_globs(&cfg.search_exclude)
        ),
        format!("files_exclude: {}", format_corpus_globs(&cfg.files_exclude)),
    ];
    if cfg.git_ignore {
        parts.push("git_ignore=true (also .gitignore)".into());
    } else {
        parts.push("git_ignore=false".into());
    }
    Some(format!(
        "Did not search paths excluded by {WORKSPACE_EXCLUDES_REL}: {}.",
        parts.join("; ")
    ))
}

/// One next step after a Search-corpus miss. Names schema fields, not bash.
pub fn search_corpus_next_step() -> &'static str {
    "If the text may live in an excluded tree, set path there or no_ignore=true; otherwise it is absent from this corpus."
}

fn format_corpus_globs(globs: &[String]) -> String {
    if globs.is_empty() {
        return "(none)".into();
    }
    if globs.len() <= CORPUS_LIST_CAP {
        return globs.join(", ");
    }
    format!(
        "{} (and {} more)",
        globs[..CORPUS_LIST_CAP].join(", "),
        globs.len() - CORPUS_LIST_CAP
    )
}

/// Why agent glob/grep should not enter `rel` (workspace-relative, `/` separators).
pub fn ignored_discovery_reason(workspace_root: &Path, rel: &str) -> Option<&'static str> {
    if path_has_product_internal_dir(rel) {
        return Some("LiteCode runtime directory.");
    }
    let cfg = active_workspace_excludes();
    if ExcludeMatcher::from_globs(&cfg.files_exclude).matches(rel) {
        return Some("excluded by files.exclude.");
    }
    if ExcludeMatcher::from_globs(&cfg.search_exclude).matches(rel) {
        return Some("excluded by search.exclude.");
    }
    if path_gitignored(workspace_root, rel, FilterPreset::Search) {
        return Some("ignored by .gitignore.");
    }
    None
}

/// Human-readable refusal when `resolved` is an excluded directory under the workspace.
pub fn ignored_discovery_message(workspace_root: &Path, resolved: &Path) -> Option<String> {
    let rel = cheap_rel_under(workspace_root, resolved)?;
    let rel = rel.replace('\\', "/");
    if rel.is_empty() {
        return None;
    }
    let reason = ignored_discovery_reason(workspace_root, &rel)?;
    Some(format!("path '{rel}' is not searched: {reason}"))
}
pub use walk::{
    WalkOptions, configure_walk, configure_walk_under, configure_walk_with, walk_builder,
    walk_builder_with,
};
pub use workspace_excludes::{
    WORKSPACE_EXCLUDES_REL, WorkspaceExcludesFile, WorkspaceExcludesLists, WorkspaceExcludesView,
    activate_workspace_excludes, active_workspace_excludes, ensure_workspace_excludes,
    is_workspace_excludes_rel, path_triggers_code_index_sync, persist_workspace_excludes,
    read_workspace_excludes, reload_workspace_excludes_from_disk, workspace_excludes_path,
    write_workspace_excludes,
};

#[cfg(test)]
pub(crate) use workspace_excludes::{lock_excludes_cache_for_test, with_excludes_cache_for_test};

#[cfg(test)]
mod corpus_note_tests {
    use super::*;

    #[test]
    fn search_corpus_note_omits_when_no_ignore() {
        assert!(search_corpus_note(true).is_none());
    }

    #[test]
    fn search_corpus_note_cites_excludes_file_and_caps_lists() {
        let mut file = WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude = (0..10).map(|i| format!("**/skip{i}")).collect();
        file.files_exclude = vec!["**/.git".into()];
        file.git_ignore = true;
        with_excludes_cache_for_test(file, || {
            let note = search_corpus_note(false).expect("note");
            assert!(
                note.contains(WORKSPACE_EXCLUDES_REL),
                "must name config file, got: {note}"
            );
            assert!(note.contains("**/skip0"), "got: {note}");
            assert!(note.contains("(and 2 more)"), "got: {note}");
            assert!(
                !note.contains("**/skip9"),
                "cap must hide tail, got: {note}"
            );
            assert!(
                note.contains("git_ignore=true (also .gitignore)"),
                "got: {note}"
            );
            assert!(
                !note.contains("watcher_exclude"),
                "watcher is not a grep corpus, got: {note}"
            );
        });
    }

    #[test]
    fn search_corpus_next_step_names_fields() {
        let step = search_corpus_next_step();
        assert!(step.contains("no_ignore=true"), "{step}");
        assert!(step.contains("path"), "{step}");
        assert!(!step.to_ascii_lowercase().contains("bash"), "{step}");
    }
}
