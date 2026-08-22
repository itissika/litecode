//! Unified workspace path filtering: orthogonal layers + named presets.
//!
//! Exclude globs are workspace-owned (`.litecode/excludes.json`), seeded from
//! VS Code (`files.exclude` / `search.exclude` / `files.watcherExclude`).
//! Walks also compose ripgrep (gitignore / hidden / binary). Index
//! content gates are the product [`index_policy`] migrated from code_search.
//! Directory discovery for search / Agent / Index shares files∪search; product
//! internal dirs and snapshot-only trees compose via [`dirs`].

mod binary;
mod defaults;
mod dirs;
mod exclude;
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
pub use index_policy::{
    MAX_INDEX_FILE_BYTES, SKIP_DIRS, TEXT_EXTENSIONS, is_indexable_rel_path, is_noise_basename,
    is_scannable_rel_path, path_has_skipped_dir, should_queue_index_update,
};
pub use layers::FilterLayers;
pub use path::{RelPathCtx, cheap_rel_under, rel_path_under};
pub use path_glob::{
    PathGlobMatcher, compile_include_pattern, compile_include_patterns, normalize_pattern,
    path_matches_include,
};
pub use preset::{FilterPreset, exclude_globs, search_and_files_exclude_globs};
pub use walk::{
    WalkOptions, configure_walk, configure_walk_under, configure_walk_with, walk_builder,
    walk_builder_with,
};
pub use workspace_excludes::{
    WorkspaceExcludesFile, WorkspaceExcludesLists, WorkspaceExcludesView,
    activate_workspace_excludes, active_workspace_excludes, ensure_workspace_excludes,
    persist_workspace_excludes, read_workspace_excludes, workspace_excludes_path,
    write_workspace_excludes,
};
