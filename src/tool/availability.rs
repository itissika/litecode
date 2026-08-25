//! Bind-card availability: definitions present in this workspace.

use crate::config::global_db::tools::{
    core_tool_ids, is_mcp_catalog_id, is_workspace_optional, mcp_catalog_id, optional_builtin_ids,
};
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AvailableKind, AvailableTool, ToolOrigin, ToolReadiness};
use crate::engines::WorkspaceEngines;
use crate::optional::EngineManager;

pub fn is_available(resolved: &ResolvedConfig, tool_id: &str) -> bool {
    if core_tool_ids().iter().any(|id| id == tool_id) {
        return true;
    }
    if is_workspace_optional(tool_id) {
        return resolved.workspace_tool_readiness().get(tool_id).copied()
            == Some(ToolReadiness::Ready);
    }
    if let Some(server_id) = tool_id.strip_prefix("mcp_") {
        return resolved.mcp_servers().contains_key(server_id);
    }
    resolved
        .custom_tools()
        .iter()
        .any(|tool| tool.name == tool_id)
}

pub fn available_tools(resolved: &ResolvedConfig) -> Vec<AvailableTool> {
    let mut out = Vec::new();
    for id in core_tool_ids() {
        out.push(AvailableTool {
            id,
            kind: AvailableKind::Core,
            origin: ToolOrigin::Builtin,
            overridden: false,
        });
    }
    for id in optional_builtin_ids() {
        if resolved.workspace_tool_readiness().get(*id).copied() == Some(ToolReadiness::Ready) {
            out.push(AvailableTool {
                id: (*id).to_string(),
                kind: AvailableKind::Engine,
                origin: ToolOrigin::Workspace,
                overridden: false,
            });
        }
    }
    let custom = resolved.custom_tools();
    for tool in custom {
        let origin = resolved
            .custom_origin(&tool.name)
            .unwrap_or(ToolOrigin::Global);
        let overridden = origin == ToolOrigin::Workspace
            && resolved
                .global_custom_tools()
                .iter()
                .any(|t| t.name == tool.name);
        out.push(AvailableTool {
            id: tool.name,
            kind: AvailableKind::Custom,
            origin,
            overridden,
        });
    }
    let mut mcp_ids: Vec<String> = resolved.mcp_servers().keys().cloned().collect();
    mcp_ids.sort();
    for server_id in mcp_ids {
        let origin = resolved
            .mcp_origin(&server_id)
            .unwrap_or(ToolOrigin::Global);
        let overridden = origin == ToolOrigin::Workspace
            && resolved.global_mcp_servers().contains_key(&server_id);
        out.push(AvailableTool {
            id: mcp_catalog_id(&server_id),
            kind: AvailableKind::Mcp,
            origin,
            overridden,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn agent_tool_enabled(resolved: &ResolvedConfig, agent_id: &str, tool_id: &str) -> bool {
    resolved
        .agents()
        .get(agent_id)
        .and_then(|profile| profile.tools.get(tool_id))
        .is_some_and(|binding| binding.enabled)
}

/// Full LLM-list gate: bound on this agent and currently available.
pub fn should_include_in_llm_list(
    resolved: &ResolvedConfig,
    agent_id: &str,
    tool_id: &str,
    _engines: &EngineManager,
    _workspace_engines: &WorkspaceEngines,
) -> bool {
    if !is_available(resolved, tool_id) {
        return false;
    }
    if !agent_tool_enabled(resolved, agent_id, tool_id) {
        return false;
    }
    if crate::config::schema::SUBAGENT_SERIES_TOOL_IDS
        .iter()
        .any(|id| *id == tool_id)
        || tool_id.starts_with("subagent_")
    {
        return true;
    }
    true
}

pub fn is_subagent_series(tool_id: &str) -> bool {
    crate::config::schema::SUBAGENT_SERIES_TOOL_IDS.contains(&tool_id)
        || tool_id.starts_with("subagent_")
}

pub fn is_mcp_tool_id(tool_id: &str) -> bool {
    is_mcp_catalog_id(tool_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigManager;
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{GlobalSettings, McpServerDefinition, McpTransport};

    #[test]
    fn workspace_mcp_overrides_global_command() {
        let mut global = GlobalSettings::default();
        global.mcp_servers.insert(
            "echo".into(),
            McpServerDefinition {
                command: "global-cmd".into(),
                args: vec![],
                env: Default::default(),
                transport: McpTransport::Stdio,
                ..Default::default()
            },
        );
        let mut workspace = WorkspaceState::new("/tmp/avail-ws");
        workspace.workspace_mcp_servers.insert(
            "echo".into(),
            McpServerDefinition {
                command: "workspace-cmd".into(),
                args: vec![],
                env: Default::default(),
                transport: McpTransport::Stdio,
                ..Default::default()
            },
        );
        let resolved = ConfigManager::resolve(global, workspace);
        assert_eq!(
            resolved.mcp_servers().get("echo").unwrap().command,
            "workspace-cmd"
        );
        assert_eq!(resolved.mcp_pool_key("echo"), "workspace:echo");
        let card = available_tools(&resolved)
            .into_iter()
            .find(|t| t.id == "mcp_echo")
            .expect("card");
        assert!(card.overridden);
        assert_eq!(card.origin, ToolOrigin::Workspace);
    }

    #[test]
    fn core_webfetch_available_without_bind() {
        let resolved =
            ConfigManager::resolve(GlobalSettings::default(), WorkspaceState::new("/tmp/core"));
        assert!(is_available(&resolved, "webfetch"));
        assert!(is_available(&resolved, "websearch"));
        assert!(!agent_tool_enabled(&resolved, "default", "webfetch"));
    }
}
