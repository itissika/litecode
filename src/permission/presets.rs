use crate::config::global_db::tools::{
    core_configurable_tools, network_core_tools, optional_builtin_ids,
};
use crate::config::schema::{PermissionSurface, ToolPreset};

use super::action::PermissionAction;
use super::matchers::ArgMatcher;
use super::policy::{BindingPathMode, PolicyRule, ToolPolicy};

/// Coding tools whose ALL and SAFE behaviors differ, plus workspace config.
/// Everything else is bind on/off only. New tools stay fixed until added here.
const PRESET_TOOL_IDS: &[&str] = &[
    "read",
    "grep",
    "glob",
    "write",
    "edit",
    "bash",
    "litecode_workspace",
];

pub fn has_permission_preset(tool_id: &str) -> bool {
    PRESET_TOOL_IDS.contains(&tool_id)
}

pub fn permission_surface(tool_id: &str) -> PermissionSurface {
    if has_permission_preset(tool_id) {
        PermissionSurface::Preset
    } else {
        PermissionSurface::Fixed
    }
}

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
        .chain(optional_builtin_ids().iter())
    {
        if !has_permission_preset(tool) {
            continue;
        }
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
        // Read actions stay open. `refresh` validates excludes / MCP / custom tools.
        "litecode_workspace" => ToolPolicy {
            default: PermissionAction::Allow,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![PolicyRule {
                id: "refresh".into(),
                when: ArgMatcher::ArgGlob {
                    name: "action".into(),
                    pattern: "refresh*".into(),
                },
                action: PermissionAction::Ask,
            }],
        },
        _ => ToolPolicy {
            default: PermissionAction::Ask,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::evaluate::evaluate;
    use crate::permission::matchers::MatchContext;

    #[test]
    fn preset_surface_is_opt_in() {
        for id in [
            "read",
            "grep",
            "glob",
            "write",
            "edit",
            "bash",
            "litecode_workspace",
        ] {
            assert!(has_permission_preset(id), "{id}");
            assert_eq!(permission_surface(id), PermissionSurface::Preset);
        }
        for id in [
            "session_search",
            "code_search",
            "lsp",
            "websearch",
            "webfetch",
            "kill_shell",
            "wait_shell",
            "plan",
            "todo",
            "knowledge",
            "subagent_launch",
            "echo_py",
            "mcp_github",
        ] {
            assert!(!has_permission_preset(id), "{id}");
            assert_eq!(permission_surface(id), PermissionSurface::Fixed);
        }
    }

    #[test]
    fn litecode_workspace_safe_asks_only_on_refresh() {
        let (policy, path_mode) = binding_for_tool("litecode_workspace", ToolPreset::Safe);
        assert_eq!(path_mode, BindingPathMode::WorkspaceOnly);
        let ctx = MatchContext {
            workspace_root: std::path::Path::new("/tmp"),
            path_mode,
        };
        let status = evaluate(&policy, &serde_json::json!({"action": "status"}), &ctx);
        assert_eq!(status.action, PermissionAction::Allow);
        let refresh = evaluate(&policy, &serde_json::json!({"action": "refresh"}), &ctx);
        assert_eq!(refresh.action, PermissionAction::Ask);
        assert_eq!(refresh.rule_id, "refresh");
        let refresh_mcp = evaluate(&policy, &serde_json::json!({"action": "refresh mcp"}), &ctx);
        assert_eq!(refresh_mcp.action, PermissionAction::Ask);
        let (all, _) = binding_for_tool("litecode_workspace", ToolPreset::All);
        let opened = evaluate(&all, &serde_json::json!({"action": "refresh"}), &ctx);
        assert_eq!(opened.action, PermissionAction::Allow);
    }

    #[test]
    fn coding_safe_rules_stay_in_place() {
        let (read, _) = binding_for_tool("read", ToolPreset::Safe);
        assert_eq!(read.default, PermissionAction::Allow);
        assert_eq!(read.rules[0].id, "outside_workspace");
        let (write, _) = binding_for_tool("write", ToolPreset::Safe);
        assert_eq!(write.default, PermissionAction::Ask);
        let (bash, _) = binding_for_tool("bash", ToolPreset::Safe);
        assert_eq!(bash.default, PermissionAction::Deny);
        assert_eq!(bash.rules[0].id, "readonly_command");
    }
}
