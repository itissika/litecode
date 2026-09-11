use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::ResolvedConfig;
use crate::config::schema::AgentRole;
use crate::context_pipeline::Context;
use crate::runtime::RuntimeHandle;
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::hub::{LaunchSpec, SpawnDeps, SubagentHub};
use super::status;

pub struct SubagentLaunchTool {
    runtime: RuntimeHandle,
    parent_agent_id: String,
    depth: u32,
    /// Retained for constructor/pipeline compatibility only. Parent turn
    /// cancellation is deliberately not propagated into a child turn;
    /// `subagent_stop` is the explicit session-level cancel path.
    parent_cancel: CancellationToken,
    sessions: Arc<SessionManager>,
    parent_session_id: String,
    hub: Arc<SubagentHub>,
    parent_call_id: String,
}

impl SubagentLaunchTool {
    pub fn new(
        runtime: RuntimeHandle,
        parent_agent_id: impl Into<String>,
        depth: u32,
        parent_cancel: CancellationToken,
        sessions: Arc<SessionManager>,
        parent_session_id: impl Into<String>,
        hub: Arc<SubagentHub>,
    ) -> Self {
        Self {
            runtime,
            parent_agent_id: parent_agent_id.into(),
            depth,
            parent_cancel,
            sessions,
            parent_session_id: parent_session_id.into(),
            hub,
            parent_call_id: String::new(),
        }
    }

    fn clone_for_call(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            depth: self.depth,
            parent_cancel: self.parent_cancel.clone(),
            sessions: Arc::clone(&self.sessions),
            parent_session_id: self.parent_session_id.clone(),
            hub: Arc::clone(&self.hub),
            parent_call_id: self.parent_call_id.clone(),
        }
    }

    fn allowed_subagent_ids(&self) -> Vec<String> {
        self.runtime
            .resolved
            .agents()
            .get(&self.parent_agent_id)
            .map(|p| p.allowed_subagents.clone())
            .unwrap_or_default()
    }

    fn format_available_subagents(&self) -> Option<String> {
        format_available_subagents(&self.runtime.resolved, &self.allowed_subagent_ids())
    }

    fn parse_launch(&self, input: &Value) -> std::result::Result<LaunchSpec, ToolCallResult> {
        let agent_name = crate::tool::require_nonempty_string(input, "agent")
            .map_err(ToolCallResult::error)?
            .to_string();
        let prompt = crate::tool::require_nonempty_string(input, "prompt")
            .map_err(ToolCallResult::error)?
            .to_string();

        let resolved = &self.runtime.resolved;
        let parent = resolved
            .agents()
            .get(&self.parent_agent_id)
            .ok_or_else(|| {
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

        let model_id_override: Option<String> = if let Some(model_id) = input["model"].as_str() {
            if !resolved.global().models.contains_key(model_id) {
                return Err(ToolCallResult::error(format!(
                    "model '{}' is not a models registry id; use an id from the models table",
                    model_id
                )));
            }
            Some(model_id.to_string())
        } else {
            None
        };

        Ok(LaunchSpec {
            agent_name,
            prompt,
            model_id_override,
            max_steps_override: input["max_steps"].as_u64().map(|n| (n as u32).min(100)),
        })
    }

    async fn launch(&self, input: Value) -> ToolCallResult {
        let spec = match self.parse_launch(&input) {
            Ok(spec) => spec,
            Err(e) => return e,
        };
        let deps = SpawnDeps {
            runtime: self.runtime.clone(),
            depth: self.depth,
            sessions: Arc::clone(&self.sessions),
        };

        let child_id = match self
            .hub
            .spawn(&self.parent_session_id, &self.parent_call_id, spec, deps)
            .await
        {
            Ok(id) => id,
            Err(e) => return ToolCallResult::error(e),
        };

        let jobs = self.hub.running(&self.parent_session_id);
        with_child_meta(
            ToolCallResult::ok(status::format_running_status(&child_id, &jobs)),
            &child_id,
        )
    }
}

fn with_child_meta(mut result: ToolCallResult, child_id: &str) -> ToolCallResult {
    result.metadata = Some(serde_json::json!({ "child_session_id": child_id }));
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
        "subagent_launch"
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
        Box::pin(async move {
            let mut result = tool.launch(input).await.finalize_signals();
            let max = tool.max_result_size();
            if max < usize::MAX {
                result.content = Session::truncated_tool_result(&result.content, max);
            }
            result
        })
    }

    fn schema(&self) -> Value {
        let agent_desc = match self.format_available_subagents() {
            Some(catalog) => {
                format!("Subagent id from the parent primary agent allowlist. Available: {catalog}")
            }
            None => "No subagents configured for this primary agent".to_string(),
        };

        serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "description": agent_desc
                },
                "prompt": {
                    "type": "string",
                    "description": "The task prompt for the sub-agent"
                },
                "model": {
                    "type": "string",
                    "description": "Optional models registry id override (must exist in models table)"
                },
                "max_steps": {
                    "type": "integer",
                    "description": "Optional max_steps override"
                },
            },
            "required": ["agent", "prompt"]
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        match self.parse_launch(&input) {
            Err(e) => e,
            Ok(_) => ToolCallResult::error(
                "subagent_launch must be invoked via execute (async tool path)",
            ),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        match self.format_available_subagents() {
            None => {
                "Delegate a task to a sub-agent. Launch returns immediately with child_session_id; the subagent runs in the background. Use subagent_list to list sessions, subagent_wait to wait, subagent_stop to cancel its current turn, and session_search to read its transcript.".into()
            }
            Some(catalog) => format!(
                "Delegate a task to a sub-agent. Launch returns immediately with child_session_id; the subagent runs in the background. Use subagent_list to list sessions, subagent_wait to wait, subagent_stop to cancel its current turn, and session_search to read its transcript. Available: {catalog}."
            ),
        }
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn timeout(&self) -> Option<u64> {
        None
    }

    fn agent_subagents(&self) -> Option<Arc<SubagentHub>> {
        Some(Arc::clone(&self.hub))
    }

    fn set_active_session(&self, session_id: String) {
        let _ = session_id;
    }
}

#[cfg(test)]
mod tests {
    use super::format_available_subagents;
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};

    #[test]
    fn format_available_includes_descriptions() {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                allowed_subagents: vec!["reviewer".into(), "worker".into()],
                ..Default::default()
            },
        );
        global.agents.insert(
            "reviewer".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                description: "Reviews code for bugs".into(),
                ..Default::default()
            },
        );
        global.agents.insert(
            "worker".into(),
            AgentProfile {
                role: AgentRole::Subagent,
                description: String::new(),
                ..Default::default()
            },
        );
        let resolved = resolve(global, WorkspaceState::new("/tmp"));
        let catalog = format_available_subagents(&resolved, &["reviewer".into(), "worker".into()])
            .expect("catalog");
        assert_eq!(catalog, "reviewer (Reviews code for bugs), worker");
    }

    #[test]
    fn format_available_empty_allowlist_is_none() {
        let resolved = resolve(GlobalSettings::default(), WorkspaceState::new("/tmp"));
        assert!(format_available_subagents(&resolved, &[]).is_none());
    }
}
