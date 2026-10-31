//! Config Manager — load, validate, resolve.

use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

use std::sync::Arc;

use crate::provider_catalog::{self, ProviderCatalog};

use super::binding_scope::scope_of;
use super::global_db;
use super::resolved::{ResolvedConfig, WorkspaceState, resolve};
use super::schema::{AgentRole, GlobalSettings};
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

    /// Assemble read-only resolved view from disjoint global, workspace and
    /// catalog layers.
    pub fn resolve(
        global: GlobalSettings,
        workspace: WorkspaceState,
        catalog: Arc<ProviderCatalog>,
    ) -> ResolvedConfig {
        resolve(global, workspace, catalog)
    }

    /// Fixture helper: resolve with an empty catalog (no providers, no models).
    ///
    /// Only for code that exercises settings shape (tool availability, engine
    /// reconcile) without touching an LLM binding.
    pub fn resolve_without_catalog(
        global: GlobalSettings,
        workspace: WorkspaceState,
    ) -> ResolvedConfig {
        let catalog = Arc::new(
            ProviderCatalog::parse(
                "version = 1
",
                Path::new("<empty-catalog>"),
            )
            .expect("an empty catalog is valid"),
        );
        Self::resolve(global, workspace, catalog)
    }

    /// Validate global settings: agent bindings, extensions, log level.
    ///
    /// Model references are validated where they are written (the settings API
    /// refuses an unknown reference) and re-checked at turn resolve time; a stale
    /// reference must never lock the Settings service.
    pub fn validate(global: &GlobalSettings) -> Result<()> {
        Self::validate_structural(global)
    }

    fn validate_structural(global: &GlobalSettings) -> Result<()> {
        validate_log_level(&global.log)?;

        for (agent_id, profile) in &global.agents {
            for tool_id in profile.tools.keys() {
                if tool_id.is_empty() {
                    return Err(LitecodeError::Config(format!(
                        "agent '{agent_id}' has an empty tool binding id"
                    )));
                }
            }

            if profile.role == AgentRole::Subagent {
                for tool_id in profile.tools.keys() {
                    if !scope_of(tool_id).allows_role(AgentRole::Subagent) {
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

    /// Load global + workspace + catalog and assemble the read-only resolved view.
    ///
    /// The catalog is this build's embedded seed. The database is opened (and
    /// migrated) before the one-time legacy credential / model-reference
    /// migration reads the old tables.
    pub fn load_runtime_bundle(override_path: Option<&Path>) -> Result<ResolvedConfig> {
        Self::load_runtime_bundle_with(override_path, workspace::WarmSnapshot::Now)
    }

    /// WP-166: as [`Self::load_runtime_bundle`]; `serve` passes
    /// `WarmSnapshot::Deferred` and warms the snapshot repo after bind.
    pub fn load_runtime_bundle_with(
        override_path: Option<&Path>,
        warm: workspace::WarmSnapshot,
    ) -> Result<ResolvedConfig> {
        Self::load_runtime_bundle_from_with(&global_db::default_db_path(), override_path, warm)
    }

    pub fn load_runtime_bundle_from(
        db_path: &Path,
        override_path: Option<&Path>,
    ) -> Result<ResolvedConfig> {
        Self::load_runtime_bundle_from_with(db_path, override_path, workspace::WarmSnapshot::Now)
    }

    fn load_runtime_bundle_from_with(
        db_path: &Path,
        override_path: Option<&Path>,
        warm: workspace::WarmSnapshot,
    ) -> Result<ResolvedConfig> {
        let catalog = provider_catalog::shared_for_db(db_path)?;
        global_db::with_conn(db_path, |conn| {
            global_db::legacy::migrate_once(conn, &catalog)?;
            global_db::repair_agent_models_on_boot(conn, &catalog)
        })?;
        let global = Self::load_global_from(db_path)?;
        Self::validate_structural(&global)?;
        let workspace = workspace::load_workspace_state_with(override_path, warm)?;
        let resolved = Self::resolve(global, workspace, catalog);
        super::bridge::warn_unresolved_agent_models(&resolved);
        Ok(resolved)
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
    use super::global_db::tools;
    let mut names = std::collections::HashSet::new();
    for (suite_name, suite) in &global.custom_suites {
        crate::config::settings_writer::validate_tool_id(suite_name)?;
        if tools::is_core_tool(suite_name) || tools::is_optional_builtin(suite_name) {
            return Err(LitecodeError::Config(format!(
                "custom suite '{suite_name}' conflicts with a builtin tool"
            )));
        }
        if let Some(error) = crate::permission::policy::custom_rule_ids_error(&suite.rules) {
            return Err(LitecodeError::Config(format!(
                "custom suite '{suite_name}' has invalid rules: {error}"
            )));
        }
    }
    for tool in &global.custom_tools {
        if !names.insert(tool.name.as_str()) {
            return Err(LitecodeError::Config(format!(
                "duplicate custom tool name '{}'",
                tool.name
            )));
        }
        let mut checked = tool.clone();
        crate::config::settings_writer::validate_custom_definition_with_suites(
            &tool.name,
            &mut checked,
            &global.custom_suites,
        )?;
    }
    Ok(())
}

fn validate_mcp_servers(global: &GlobalSettings) -> Result<()> {
    use super::global_db::tools;
    for id in global.mcp_servers.keys() {
        if tools::is_core_tool(id) || tools::is_optional_builtin(id) {
            return Err(LitecodeError::Config(format!(
                "MCP server id '{id}' conflicts with a builtin tool"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::schema::{AgentProfile, AgentRole, AgentToolBinding};
    use std::collections::HashMap;

    fn minimal_global() -> GlobalSettings {
        let mut global = GlobalSettings::default();
        global
            .provider_credentials
            .insert("main".into(), "sk-test".into());
        global
    }

    fn binding() -> AgentToolBinding {
        AgentToolBinding {
            enabled: true,
            preset: None,
            allowed_tools: None,
        }
    }

    #[test]
    fn validate_subagent_cannot_bind_primary_root_only() {
        let mut global = minimal_global();
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "main/default".into(),
                tools: HashMap::from([("subagent_launch".into(), binding())]),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(err.to_string().contains("subagent_launch"));
    }
}
