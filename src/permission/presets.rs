use crate::config::global_db::tools::{core_configurable_tools, optional_builtin_ids};
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
    "exec",
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
        "exec" => ToolPolicy::allow_all(),
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
                    name: "path".into(),
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
                when: ArgMatcher::AnyOf {
                    matchers: vec![
                        ArgMatcher::PathOutsideWorkspace {
                            name: "path".into(),
                        },
                        ArgMatcher::PathOutsideWorkspace {
                            name: "file_path".into(),
                        },
                    ],
                },
                action: PermissionAction::Deny,
            }],
        },
        "exec" => ToolPolicy {
            default: PermissionAction::Deny,
            default_id: super::policy::DEFAULT_RULE_ID.into(),
            rules: vec![
                // SAFE strictly denies workdir outside the workspace even for
                // readonly commands, matching read/glob/grep.
                PolicyRule {
                    id: "outside_workspace".into(),
                    when: ArgMatcher::PathOutsideWorkspace {
                        name: "workdir".into(),
                    },
                    action: PermissionAction::Deny,
                },
                PolicyRule {
                    id: "readonly_command".into(),
                    when: ArgMatcher::BashReadonlyCommand,
                    action: PermissionAction::Allow,
                },
            ],
        },
        // Read actions (`workspace.help`, `workspace.status`) stay open.
        // `workspace.refresh` reloads workspace config and asks under SAFE.
        "litecode_workspace" => ToolPolicy::allow_all(),
        "workspace.refresh" => ToolPolicy {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::evaluate::evaluate;
    use crate::permission::matchers::MatchContext;

    #[test]
    fn safe_exec_denies_outside_workspace_workdir_even_for_readonly_command() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (policy, path_mode) = binding_for_tool("exec", ToolPreset::Safe);
        let ctx = MatchContext {
            workspace_root: root.path(),
            path_mode,
        };

        let in_ws = evaluate(
            &policy,
            &serde_json::json!({ "cmd": "ls", "workdir": "." }),
            &ctx,
        );
        assert_eq!(in_ws.action, PermissionAction::Allow);

        let external = evaluate(
            &policy,
            &serde_json::json!({
                "cmd": "ls",
                "workdir": outside.path().to_string_lossy()
            }),
            &ctx,
        );
        assert_eq!(external.action, PermissionAction::Deny);
        assert_eq!(external.rule_id, "outside_workspace");

        let (all_policy, all_mode) = binding_for_tool("exec", ToolPreset::All);
        let all_ctx = MatchContext {
            workspace_root: root.path(),
            path_mode: all_mode,
        };
        let all_external = evaluate(
            &all_policy,
            &serde_json::json!({
                "cmd": "ls",
                "workdir": outside.path().to_string_lossy()
            }),
            &all_ctx,
        );
        assert_eq!(all_external.action, PermissionAction::Allow);
    }
}
