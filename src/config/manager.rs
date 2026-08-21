//! Config Manager — load, validate, resolve.

use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

use super::global_db;
use super::resolved::{ResolvedConfig, WorkspaceState, resolve};
use super::schema::{AgentRole, GlobalSettings, InitScope, SUBAGENT_SERIES_TOOL_IDS, ToolTier};
use super::workspace::{self, init_workspace, load_workspace_state};

/// Single configuration entry point (L1).
pub struct ConfigManager;

impl ConfigManager {
    /// Open global DB, migrate, seed if needed, and load settings.
    pub fn load_global() -> Result<GlobalSettings> {
        global_db::load_global()
    }

    pub fn load_global_from(path: &Path) -> Result<GlobalSettings> {
        global_db::load_global_from_path(path)
    }

    /// Assemble read-only resolved view from disjoint global + workspace layers.
    pub fn resolve(global: GlobalSettings, workspace: WorkspaceState) -> ResolvedConfig {
        resolve(global, workspace)
    }

    /// Validate global settings: adapters, provider/model links, agent refs, log level.
    pub fn validate(global: &GlobalSettings) -> Result<()> {
        Self::validate_structural(global)
    }

    /// Structural validation. Serve may start with empty providers/models and empty
    /// agent `model_ref`; any present LLM rows must be adapter-consistent and ready.
    fn validate_structural(global: &GlobalSettings) -> Result<()> {
        validate_log_level(&global.log)?;
        validate_llm_registry(global)?;

        for (agent_id, profile) in &global.agents {
            if !profile.model_ref.is_empty() && !global.models.contains_key(&profile.model_ref) {
                return Err(LitecodeError::Config(format!(
                    "agent '{agent_id}' model_ref '{}' does not exist in models registry",
                    profile.model_ref
                )));
            }
            for tool_id in profile.tools.keys() {
                if !global.tool_catalog.contains_key(tool_id) {
                    return Err(LitecodeError::Config(format!(
                        "agent '{agent_id}' tool binding '{tool_id}' does not exist in tool catalog"
                    )));
                }
            }

            if profile.role == AgentRole::Subagent {
                for tool_id in SUBAGENT_SERIES_TOOL_IDS {
                    if profile.tools.contains_key(*tool_id) {
                        return Err(LitecodeError::Config(format!(
                            "agent '{agent_id}' (subagent) must not bind '{tool_id}'"
                        )));
                    }
                }
            }

            if profile.role == AgentRole::Hidden && !profile.tools.is_empty() {
                return Err(LitecodeError::Config(format!(
                    "agent '{agent_id}' (hidden) must not have tool bindings"
                )));
            }

            match profile.role {
                AgentRole::Primary => {
                    for sub_id in &profile.allowed_subagents {
                        let Some(sub) = global.agents.get(sub_id) else {
                            return Err(LitecodeError::Config(format!(
                                "agent '{agent_id}' allowed_subagents references unknown agent '{sub_id}'"
                            )));
                        };
                        if sub.role != AgentRole::Subagent {
                            return Err(LitecodeError::Config(format!(
                                "agent '{agent_id}' allowed_subagents '{sub_id}' is not a subagent"
                            )));
                        }
                    }
                }
                _ => {
                    if !profile.allowed_subagents.is_empty() {
                        return Err(LitecodeError::Config(format!(
                            "agent '{agent_id}' (non-primary) must not have allowed_subagents"
                        )));
                    }
                }
            }
        }

        validate_custom_tools(global)?;
        validate_mcp_servers(global)?;

        Ok(())
    }

    /// Load global + workspace and assemble read-only resolved view.
    pub fn load_runtime_bundle(override_path: Option<&Path>) -> Result<ResolvedConfig> {
        let global = Self::load_global()?;
        Self::validate_structural(&global)?;
        super::bridge::warn_bridge_fallbacks(&global);
        let workspace = Self::load_workspace(override_path)?;
        Ok(Self::resolve(global, workspace))
    }

