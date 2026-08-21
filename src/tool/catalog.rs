//! Tool catalog gate: readiness × catalog_enabled + init scopes (CONFIG §2.4–2.5).

use std::collections::HashMap;

use crate::config::resolved::ResolvedConfig;
use crate::config::runtime_catalog_state::RuntimeCatalogState;
use crate::config::schema::{
    AgentToolBinding, InitScope, ToolCatalogEntry, ToolReadiness, ToolTier,
};
use crate::engines::WorkspaceEngines;
use crate::optional::EngineManager;

/// Effective readiness for a catalog entry, accounting for runtime catalog state
/// (global scope) and workspace-scoped state (workspace scope).
pub fn effective_readiness(
    entry: &ToolCatalogEntry,
    workspace_readiness: &HashMap<String, ToolReadiness>,
    runtime_catalog_state: &RuntimeCatalogState,
) -> ToolReadiness {
    match entry.init_scope {
        InitScope::Workspace => workspace_readiness
            .get(&entry.id)
            .copied()
            .unwrap_or(ToolReadiness::NotReady),
        InitScope::Global => runtime_catalog_state
            .readiness
            .get(&entry.id)
            .copied()
            .unwrap_or(ToolReadiness::NotReady),
        InitScope::None => {
            // Core tools are always ready (no init required).
            if entry.tier == ToolTier::Core {
                ToolReadiness::Ready
            } else {
                ToolReadiness::NotReady
            }
        }
    }
}

/// `catalog(readiness=ready ∧ catalog_enabled=true)` (CONFIG §2.5).
pub fn is_catalog_candidate(
    entry: &ToolCatalogEntry,
    workspace_readiness: &HashMap<String, ToolReadiness>,
    runtime_catalog_state: &RuntimeCatalogState,
) -> bool {
    entry.catalog_enabled
        && effective_readiness(entry, workspace_readiness, runtime_catalog_state)
            == ToolReadiness::Ready
}

/// Drop bindings for tools that are not catalog-ready (used when serving agent profiles).
pub fn prune_non_catalog_agent_bindings(
    catalog: &HashMap<String, ToolCatalogEntry>,
    workspace_readiness: &HashMap<String, ToolReadiness>,
    runtime_catalog_state: &RuntimeCatalogState,
    tools: &mut HashMap<String, AgentToolBinding>,
) {
    tools.retain(|tool_id, _| {
        catalog.get(tool_id).is_some_and(|entry| {
            is_catalog_candidate(entry, workspace_readiness, runtime_catalog_state)
        })
    });
}

/// Keep only bindings for catalog-ready tools; reject enabled bindings on non-candidates.
pub fn normalize_agent_tool_bindings(
    catalog: &HashMap<String, ToolCatalogEntry>,
    workspace_readiness: &HashMap<String, ToolReadiness>,
    runtime_catalog_state: &RuntimeCatalogState,
    tools: &mut HashMap<String, AgentToolBinding>,
) -> crate::types::Result<()> {
    let mut to_remove = Vec::new();
    for (tool_id, binding) in tools.iter() {
        let Some(entry) = catalog.get(tool_id) else {
            if binding.enabled {
                return Err(crate::types::LitecodeError::Config(format!(
                    "agent tool binding '{tool_id}' does not exist in tool catalog"
                )));
            }
            to_remove.push(tool_id.clone());
            continue;
        };
        if binding.enabled
            && !is_catalog_candidate(entry, workspace_readiness, runtime_catalog_state)
        {
            return Err(crate::types::LitecodeError::Config(format!(
                "agent tool binding '{tool_id}' is enabled but tool is not catalog-ready (enable it in Tool Catalog first)"
            )));
        }
        if !is_catalog_candidate(entry, workspace_readiness, runtime_catalog_state) {
            to_remove.push(tool_id.clone());
        }
    }
    for id in to_remove {
        tools.remove(&id);
    }
    Ok(())
}

