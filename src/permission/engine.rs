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
        Self {
            resolved,
            agent_id,
            view,
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
        self.resolved
            .agents()
            .get(&self.agent_id)
            .and_then(|profile| profile.tools.get(tool_name))
    }

    pub fn path_mode(&self, tool_name: &str) -> BindingPathMode {
        self.binding(tool_name)
            .map(|b| b.path_mode)
            .unwrap_or_default()
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
    /// not a builtin. Reserved names (`bash`, `write`, …) always keep the binding.
    fn custom_tool_rules(&self, tool_name: &str) -> Option<Vec<crate::permission::PolicyRule>> {
        if let Some(tool) = self.resolved.workspace_custom_tools().get(tool_name)
            && crate::config::global_db::tools::is_custom_tool_identity(tool_name, &tool.name)
        {
            return Some(tool.rules.clone());
        }
        self.resolved
            .global_custom_tools()
            .iter()
            .find(|tool| {
                crate::config::global_db::tools::is_custom_tool_identity(tool_name, &tool.name)
            })
            .map(|tool| tool.rules.clone())
    }

    /// SAFE walks the current rules and allows when none match. ALL, or a tool
    /// whose rules were removed, allows the call. Other tools keep the stored binding.
    fn effective_policy(&self, tool_name: &str) -> (ToolPolicy, BindingPathMode) {
        if let Some(rules) = self.custom_tool_rules(tool_name) {
            let preset = self.binding(tool_name).and_then(|b| b.last_applied_preset);
            if preset == Some(ToolPreset::Safe) && !rules.is_empty() {
                return (ToolPolicy::from_rules(rules), BindingPathMode::Unrestricted);
            }
            return (ToolPolicy::allow_all(), BindingPathMode::Unrestricted);
        }
        let policy = self
            .binding(tool_name)
            .map(|b| b.policy.clone())
            .unwrap_or_default();
        (policy, self.path_mode(tool_name))
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

    fn outside_rule() -> PolicyRule {
        PolicyRule {
            id: "outside_workspace".into(),
            when: ArgMatcher::PathOutsideWorkspace {
                name: "path".into(),
            },
            action: PermissionAction::Deny,
        }
    }

    fn tool(rules: Vec<PolicyRule>) -> CustomToolDefinition {
        CustomToolDefinition {
            name: "demo".into(),
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
        }
    }

    fn engine(
        rules: Vec<PolicyRule>,
        workspace_tool: Option<CustomToolDefinition>,
        preset: Option<ToolPreset>,
        stored: ToolPolicy,
        depth: u32,
    ) -> (PermissionEngine, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut global = GlobalSettings::default();
        let mut profile = AgentProfile::default();
        profile.tools.insert(
            "demo".into(),
            AgentToolBinding {
                enabled: true,
                policy: stored,
                path_mode: BindingPathMode::Unrestricted,
                last_applied_preset: preset,
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
    fn safe_uses_the_live_rules() {
        let (engine, dir) = engine(
            vec![outside_rule()],
            None,
            Some(ToolPreset::Safe),
            ToolPolicy::allow_all(),
            0,
        );
        let outside = tempfile::tempdir().unwrap();
        let denied = engine.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": outside.path().to_string_lossy() }),
            dir.path(),
        );
        assert_eq!(denied.action, PermissionAction::Deny);
        assert_eq!(denied.rule_id, "outside_workspace");
        let allowed = engine.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": "notes.txt" }),
            dir.path(),
        );
        assert_eq!(allowed.action, PermissionAction::Allow);
    }

    #[test]
    fn all_and_removed_rules_allow() {
        let (opened, dir) = engine(
            vec![outside_rule()],
            None,
            Some(ToolPreset::All),
            ToolPolicy::from_rules(vec![outside_rule()]),
            0,
        );
        let outside = tempfile::tempdir().unwrap();
        let allowed = opened.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": outside.path().to_string_lossy() }),
            dir.path(),
        );
        assert_eq!(allowed.action, PermissionAction::Allow);

        let stale = ToolPolicy::with_default(PermissionAction::Deny);
        let (cleared, dir) = engine(Vec::new(), None, Some(ToolPreset::Safe), stale, 0);
        let allowed = cleared.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": "notes.txt" }),
            dir.path(),
        );
        assert_eq!(allowed.action, PermissionAction::Allow);
    }

    #[test]
    fn workspace_tool_without_rules_drops_the_global_rules() {
        let (engine, dir) = engine(
            vec![outside_rule()],
            Some(tool(Vec::new())),
            Some(ToolPreset::Safe),
            ToolPolicy::from_rules(vec![outside_rule()]),
            0,
        );
        let outside = tempfile::tempdir().unwrap();
        let allowed = engine.evaluate_tool(
            "demo",
            &serde_json::json!({ "path": outside.path().to_string_lossy() }),
            dir.path(),
        );
        assert_eq!(allowed.action, PermissionAction::Allow);
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
        for id in ["bash", "write"] {
            let (policy, path_mode) =
                crate::permission::presets::binding_for_tool(id, ToolPreset::Safe);
            profile.tools.insert(
                id.into(),
                AgentToolBinding {
                    enabled: true,
                    policy,
                    path_mode,
                    last_applied_preset: Some(ToolPreset::Safe),
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
            vec![named_tool("bash", vec![]), named_tool("write", vec![])],
            vec![
                named_tool("bash", vec![open.clone()]),
                named_tool("write", vec![open]),
            ],
        ] {
            let (engine, dir) = safe_builtin_engine(tools, vec![]);
            let bash = engine.evaluate_tool(
                "bash",
                &serde_json::json!({ "command": "mkdir scratch" }),
                dir.path(),
            );
            assert_eq!(bash.action, PermissionAction::Deny);
            let write = engine.evaluate_tool(
                "write",
                &serde_json::json!({ "file_path": "notes.txt" }),
                dir.path(),
            );
            assert_eq!(write.action, PermissionAction::Ask);
        }

        let (engine, dir) = safe_builtin_engine(vec![], vec![named_tool("bash", vec![])]);
        let bash = engine.evaluate_tool(
            "bash",
            &serde_json::json!({ "command": "mkdir scratch" }),
            dir.path(),
        );
        assert_eq!(bash.action, PermissionAction::Deny);
    }
}
