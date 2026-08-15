use serde_json::Value;

use crate::config::global_db::tools::core_none_tools;
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::{AgentRole, AgentToolBinding};
use crate::permission::action::PermissionAction;
use crate::permission::policy::BindingPathMode;

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

        let ctx = MatchContext {
            workspace_root,
            path_mode: self.path_mode(tool_name),
        };

        if let Some(floor) = check_floor(tool_name, args, &ctx) {
            return floor;
        }

        let policy = self
            .binding(tool_name)
            .map(|b| &b.policy)
            .cloned()
            .unwrap_or_default();

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
}