    /// Canonical workspace root from optional CLI override.
    pub fn resolve_workspace_root(override_path: Option<&Path>) -> Result<PathBuf> {
        workspace::resolve_workspace_root(override_path)
    }

    /// Initialize workspace contract shell and `.litecode/` layout.
    pub fn init_workspace(workspace_root: &Path) -> Result<()> {
        init_workspace(workspace_root)
    }

    /// Load workspace layer (resolve root, init, read contract).
    pub fn load_workspace(override_path: Option<&Path>) -> Result<WorkspaceState> {
        load_workspace_state(override_path)
    }

    /// Initialize catalog readiness for tools with the given init scope.
    pub fn init_tool_catalog(
        resolved: &mut ResolvedConfig,
        scope: InitScope,
    ) -> crate::tool::catalog::CatalogInitOutcome {
        crate::tool::catalog::init(resolved, scope)
    }

    /// Init global-scope tools and record readiness in process-memory runtime catalog state only.
    /// No longer persists readiness to the global DB (R3: runtime readiness is never persisted).
    pub fn init_global_catalog_at_startup(
        _db_path: &Path,
        resolved: &mut ResolvedConfig,
    ) -> Result<Vec<String>> {
        let outcome = Self::init_tool_catalog(resolved, InitScope::Global);
        Ok(outcome.initialized)
    }
}

fn validate_llm_registry(global: &GlobalSettings) -> Result<()> {
    for (id, provider) in &global.providers {
        if provider.id != *id {
            return Err(LitecodeError::Config(format!(
                "provider map key '{id}' does not match provider.id '{}'",
                provider.id
            )));
        }
        crate::llm::validate_provider_config(provider)?;
        if !crate::llm::provider_ready(provider) {
            return Err(LitecodeError::Config(format!(
                "provider '{id}' is not ready (endpoint and api_key required)"
            )));
        }
    }

    for (model_id, model) in &global.models {
        if model.id != *model_id {
            return Err(LitecodeError::Config(format!(
                "model map key '{model_id}' does not match model.id '{}'",
                model.id
            )));
        }
        crate::llm::validate_model_config(model_id, &model.adapter_id, &model.config)?;
        if model.provider_ref.is_empty() {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' provider_ref must not be empty"
            )));
        }
        let Some(provider) = global.providers.get(&model.provider_ref) else {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' provider_ref '{}' does not exist",
                model.provider_ref
            )));
        };
        if provider.adapter_id != model.adapter_id {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' adapter_id '{}' does not match provider '{}' adapter_id '{}'",
                model.adapter_id, model.provider_ref, provider.adapter_id
            )));
        }
        if !crate::llm::provider_ready(provider) {
            return Err(LitecodeError::Config(format!(
                "model '{model_id}' links to provider '{}' which is not ready",
                model.provider_ref
            )));
        }
    }

    Ok(())
}

const VALID_LOG_LEVELS: &[&str] = &["trace", "debug", "info", "warn", "error", "off"];

fn validate_log_level(log: &super::schema::LogSettings) -> Result<()> {
    let Some(level) = log
        .level
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };

    if VALID_LOG_LEVELS
        .iter()
        .any(|valid| valid.eq_ignore_ascii_case(level))
    {
        return Ok(());
    }

    Err(LitecodeError::Config(format!(
        "log.level must be one of: {}",
        VALID_LOG_LEVELS.join(", ")
    )))
}

fn validate_custom_tools(global: &GlobalSettings) -> Result<()> {
    let custom_names: std::collections::HashSet<&str> = global
        .custom_tools
        .iter()
        .map(|t| t.name.as_str())
        .collect();

    for entry in global.tool_catalog.values() {
        if entry.tier != ToolTier::Custom {
            continue;
        }
        if !custom_names.contains(entry.id.as_str()) {
            return Err(LitecodeError::Config(format!(
                "tool catalog entry '{}' has tier custom but no matching custom_tools definition",
                entry.id
            )));
        }
        if entry.init_scope != InitScope::Global {
            return Err(LitecodeError::Config(format!(
                "custom tool '{}' must have init_scope global",
                entry.id
            )));
        }
    }

    for tool in &global.custom_tools {
        let Some(entry) = global.tool_catalog.get(&tool.name) else {
            return Err(LitecodeError::Config(format!(
                "custom tool '{}' has no matching tool catalog entry",
                tool.name
            )));
        };
        if entry.tier != ToolTier::Custom {
            return Err(LitecodeError::Config(format!(
                "custom tool '{}' catalog entry must have tier custom",
                tool.name
            )));
        }
    }

    Ok(())
}

