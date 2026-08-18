//! Product hard-deny rules evaluated before user-configurable policy.

use serde_json::Value;

use crate::tools::bash_safety::check_dangerous_command;

use super::action::PermissionAction;
use super::evaluate::EvalResult;
use super::matchers::MatchContext;
use super::sensitive::is_sensitive_system_path;

/// Returns `Some(Deny)` when a floor rule blocks the call.
pub fn check_floor(tool_name: &str, args: &Value, _ctx: &MatchContext<'_>) -> Option<EvalResult> {
    // G4: sensitive writes are denied unconditionally — regardless of path_mode.
    // A user-configurable preset (e.g. All → Unrestricted) must never weaken this
    // floor, so it no longer depends on `path_mode == WorkspaceOnly`.
    if matches!(tool_name, "write" | "edit")
        && let Some(path) = args.get("file_path").and_then(Value::as_str)
            && is_sensitive_system_path(path)
        {
            return Some(EvalResult {
                rule_id: "floor_sensitive_write".into(),
                action: PermissionAction::Deny,
            });
        }

    if tool_name == "bash"
        && let Some(command) = args.get("command").and_then(Value::as_str)
    {
        let action = check_dangerous_command(command);
        if action == PermissionAction::Deny {
            return Some(EvalResult {
                rule_id: "floor_dangerous_command".into(),
                action: PermissionAction::Deny,
            });
        }
    }

    None
}
