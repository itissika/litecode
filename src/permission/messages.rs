//! User-facing permission denial messages.

use serde_json::Value;

use super::policy::DEFAULT_RULE_ID;

pub fn permission_denied_message(tool_name: &str, rule_id: &str, input: &Value) -> String {
    let file_path = input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or(tool_name);
    match (tool_name, rule_id) {
        (_, "floor_sensitive_write") => format!(
            "blocked write to sensitive system path '{file_path}'. System locations are blocked in workspace-only mode."
        ),
        (_, "outside_workspace") => format!(
            "blocked: '{file_path}' is outside the workspace. Use a workspace-relative path, or enable unrestricted path mode (ALL preset) for paths outside the workspace."
        ),
        // Safety floor: destructive patterns are hard-denied under every preset.
        ("bash", "floor_dangerous_command") => format!(
            "blocked: command '{}' matches a hard-deny safety rule (e.g. rm -rf /, fork bomb, \
             mkfs/dd or redirection to a raw block device). This applies under every permission \
             preset — do not retry or obfuscate it; ask the user to run it manually if it is \
             genuinely needed.",
            bash_command(input)
        ),
        // SAFE preset: bash defaults to Deny and only auto-allows read-only commands.
        ("bash", DEFAULT_RULE_ID) => format!(
            "permission denied for bash: the current policy denies non-read-only commands by \
             default (the SAFE preset only auto-allows read-only commands such as ls, cat, \
             git status, cargo check, and pipelines ending in head/less/more). Command: '{}'. \
             Ask the user to switch the bash tool preset to ALL, or to run the command themselves.",
            bash_command(input)
        ),
        (_, _) => format!("permission denied for '{tool_name}'"),
    }
}

/// Message for a call the user explicitly rejected at an Ask prompt. Distinct from a
/// policy Deny: the right guidance is to stop retrying, not to change presets.
pub fn permission_denied_by_user_message(tool_name: &str, summary: &str) -> String {
    if summary.is_empty() {
        return format!(
            "the user denied this '{tool_name}' request. Do not retry the same call; ask the user for guidance or use an alternative approach."
        );
    }
    format!(
        "the user denied this '{tool_name}' request ('{summary}'). Do not retry the same call; ask the user for guidance or use an alternative approach."
    )
}

fn bash_command(input: &Value) -> &str {
    input.get("command").and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bash_safe_default_deny_explains_readonly_policy() {
        let msg = permission_denied_message(
            "bash",
            DEFAULT_RULE_ID,
            &serde_json::json!({"command": "cargo run"}),
        );
        assert!(msg.contains("cargo run"), "echoes the command: {msg}");
        assert!(msg.contains("read-only"), "explains the policy: {msg}");
        assert!(msg.contains("ALL"), "offers next steps: {msg}");
    }

    #[test]
    fn bash_floor_deny_explains_safety_rule() {
        let msg = permission_denied_message(
            "bash",
            "floor_dangerous_command",
            &serde_json::json!({"command": "rm -rf /"}),
        );
        assert!(msg.contains("rm -rf /"), "echoes the command: {msg}");
        assert!(msg.contains("hard-deny"), "names the rule class: {msg}");
        assert!(msg.contains("do not retry"), "gives guidance: {msg}");
    }

    #[test]
    fn other_tools_keep_generic_message() {
        let msg = permission_denied_message(
            "webfetch",
            DEFAULT_RULE_ID,
            &serde_json::json!({"url": "https://example.com"}),
        );
        assert_eq!(msg, "permission denied for 'webfetch'");
    }

    #[test]
    fn user_denial_mentions_no_retry() {
        let msg = permission_denied_by_user_message("bash", "cargo build");
        assert!(msg.contains("user denied"), "{msg}");
        assert!(msg.contains("cargo build"), "{msg}");
        assert!(msg.contains("Do not retry"), "{msg}");
    }
}
