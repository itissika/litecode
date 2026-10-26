//! Capability catalog: the single description of what can be bound and which
//! concrete model functions sit behind each card.

use std::collections::{BTreeMap, HashMap};

use crate::config::binding_scope::{scope_of, BindingScope};
use crate::config::global_db::tools::{
    core_configurable_tools, core_none_tools, mcp_catalog_id, network_core_tools,
    optional_builtin_ids,
};
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{
    AgentRole, AvailableKind, CallSurface, CustomToolDefinition, PermissionSurface, ToolOrigin,
    SUBAGENT_CAPABILITY_ID, SUBAGENT_MEMBER_TOOL_IDS,
};
use crate::permission::has_permission_preset;
use crate::tools::lsp::{LSP_CARD_ID, LSP_CODE_TOOL_NAMES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityKind {
    Core,
    Engine,
    Custom,
    Mcp,
}

impl CapabilityKind {
    pub fn available_kind(self) -> AvailableKind {
        match self {
            Self::Core => AvailableKind::Core,
            Self::Engine => AvailableKind::Engine,
            Self::Custom => AvailableKind::Custom,
            Self::Mcp => AvailableKind::Mcp,
        }
    }
}

/// How a selected capability is instantiated. Builtin also covers the
/// engine-backed builtins; their adapters are looked up by member name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySource {
    Builtin,
    Lsp,
    Subagent,
    Custom,
    Mcp { server_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSpec {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    pub id: String,
    pub kind: CapabilityKind,
    pub source: CapabilitySource,
    pub members: Vec<MemberSpec>,
    pub scope: BindingScope,
    pub origin: ToolOrigin,
    pub overridden: bool,
    pub permission_surface: PermissionSurface,
    pub surface: CallSurface,
    /// Human label for a grouped custom suite. Other cards use their id.
    pub label: Option<String>,
}

impl Capability {
    pub fn member_ids(&self) -> impl Iterator<Item = &str> {
        self.members.iter().map(|member| member.id.as_str())
    }
}

/// The complete catalog for one resolved workspace. Errors is non-empty when
/// two capabilities claim the same id, or two capabilities claim the same
/// member name. Callers must surface it; selection still uses the first owner
/// so one malformed custom definition cannot make the whole turn unusable.
#[derive(Debug, Clone, Default)]
pub struct CapabilityCatalog {
    pub capabilities: Vec<Capability>,
    pub member_owner: HashMap<String, String>,
    pub errors: Vec<String>,
}

impl CapabilityCatalog {
    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|capability| capability.id == id)
    }

    pub fn selected_for_agent<'a>(
        &'a self,
        resolved: &ResolvedConfig,
        agent_id: &str,
        depth: u32,
    ) -> Vec<&'a Capability> {
        let role = resolved
            .agents()
            .get(agent_id)
            .map(|profile| profile.role)
            .unwrap_or(AgentRole::Primary);
        let profile = resolved.agents().get(agent_id);
        self.capabilities
            .iter()
            .filter(|capability| {
                capability.scope.allows(role, depth)
                    && profile
                        .and_then(|agent| agent.tools.get(&capability.id))
                        .is_some_and(|binding| binding.enabled)
            })
            .collect()
    }

    pub fn member_capability_id(&self, member: &str) -> Option<&str> {
        self.member_owner.get(member).map(String::as_str)
    }
}

fn member(id: &str) -> MemberSpec {
    MemberSpec { id: id.to_string() }
}

fn builtin_surface(id: &str) -> CallSurface {
    match id {
        "read" | "grep" | "glob" | "write" | "edit" | "apply_edits" | "run" | "wait" | "kill"
        | "webfetch" | "websearch" => CallSurface::Code,
        _ => CallSurface::Direct,
    }
}

fn custom_overridden(resolved: &ResolvedConfig, name: &str) -> bool {
    resolved.workspace_custom_tools().contains_key(name)
        && resolved
            .global_custom_tools()
            .iter()
            .any(|tool| tool.name == name)
}

fn custom_permission_surface(tools: &[CustomToolDefinition]) -> PermissionSurface {
    if tools.iter().any(|tool| !tool.rules.is_empty()) {
        PermissionSurface::Preset
    } else {
        PermissionSurface::Fixed
    }
}

