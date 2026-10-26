//! Settings projection of the capability catalog. This module does not
//! decide visibility; it only maps catalog entries to the wire DTO.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AvailableStatus, AvailableTool};
use crate::tool::capability::{Capability, capability_catalog, member_owner};
use crate::tool::project::{RejectedTool, project_tools};
use crate::tool::trait_::Tool;
use serde::Serialize;

fn project_capability(capability: &Capability) -> AvailableTool {
    AvailableTool {
        id: capability.id.clone(),
        kind: capability.kind.available_kind(),
        origin: capability.origin,
        overridden: capability.overridden,
        permission_surface: capability.permission_surface,
        surface: capability.surface,
        scope: capability.scope,
        members: capability.member_ids().map(str::to_string).collect(),
        label: capability.label.clone(),
        status: None,
    }
}

/// All cards declared in this workspace. Engine state is intentionally absent:
/// cards are definitions, not runtime readiness.
pub fn available_tools(resolved: &ResolvedConfig) -> Vec<AvailableTool> {
    project_catalog(&capability_catalog(resolved))
}

fn project_catalog(catalog: &crate::tool::capability::CapabilityCatalog) -> Vec<AvailableTool> {
    let mut out: Vec<AvailableTool> = catalog
        .capabilities
        .iter()
        .map(project_capability)
        .collect();
    out.sort_by(|left, right| left.id.cmp(&right.id));
    out
}

/// Settings catalog plus code-admission failures for the same response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvailableToolsPayload {
    pub tools: Vec<AvailableTool>,
    pub code_rejections: Vec<RejectedTool>,
    /// Definitions rejected while building the catalog (duplicate ids/members).
    /// Turn assembly fails on these; settings surfaces them instead of hiding rows.
    pub catalog_errors: Vec<String>,
}

pub fn available_tools_payload(
    resolved: &ResolvedConfig,
    code_candidates: Vec<Arc<dyn Tool>>,
    statuses: HashMap<String, AvailableStatus>,
) -> AvailableToolsPayload {
    let catalog = capability_catalog(resolved);
    let mut tools = project_catalog(&catalog);
    for tool in &mut tools {
        tool.status = statuses.get(&tool.id).copied();
    }
    AvailableToolsPayload {
        tools,
        code_rejections: project_tools(code_candidates).rejected,
        catalog_errors: catalog.errors,
    }
}

/// A catalog id (or a member name) exists in this workspace.
pub fn is_available(resolved: &ResolvedConfig, tool_id: &str) -> bool {
    let catalog = capability_catalog(resolved);
    catalog.get(tool_id).is_some() || catalog.member_capability_id(tool_id).is_some()
}

pub fn agent_tool_enabled(resolved: &ResolvedConfig, agent_id: &str, tool_id: &str) -> bool {
    let bind_id = member_owner(resolved, tool_id).unwrap_or_else(|| tool_id.to_string());
    resolved
        .agents()
        .get(agent_id)
        .and_then(|profile| profile.tools.get(&bind_id))
        .is_some_and(|binding| binding.enabled)
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspaceState;
    use crate::config::resolved::resolve_without_catalog;
    use crate::config::schema::GlobalSettings;

    fn resolved() -> ResolvedConfig {
        resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new("/tmp"))
    }

    #[test]
    fn settings_has_one_lsp_card_with_seven_members() {
        let tools = available_tools(&resolved());
        let lsp = tools.iter().find(|tool| tool.id == "lsp").expect("lsp");
        assert_eq!(lsp.members.len(), 7);
        assert_eq!(lsp.surface, crate::config::schema::CallSurface::Code);
    }

    #[test]
    fn code_transport_is_not_a_card() {
        let tools = available_tools(&resolved());
        assert!(!tools.iter().any(|tool| tool.id == "run_code"));
    }

    #[test]
    fn status_is_projected_without_changing_the_card_set() {
        let resolved = resolved();
        let statuses = HashMap::from([("lsp".to_string(), AvailableStatus::Failed)]);
        let payload = available_tools_payload(&resolved, Vec::new(), statuses);
        let lsp = payload
            .tools
            .iter()
            .find(|tool| tool.id == "lsp")
            .expect("lsp card");
        assert_eq!(lsp.status, Some(AvailableStatus::Failed));
        assert_eq!(payload.tools.len(), available_tools(&resolved).len());
    }

    #[test]
    fn member_names_resolve_to_their_owning_card() {
        let resolved = resolved();
        assert!(is_available(&resolved, "wait"));
        assert_eq!(member_owner(&resolved, "wait").as_deref(), Some("run"));
        assert_eq!(
            member_owner(&resolved, "apply_edits").as_deref(),
            Some("edit")
        );
    }
}
