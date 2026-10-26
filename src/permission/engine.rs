use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::config::global_db::tools::core_none_tools;
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AgentRole, AgentToolBinding, ToolPreset};
use crate::permission::action::PermissionAction;
use crate::permission::policy::{BindingPathMode, ToolPolicy};

use super::evaluate::{EvalResult, evaluate};
use super::floor::check_floor;
use super::matchers::MatchContext;

/// Static permission view for a turn (primary allows Ask; subagent is allow/deny only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionView {
    Primary,
    Subagent,
}

#[derive(Debug, Clone)]
pub struct PermissionEngine {
    resolved: ResolvedConfig,
    agent_id: String,
    view: PermissionView,
    /// Member name → capability id, built once instead of rebuilding the
    /// catalog for every `binding()` lookup during a tool call.
    member_owner: Arc<HashMap<String, String>>,
}

impl PermissionEngine {
    pub fn resolver(resolved: ResolvedConfig, agent_id: impl Into<String>, depth: u32) -> Self {
        let agent_id = agent_id.into();
        let role = resolved
            .agents()
            .get(&agent_id)
            .map(|p| p.role)
            .unwrap_or(AgentRole::Primary);
        let view = if depth > 0 || role == AgentRole::Subagent {
            PermissionView::Subagent
        } else {
            PermissionView::Primary
        };
        let member_owner = Arc::new(
            crate::tool::capability::capability_catalog(&resolved).member_owner,
        );
        Self {
            resolved,
            agent_id,
            view,
            member_owner,
        }
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn view(&self) -> PermissionView {
        self.view
    }

    pub fn is_subagent_view(&self) -> bool {
        self.view == PermissionView::Subagent
    }

    pub fn binding(&self, tool_name: &str) -> Option<&AgentToolBinding> {
        let bind_name = self
            .member_owner
            .get(tool_name)
            .map(String::as_str)
            .unwrap_or(tool_name);
        self.resolved
            .agents()
            .get(&self.agent_id)
            .and_then(|profile| profile.tools.get(bind_name))
    }

    pub fn path_mode(&self, tool_name: &str) -> BindingPathMode {
        self.effective_policy(tool_name).1
    }

    pub fn evaluate_tool(
        &self,
        tool_name: &str,
        args: &Value,
        workspace_root: &std::path::Path,
    ) -> EvalResult {
        if core_none_tools().contains(&tool_name) {
            return EvalResult {
                rule_id: super::policy::DEFAULT_RULE_ID.into(),
                action: PermissionAction::Allow,
            };
        }

        let (policy, path_mode) = self.effective_policy(tool_name);
        let ctx = MatchContext {
            workspace_root,
            path_mode,
        };

        if let Some(floor) = check_floor(tool_name, args, &ctx) {
            return floor;
        }

        let result = evaluate(&policy, args, &ctx);

        if self.view == PermissionView::Subagent && result.action == PermissionAction::Ask {
            EvalResult {
                rule_id: result.rule_id,
                action: PermissionAction::Deny,
            }
        } else {
            result
        }
    }

    /// Rules on this custom tool. Workspace replaces the global entry.
    /// `None` means the name is not a custom tool.
    ///
    /// A catalog key owns these rules only when it is that tool's body name and
    /// not a builtin. Reserved names (`run`, `write`, …) always keep the binding.
    fn custom_tool_rules(&self, tool_name: &str) -> Option<Vec<crate::permission::PolicyRule>> {
        // Code suites are called as `suite.member`; the actual Tool identity
        // (and the binding) stays the member name.
        let member = tool_name
            .rsplit_once('.')
            .map(|(_, member)| member)
            .unwrap_or(tool_name);
        let (definition, suites) = if let Some(tool) = self.resolved.workspace_custom_tools().get(member)
            && crate::config::global_db::tools::is_custom_tool_identity(member, &tool.name)
        {
            (tool, self.resolved.workspace_custom_suites())
        } else if let Some(tool) = self
            .resolved
            .global_custom_tools()
            .iter()
            .find(|tool| {
                crate::config::global_db::tools::is_custom_tool_identity(member, &tool.name)
            })
        {
            (tool, self.resolved.global_custom_suites())
        } else {
            return None;
        };
        match definition
            .suite
            .as_deref()
            .map(str::trim)
            .filter(|suite| !suite.is_empty())
        {
            Some(suite) => suites.get(suite).map(|suite| suite.rules.clone()),
            None => Some(definition.rules.clone()),
        }
    }

    fn policy_name(tool_name: &str) -> &str {
        if tool_name == "apply_edits" {
            "edit"
        } else {
            tool_name
        }
    }

    /// Derive the effective policy from the binding intent and the current
    /// definition. Fixed tools derive allow-all; preset tools derive from their
    /// preset; custom tools/suites derive from their current rules.
    ///
    /// SAFE walks the current rules and allows when none match. ALL, or a tool
    /// whose rules were removed, allows the call.
    fn effective_policy(&self, tool_name: &str) -> (ToolPolicy, BindingPathMode) {
        if let Some(rules) = self.custom_tool_rules(tool_name) {
            let preset = self.binding(tool_name).and_then(|b| b.preset);
            if preset == Some(ToolPreset::Safe) && !rules.is_empty() {
                return (ToolPolicy::from_rules(rules), BindingPathMode::Unrestricted);
            }
            return (ToolPolicy::allow_all(), BindingPathMode::Unrestricted);
        }
        let bind_name = Self::policy_name(tool_name);
        if crate::permission::has_permission_preset(bind_name) {
            let preset = self.binding(bind_name).and_then(|b| b.preset);
            if preset == Some(ToolPreset::Safe) {
                return crate::permission::presets::binding_for_tool(bind_name, preset.unwrap());
            }
        }
        (ToolPolicy::allow_all(), BindingPathMode::Unrestricted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::manager::ConfigManager;
    use crate::config::resolved::WorkspaceState;
    use crate::config::schema::{
        AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, ToolPreset,
        ToolSchema,
    };
    use crate::permission::matchers::ArgMatcher;
    use crate::permission::policy::PolicyRule;

    fn tool(rules: Vec<PolicyRule>) -> CustomToolDefinition {
        CustomToolDefinition {
            name: "demo".into(),
            output_schema: None,
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "demo".into(),
            args: Vec::new(),
            timeout: 120,
            rules,
            surface: Some(crate::config::schema::CallSurface::Direct),
            suite: None,
        }
    }

    fn engine(
        rules: Vec<PolicyRule>,
        workspace_tool: Option<CustomToolDefinition>,
        preset: Option<ToolPreset>,
        _stored: ToolPolicy,
        depth: u32,
    ) -> (PermissionEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert(
            "demo".into(),
            AgentToolBinding {
                enabled: true,
                preset: preset,
                allowed_tools: None,
            },
        );
        global.agents.insert("default".into(), profile);
        global.custom_tools.push(tool(rules));
        let mut workspace = WorkspaceState::new(dir.path());
        if let Some(workspace_tool) = workspace_tool {
            workspace
                .workspace_custom_tools
                .insert(workspace_tool.name.clone(), workspace_tool);
        }
        let resolved = ConfigManager::resolve_without_catalog(global, workspace);
        (PermissionEngine::resolver(resolved, "default", depth), dir)
    }

    #[test]
    fn code_suite_members_share_one_permission_face_and_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert(
            "suite:deck".into(),
            AgentToolBinding {
                enabled: true,
                preset: Some(ToolPreset::Safe),
                allowed_tools: None,
            },
        );
        global.agents.insert("default".into(), profile);
        global.custom_suites.insert(
            "deck".into(),
            crate::config::schema::CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Code,
                label: "Deck".into(),
                rules: vec![PolicyRule {
                    id: "deny_all".into(),
                    when: ArgMatcher::Any,
                    action: PermissionAction::Deny,
                }],
            },
        );
        for name in ["first", "second"] {
            let mut def = tool(Vec::new());
            def.name = name.into();
            def.command = "echo".into();
            def.surface = None;
            def.suite = Some("deck".into());
            def.output_schema = Some(serde_json::json!({
                "type": "object", "properties": {}, "required": []
            }));
            global.custom_tools.push(def);
        }
        let resolved =
            ConfigManager::resolve_without_catalog(global, WorkspaceState::new(dir.path()));
        let engine = PermissionEngine::resolver(resolved, "default", 0);

        for name in ["first", "second", "deck.first", "deck.second"] {
            assert_eq!(
                engine
                    .evaluate_tool(name, &serde_json::json!({}), dir.path())
                    .action,
                PermissionAction::Deny,
                "{name}"
            );
        }
        assert!(engine.binding("deck.first").is_some());
        assert!(engine.binding("deck.second").is_some());
    }

    #[test]
    fn subagent_member_uses_the_capability_binding() {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert(
            "subagent_tools".into(),
            AgentToolBinding {
                enabled: true,
                preset: None,
                allowed_tools: None,
            },
        );
        global.agents.insert("default".into(), profile);
        let resolved =
            ConfigManager::resolve_without_catalog(global, WorkspaceState::new(dir.path()));
        let engine = PermissionEngine::resolver(resolved, "default", 0);

        let binding = engine.binding("subagent_launch").expect("capability binding");
        assert!(binding.enabled);
        assert_eq!(
            engine
                .evaluate_tool("subagent_launch", &serde_json::json!({}), dir.path())
                .action,
            PermissionAction::Allow
        );
    }

    #[test]
    fn custom_rules_are_read_from_the_current_definition() {
        let deny = PolicyRule {
            id: "deny".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Deny,
        };
        let allow = PolicyRule {
            id: "allow".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Allow,
        };
        let (denied, dir) = engine(
            vec![deny],
            None,
            Some(ToolPreset::Safe),
            ToolPolicy::allow_all(),
            0,
        );
        assert_eq!(
            denied
                .evaluate_tool("demo", &serde_json::json!({}), dir.path())
                .action,
            PermissionAction::Deny
        );

        let (allowed, dir) = engine(
            vec![allow],
            None,
            Some(ToolPreset::Safe),
            ToolPolicy::allow_all(),
            0,
        );
        assert_eq!(
            allowed
                .evaluate_tool("demo", &serde_json::json!({}), dir.path())
                .action,
            PermissionAction::Allow
        );
    }

    #[test]
    fn subagent_turns_safe_ask_into_deny() {
        let ask = PolicyRule {
            id: "confirm".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Ask,
        };
        let (engine, dir) = engine(
            vec![ask],
            None,
            Some(ToolPreset::Safe),
            ToolPolicy::allow_all(),
            1,
        );
        let denied = engine.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": "notes.txt" }),
            dir.path(),
        );
        assert_eq!(denied.action, PermissionAction::Deny);
    }