fn push_capability(catalog: &mut CapabilityCatalog, capability: Capability) {
    if catalog
        .capabilities
        .iter()
        .any(|existing| existing.id == capability.id)
    {
        catalog.errors.push(format!(
            "capability id '{}' is declared more than once",
            capability.id
        ));
        return;
    }
    for member in capability.member_ids() {
        if let Some(owner) = catalog.member_owner.get(member) {
            catalog.errors.push(format!(
                "member '{member}' is claimed by both '{owner}' and '{}'",
                capability.id
            ));
        } else {
            catalog
                .member_owner
                .insert(member.to_string(), capability.id.clone());
        }
    }
    catalog.capabilities.push(capability);
}

fn push_builtin(
    catalog: &mut CapabilityCatalog,
    _resolved: &ResolvedConfig,
    id: &str,
    members: &[&str],
    kind: CapabilityKind,
    origin: ToolOrigin,
) {
    push_capability(
        catalog,
        Capability {
            id: id.to_string(),
            kind,
            source: CapabilitySource::Builtin,
            members: members.iter().map(|member_id| member(member_id)).collect(),
            scope: scope_of(id),
            origin,
            overridden: false,
            permission_surface: if has_permission_preset(id) {
                PermissionSurface::Preset
            } else {
                PermissionSurface::Fixed
            },
            surface: builtin_surface(id),
            label: None,
        },
    );
}

fn push_custom(
    catalog: &mut CapabilityCatalog,
    resolved: &ResolvedConfig,
    id: String,
    members: Vec<CustomToolDefinition>,
    label: Option<String>,
) {
    let origin = members
        .iter()
        .find_map(|tool| resolved.custom_origin(&tool.name))
        .unwrap_or(ToolOrigin::Global);
    let overridden = members
        .iter()
        .any(|tool| custom_overridden(resolved, &tool.name));
    let surface = if members
        .iter()
        .all(|tool| tool.surface == CallSurface::Code)
    {
        CallSurface::Code
    } else {
        CallSurface::Direct
    };
    let scope = scope_of(&id);
    push_capability(
        catalog,
        Capability {
            id,
            kind: CapabilityKind::Custom,
            source: CapabilitySource::Custom,
            members: members
                .iter()
                .map(|tool| member(&tool.name))
                .collect(),
            scope,
            origin,
            overridden,
            permission_surface: custom_permission_surface(&members),
            surface,
            label,
        },
    );
}

fn push_mcp(catalog: &mut CapabilityCatalog, resolved: &ResolvedConfig, server_id: &str) {
    let id = mcp_catalog_id(server_id);
    let origin = resolved
        .mcp_origin(server_id)
        .unwrap_or(ToolOrigin::Global);
    let overridden = origin == ToolOrigin::Workspace
        && resolved.global_mcp_servers().contains_key(server_id);
    push_capability(
        catalog,
        Capability {
            id: id.clone(),
            kind: CapabilityKind::Mcp,
            source: CapabilitySource::Mcp {
                server_id: server_id.to_string(),
            },
            // MCP members are discovered after the handshake. The binding id
            // remains the stable card; the turn fills the members dynamically.
            members: Vec::new(),
            scope: scope_of(&id),
            origin,
            overridden,
            permission_surface: PermissionSurface::Fixed,
            surface: CallSurface::Direct,
            label: None,
        },
    );
}

fn push_custom_suites(catalog: &mut CapabilityCatalog, resolved: &ResolvedConfig) {
    let mut suites: BTreeMap<String, Vec<CustomToolDefinition>> = BTreeMap::new();
    for tool in resolved.custom_tools() {
        match tool
            .suite
            .as_deref()
            .map(str::trim)
            .filter(|suite| !suite.is_empty())
        {
            Some(suite) => suites.entry(suite.to_string()).or_default().push(tool),
            None => push_custom(catalog, resolved, tool.name.clone(), vec![tool], None),
        }
    }
    for (suite, members) in suites {
        let label = members
            .iter()
            .find_map(|tool| tool.suite_label.clone())
            .or_else(|| Some(suite.clone()));
        push_custom(catalog, resolved, format!("suite:{suite}"), members, label);
    }
}

