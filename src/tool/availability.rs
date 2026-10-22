//! Bind-card availability: definitions present in this workspace.

use std::sync::Arc;

use crate::config::global_db::tools::{
    core_tool_ids, is_mcp_catalog_id, is_workspace_optional, mcp_catalog_id, optional_builtin_ids,
};
use crate::config::resolved::ResolvedConfig;
use serde::Serialize;

use crate::config::schema::{
    AvailableKind, AvailableTool, CallSurface, PermissionSurface, ToolOrigin, ToolReadiness,
};
use crate::engines::WorkspaceEngines;
use crate::optional::EngineManager;
use crate::permission::permission_surface;
use crate::tool::project::{RejectedTool, project_tools};
use crate::tool::trait_::{Tool, ToolSurface};

/// Settings switches never list the code transport. Same name as
/// `crate::tools::code::CODE_TOOL_NAME`.
const CODE_TRANSPORT_ID: &str = "run_code";

pub fn is_available(resolved: &ResolvedConfig, tool_id: &str) -> bool {
    if core_tool_ids().iter().any(|id| id == tool_id) {
        return true;
    }
    // WP-148: code_search visibility is bind-only (engine state never gates it).
    if tool_id == "code_search" {
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

fn series_for(tool_id: &str) -> (Option<String>, Option<String>) {
    for (series_id, series_label, members) in crate::config::schema::core_tool_enable_series() {
        if members.contains(&tool_id) {
            return (
                Some((*series_id).to_string()),
                Some((*series_label).to_string()),
            );
        }
    }
    (None, None)
}

fn tool_card(
    id: String,
    kind: AvailableKind,
    origin: ToolOrigin,
    overridden: bool,
    surface: Option<PermissionSurface>,
    series_id: Option<String>,
    series_label: Option<String>,
) -> AvailableTool {
    let permission_surface = surface.unwrap_or_else(|| permission_surface(&id));
    let (series_id, series_label) = if series_id.is_some() {
        (series_id, series_label)
    } else {
        series_for(&id)
    };
    AvailableTool {
        id,
        kind,
        origin,
        overridden,
        permission_surface,
        surface: CallSurface::Direct,
        series_id,
        series_label,
    }
}

pub fn available_tools(resolved: &ResolvedConfig) -> Vec<AvailableTool> {
    let mut out = Vec::new();
    for id in core_tool_ids() {
        out.push(tool_card(
            id,
            AvailableKind::Core,
            ToolOrigin::Builtin,
            false,
            None,
            None,
            None,
        ));
    }
    for id in optional_builtin_ids() {
        if *id == "code_search"
            || resolved.workspace_tool_readiness().get(*id).copied() == Some(ToolReadiness::Ready)
        {
            out.push(tool_card(
                (*id).to_string(),
                AvailableKind::Engine,
                ToolOrigin::Workspace,
                false,
                None,
                None,
                None,
            ));
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
        let surface = if tool.rules.is_empty() {
            PermissionSurface::Fixed
        } else {
            PermissionSurface::Preset
        };
        let series_id = tool.suite.clone();
        let series_label = tool.suite_label.clone().or_else(|| tool.suite.clone());
        out.push(tool_card(
            tool.name,
            AvailableKind::Custom,
            origin,
            overridden,
            Some(surface),
            series_id,
            series_label,
        ));
    }
    let mut mcp_ids: Vec<String> = resolved.mcp_servers().keys().cloned().collect();
    mcp_ids.sort();
    for server_id in mcp_ids {
        let origin = resolved
            .mcp_origin(&server_id)
            .unwrap_or(ToolOrigin::Global);
        let overridden = origin == ToolOrigin::Workspace
            && resolved.global_mcp_servers().contains_key(&server_id);
        out.push(tool_card(
            mcp_catalog_id(&server_id),
            AvailableKind::Mcp,
            origin,
            overridden,
            None,
            None,
            None,
        ));
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Settings catalog plus code-admission failures for the same response.
///
/// `code_candidates` is the caller's visible code-surface set (builtins today;
/// a non-builtin test tool is the same projection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvailableToolsPayload {
    pub tools: Vec<AvailableTool>,
    pub code_rejections: Vec<RejectedTool>,
}

pub fn available_tools_payload(
    resolved: &ResolvedConfig,
    code_candidates: Vec<Arc<dyn Tool>>,
) -> AvailableToolsPayload {
    let mut tools = available_tools(resolved);
    omit_code_transport(&mut tools);
    stamp_declared_surfaces(&mut tools, &code_candidates);
    AvailableToolsPayload {
        tools,
        code_rejections: project_tools(code_candidates).rejected,
    }
}

fn omit_code_transport(tools: &mut Vec<AvailableTool>) {
    tools.retain(|tool| tool.id != CODE_TRANSPORT_ID);
}

/// Copy each candidate's own [`Tool::surface`] onto the matching catalog card.
/// A candidate that is not already in the catalog does not add a card.
fn stamp_declared_surfaces(tools: &mut [AvailableTool], candidates: &[Arc<dyn Tool>]) {
    for tool in tools {
        let Some(declared) = candidates
            .iter()
            .find(|candidate| candidate.name() == tool.id && !candidate.is_code_transport())
        else {
            continue;
        };
        tool.surface = match declared.surface() {
            ToolSurface::Code => CallSurface::Code,
            ToolSurface::Direct => CallSurface::Direct,
        };
    }
}

pub fn agent_tool_enabled(resolved: &ResolvedConfig, agent_id: &str, tool_id: &str) -> bool {
    resolved
        .agents()
        .get(agent_id)
        .and_then(|profile| profile.tools.get(tool_id))
        .is_some_and(|binding| binding.enabled)
}

/// session_search keeps its WP-057 gate: `desired=false` or not Warm ⇒ hidden.
/// WP-148: code_search left this bucket (bind is its sole visibility gate).
fn is_session_search_gated(tool_id: &str) -> bool {
    tool_id == "session_search"
}

fn session_search_visible(resolved: &ResolvedConfig, workspace_engines: &WorkspaceEngines) -> bool {
    if !crate::config::workspace::workspace_engine_desired(resolved.workspace_root(), "code_search")
    {
        return false;
    }
    // IDE signal: Warm + call_gate Ready (silent refresh still searchable).
    workspace_engines.is_searchable("code_search")
}

/// Full LLM-list gate: bound on this agent and currently available.
pub fn should_include_in_llm_list(
    resolved: &ResolvedConfig,
    agent_id: &str,
    tool_id: &str,
    _engines: &EngineManager,
    workspace_engines: &WorkspaceEngines,
) -> bool {
    if !is_available(resolved, tool_id) {
        return false;
    }
    if !agent_tool_enabled(resolved, agent_id, tool_id) {
        return false;
    }
    if is_session_search_gated(tool_id) && !session_search_visible(resolved, workspace_engines) {
        return false;
    }
    if crate::config::schema::SUBAGENT_SERIES_TOOL_IDS.contains(&tool_id)
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
    
    
    
    


}
