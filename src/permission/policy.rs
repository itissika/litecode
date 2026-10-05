use serde::{Deserialize, Serialize};

use super::action::PermissionAction;
use super::matchers::ArgMatcher;

pub const DEFAULT_RULE_ID: &str = "__default";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: String,
    pub when: ArgMatcher,
    pub action: PermissionAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Custom-tool SAFE policy: these rules in order, allow when none match.
    pub fn from_rules(rules: Vec<PolicyRule>) -> Self {
        Self {
            default: PermissionAction::Allow,
            default_id: DEFAULT_RULE_ID.to_string(),
            rules,
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

/// Reject blank, duplicate, and reserved rule ids. Comparison uses the trimmed id.
pub fn custom_rule_ids_error(rules: &[PolicyRule]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for rule in rules {
        let id = rule.id.trim();
        if id.is_empty() {
            return Some("custom tool rule id must not be empty".into());
        }
        if id == DEFAULT_RULE_ID {
            return Some(format!(
                "custom tool rule id '{DEFAULT_RULE_ID}' is reserved"
            ));
        }
        if !seen.insert(id.to_string()) {
            return Some(format!("duplicate custom tool rule id '{id}'"));
        }
    }
    None
}

impl BindingPathMode {
    pub fn to_tool_path_mode(self) -> crate::workspace::ToolPathMode {
        match self {
            BindingPathMode::WorkspaceOnly => crate::workspace::ToolPathMode::Safe,
            BindingPathMode::Unrestricted => crate::workspace::ToolPathMode::All,
        }
    }
}