fn validate_mcp_servers(global: &GlobalSettings) -> Result<()> {
    use super::global_db::tools;

    for id in global.mcp_servers.keys() {
        let catalog_id = tools::mcp_catalog_id(id);
        let Some(entry) = global.tool_catalog.get(&catalog_id) else {
            return Err(LitecodeError::Config(format!(
                "MCP server '{id}' has no matching tool catalog entry '{catalog_id}'"
            )));
        };
        if entry.tier != ToolTier::Mcp {
            return Err(LitecodeError::Config(format!(
                "MCP server '{id}' catalog entry must have tier mcp"
            )));
        }
        if entry.init_scope != InitScope::Global {
            return Err(LitecodeError::Config(format!(
                "MCP server '{id}' must have init_scope global"
            )));
        }
    }

    for entry in global.tool_catalog.values() {
        if entry.tier != ToolTier::Mcp {
            continue;
        }
        let Some(server_id) = entry.id.strip_prefix("mcp_") else {
            return Err(LitecodeError::Config(format!(
                "tool catalog entry '{}' has tier mcp but id is not mcp_<server>",
                entry.id
            )));
        };
        if !global.mcp_servers.contains_key(server_id) {
            return Err(LitecodeError::Config(format!(
                "tool catalog entry '{}' has tier mcp but no matching mcp_servers definition",
                entry.id
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{
        ADAPTER_OPENAI_RESPONSES, AgentProfile, AgentRole, AgentToolBinding, ModelAdapterConfig,
        ModelCapability, ModelDefinition, ProviderAuth, ProviderConnectionConfig,
        ProviderDefinition,
    };
    use std::collections::HashMap;

    fn ready_provider(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            label: id.into(),
            config: ProviderConnectionConfig {
                endpoint: "https://api.example.com/v1".into(),
                api_key: "sk-test".into(),
                auth: ProviderAuth::Bearer,
            },
        }
    }

    fn sample_model(id: &str, provider_ref: &str) -> ModelDefinition {
        ModelDefinition {
            id: id.into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            provider_ref: provider_ref.into(),
            label: id.into(),
            config: ModelAdapterConfig {
                api_model_id: "gpt-4".into(),
                context_window: 200_000,
                max_tokens: 4096,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![ModelCapability::Text],
            },
        }
    }

    fn minimal_global() -> GlobalSettings {
        GlobalSettings {
            providers: HashMap::from([("main".into(), ready_provider("main"))]),
            models: HashMap::from([("default".into(), sample_model("default", "main"))]),
            ..Default::default()
        }
    }

    #[test]
    fn resolve_global_union_workspace_field_disjoint_property() {
        let global = minimal_global();
        let workspace = WorkspaceState::new("/tmp/ws");

        let resolved = ConfigManager::resolve(global.clone(), workspace.clone());

        let global_names: std::collections::HashSet<_> =
            GlobalSettings::FIELD_NAMES.iter().copied().collect();
        let workspace_names: std::collections::HashSet<_> =
            WorkspaceState::FIELD_NAMES.iter().copied().collect();
        assert!(global_names.is_disjoint(&workspace_names));

        let resolved_names: std::collections::HashSet<_> =
            ResolvedConfig::FIELD_NAMES.iter().copied().collect();
        assert_eq!(
            resolved_names.len(),
            global_names.len() + workspace_names.len()
        );
        for name in global_names {
            assert!(resolved_names.contains(name));
        }
        for name in workspace_names {
            assert!(resolved_names.contains(name));
        }

        assert_eq!(resolved.providers(), &global.providers);
        assert_eq!(resolved.models(), &global.models);
        assert_eq!(
            resolved.workspace_root(),
            workspace.workspace_root.as_path()
        );
        assert_eq!(resolved.paths(), &workspace.paths);
    }

    #[test]
    fn validate_missing_endpoint_returns_error() {
        let mut global = minimal_global();
        global.providers.get_mut("main").unwrap().config.endpoint = String::new();
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg.contains("endpoint")));
    }

    #[test]
    fn validate_missing_api_key_returns_error() {
        let mut global = minimal_global();
        global.providers.get_mut("main").unwrap().config.api_key = String::new();
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg.contains("api_key")));
    }

    #[test]
    fn validate_empty_model_ref_ok() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: String::new(),
                ..Default::default()
            },
        );
        ConfigManager::validate(&global).unwrap();
    }

    #[test]
    fn validate_orphan_tool_binding_fails() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    "nonexistent-tool".into(),
                    AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg.contains("nonexistent-tool")));
    }

    #[test]
    fn validate_dangling_model_ref_fails() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "missing-model".into(),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg.contains("missing-model")));
    }

    #[test]
    fn validate_ok_with_valid_refs() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                ..Default::default()
            },
        );
        ConfigManager::validate(&global).unwrap();
    }

    #[test]
    fn validate_invalid_log_level_fails() {
        let mut global = minimal_global();
        global.log.level = Some("verbose".into());
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg.contains("log.level")));
    }

    #[test]
    fn validate_log_level_accepts_standard_values() {
        let mut global = minimal_global();
        for level in ["trace", "debug", "info", "warn", "error", "off"] {
            global.log.level = Some(level.into());
            ConfigManager::validate(&global).unwrap();
        }
    }

    #[test]
    fn validate_subagent_cannot_bind_subagent_series() {
        let mut global = minimal_global();
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    "subagent_launch".into(),
                    AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(err.to_string().contains("subagent_launch"));
    }

    #[test]
    fn validate_primary_allowlist_references_subagent() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                allowed_subagents: vec!["ghost".into()],
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn validate_primary_allowlist_ok_for_subagent() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                allowed_subagents: vec!["worker".into()],
                ..Default::default()
            },
        );
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "default".into(),
                ..Default::default()
            },
        );
        ConfigManager::validate(&global).unwrap();
    }

    #[test]
    fn init_global_catalog_at_startup_marks_global_tools_ready_in_runtime_state() {
        use crate::config::schema::{InitScope, ToolCatalogEntry, ToolReadiness, ToolTier};
        use tempfile::TempDir;

        let dir = TempDir::new().expect("dir");
        let db_path = dir.path().join("litecode.db");
        let mut settings = ConfigManager::load_global_from(&db_path).expect("seed");
        settings.tool_catalog.insert(
            "webfetch".into(),
            ToolCatalogEntry {
                id: "webfetch".into(),
                tier: ToolTier::Custom,
                init_scope: InitScope::Global,
                catalog_enabled: true,
            },
        );
        global_db::import_into(&db_path, &settings).expect("import");

        let workspace = WorkspaceState::new("/tmp/serve-startup");
        let mut resolved = ConfigManager::resolve(
            ConfigManager::load_global_from(&db_path).expect("load"),
            workspace,
        );

        let initialized =
            ConfigManager::init_global_catalog_at_startup(&db_path, &mut resolved).expect("init");
        assert!(
            initialized.iter().any(|id| id == "webfetch"),
            "expected webfetch init, got {initialized:?}"
        );
        assert_eq!(
            resolved.runtime_catalog_state().readiness.get("webfetch"),
            Some(&ToolReadiness::Ready)
        );

        let reloaded = ConfigManager::load_global_from(&db_path).expect("reload");
        assert!(
            reloaded.tool_catalog.contains_key("webfetch"),
            "webfetch should still exist in catalog"
        );
    }
}