/// Strip subagent orchestration tools from subagent-role profiles (CONFIG §2.5).
pub fn strip_subagent_series_bindings(tools: &mut HashMap<String, AgentToolBinding>) {
    for tool_id in crate::config::schema::SUBAGENT_SERIES_TOOL_IDS {
        tools.remove(*tool_id);
    }
}

/// Normalize agent profile fields by role before persist.
pub fn normalize_agent_profile(agent_id: &str, profile: &mut crate::config::schema::AgentProfile) {
    use crate::config::schema::AgentRole;

    if profile.role == AgentRole::Hidden || agent_id == "compaction" {
        profile.tools.clear();
        profile.allowed_subagents.clear();
        return;
    }

    if profile.role != AgentRole::Primary {
        profile.allowed_subagents.clear();
    }

    if profile.role == AgentRole::Subagent {
        strip_subagent_series_bindings(&mut profile.tools);
    }
}

pub fn agent_tool_enabled(resolved: &ResolvedConfig, agent_id: &str, tool_id: &str) -> bool {
    resolved
        .agents()
        .get(agent_id)
        .and_then(|profile| profile.tools.get(tool_id))
        .is_some_and(|binding| binding.enabled)
}

/// Full LLM-list gate.
///
/// Workspace engines (`code_search` / `lsp`): agent binding ∧ engine warm.
/// Catalog readiness for those tools is derived from `engines.json`;
/// `catalog_enabled` still gates agent binding eligibility via
/// [`is_catalog_candidate`].
///
/// Global optional tools: catalog candidate ∧ agent binding ∧ engine warm.
pub fn should_include_in_llm_list(
    resolved: &ResolvedConfig,
    agent_id: &str,
    tool_id: &str,
    engines: &EngineManager,
    workspace_engines: &WorkspaceEngines,
) -> bool {
    let Some(entry) = resolved.tool_catalog().get(tool_id) else {
        return false;
    };
    if !agent_tool_enabled(resolved, agent_id, tool_id) {
        return false;
    }
    if crate::config::global_db::tools::is_workspace_optional(tool_id) {
        // Tool catalog depends on engine readiness; engine lifecycle does not
        // depend on the catalog.
        let configured = is_catalog_candidate(
            entry,
            resolved.workspace_tool_readiness(),
            resolved.runtime_catalog_state(),
        );
        // Keep the LSP tool visible while its workspace engine is warming so
        // the Agent receives a precise retryable Loading result rather than
        // silently losing the capability for the entire turn.
        return configured && (tool_id == "lsp" || workspace_engines.is_warmed(tool_id));
    }
    is_catalog_candidate(
        entry,
        resolved.workspace_tool_readiness(),
        resolved.runtime_catalog_state(),
    ) && engines.is_warmed(tool_id, resolved)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogInitFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogInitOutcome {
    pub initialized: Vec<String>,
    pub failed: Vec<CatalogInitFailure>,
}

/// Mark tools ready for the given init scope.
///
/// Global scope: writes readiness to `resolved.runtime_catalog_state` (process memory only).
/// Workspace scope: no-op for infrastructure engines (`code_search` / `lsp`) —
/// their readiness is owned by `.litecode/engines.json` via
/// [`crate::config::workspace::enable_code_search_engine`] /
/// [`crate::config::workspace::write_lsp_init`].
pub fn init(resolved: &mut ResolvedConfig, scope: InitScope) -> CatalogInitOutcome {
    match scope {
        InitScope::None => CatalogInitOutcome::default(),
        InitScope::Global => {
            let mut outcome = CatalogInitOutcome::default();
            let catalog_entries: Vec<(String, InitScope)> = resolved
                .tool_catalog()
                .values()
                .map(|e| (e.id.clone(), e.init_scope))
                .collect();
            for (id, init_scope) in catalog_entries {
                if init_scope != InitScope::Global
                    || resolved.runtime_catalog_state().readiness.get(&id).copied()
                        == Some(ToolReadiness::Ready)
                {
                    continue;
                }
                resolved
                    .runtime_catalog_state_mut()
                    .readiness
                    .insert(id.clone(), ToolReadiness::Ready);
                outcome.initialized.push(id);
            }
            outcome
        }
        InitScope::Workspace => {
            // Workspace infrastructure engines are not catalog-initialized.
            CatalogInitOutcome::default()
        }
    }
}

/// Refresh workspace-tool readiness from `engines.json` (boot / settings reconcile).
/// Does not enable engines and does not write `engines.json`.
pub fn refresh_workspace_engine_readiness(resolved: &mut ResolvedConfig) {
    let readiness =
        crate::config::workspace::workspace_readiness_from_engines(resolved.workspace_root());
    resolved.workspace_mut().workspace_tool_readiness = readiness;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::init_workspace;
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{AgentProfile, AgentToolBinding, GlobalSettings, ToolTier};
    use crate::engines::WorkspaceEngines;
    use crate::optional::EngineManager;
    use std::collections::HashMap;

    fn catalog_entry(id: &str, scope: InitScope) -> ToolCatalogEntry {
        ToolCatalogEntry {
            id: id.into(),
            tier: ToolTier::Optional,
            init_scope: scope,
            catalog_enabled: true,
        }
    }

    #[test]
    fn global_init_marks_global_scope_tools_ready() {
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "webfetch".into(),
            catalog_entry("webfetch", InitScope::Global),
        );
        let mut resolved = resolve(global, WorkspaceState::new("/tmp"));
        init(&mut resolved, InitScope::Global);
        assert_eq!(
            resolved.runtime_catalog_state().readiness.get("webfetch"),
            Some(&ToolReadiness::Ready)
        );
    }

    #[test]
    fn global_init_marks_websearch_ready_by_default() {
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "websearch".into(),
            catalog_entry("websearch", InitScope::Global),
        );
        let mut resolved = resolve(global, WorkspaceState::new("/tmp"));
        let outcome = init(&mut resolved, InitScope::Global);
        assert!(outcome.initialized.iter().any(|id| id == "websearch"));
        assert_eq!(
            resolved.runtime_catalog_state().readiness.get("websearch"),
            Some(&ToolReadiness::Ready)
        );
    }

    #[test]
    fn global_init_marks_websearch_ready_when_endpoint_overridden() {
        let mut global = GlobalSettings::default();
        global.websearch.search_endpoint = Some("http://localhost:8080".into());
        global.tool_catalog.insert(
            "websearch".into(),
            catalog_entry("websearch", InitScope::Global),
        );
        let mut resolved = resolve(global, WorkspaceState::new("/tmp"));
        let outcome = init(&mut resolved, InitScope::Global);
        assert!(outcome.initialized.iter().any(|id| id == "websearch"));
        assert_eq!(
            resolved.runtime_catalog_state().readiness.get("websearch"),
            Some(&ToolReadiness::Ready)
        );
    }

    #[test]
    fn engine_enable_sets_workspace_readiness_map() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        init_workspace(root).unwrap();
        crate::config::workspace::enable_code_search_engine(root).unwrap();
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "code_search".into(),
            catalog_entry("code_search", InitScope::Workspace),
        );
        let mut resolved = resolve(global, WorkspaceState::new(root));
        refresh_workspace_engine_readiness(&mut resolved);
        assert_eq!(
            resolved.workspace_tool_readiness().get("code_search"),
            Some(&ToolReadiness::Ready)
        );
    }

    #[test]
    fn replace_workspace_clears_workspace_scope_readiness() {
        let mut global = GlobalSettings::default();
        global
            .tool_catalog
            .insert("lsp".into(), catalog_entry("lsp", InitScope::Workspace));
        let mut resolved = resolve(global, WorkspaceState::new("/tmp/a"));
        resolved
            .workspace_mut()
            .workspace_tool_readiness
            .insert("lsp".into(), ToolReadiness::Ready);
        assert!(is_catalog_candidate(
            resolved.tool_catalog().get("lsp").unwrap(),
            resolved.workspace_tool_readiness(),
            resolved.runtime_catalog_state(),
        ));

        resolved.replace_workspace(WorkspaceState::new("/tmp/b"));
        assert!(!is_catalog_candidate(
            resolved.tool_catalog().get("lsp").unwrap(),
            resolved.workspace_tool_readiness(),
            resolved.runtime_catalog_state(),
        ));
    }

    #[test]
    fn normalize_agent_bindings_rejects_enabled_non_candidate() {
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "webfetch".into(),
            catalog_entry("webfetch", InitScope::Global),
        );
        let mut tools = HashMap::from([(
            "webfetch".into(),
            AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        )]);
        let runtime_state = RuntimeCatalogState::default();
        let err = normalize_agent_tool_bindings(
            &global.tool_catalog,
            &HashMap::new(),
            &runtime_state,
            &mut tools,
        )
        .unwrap_err();
        assert!(err.to_string().contains("catalog-ready"));
    }

    #[test]
    fn normalize_agent_bindings_prunes_disabled_non_candidate() {
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "webfetch".into(),
            catalog_entry("webfetch", InitScope::Global),
        );
        let mut tools = HashMap::from([(
            "webfetch".into(),
            AgentToolBinding {
                enabled: false,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        )]);
        let runtime_state = RuntimeCatalogState::default();
        normalize_agent_tool_bindings(
            &global.tool_catalog,
            &HashMap::new(),
            &runtime_state,
            &mut tools,
        )
        .unwrap();
        assert!(tools.is_empty());
    }

    #[test]
    fn llm_list_requires_catalog_and_binding() {
        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "read".into(),
            ToolCatalogEntry {
                id: "read".into(),
                tier: ToolTier::Core,
                init_scope: InitScope::None,
                catalog_enabled: true,
            },
        );
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: HashMap::from([(
                    "read".into(),
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
        let resolved = resolve(global, WorkspaceState::new("/tmp"));
        let engines = EngineManager::new();
        let workspace_engines = WorkspaceEngines::new();
        assert!(should_include_in_llm_list(
            &resolved,
            "default",
            "read",
            &engines,
            &workspace_engines
        ));

        // Construct a new ResolvedConfig with the agent tool disabled for testing.
        let mut disabled_global = resolved.global().clone();
        disabled_global
            .agents
            .get_mut("default")
            .unwrap()
            .tools
            .get_mut("read")
            .unwrap()
            .enabled = false;
        let disabled = resolve(disabled_global, WorkspaceState::new("/tmp"));
        assert!(!should_include_in_llm_list(
            &disabled,
            "default",
            "read",
            &engines,
            &workspace_engines
        ));
    }

    #[test]
    fn llm_list_requires_engine_warmup_for_optional() {
        use crate::config::schema::ToolPreset;

        let mut global = GlobalSettings::default();
        global.tool_catalog.insert(
            "webfetch".into(),
            ToolCatalogEntry {
                id: "webfetch".into(),
                tier: ToolTier::Optional,
                init_scope: InitScope::Global,
                catalog_enabled: true,
            },
        );
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: HashMap::from([(
                    "webfetch".into(),
                    AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::presets::binding_for_tool(
                            "webfetch",
                            ToolPreset::All,
                        )
                        .0,
                        path_mode: crate::permission::presets::binding_for_tool(
                            "webfetch",
                            ToolPreset::All,
                        )
                        .1,
                        last_applied_preset: Some(ToolPreset::All),
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let mut resolved = resolve(global, WorkspaceState::new("/tmp"));
        // Init global scope to mark webfetch ready in runtime_catalog_state
        init(&mut resolved, InitScope::Global);
        let engines = EngineManager::new();
        let workspace_engines = WorkspaceEngines::new();
        assert!(!should_include_in_llm_list(
            &resolved,
            "default",
            "webfetch",
            &engines,
            &workspace_engines
        ));
        engines.reconcile(&resolved);
        assert!(should_include_in_llm_list(
            &resolved,
            "default",
            "webfetch",
            &engines,
            &workspace_engines
        ));
        assert!(engines.is_warmed("webfetch", &resolved));
    }
}
