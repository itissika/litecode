use serde::{Deserialize, Serialize};

use super::action::PermissionAction;
use super::matchers::ArgMatcher;

pub const DEFAULT_RULE_ID: &str = "__default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub when: ArgMatcher,
    pub action: PermissionAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub default: PermissionAction,
    #[serde(default = "default_rule_id")]
    pub default_id: String,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

fn default_rule_id() -> String {
    DEFAULT_RULE_ID.to_string()
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            default: PermissionAction::Allow,
            default_id: DEFAULT_RULE_ID.to_string(),
            rules: Vec::new(),
        }
    }
}

impl ToolPolicy {
    pub fn allow_all() -> Self {
        Self::default()
    }

    pub fn with_default(action: PermissionAction) -> Self {
        Self {
            default: action,
            default_id: DEFAULT_RULE_ID.to_string(),
            rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum BindingPathMode {
    WorkspaceOnly,
    #[default]
    Unrestricted,
}

impl BindingPathMode {
    pub fn to_tool_path_mode(self) -> crate::workspace::ToolPathMode {
        match self {
            BindingPathMode::WorkspaceOnly => crate::workspace::ToolPathMode::Safe,
            BindingPathMode::Unrestricted => crate::workspace::ToolPathMode::All,
        }
    }
}

