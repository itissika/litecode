//! Resolved configuration: `GlobalSettings` ∪ `WorkspaceState`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::snapshot_paths::{snapshots_dir_for_id, workspace_snapshot_id};

use super::schema::{
    AgentProfile, CustomToolDefinition, GlobalSettings, LogSettings, McpServerDefinition,
    ModelDefinition, ProviderDefinition, ToolOrigin, ToolReadiness, WebSearchSettings,
};

/// Workspace runtime paths: session/plan/logs under `<workspace>/.litecode/`;
/// file-revert snapshots live outside the tree (`~/.litecode/snapshots/<workspace_id>/`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePaths {
    pub sessions_db: PathBuf,
    pub logs_dir: PathBuf,
    pub plan_dir: PathBuf,
    pub snapshots_dir: PathBuf,
}

impl WorkspacePaths {
    /// Build paths for a workspace root + stable identity.
    /// Snapshots are never under `.litecode/`.
    pub fn for_workspace(workspace_root: &Path, workspace_id: &str) -> Self {
        let litecode_dir = workspace_root.join(".litecode");
        Self {
            sessions_db: litecode_dir.join("sessions.db"),
            logs_dir: litecode_dir.join("logs"),
            plan_dir: litecode_dir.join("plan"),
            snapshots_dir: snapshots_dir_for_id(workspace_id),
        }
    }

    /// Test / ephemeral helper: path-derived snapshot id without writing identity files.
    pub fn for_legacy_root(workspace_root: &Path) -> Self {
        let id = workspace_snapshot_id(workspace_root);
        Self::for_workspace(workspace_root, &id)
    }
}

/// Workspace-layer state — paths, contract, workspace-scoped tool readiness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub workspace_root: PathBuf,
    /// Stable id persisted in `.litecode/workspace.json` (host-global association).
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub contract: String,
    pub paths: WorkspacePaths,
    /// Workspace-tool readiness, synced from engines.json.
    #[serde(default)]
    pub workspace_tool_readiness: HashMap<String, ToolReadiness>,
    #[serde(default)]
    pub workspace_mcp_servers: HashMap<String, McpServerDefinition>,
    #[serde(default)]
    pub workspace_custom_tools: HashMap<String, CustomToolDefinition>,
}

impl WorkspaceState {
    /// Top-level field names owned exclusively by the workspace layer (for partition tests).
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "workspace_root",
        "workspace_id",
        "contract",
        "paths",
        "workspace_tool_readiness",
        "workspace_mcp_servers",
        "workspace_custom_tools",
    ];

    /// Ephemeral / test constructor: path-derived id, no disk identity write.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let workspace_id = workspace_snapshot_id(&workspace_root);
        Self {
            paths: WorkspacePaths::for_workspace(&workspace_root, &workspace_id),
            workspace_root,
            workspace_id,
            contract: String::new(),
            workspace_tool_readiness: HashMap::new(),
            workspace_mcp_servers: HashMap::new(),
            workspace_custom_tools: HashMap::new(),
        }
    }

    /// Construct with an already-ensured stable identity.
    pub fn with_identity(workspace_root: PathBuf, workspace_id: String) -> Self {
        let paths = WorkspacePaths::for_workspace(&workspace_root, &workspace_id);
        Self {
            workspace_root,
            workspace_id,
            contract: String::new(),
            paths,
            workspace_tool_readiness: HashMap::new(),
            workspace_mcp_servers: HashMap::new(),
            workspace_custom_tools: HashMap::new(),
        }
    }
}

/// Read-only resolved view. Global settings are immutable after construction;
/// workspace-scoped readiness is the only mutable layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedConfig {
    global: GlobalSettings,
    workspace: WorkspaceState,
}

