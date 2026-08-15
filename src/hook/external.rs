use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;

use super::types::{HookAction, HookInjection, HookOutput, HookPayload, InjectPlacement};

use crate::config::HookCommand;
use crate::context_pipeline::Context;
use crate::types::Result;

pub struct ExternalHookAdapter {
    point: &'static str,
    command: HookCommand,
}

impl ExternalHookAdapter {
    pub fn new(point: &'static str, command: HookCommand) -> Self {
        Self { point, command }
    }

    pub fn run<'a>(
        &'a self,
        payload: &'a HookPayload,
        _ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = HookOutput> + Send + 'a>> {
        Box::pin(async move {
            match Self::execute_command(&self.command, payload).await {
                Ok((action, output)) => Self::parse_response(action, output),
                Err(e) => {
                    tracing::warn!(
                        hook_point = %self.point,
                        command = %self.command.command,
                        error = %e,
                        "external hook failed"
                    );
                    HookOutput::ok()
                }
            }
        })
    }

    async fn execute_command(
        hook: &HookCommand,
        payload: &HookPayload,
    ) -> Result<(HookAction, serde_json::Value)> {
        let mut cmd = TokioCommand::new(&hook.command);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            let input = serde_json::to_string(payload)?;
            stdin.write_all(input.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
        }

        let status = child.wait().await?;

        let stdout = if let Some(stdout) = child.stdout.take() {
            let mut reader = BufReader::new(stdout);
            let mut buf = String::new();
            reader.read_line(&mut buf).await?;
            buf.trim().to_string()
        } else {
            String::new()
        };

        let output: serde_json::Value = if stdout.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&stdout).unwrap_or(serde_json::Value::String(stdout))
        };

        let action = match status.code() {
            Some(0) => HookAction::Continue,
            Some(2) => HookAction::Block,
            _ => HookAction::Continue,
        };

        Ok((action, output))
    }

    fn parse_response(action: HookAction, output: Value) -> HookOutput {
        let mut result = HookOutput {
            action,
            ..Default::default()
        };

        if let Value::Object(ref map) = output {
            if let Some(decision) = map
                .get("hookSpecificOutput")
                .and_then(|v| v.get("permissionDecision"))
                .and_then(|v| v.as_str())
            {
                result.action = match decision {
                    "allow" => HookAction::Allow,
                    "deny" => HookAction::Block,
                    _ => result.action,
                };
            }

            if let Some(entries) = map.get("injectItems").and_then(|v| v.as_array()) {
                for entry in entries {
                    if let Some(inj) = parse_inject_entry(entry) {
                        result.inject_items.push(inj);
                    }
                }
            }
            if let Some(display) = map.get("displayMessage").and_then(|v| v.as_str()) {
                result.display_message = Some(display.to_string());
            }
            if let Some(updated) = map.get("updatedInput") {
                result.updated_input = Some(updated.clone());
            }
        }

        result
    }
}

/// Convert a wire convenience `{role, content, placement}` into `HookInjection` once.
fn parse_inject_entry(entry: &Value) -> Option<HookInjection> {
    let role = entry.get("role").and_then(|v| v.as_str())?;
    let content = entry.get("content").and_then(|v| v.as_str())?;
    let placement_str = entry.get("placement").and_then(|v| v.as_str())?;
    let placement = match placement_str {
        "PreTurn" => InjectPlacement::PreTurn,
        "PostToolResults" => InjectPlacement::PostToolResults,
        "Tail" => InjectPlacement::Tail,
        _ => InjectPlacement::Head,
    };
    Some(match role {
        "assistant" => HookInjection::assistant_text(content, placement),
        _ => HookInjection::user_text(content, placement),
    })
}
