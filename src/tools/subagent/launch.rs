use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::ResolvedConfig;
use crate::config::schema::{AgentProfile, AgentRole};
use crate::config::workspace::workspace_root_from_paths;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::SessionService;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::status;
use super::turn::start_turn_like_human;

/// Spawn dependencies shared by the launch tool and its contract tests.
pub struct SpawnDeps {
    /// Live runtime handle of the parent turn. The child re-applies settings
    /// from the global DB at spawn time (same as a main-session turn) and
    /// resolves its own LLM binding from the child session's row (model / tier /
    /// context mode) — never from the parent session's provider or the agent
    /// profile's runtime config.
    pub runtime: RuntimeHandle,
    pub depth: u32,
    pub service: Arc<SessionService>,
}

pub struct LaunchSpec {
    pub agent_name: String,
    pub responsibility: String,
    pub prompt: String,
}

/// Open a child session and start its first turn through the same session
/// business operation a human turn uses.
pub async fn spawn_child_job(
    deps: &SpawnDeps,
    parent_session_id: &str,
    call_id: &str,
    spec: LaunchSpec,
) -> Result<(String, String), String> {
    if call_id.is_empty() {
        tracing::error!(parent_session_id, "subagent_launch missing tool call_id");
        return Err(
            "subagent_launch requires an active tool call_id (missing execution context)".into(),
        );
    }
    let mut runtime = deps.runtime.clone();
    runtime
        .apply_non_engine()
        .map_err(|error| format!("child turn start failed: {error}"))?;
    runtime.sync_workspace_tool_readiness();

    let project = deps.service.project(parent_session_id).unwrap_or_else(|| {
        workspace_root_from_paths(runtime.resolved.paths())
            .to_string_lossy()
            .to_string()
    });
    let seed_model = crate::runtime::seed_model_ref(&runtime.resolved, &spec.agent_name);
    let child_session_id = deps
        .service
        .open_child_session_with_responsibility(
            &project,
            &spec.agent_name,
            seed_model.as_deref(),
            parent_session_id,
            call_id,
            &spec.responsibility,
        )
        .map_err(|error| format!("child turn start failed: {error}"))?;

    if !deps.service.publish_internal(
        parent_session_id,
        crate::runtime::observer::InternalEvent::SubagentBound {
            call_id: call_id.to_string(),
            child_session_id: child_session_id.clone(),
        },
    ) {
        tracing::warn!(
            parent_session_id,
            child_session_id = %child_session_id,
            "subagent bound event dropped (parent session missing)"
        );
    }

    let opts = TurnOptions::child(spec.agent_name.clone());
    let turn_id = match start_turn_like_human(
        &runtime,
        &deps.service,
        &child_session_id,
        spec.prompt.clone(),
        &spec.agent_name,
        &project,
        opts,
    )
    .await
    {
        Ok(turn_id) => turn_id,
        Err(error) => {
            tracing::error!(
                parent_session_id,
                agent = %spec.agent_name,
                error = %error,
                "subagent child turn failed to start"
            );
            let _ = deps.service.remove_session(&child_session_id);
            return Err(format!("child turn start failed: {error}"));
        }
    };
    tracing::info!(
        parent_session_id,
        child_session_id = %child_session_id,
        agent = %spec.agent_name,
        call_id,
        turn_id,
        "subagent child turn started"
    );
    Ok((child_session_id, turn_id))
}

pub struct SubagentLaunchTool {
    runtime: Option<RuntimeHandle>,
    parent_agent_id: String,
    depth: u32,
    /// Retained for constructor/pipeline compatibility only. Parent turn
    /// cancellation is deliberately not propagated into a child turn;
    /// `subagent_stop` is the explicit session-level cancel path.
    parent_cancel: CancellationToken,
    service: Option<Arc<SessionService>>,
    parent_session_id: String,
    parent_call_id: String,
    /// Turn-start body snapshot. None only for direct test/tool construction;
    /// production always binds this so a mid-turn edit cannot change launch
    /// authorization.
    parent_profile: Option<AgentProfile>,
}