    fn named_tool(name: &str, rules: Vec<PolicyRule>) -> CustomToolDefinition {
        let mut def = tool(rules);
        def.name = name.to_string();
        def.command = "echo".into();
        def
    }

    fn safe_builtin_engine(
        workspace_tools: Vec<CustomToolDefinition>,
        global_tools: Vec<CustomToolDefinition>,
    ) -> (PermissionEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        for id in ["run", "write"] {
            profile.tools.insert(
                id.into(),
                AgentToolBinding {
                    enabled: true,
                    preset: Some(ToolPreset::Safe),
                    allowed_tools: None,
                },
            );
        }
        global.agents.insert("default".into(), profile);
        global.custom_tools = global_tools;
        let mut workspace = WorkspaceState::new(dir.path());
        for def in workspace_tools {
            workspace
                .workspace_custom_tools
                .insert(def.name.clone(), def);
        }
        let resolved = ConfigManager::resolve_without_catalog(global, workspace);
        (PermissionEngine::resolver(resolved, "default", 0), dir)
    }

    #[test]
    fn reserved_names_keep_the_safe_binding() {
        let open = PolicyRule {
            id: "open".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Allow,
        };
        for tools in [
            vec![
                named_tool("run", vec![]),
                named_tool("write", vec![]),
            ],
            vec![
                named_tool("run", vec![open.clone()]),
                named_tool("write", vec![open]),
            ],
        ] {
            let (engine, dir) = safe_builtin_engine(tools, vec![]);
            let command = engine.evaluate_tool(
                "run",
                &serde_json::json!({ "command": "mkdir scratch" }),
                dir.path(),
            );
            assert_eq!(command.action, PermissionAction::Deny);
            let write = engine.evaluate_tool(
                "write",
                &serde_json::json!({ "file_path": "notes.txt" }),
                dir.path(),
            );
            assert_eq!(write.action, PermissionAction::Ask);
        }

        let (engine, dir) = safe_builtin_engine(vec![], vec![named_tool("run", vec![])]);
        let command = engine.evaluate_tool(
            "run",
            &serde_json::json!({ "command": "mkdir scratch" }),
            dir.path(),
        );
        assert_eq!(command.action, PermissionAction::Deny);
    }

    #[test]
    fn a_tool_named_run_code_keeps_safe_deny() {
        let deny = PolicyRule {
            id: "block".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Deny,
        };
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert(
            "run_code".into(),
            AgentToolBinding {
                enabled: true,
                preset: Some(ToolPreset::Safe),
                allowed_tools: None,
            },
        );
        global.agents.insert("default".into(), profile);
        global.custom_tools.push(named_tool("run_code", vec![deny]));
        let resolved =
            ConfigManager::resolve_without_catalog(global, WorkspaceState::new(dir.path()));
        let engine = PermissionEngine::resolver(resolved, "default", 0);
        let decision = engine.evaluate_tool("run_code", &serde_json::json!({}), dir.path());
        assert_eq!(decision.action, PermissionAction::Deny);
        assert_eq!(decision.rule_id, "block");
    }
}
