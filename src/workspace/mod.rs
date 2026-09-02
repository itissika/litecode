mod agent_path;
mod change;
pub mod file_ops;
pub mod filter;
mod git;
mod path_sort;
mod recycle;
mod routes;
mod sandbox;
mod service;
pub mod text_codec;
mod tool_path;
mod tree;
mod watcher;

pub use change::WorkspaceChange;
pub use filter::{
    ExcludeMatcher, FilterLayers, FilterPreset, RelPathCtx, path_excluded, rel_path_under,
    walk_builder,
};
pub use path_sort::{glob_hit_key, sort_glob_hits};
pub use routes::{WorkspaceState, router as workspace_router};
pub use sandbox::{Sandbox, SandboxError};
pub use service::{MAX_FILE_SIZE, WorkspaceError, WorkspaceService};
pub use tool_path::{
    AGENT_FILE_PATH_HINT, ToolPathError, ToolPathMode, is_resolved_outside_workspace,
    raw_path_outside_workspace, resolve_agent, resolve_human_relative, resolve_lsp_workspace,
    resolve_tool_path, resolve_tool_path_from,
};
pub use tree::{GlobListing, TreeEntry, TreeError, list_glob, list_tree, list_tree_reveal};
pub use watcher::{
    WorkspaceWatcher, filter_change_for_ui, restart_watcher, spawn_watcher,
};