impl ResolvedConfig {
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "providers",
        "models",
        "agents",
        "custom_tools",
        "mcp_servers",
        "auth",
        "log",
        "websearch",
        "workspace_root",
        "workspace_id",
        "contract",
        "paths",
        "workspace_tool_readiness",
        "workspace_mcp_servers",
        "workspace_custom_tools",
    ];

    pub fn new(global: GlobalSettings, workspace: WorkspaceState) -> Self {
        Self { global, workspace }
    }

    pub fn global(&self) -> &GlobalSettings {
        &self.global
    }

    pub fn workspace(&self) -> &WorkspaceState {
        &self.workspace
    }

    pub fn providers(&self) -> &HashMap<String, ProviderDefinition> {
        &self.global.providers
    }

    pub fn models(&self) -> &HashMap<String, ModelDefinition> {
        &self.global.models
    }

    pub fn agents(&self) -> &HashMap<String, AgentProfile> {
        &self.global.agents
    }

    pub fn global_custom_tools(&self) -> &[CustomToolDefinition] {
        &self.global.custom_tools
    }

    pub fn workspace_custom_tools(&self) -> &HashMap<String, CustomToolDefinition> {
        &self.workspace.workspace_custom_tools
    }

    /// Merged custom tools (workspace wins on name).
    pub fn custom_tools(&self) -> Vec<CustomToolDefinition> {
        let mut map: HashMap<String, CustomToolDefinition> = HashMap::new();
        for tool in &self.global.custom_tools {
            map.insert(tool.name.clone(), tool.clone());
        }
        for (name, tool) in &self.workspace.workspace_custom_tools {
            map.insert(name.clone(), tool.clone());
        }
        let mut out: Vec<_> = map.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn global_mcp_servers(&self) -> &HashMap<String, McpServerDefinition> {
        &self.global.mcp_servers
    }

    pub fn workspace_mcp_servers(&self) -> &HashMap<String, McpServerDefinition> {
        &self.workspace.workspace_mcp_servers
    }

    /// Merged MCP servers (workspace wins on id).
    pub fn mcp_servers(&self) -> HashMap<String, McpServerDefinition> {
        let mut map = self.global.mcp_servers.clone();
        for (id, def) in &self.workspace.workspace_mcp_servers {
            map.insert(id.clone(), def.clone());
        }
        map
    }

    pub fn mcp_origin(&self, server_id: &str) -> Option<ToolOrigin> {
        if self.workspace.workspace_mcp_servers.contains_key(server_id) {
            Some(ToolOrigin::Workspace)
        } else if self.global.mcp_servers.contains_key(server_id) {
            Some(ToolOrigin::Global)
        } else {
            None
        }
    }

    pub fn custom_origin(&self, name: &str) -> Option<ToolOrigin> {
        if self.workspace.workspace_custom_tools.contains_key(name) {
            Some(ToolOrigin::Workspace)
        } else if self.global.custom_tools.iter().any(|t| t.name == name) {
            Some(ToolOrigin::Global)
        } else {
            None
        }
    }

    pub fn mcp_pool_key(&self, server_id: &str) -> String {
        match self.mcp_origin(server_id) {
            Some(ToolOrigin::Workspace) => format!("workspace:{server_id}"),
            _ => format!("global:{server_id}"),
        }
    }

    pub fn log(&self) -> &LogSettings {
        &self.global.log
    }

    pub fn websearch(&self) -> &WebSearchSettings {
        &self.global.websearch
    }

    // --- Workspace-layer read-only accessors ---

    pub fn workspace_root(&self) -> &Path {
        &self.workspace.workspace_root
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace.workspace_id
    }

    pub fn contract(&self) -> &str {
        &self.workspace.contract
    }

    pub fn paths(&self) -> &WorkspacePaths {
        &self.workspace.paths
    }

    pub fn workspace_tool_readiness(&self) -> &HashMap<String, ToolReadiness> {
        &self.workspace.workspace_tool_readiness
    }

    pub fn workspace_mut(&mut self) -> &mut WorkspaceState {
        &mut self.workspace
    }

    /// Replace workspace layer; clears workspace-scoped tool readiness (CONFIG §2.4).
    pub fn replace_workspace(&mut self, workspace: WorkspaceState) {
        self.workspace = workspace;
    }
}

/// Assemble `ResolvedConfig` from disjoint global and workspace inputs.
pub fn resolve(global: GlobalSettings, workspace: WorkspaceState) -> ResolvedConfig {
    ResolvedConfig::new(global, workspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::GlobalSettings;

    #[test]
    fn global_and_workspace_partitions_are_disjoint() {
        let global: std::collections::HashSet<_> =
            GlobalSettings::FIELD_NAMES.iter().copied().collect();
        let workspace: std::collections::HashSet<_> =
            WorkspaceState::FIELD_NAMES.iter().copied().collect();
        let overlap: Vec<_> = global.intersection(&workspace).copied().collect();
        assert!(
            overlap.is_empty(),
            "global and workspace must not share field names: {overlap:?}"
        );
    }

    #[test]
    fn resolved_field_names_are_union_of_partitions() {
        let mut expected = GlobalSettings::FIELD_NAMES.to_vec();
        expected.extend_from_slice(WorkspaceState::FIELD_NAMES);
        assert_eq!(ResolvedConfig::FIELD_NAMES, expected.as_slice());
    }

    #[test]
    fn resolve_preserves_both_layers() {
        let mut global = GlobalSettings::default();
        global.providers.insert(
            "main".into(),
            crate::config::schema::ProviderDefinition {
                id: "main".into(),
                adapter_id: crate::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
                label: "main".into(),
                config: crate::config::schema::ProviderConnectionConfig {
                    endpoint: "https://api.example.com/v1".into(),
                    api_key: "sk-test".into(),
                    auth: crate::config::schema::ProviderAuth::Bearer,
                },
            },
        );

        let mut workspace = WorkspaceState::new("/tmp/project");
        workspace.contract = "# contract".into();

        let resolved = resolve(global.clone(), workspace.clone());

        assert_eq!(
            resolved
                .providers()
                .get("main")
                .map(|p| p.config.endpoint.as_str()),
            global
                .providers
                .get("main")
                .map(|p| p.config.endpoint.as_str())
        );
        assert_eq!(
            resolved.workspace_root(),
            workspace.workspace_root.as_path()
        );
        assert_eq!(resolved.contract(), "# contract");
        assert_eq!(resolved.paths().sessions_db, workspace.paths.sessions_db);
    }
}
