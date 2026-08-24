use std::path::Path;

use litecode::config::{WorkspacePaths, WorkspaceState, init_workspace, peek_workspace_id};

pub fn test_workspace(dir: &Path) -> WorkspaceState {
    init_workspace(dir).expect("init workspace");
    let workspace_id = peek_workspace_id(dir).expect("workspace identity after init");
    WorkspaceState {
        workspace_root: dir.to_path_buf(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: WorkspacePaths::for_workspace(dir, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    }
}

pub fn test_db_path(dir: &Path) -> String {
    test_workspace(dir)
        .paths
        .sessions_db
        .to_string_lossy()
        .into_owned()
}
