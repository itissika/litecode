use serde_json::Value;

use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{
    AskOutcome, PermissionAction, PermissionEngine, PermissionSink, check_runtime_grant,
    grant_runtime, permission_denied_message,
};
use crate::tool::schema_validate::{check_tool_input, invalid_input_for, parse_tool_arguments};
use crate::tool::trait_::Tool;
use crate::types::FunctionToolCall;

#[derive(Debug, Clone)]
pub enum AuthResult {
    Denied(String),
    Aborted,
    Proceed { effective_input: Value },
}

fn permission_summary(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "bash" => input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "read" | "write" | "edit" => input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "glob" => input
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => serde_json::to_string(input).unwrap_or_default(),
    }
}

pub async fn authorize(
    inv: &FunctionToolCall,
    tool: &dyn Tool,
    permission: &PermissionEngine,
    ctx: &Context,
    agent_name: &str,
    sink: &dyn PermissionSink,
    cancel: &CancellationToken,
) -> AuthResult {
    let name = inv.name.clone();
    let input = match parse_tool_arguments(&inv.arguments) {
        Ok(value) => value,
        Err(detail) => return AuthResult::Denied(invalid_input_for(&name, detail)),
    };
    if let Err(msg) = check_tool_input(tool, &input) {
        return AuthResult::Denied(invalid_input_for(&name, msg));
    }

    let effective_input = input.clone();

    let eval = permission.evaluate_tool(&name, &effective_input, &ctx.cwd);

    let mut action = eval.action;
    if action == PermissionAction::Ask
        && let Some(granted) = check_runtime_grant(agent_name, &name, &eval.rule_id)
        && granted == PermissionAction::Allow
    {
        action = PermissionAction::Allow;
    }

    match action {
        PermissionAction::Deny => {
            return AuthResult::Denied(permission_denied_message(
                &name,
                &eval.rule_id,
                &effective_input,
            ));
        }
        PermissionAction::Ask => {
            if permission.is_subagent_view() {
                return AuthResult::Denied(permission_denied_message(
                    &name,
                    &eval.rule_id,
                    &effective_input,
                ));
            }

            let summary = permission_summary(&name, &effective_input);
            match sink.ask_permission(&name, &eval.rule_id, &summary, cancel) {
                AskOutcome::Aborted => return AuthResult::Aborted,
                AskOutcome::Deny => {
                    return AuthResult::Denied(permission_denied_message(
                        &name,
                        &eval.rule_id,
                        &effective_input,
                    ));
                }
                AskOutcome::Allow { always } => {
                    if always {
                        grant_runtime(agent_name, &name, &eval.rule_id, PermissionAction::Allow);
                        tracing::info!(
                            tool = %name,
                            rule_id = %eval.rule_id,
                            "permission granted with 'always'"
                        );
                    }
                }
            }
        }
        PermissionAction::Allow => {}
    }

    AuthResult::Proceed { effective_input }
}
