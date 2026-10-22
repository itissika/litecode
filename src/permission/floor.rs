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
        && let Some(path) = code_file_arg(args)
        && is_sensitive_system_path(path)
    {
        return Some(EvalResult {
            rule_id: "floor_sensitive_write".into(),
            action: PermissionAction::Deny,
        });
    }

    if tool_name == "apply_edits"
        && let Some(edits) = args.get("edits").and_then(Value::as_array)
    {
        for item in edits {
            if let Some(path) = item
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| item.get("file_path").and_then(Value::as_str))
                && is_sensitive_system_path(path)
            {
                return Some(EvalResult {
                    rule_id: "floor_sensitive_write".into(),
                    action: PermissionAction::Deny,
                });
            }
        }
    }

    if tool_name == "run"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::BindingPathMode;

    fn ctx() -> MatchContext<'static> {
        MatchContext {
            workspace_root: std::path::Path::new("."),
            path_mode: BindingPathMode::Unrestricted,
        }
    }

    #[test]
    fn run_hits_the_dangerous_command_floor() {
        let hit = check_floor(
            "run",
            &serde_json::json!({"command": "rm -rf /"}),
            &ctx(),
        )
        .expect("floor");
        assert_eq!(hit.rule_id, "floor_dangerous_command");
        assert_eq!(hit.action, PermissionAction::Deny);
        assert!(
            check_floor("run", &serde_json::json!({"command": "ls"}), &ctx()).is_none()
        );
    }

}

/// `path` is the code-tool argument. `file_path` is still denied when a caller
/// sends it, so dropping the tool alias does not open the sensitive-path floor.
fn code_file_arg(args: &Value) -> Option<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .or_else(|| args.get("file_path").and_then(Value::as_str))
}
