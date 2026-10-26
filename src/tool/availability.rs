//! Settings projection of the capability catalog. This module does not
//! decide visibility; it only maps catalog entries to the wire DTO.

use std::collections::HashMap;

use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AvailableStatus, AvailableTool};
use crate::tool::capability::{Capability, capability_catalog, member_owner};
use crate::tool::project::{RejectedTool, project_code_specs};
use crate::tool::spec::CodeFunctionSpec;
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
    /// Turn assembly logs these and keeps the first owner; settings projects the
    /// same catalog instead of hiding rows.
    pub catalog_errors: Vec<String>,
    /// Invalid custom definitions from both layers. They stay out of the
    /// catalog but must remain visible in Settings.
    pub custom_diagnostics: Vec<crate::config::schema::CustomToolDiagnostic>,
}

pub fn available_tools_payload(
    resolved: &ResolvedConfig,
    code_candidates: Vec<CodeFunctionSpec>,
    statuses: HashMap<String, AvailableStatus>,
) -> AvailableToolsPayload {
    let catalog = capability_catalog(resolved);
    let mut tools = project_catalog(&catalog);
    for tool in &mut tools {
        tool.status = statuses.get(&tool.id).copied();
    }
    AvailableToolsPayload {
        tools,
        code_rejections: project_code_specs(code_candidates),
        catalog_errors: catalog.errors,
        custom_diagnostics: custom_tool_diagnostics(resolved),
    }
}

/// Settings-only code admission. Builtin contracts plus every valid custom
/// definition; invalid custom definitions are already in
/// [`custom_tool_diagnostics`], so they are not duplicated here.
pub fn code_rejections_for(resolved: &ResolvedConfig) -> Vec<RejectedTool> {
    let mut specs = crate::tool::registry::builtin_code_specs();
    specs.extend(resolved.custom_tools().iter().filter_map(|def| {
        crate::tool::spec::FunctionSpec::from_custom_definition(def)
            .ok()?
            .into_code()
    }));
    project_code_specs(specs)
}

/// Validate every stored custom definition without mutating it. Invalid entries
/// never enter the catalog; Settings still needs their field and fix.
pub fn custom_tool_diagnostics(
    resolved: &ResolvedConfig,
) -> Vec<crate::config::schema::CustomToolDiagnostic> {
    use crate::config::schema::{CustomSuiteDefinition, CustomToolDiagnostic, CustomToolDefinition};

    fn suite_issue(
        id: &str,
        suite: &CustomSuiteDefinition,
        source: &str,
    ) -> Option<CustomToolDiagnostic> {
        let suite_name = id.to_string();
        let issue = |field: &str, actual: Option<String>, expected: &str, fix: &str| {
            let actual_note = actual
                .as_deref()
                .map(|actual| format!("; actual `{actual}`"))
                .unwrap_or_default();
            CustomToolDiagnostic {
                source: source.to_string(),
                file: (source == "workspace")
                    .then(|| crate::config::workspace::WORKSPACE_CUSTOM_TOOLS_REL.to_string()),
                tool: None,
                suite: Some(suite_name.clone()),
                field: field.to_string(),
                actual,
                expected: expected.to_string(),
                fix: fix.to_string(),
                message: format!(
                    "{source}: custom suite `{id}` field `{field}`: expected {expected}{actual_note}; fix: {fix}"
                ),
            }
        };
        if crate::config::settings_writer::validate_tool_id(id).is_err() {
            return Some(issue(
                "name",
                Some(id.to_string()),
                "[a-z][a-z0-9_]*",
                "rename the suite",
            ));
        }
        if let Some(error) = crate::permission::policy::custom_rule_ids_error(&suite.rules) {
            return Some(issue(
                "rules",
                Some(error),
                "rules with unique non-empty ids",
                "fix the suite rule ids",
            ));
        }
        None
    }

    fn layer(
        tools: Vec<(String, CustomToolDefinition)>,
        suites: &HashMap<String, CustomSuiteDefinition>,
        source: &str,
        out: &mut Vec<CustomToolDiagnostic>,
    ) {
        for (suite_name, suite) in suites {
            if let Some(issue) = suite_issue(suite_name, suite, source) {
                out.push(issue);
            }
        }
        let mut seen = std::collections::HashSet::new();
        for (key, mut def) in tools {
            crate::config::settings_writer::normalize_custom_definition(&key, &mut def);
            if !seen.insert(def.name.clone()) {
                out.push(CustomToolDiagnostic {
                    source: source.to_string(),
                    file: (source == "workspace")
                        .then(|| crate::config::workspace::WORKSPACE_CUSTOM_TOOLS_REL.to_string()),
                    tool: Some(def.name.clone()),
                    suite: def.suite.clone(),
                    field: "name".into(),
                    actual: Some(def.name.clone()),
                    expected: "a unique tool name".into(),
                    fix: "rename one definition; the catalog has one function per name".into(),
                    message: format!(
                        "{source}: custom tool `{}` is declared more than once; fix: rename one definition",
                        def.name
                    ),
                });
                continue;
            }
            let file = (source == "workspace")
                .then(|| crate::config::workspace::WORKSPACE_CUSTOM_TOOLS_REL.to_string());
            if let Some(issue) = crate::config::settings_writer::custom_definition_diagnostic(
                &key, &def, suites, source, file,
            ) {
                out.push(issue);
            }
        }
    }

    let mut out = Vec::new();
    layer(
        resolved
            .global_custom_tools()
            .iter()
            .map(|tool| (tool.name.clone(), tool.clone()))
            .collect(),
        resolved.global_custom_suites(),
        "global",
        &mut out,
    );
    layer(
        resolved
            .workspace_custom_tools()
            .iter()
            .map(|(key, tool)| (key.clone(), tool.clone()))
            .collect(),
        resolved.workspace_custom_suites(),
        "workspace",
        &mut out,
    );
    out.sort_by(|left, right| {
        (left.source.as_str(), left.tool.as_deref(), left.suite.as_deref(), left.field.as_str())
            .cmp(&(
                right.source.as_str(),
                right.tool.as_deref(),
                right.suite.as_deref(),
                right.field.as_str(),
            ))
    });
    out
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
