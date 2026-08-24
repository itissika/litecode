use crate::config::global_db::tools::{
    core_configurable_tools, network_core_tools, optional_builtin_ids,
};
use crate::config::schema::ToolPreset;

use super::action::PermissionAction;
use super::matchers::ArgMatcher;
use super::policy::{BindingPathMode, PolicyRule, ToolPolicy};

pub fn binding_for_tool(tool_id: &str, preset: ToolPreset) -> (ToolPolicy, BindingPathMode) {
    match preset {
        ToolPreset::All => (policy_all(tool_id), BindingPathMode::Unrestricted),
        ToolPreset::Safe => (policy_safe(tool_id), BindingPathMode::WorkspaceOnly),
    }
}

pub fn apply_preset_to_tools(preset: ToolPreset) -> Vec<(String, ToolPolicy, BindingPathMode)> {
    let mut out = Vec::new();
    for tool in core_configurable_tools()
        .iter()
        .chain(network_core_tools().iter())
    {
        let (policy, path_mode) = binding_for_tool(tool, preset);
        out.push(((*tool).to_string(), policy, path_mode));
    }
    for tool in optional_builtin_ids() {
        let (policy, path_mode) = binding_for_tool(tool, preset);
        out.push(((*tool).to_string(), policy, path_mode));
    }
    out
}

fn policy_all(tool_id: &str) -> ToolPolicy {
    match tool_id {
        "bash" => ToolPolicy::allow_all(),
        "write" | "edit" => ToolPolicy::allow_all(),
        _ => ToolPolicy::allow_all(),
    }
}

fn policy_safe(tool_id: &str) -> ToolPolicy {
    match tool_id {
        "read" => ToolPolicy {
            default: PermissionAction::Allow,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "outside_workspace".into(),
                when: ArgMatcher::PathOutsideWorkspace {
                    name: "file_path".into(),
                },
                action: PermissionAction::Deny,
            }],
        },
        // grep reads content but never mutates. SAFE denies the `path` arg when it
        // names a location outside the workspace (audit parity with read/glob);
        // execute-time resolve_agent enforces the same boundary via path_mode.
        "grep" => ToolPolicy {
            default: PermissionAction::Allow,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "outside_workspace".into(),
                when: ArgMatcher::PathOutsideWorkspace {
                    name: "path".into(),
                },
                action: PermissionAction::Deny,
            }],
        },
        "glob" => ToolPolicy {
            default: PermissionAction::Allow,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "outside_workspace".into(),
                when: ArgMatcher::PathOutsideWorkspace {
                    name: "path".into(),
                },
                action: PermissionAction::Deny,
            }],
        },
        "write" | "edit" => ToolPolicy {
            default: PermissionAction::Ask,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "outside_workspace".into(),
                when: ArgMatcher::PathOutsideWorkspace {
                    name: "file_path".into(),
                },
                action: PermissionAction::Deny,
            }],
        },
        "bash" => ToolPolicy {
            default: PermissionAction::Deny,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "readonly_command".into(),
                when: ArgMatcher::BashReadonlyCommand,
                action: PermissionAction::Allow,
            }],
        },
        "kill_shell" | "wait_shell" | "session_search" => ToolPolicy::allow_all(),
        "webfetch" | "websearch" | "code_search" | "lsp" => ToolPolicy {
            default: PermissionAction::Ask,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![],
        },
        _ => ToolPolicy {
            default: PermissionAction::Ask,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![],
        },
    }
}

pub fn default_policy_for_custom() -> (ToolPolicy, BindingPathMode) {
    binding_for_tool("custom", ToolPreset::All)
}

pub fn safe_policy_for_custom() -> (ToolPolicy, BindingPathMode) {
    binding_for_tool("custom", ToolPreset::Safe)
}