/// Build the one catalog consumed by settings and turn assembly.
pub fn capability_catalog(resolved: &ResolvedConfig) -> CapabilityCatalog {
    let mut catalog = CapabilityCatalog::default();

    for id in core_configurable_tools() {
        match *id {
            "run" => push_builtin(
                &mut catalog,
                resolved,
                "run",
                &["run", "wait", "kill"],
                CapabilityKind::Core,
                ToolOrigin::Builtin,
            ),
            "edit" => push_builtin(
                &mut catalog,
                resolved,
                "edit",
                &["edit", "apply_edits"],
                CapabilityKind::Core,
                ToolOrigin::Builtin,
            ),
            _ => push_builtin(
                &mut catalog,
                resolved,
                id,
                &[*id],
                CapabilityKind::Core,
                ToolOrigin::Builtin,
            ),
        }
    }

    for id in core_none_tools() {
        if *id == SUBAGENT_CAPABILITY_ID {
            push_capability(
                &mut catalog,
                Capability {
                    id: (*id).to_string(),
                    kind: CapabilityKind::Core,
                    source: CapabilitySource::Subagent,
                    members: SUBAGENT_MEMBER_TOOL_IDS
                        .iter()
                        .map(|member_id| member(member_id))
                        .collect(),
                    scope: BindingScope::PrimaryRootOnly,
                    origin: ToolOrigin::Builtin,
                    overridden: false,
                    permission_surface: PermissionSurface::Fixed,
                    surface: CallSurface::Direct,
                    label: None,
                },
            );
        } else {
            push_builtin(
                &mut catalog,
                resolved,
                id,
                &[*id],
                CapabilityKind::Core,
                ToolOrigin::Builtin,
            );
        }
    }

    for id in network_core_tools() {
        push_builtin(
            &mut catalog,
            resolved,
            id,
            &[*id],
            CapabilityKind::Core,
            ToolOrigin::Builtin,
        );
    }

    for id in optional_builtin_ids() {
        match *id {
            LSP_CARD_ID => push_capability(
                &mut catalog,
                Capability {
                    id: LSP_CARD_ID.to_string(),
                    kind: CapabilityKind::Engine,
                    source: CapabilitySource::Lsp,
                    members: LSP_CODE_TOOL_NAMES
                        .iter()
                        .map(|member_id| member(member_id))
                        .collect(),
                    scope: scope_of(LSP_CARD_ID),
                    origin: ToolOrigin::Workspace,
                    overridden: false,
                    permission_surface: PermissionSurface::Fixed,
                    surface: CallSurface::Code,
                    label: None,
                },
            ),
            _ => push_builtin(
                &mut catalog,
                resolved,
                id,
                &[*id],
                CapabilityKind::Engine,
                ToolOrigin::Workspace,
            ),
        }
    }

    push_custom_suites(&mut catalog, resolved);

    let mut server_ids: Vec<String> = resolved.mcp_servers().keys().cloned().collect();
    server_ids.sort();
    for server_id in server_ids {
        push_mcp(&mut catalog, resolved, &server_id);
    }

    catalog
        .capabilities
        .sort_by(|left, right| left.id.cmp(&right.id));
    catalog
}

/// Reverse ownership for execution and permissions. This is the same map the
/// catalog builds, so a member can never be selected without its owning card.
pub fn member_owner(resolved: &ResolvedConfig, member_name: &str) -> Option<String> {
    capability_catalog(resolved)
        .member_capability_id(member_name)
        .map(str::to_string)
}