impl SubagentLaunchTool {
    pub fn new(
        runtime: RuntimeHandle,
        parent_agent_id: impl Into<String>,
        depth: u32,
        parent_cancel: CancellationToken,
        service: Arc<SessionService>,
        parent_session_id: impl Into<String>,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            parent_agent_id: parent_agent_id.into(),
            depth,
            parent_cancel,
            service: Some(service),
            parent_session_id: parent_session_id.into(),
            parent_call_id: String::new(),
            parent_profile: None,
        }
    }

    pub fn cold() -> Self {
        Self {
            runtime: None,
            parent_agent_id: String::new(),
            depth: 0,
            parent_cancel: CancellationToken::new(),
            service: None,
            parent_session_id: String::new(),
            parent_call_id: String::new(),
            parent_profile: None,
        }
    }

    /// Bind the body captured at turn start. The live resolver remains a
    /// compatibility fallback for tests and direct tool construction.
    pub fn with_profile(mut self, profile: AgentProfile) -> Self {
        self.parent_profile = Some(profile);
        self
    }

    fn clone_for_call(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            depth: self.depth,
            parent_cancel: self.parent_cancel.clone(),
            service: self.service.clone(),
            parent_session_id: self.parent_session_id.clone(),
            parent_call_id: self.parent_call_id.clone(),
            parent_profile: self.parent_profile.clone(),
        }
    }

    /// The caller's effective body. Prefer the turn-start snapshot; a fallback
    /// exists only for direct tool construction/tests.
    fn parent_profile(&self) -> Option<AgentProfile> {
        if let Some(profile) = &self.parent_profile {
            return Some(profile.clone());
        }
        let runtime = self.runtime.as_ref()?;
        crate::bot::resolve_agent_for_session(
            &runtime.resolved,
            runtime.bots().map(Arc::as_ref),
            &self.parent_session_id,
            &self.parent_agent_id,
        )
        .map(|resolution| resolution.profile)
    }

    fn allowed_subagent_ids(&self) -> Vec<String> {
        self.parent_profile()
            .map(|profile| profile.allowed_subagents)
            .unwrap_or_default()
    }

    fn format_available_subagents(&self) -> Option<String> {
        let runtime = self.runtime.as_ref()?;
        format_available_subagents(&runtime.resolved, &self.allowed_subagent_ids())
    }

    fn parse_launch(&self, input: &Value) -> std::result::Result<LaunchSpec, ToolCallResult> {
        let agent_name = crate::tool::require_nonempty_string(input, "agent")
            .map_err(ToolCallResult::error)?
            .to_string();
        let prompt = crate::tool::require_nonempty_string(input, "prompt")
            .map_err(ToolCallResult::error)?
            .to_string();
        let responsibility = crate::tool::require_nonempty_string(input, "responsibility")
            .map_err(ToolCallResult::error)?
            .to_string();

        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| ToolCallResult::error("subagent.launch requires an active runtime"))?;
        let resolved = &runtime.resolved;
        let parent = self.parent_profile().ok_or_else(|| {
            ToolCallResult::error(format!(
                "parent primary agent '{}' not found in configuration",
                self.parent_agent_id
            ))
        })?;
        let allowed = parent.allowed_subagents.clone();
        let available = format_available_subagents(resolved, &allowed);
        let profile = match resolved.agents().get(&agent_name) {
            Some(p) => p,
            None => {
                return Err(match &available {
                    None => ToolCallResult::error(format!(
                        "agent '{}' not found. No subagents are configured for primary '{}'",
                        agent_name, self.parent_agent_id
                    )),
                    Some(catalog) => ToolCallResult::error(format!(
                        "agent '{agent_name}' not found. Available subagents: {catalog}"
                    )),
                });
            }
        };

        if profile.role != AgentRole::Subagent {
            return Err(ToolCallResult::error(format!(
                "agent '{}' is not a subagent (role={:?}); only subagent role agents can be launched",
                agent_name, profile.role
            )));
        }

        if !parent.allowed_subagents.contains(&agent_name) {
            let allowed_list = available.as_deref().unwrap_or("none configured");
            return Err(ToolCallResult::error(format!(
                "agent '{}' is not in primary '{}' allowed_subagents (allowed: {allowed_list})",
                agent_name, self.parent_agent_id
            )));
        }

        if self.parent_call_id.is_empty() {
            return Err(ToolCallResult::error(
                "subagent_launch requires an active tool call_id (missing execution context)",
            ));
        }

        Ok(LaunchSpec {
            agent_name,
            responsibility,
            prompt,
        })
    }

    async fn launch(&self, input: Value) -> ToolCallResult {
        let spec = match self.parse_launch(&input) {
            Ok(spec) => spec,
            Err(e) => return e,
        };
        let (Some(runtime), Some(service)) = (self.runtime.clone(), self.service.clone()) else {
            return ToolCallResult::error("subagent.launch requires an active runtime");
        };
        let deps = SpawnDeps {
            runtime,
            depth: self.depth,
            service,
        };

        let responsibility = spec.responsibility.clone();
        let (child_id, turn_id) =
            match spawn_child_job(&deps, &self.parent_session_id, &self.parent_call_id, spec).await
            {
                Ok(started) => started,
                Err(e) => return ToolCallResult::error(e),
            };

        let text = status::format_started(&child_id, &turn_id, Some(&responsibility));
        with_child_meta(
            ToolCallResult::ok(text).with_data(serde_json::json!({
                "id": child_id,
            })),
            &child_id,
            &turn_id,
        )
    }
}

fn with_child_meta(mut result: ToolCallResult, child_id: &str, turn_id: &str) -> ToolCallResult {
    result.metadata = Some(serde_json::json!({ "child_session_id": child_id, "turn_id": turn_id }));
    result
}

/// Format allowlisted subagent ids with their config descriptions for tool discovery.
pub(crate) fn format_available_subagents(
    resolved: &ResolvedConfig,
    allowed: &[String],
) -> Option<String> {
    if allowed.is_empty() {
        return None;
    }
    let catalog = allowed
        .iter()
        .map(|id| match resolved.agents().get(id) {
            Some(profile) if !profile.description.trim().is_empty() => {
                format!("{id} ({})", profile.description.trim())
            }
            _ => id.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(catalog)
}

impl Tool for SubagentLaunchTool {
    fn name(&self) -> &str {
        "subagent.launch"
    }

    fn dynamic_sdk_note(&self) -> Option<String> {
        self.runtime.as_ref()?;
        Some(match self.format_available_subagents() {
            None => "No subagents are configured in allowed_subagents.".to_string(),
            Some(catalog) => format!("Available subagents for allowed_subagents: {catalog}"),
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let mut tool = self.clone_for_call();
        tool.parent_call_id = execution.call_id.clone();
        tool.parent_cancel = execution.cancel.clone();
        if !execution.session_id.is_empty() {
            tool.parent_session_id = execution.session_id.clone();
        }
        Box::pin(async move { tool.launch(input).await.finalize_signals() })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        match self.parse_launch(&input) {
            Err(e) => e,
            Ok(_) => ToolCallResult::error(
                "subagent.launch must be invoked via execute (async tool path)",
            ),
        }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        None
    }

    fn set_active_session(&self, session_id: String) {
        let _ = session_id;
    }
}

#[cfg(test)]
mod tests {}
