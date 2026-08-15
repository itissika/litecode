//! User-facing permission denial messages.

use serde_json::Value;

pub fn permission_denied_message(tool_name: &str, rule_id: &str, input: &Value) -> String {
    let file_path = input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or(tool_name);
    match rule_id {
        "floor_sensitive_write" => format!(
            "blocked write to sensitive system path '{file_path}'. System locations are blocked in workspace-only mode."
        ),
        "outside_workspace" => format!(
            "blocked: '{file_path}' is outside the workspace. Use a workspace-relative path, or enable unrestricted path mode (ALL preset) for paths outside the workspace."
        ),
        _ => format!("permission denied for '{tool_name}'"),
    }
}
