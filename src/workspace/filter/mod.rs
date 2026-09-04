//! Unified workspace path filtering: orthogonal layers + named presets.
//!
//! Exclude globs are workspace-owned (`.litecode/excludes.json`), seeded from
//! VS Code (`files.exclude` / `search.exclude` / `files.watcherExclude`).
//! Three faces: Explorer (tree), Search (human/agent/index), Watcher (hard cut).
//! Walks compose ripgrep gitignore / binary. Index content gates (binary /
//! size only) stay in [`index_policy`]. Product-internal dirs compose via
//! [`dirs`].

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
    path_matches_include,
};
pub use preset::{FilterPreset, exclude_globs, search_and_files_exclude_globs};

/// Footer when grep/glob searched zero files under default filters.
pub fn empty_discovery_hint() -> &'static str {
    "Default filters may hide paths (.gitignore, files_exclude, search_exclude). Workspace exclude lists are in .litecode/excludes.json."
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
