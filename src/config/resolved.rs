//! Resolved configuration: `GlobalSettings` ∪ `WorkspaceState` ∪ provider catalog.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::snapshot_paths::{snapshots_dir_for_id, workspace_snapshot_id};

use std::sync::Arc;

use crate::provider_catalog::ProviderCatalog;

use super::schema::{
    AgentProfile, CustomToolDefinition, GlobalSettings, LogSettings, McpServerDefinition,
    ToolOrigin, ToolReadiness, WebSearchSettings,
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

/// Read-only resolved view. Global settings and the catalog are immutable after
/// construction; workspace-scoped readiness is the only mutable layer.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    global: GlobalSettings,
    workspace: WorkspaceState,
    /// Provider/model facts for this process lifetime (edits need a restart).
    #[serde(skip)]
    catalog: Arc<ProviderCatalog>,
}

impl PartialEq for ResolvedConfig {
    fn eq(&self, other: &Self) -> bool {
        self.global == other.global
            && self.workspace == other.workspace
            && Arc::ptr_eq(&self.catalog, &other.catalog)
    }
}

impl ResolvedConfig {
    pub const FIELD_NAMES: &'static [&'static str] = &[
        "provider_credentials",
        "disabled_models",
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

    pub fn new(
        global: GlobalSettings,
        workspace: WorkspaceState,
        catalog: Arc<ProviderCatalog>,
    ) -> Self {
        Self {
            global,
            workspace,
            catalog,
        }
    }

    pub fn global(&self) -> &GlobalSettings {
        &self.global
    }

    pub fn workspace(&self) -> &WorkspaceState {
        &self.workspace
    }

    /// The single source of provider/model facts.
    pub fn catalog(&self) -> &Arc<ProviderCatalog> {
        &self.catalog
    }

    /// API key for a catalog provider, when the user configured one.
    pub fn provider_api_key(&self, provider_id: &str) -> Option<&str> {
        self.global
            .provider_credentials
            .get(provider_id)
            .map(String::as_str)
            .filter(|key| !key.trim().is_empty())
    }

    /// Providers that have a credential, in catalog order.
    pub fn configured_providers(&self) -> Vec<Arc<crate::provider_catalog::ResolvedProvider>> {
        self.catalog
            .providers()
            .iter()
            .filter(|provider| self.provider_api_key(&provider.id).is_some())
            .cloned()
            .collect()
    }

    /// Models that are selectable right now: their provider has a key and the
    /// user has not switched the model off. A reference that is already in use
    /// (an agent's `model_ref` or a stored session) keeps working — the switch
    /// governs the pickers, not the resolver.
    pub fn active_models(&self) -> Vec<Arc<crate::provider_catalog::ResolvedModel>> {
        self.catalog
            .models()
            .iter()
            .filter(|model| self.provider_api_key(&model.provider_id).is_some())
            .filter(|model| !self.global.disabled_models.contains(&model.reference))
            .cloned()
            .collect()
    }

    pub fn agents(&self) -> &HashMap<String, AgentProfile> {
        &self.global.agents
    }

    /// Catalog model for a reference that is selectable right now.
    pub fn model_for_agent_ref(
        &self,
        reference: &str,
    ) -> Option<Arc<crate::provider_catalog::ResolvedModel>> {
        let model = self.catalog.model(reference.trim())?;
        self.provider_api_key(&model.provider_id)?;
        Some(Arc::clone(model))
    }

    /// Catalog model an agent points at, whether or not its provider has a key.
    pub fn declared_model_for_agent(
        &self,
        agent_name: &str,
    ) -> Option<Arc<crate::provider_catalog::ResolvedModel>> {
        let reference = self.agents().get(agent_name)?.model_ref.trim().to_string();
        if reference.is_empty() {
            return None;
        }
        self.catalog.model(&reference).cloned()
    }

    /// Agent model that is usable right now: declared and keyed.
    pub fn model_for_agent(
        &self,
        agent_name: &str,
    ) -> Option<Arc<crate::provider_catalog::ResolvedModel>> {
        let model = self.declared_model_for_agent(agent_name)?;
        self.provider_api_key(&model.provider_id)?;
        Some(model)
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

/// Fixture helper: assemble a resolved view with an empty catalog.
///
/// For code that exercises settings shape (permission resolution, tool
/// availability) without an LLM binding.
pub fn resolve_without_catalog(global: GlobalSettings, workspace: WorkspaceState) -> ResolvedConfig {
    let catalog = Arc::new(
        ProviderCatalog::parse("version = 1
", Path::new("<empty-catalog>"))
            .expect("an empty catalog is valid"),
    );
    resolve(global, workspace, catalog)
}

/// Assemble `ResolvedConfig` from disjoint global, workspace and catalog inputs.
pub fn resolve(
    global: GlobalSettings,
    workspace: WorkspaceState,
    catalog: Arc<ProviderCatalog>,
) -> ResolvedConfig {
    ResolvedConfig::new(global, workspace, catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::GlobalSettings;
    use crate::provider_catalog::ProviderCatalog;
    use std::path::Path;

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

    const CATALOG: &str = r#"
version = 1
[[providers]]
id = "p"
name = "P"
endpoint = "https://api.example.com/v1"
endpoint_type = "responses"

[[models]]
id = "m"
provider_id = "p"
"#;

    fn catalog() -> Arc<ProviderCatalog> {
        Arc::new(ProviderCatalog::parse(CATALOG, Path::new("t.toml")).unwrap())
    }

    #[test]
    fn resolve_preserves_layers_and_catalog() {
        let mut global = GlobalSettings::default();
        global
            .provider_credentials
            .insert("p".into(), "sk-test".into());
        let mut workspace = WorkspaceState::new("/tmp/project");
        workspace.contract = "# contract".into();
        let catalog = catalog();

        let resolved = resolve(global, workspace.clone(), Arc::clone(&catalog));

        assert!(Arc::ptr_eq(resolved.catalog(), &catalog));
        assert_eq!(resolved.workspace_root(), workspace.workspace_root.as_path());
        assert_eq!(resolved.contract(), "# contract");
        assert_eq!(resolved.paths().sessions_db, workspace.paths.sessions_db);
    }

    #[test]
    fn credentials_are_serialization_safe() {
        let mut global = GlobalSettings::default();
        global
            .provider_credentials
            .insert("p".into(), "sk-secret-value".into());
        let json = serde_json::to_string(&global).unwrap();
        assert!(
            !json.contains("sk-secret-value"),
            "a credential must never reach a settings payload: {json}"
        );
    }

    #[test]
    fn active_models_follow_the_credential_map() {
        let workspace = WorkspaceState::new("/tmp/ws");
        let without = resolve(GlobalSettings::default(), workspace.clone(), catalog());
        assert!(without.active_models().is_empty());
        assert!(without.configured_providers().is_empty());

        let mut global = GlobalSettings::default();
        global.provider_credentials.insert("p".into(), "  ".into());
        let blank = resolve(global, workspace.clone(), catalog());
        assert!(
            blank.active_models().is_empty(),
            "a blank key is not a credential"
        );

        let mut global = GlobalSettings::default();
        global.provider_credentials.insert("p".into(), "sk".into());
        let ready = resolve(global, workspace, catalog());
        assert_eq!(ready.active_models().len(), 1);
        assert_eq!(ready.configured_providers().len(), 1);
        assert_eq!(ready.provider_api_key("p"), Some("sk"));
        assert_eq!(ready.provider_api_key("ghost"), None);
    }

    #[test]
    fn model_lookup_helpers_distinguish_declared_from_selectable() {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                model_ref: "p/m".into(),
                ..Default::default()
            },
        );
        let without_key = resolve(global.clone(), WorkspaceState::new("/tmp/ws"), catalog());
        assert!(without_key.declared_model_for_agent("default").is_some());
        assert!(without_key.model_for_agent("default").is_none());
        assert!(without_key.model_for_agent_ref("p/m").is_none());

        global.provider_credentials.insert("p".into(), "sk".into());
        let ready = resolve(global, WorkspaceState::new("/tmp/ws"), catalog());
        assert!(ready.model_for_agent("default").is_some());
        assert!(ready.model_for_agent_ref("p/m").is_some());
        assert!(ready.model_for_agent_ref("p/ghost").is_none());
        assert!(ready.model_for_agent("ghost").is_none());
    }
}