/// Phase 1 bridge retained for callers without a resolved config.
pub fn member_capability_id(name: &str) -> Option<&'static str> {
    SUBAGENT_MEMBER_TOOL_IDS
        .contains(&name)
        .then_some(SUBAGENT_CAPABILITY_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolved::resolve_without_catalog;
    use crate::config::schema::{AgentProfile, AgentToolBinding, GlobalSettings};
    use crate::config::WorkspaceState;
    use std::collections::HashMap;

    fn resolved() -> ResolvedConfig {
        resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new("/tmp"))
    }

    fn custom(name: &str, suite: Option<&str>, label: Option<&str>) -> CustomToolDefinition {
        CustomToolDefinition {
            name: name.into(),
            description: String::new(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 30,
            rules: Vec::new(),
            surface: CallSurface::Direct,
            suite: suite.map(str::to_string),
            suite_label: label.map(str::to_string),
            data_schema: None,
        }
    }

    #[test]
    fn catalog_has_one_capability_per_card_and_one_reverse_map() {
        let catalog = capability_catalog(&resolved());
        for id in [
            "run",
            "edit",
            "read",
            "subagent_tools",
            "lsp",
            "code_search",
            "session_search",
        ] {
            assert!(catalog.get(id).is_some(), "missing {id}");
        }
        let run = catalog.get("run").expect("run");
        assert_eq!(
            run.member_ids().collect::<Vec<_>>(),
            ["run", "wait", "kill"]
        );
        assert_eq!(catalog.member_capability_id("wait"), Some("run"));
        assert_eq!(catalog.member_capability_id("apply_edits"), Some("edit"));
        assert_eq!(
            catalog.member_capability_id("subagent_launch"),
            Some("subagent_tools")
        );
        assert!(catalog.get("wait").is_none());
        assert!(catalog.get("apply_edits").is_none());
        assert!(catalog.errors.is_empty(), "{:?}", catalog.errors);
    }

    #[test]
    fn lsp_is_one_capability_with_seven_members() {
        let catalog = capability_catalog(&resolved());
        let lsp = catalog.get("lsp").expect("lsp");
        assert_eq!(lsp.members.len(), 7);
        assert!(lsp.member_ids().any(|name| name == "hover"));
        assert!(lsp.member_ids().any(|name| name == "go_to_definition"));
    }

    #[test]
    fn custom_suite_is_one_capability_and_single_tool_is_one_member() {
        let mut global = GlobalSettings::default();
        global.custom_tools.push(custom("suite_a", Some("deck"), Some("Deck")));
        global.custom_tools.push(custom("suite_b", Some("deck"), None));
        global.custom_tools.push(custom("single", None, None));
        let catalog = capability_catalog(&resolve_without_catalog(
            global,
            WorkspaceState::new("/tmp"),
        ));
        let deck = catalog.get("suite:deck").expect("suite card");
        assert_eq!(deck.label.as_deref(), Some("Deck"));
        assert_eq!(deck.member_ids().collect::<Vec<_>>(), ["suite_a", "suite_b"]);
        assert!(catalog.get("single").is_some());
        assert_eq!(catalog.member_capability_id("suite_a"), Some("suite:deck"));
    }

    #[test]
    fn mcp_server_is_one_card_with_dynamic_members() {
        let mut global = GlobalSettings::default();
        global.mcp_servers.insert(
            "demo".into(),
            crate::config::schema::McpServerDefinition {
                command: "echo".into(),
                ..Default::default()
            },
        );
        let catalog = capability_catalog(&resolve_without_catalog(
            global,
            WorkspaceState::new("/tmp"),
        ));
        let card = catalog.get("mcp_demo").expect("mcp card");
        assert_eq!(card.kind, CapabilityKind::Mcp);
        assert!(card.members.is_empty());
        assert_eq!(card.scope, BindingScope::All);
    }

    #[test]
    fn duplicate_member_names_are_reported_not_merged() {
        let mut global = GlobalSettings::default();
        global.custom_tools.push(custom("hover", None, None));
        let catalog = capability_catalog(&resolve_without_catalog(
            global,
            WorkspaceState::new("/tmp"),
        ));
        assert!(
            catalog.errors.iter().any(|error| error.contains("hover")),
            "{:?}",
            catalog.errors
        );
    }

    #[test]
    fn every_card_is_selectable_for_primary_at_root() {
        let base = resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new("/tmp"));
        let ids: Vec<String> = capability_catalog(&base)
            .capabilities
            .iter()
            .map(|capability| capability.id.clone())
            .collect();
        let mut tools = HashMap::new();
        for id in &ids {
            tools.insert(
                id.clone(),
                AgentToolBinding {
                    enabled: true,
                    policy: crate::permission::ToolPolicy::allow_all(),
                    path_mode: crate::permission::BindingPathMode::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            );
        }
        let mut agents = base.agents().clone();
        agents.insert(
            "default".into(),
            AgentProfile {
                tools,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(
            GlobalSettings {
                agents,
                ..GlobalSettings::default()
            },
            WorkspaceState::new("/tmp"),
        );
        let catalog = capability_catalog(&resolved);
        let selected: Vec<String> = catalog
            .selected_for_agent(&resolved, "default", 0)
            .iter()
            .map(|capability| capability.id.clone())
            .collect();
        let expected: Vec<String> = catalog
            .capabilities
            .iter()
            .map(|capability| capability.id.clone())
            .collect();
        assert_eq!(selected, expected);
    }

    #[test]
    fn selection_is_binding_and_scope_only() {
        let mut global = GlobalSettings::default();
        let mut tools = HashMap::new();
        tools.insert(
            "session_search".into(),
            AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        );
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let catalog = capability_catalog(&resolved);
        let selected = catalog.selected_for_agent(&resolved, "default", 0);
        assert!(selected.iter().any(|capability| capability.id == "session_search"));
        assert!(!selected.iter().any(|capability| capability.id == "lsp"));
    }
}
