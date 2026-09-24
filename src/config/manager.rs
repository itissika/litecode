//! Config Manager — load, validate, resolve.

use std::path::{Path, PathBuf};

use crate::types::{LitecodeError, Result};

use std::sync::Arc;

use crate::provider_catalog::{self, ProviderCatalog};

use super::global_db;
use super::resolved::{ResolvedConfig, WorkspaceState, resolve};
use super::schema::{AgentRole, GlobalSettings, PLAN_TODO_TOOL_IDS, SUBAGENT_SERIES_TOOL_IDS};
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
            ProviderCatalog::parse("version = 1
", Path::new("<empty-catalog>"))
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
                for tool_id in PLAN_TODO_TOOL_IDS.iter().chain(SUBAGENT_SERIES_TOOL_IDS) {
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

    /// Load global + workspace + catalog and assemble the read-only resolved view.
    ///
    /// Order matters: the database is opened (and migrated) by the catalog load,
    /// the catalog is validated, and only then is the one-time legacy credential /
    /// model-reference migration allowed to read the old tables.
    pub fn load_runtime_bundle(override_path: Option<&Path>) -> Result<ResolvedConfig> {
        Self::load_runtime_bundle_from(&global_db::default_db_path(), override_path)
    }

    pub fn load_runtime_bundle_from(
        db_path: &Path,
        override_path: Option<&Path>,
    ) -> Result<ResolvedConfig> {
        let catalog = provider_catalog::shared_for_db(db_path)?;
        global_db::with_conn(db_path, |conn| {
            global_db::legacy::migrate_once(conn, &catalog)?;
            repair_agent_models_on_boot(conn, &catalog)
        })?;
        let global = Self::load_global_from(db_path)?;
        Self::validate_structural(&global)?;
        let workspace = Self::load_workspace(override_path)?;
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

/// Boot-time sibling of the per-commit repair in `SettingsWriter`.
///
/// A provider-catalog edit (needs a restart) or a provider key removed while the
/// app was closed leaves agents pointing at a model that cannot run, and no
/// settings write happens at startup to heal it. Writes only the repaired agent
/// rows: the catalog is the source of truth, a stored ref is derived state.
fn repair_agent_models_on_boot(
    conn: &rusqlite::Connection,
    catalog: &ProviderCatalog,
) -> Result<()> {
    let mut settings = global_db::store::load(conn)?;
    let repaired = super::bridge::repair_agent_models(&mut settings, catalog);
    if repaired.is_empty() {
        return Ok(());
    }
    tracing::info!(
        agents = ?repaired,
        "boot: auto-assigned a runnable model to agents whose model_ref could not run"
    );
    for id in repaired {
        let Some(profile) = settings.agents.get(&id) else {
            continue;
        };
        global_db::store::upsert_agent(
            conn,
            &id,
            profile.role,
            &profile.model_ref,
            &profile.system_prompt,
            profile.temperature,
            profile.max_steps,
            &profile.description,
            &profile.allowed_subagents,
        )?;
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
    use super::global_db::tools;
    let mut names = std::collections::HashSet::new();
    for tool in &global.custom_tools {
        if !names.insert(tool.name.as_str()) {
            return Err(LitecodeError::Config(format!(
                "duplicate custom tool name '{}'",
                tool.name
            )));
        }
        if tools::is_core_tool(&tool.name) || tools::is_optional_builtin(&tool.name) {
            return Err(LitecodeError::Config(format!(
                "custom tool '{}' conflicts with a builtin tool",
                tool.name
            )));
        }
        if tool.command.trim().is_empty() {
            return Err(LitecodeError::Config(format!(
                "custom tool '{}' command must not be empty",
                tool.name
            )));
        }
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
    use crate::config::resolved::WorkspaceState;
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
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        }
    }

    fn catalog() -> Arc<ProviderCatalog> {
        Arc::new(
            ProviderCatalog::parse(
                "version = 1\n[[providers]]\nid = \"main\"\nname = \"Main\"\nendpoint = \"https://api.example.com/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"default\"\nprovider_id = \"main\"\n",
                std::path::Path::new("t.toml"),
            )
            .unwrap(),
        )
    }

    #[test]
    fn resolve_global_union_workspace_field_disjoint_property() {
        let global = minimal_global();
        let workspace = WorkspaceState::new("/tmp/ws");

        let resolved = ConfigManager::resolve(global, workspace.clone(), catalog());

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
        assert_eq!(
            resolved.workspace_root(),
            workspace.workspace_root.as_path()
        );
        assert_eq!(resolved.paths(), &workspace.paths);
    }

    #[test]
    fn boot_hands_a_stale_agent_a_model_that_can_run() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("litecode.db");
        let ws = tempfile::tempdir().unwrap();
        crate::provider_catalog::store::forget(&db);
        std::fs::write(
            crate::provider_catalog::catalog_path_for_db(&db),
            "version = 1\n[[providers]]\nid = \"main\"\nname = \"Main\"\nendpoint = \"https://api.example.com/v1\"\nendpoint_type = \"responses\"\n\n[[models]]\nid = \"default\"\nprovider_id = \"main\"\n",
        )
        .unwrap();
        // Seed through the normal load path first (agents + bindings), then strand
        // the default agent on a ref the catalog does not declare.
        ConfigManager::load_global_from(&db).unwrap();
        let conn = crate::config::global_db::open(&db).unwrap();
        crate::config::global_db::store::set_provider_credential(&conn, "main", "sk-test")
            .unwrap();
        crate::config::global_db::store::upsert_agent(
            &conn,
            "default",
            AgentRole::Primary,
            "main/gone",
            "",
            0.7,
            50,
            "",
            &[],
        )
        .unwrap();
        drop(conn);

        let resolved = ConfigManager::load_runtime_bundle_from(&db, Some(ws.path())).unwrap();
        assert_eq!(resolved.agents()["default"].model_ref, "main/default");

        // Persisted, not just resolved: the next boot already reads a runnable ref.
        let reloaded = ConfigManager::load_global_from(&db).unwrap();
        assert_eq!(reloaded.agents["default"].model_ref, "main/default");
    }

    #[test]
    fn validate_accepts_a_stale_agent_model_ref() {
        // A reference left over from an older catalog must not lock Settings:
        // the settings API refuses new invalid writes, and the boot/commit repair
        // hands the agent a runnable model.
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "ghost/model".into(),
                ..Default::default()
            },
        );
        ConfigManager::validate(&global).unwrap();
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
                model_ref: "main/default".into(),
                tools: HashMap::from([("subagent_launch".into(), binding())]),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(err.to_string().contains("subagent_launch"));
    }

    #[test]
    fn validate_subagent_cannot_bind_plan_or_todo() {
        let mut global = minimal_global();
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "main/default".into(),
                tools: HashMap::from([("plan".into(), binding())]),
                ..Default::default()
            },
        );
        let err = ConfigManager::validate(&global).unwrap_err();
        assert!(err.to_string().contains("plan"));
    }

    #[test]
    fn validate_primary_allowlist_references_subagent() {
        let mut global = minimal_global();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "main/default".into(),
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
                model_ref: "main/default".into(),
                allowed_subagents: vec!["worker".into()],
                ..Default::default()
            },
        );
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "main/default".into(),
                ..Default::default()
            },
        );
        ConfigManager::validate(&global).unwrap();
    }
}
