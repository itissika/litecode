use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::ResolvedConfig;
use crate::config::schema::AgentRole;
use crate::config::workspace::workspace_root_from_paths;
use crate::context_pipeline::Context;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::llm::LlmProvider;
use crate::runtime::ProviderRegistry;
use crate::runtime::TurnHandle;
use crate::runtime::llm_resolve::binding_for_agent;
use crate::runtime::observer::{ChannelObserver, InternalEnvelope, TurnTokenStats};
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, ToolCallResult};

pub struct SubagentLaunchTool {
    resolved: ResolvedConfig,
    parent_agent_id: String,
    provider: Box<dyn LlmProvider>,
    api_key: String,
    depth: u32,
    parent_cancel: CancellationToken,
    engine_manager: crate::optional::EngineManager,
    workspace_engines: WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
    sessions: Arc<SessionManager>,
    parent_session_id: String,
    mcp_pool: Arc<crate::mcp::McpConnectionPool>,
    /// The parent tool `call_id` captured from the execution context (REV-9:
    /// passed explicitly, never via TLS).
    parent_call_id: String,
}

impl SubagentLaunchTool {
    pub fn new(
        resolved: ResolvedConfig,
        parent_agent_id: impl Into<String>,
        provider: Box<dyn LlmProvider>,
        api_key: String,
        depth: u32,
        parent_cancel: CancellationToken,
        engine_manager: crate::optional::EngineManager,
        workspace_engines: WorkspaceEngines,
        ide: Arc<IdeBaseHandle>,
        sessions: Arc<SessionManager>,
        parent_session_id: impl Into<String>,
        mcp_pool: Arc<crate::mcp::McpConnectionPool>,
    ) -> Self {
        Self {
            resolved,
            parent_agent_id: parent_agent_id.into(),
            provider,
            api_key,
            depth,
            parent_cancel,
            engine_manager,
            workspace_engines,
            ide,
            sessions,
            parent_session_id: parent_session_id.into(),
            mcp_pool,
            parent_call_id: String::new(),
        }
    }

    fn clone_for_call(&self) -> Self {
        Self {
            resolved: self.resolved.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            provider: self.provider.box_clone(),
            api_key: self.api_key.clone(),
            depth: self.depth,
            parent_cancel: self.parent_cancel.clone(),
            engine_manager: self.engine_manager.clone(),
            workspace_engines: self.workspace_engines.clone(),
            ide: Arc::clone(&self.ide),
            sessions: Arc::clone(&self.sessions),
            parent_session_id: self.parent_session_id.clone(),
            mcp_pool: Arc::clone(&self.mcp_pool),
            parent_call_id: self.parent_call_id.clone(),
        }
    }

    fn allowed_subagent_ids(&self) -> Vec<String> {
        self.resolved
            .agents()
            .get(&self.parent_agent_id)
            .map(|p| p.allowed_subagents.clone())
            .unwrap_or_default()
    }

    /// Allowlist catalog for the model: `id (description)` when description is set, else bare `id`.
    fn format_available_subagents(&self) -> Option<String> {
        format_available_subagents(&self.resolved, &self.allowed_subagent_ids())
    }
}

/// Format allowlisted subagent ids with their config descriptions for tool discovery.
fn format_available_subagents(resolved: &ResolvedConfig, allowed: &[String]) -> Option<String> {
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

struct LaunchSpec {
    agent_name: String,
    prompt: String,
    model_id_override: Option<String>,
    max_steps_override: Option<u32>,
}

/// Cancels the child turn if `execute` is dropped (pipeline timeout) or returns.
struct ChildTurnGuard {
    sessions: Arc<SessionManager>,
    child_id: String,
    turn_id: String,
    cancel: CancellationToken,
}

impl Drop for ChildTurnGuard {
    fn drop(&mut self) {
        self.cancel.cancel();
        let _ = self.sessions.finish_turn(&self.child_id, &self.turn_id);
    }
}

impl SubagentLaunchTool {
    fn parse_launch(&self, input: &Value) -> std::result::Result<LaunchSpec, ToolCallResult> {
        let agent_name = crate::tool::require_nonempty_string(input, "agent")
            .map_err(ToolCallResult::error)?
            .to_string();
        let prompt = crate::tool::require_nonempty_string(input, "prompt")
            .map_err(ToolCallResult::error)?
            .to_string();

        let resolved = &self.resolved;
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

    fn map_turn_result(
        result: crate::types::Result<String>,
        stats: TurnTokenStats,
        child_session_id: &str,
    ) -> ToolCallResult {
        let mut meta = serde_json::to_value(stats).unwrap_or_default();
        if let Some(obj) = meta.as_object_mut()
            && !child_session_id.is_empty()
        {
            obj.insert(
                "child_session_id".into(),
                serde_json::Value::String(child_session_id.to_string()),
            );
        }
        match result {
            Ok(text) => ToolCallResult::ok_with_metadata(text, meta),
            Err(LitecodeError::Canceled) => ToolCallResult::error("subagent cancelled"),
            Err(e) => ToolCallResult::error(format!("agent error: {e}")),
        }
    }

    async fn launch(&self, input: Value) -> ToolCallResult {
        let spec = match self.parse_launch(&input) {
            Ok(spec) => spec,
            Err(e) => return e,
        };

        let _lease = match self
            .sessions
            .try_acquire_subagent_slot(&self.parent_session_id)
        {
            Ok(lease) => lease,
            Err(e) => return ToolCallResult::error(e.to_string()),
        };

        let child_cancel = self.parent_cancel.child_token();
        let resolved = self.resolved.clone();
        let project = workspace_root_from_paths(resolved.paths())
            .to_string_lossy()
            .to_string();

        let seed_model = spec.model_id_override.as_deref().or_else(|| {
            resolved
                .agents()
                .get(&spec.agent_name)
                .map(|p| p.model_ref.as_str())
                .filter(|s| !s.is_empty())
        });

        let child_session_id = match self.sessions.open_child_session(
            &project,
            &spec.agent_name,
            seed_model,
            &self.parent_session_id,
            &self.parent_call_id,
        ) {
            Ok(id) => id,
            Err(e) => {
                return ToolCallResult::error(format!("child session creation failed: {e}"));
            }
        };

        let _ = self.sessions.publish_internal(
            &self.parent_session_id,
            crate::runtime::observer::InternalEvent::SubagentBound {
                call_id: self.parent_call_id.clone(),
                child_session_id: child_session_id.clone(),
            },
        );

        let abort_child = |sessions: &SessionManager, child_id: &str| {
            let _ = sessions.remove_session(child_id);
        };

        let mut registry = ProviderRegistry::new();
        let turn_llm = match binding_for_agent(
            &resolved,
            &mut registry,
            &spec.agent_name,
            spec.model_id_override.as_deref(),
            0,
        ) {
            Ok(mut binding) => {
                // Same caller runtime as the parent turn — share the HTTP client.
                binding.provider = Arc::from(self.provider.box_clone());
                binding.api_key = self.api_key.clone();
                binding
            }
            Err(e) => {
                abort_child(&self.sessions, &child_session_id);
                return ToolCallResult::error(format!("llm binding failed: {e}"));
            }
        };

        let (event_tx, event_rx) = mpsc::unbounded_channel::<InternalEnvelope>();
        let observer = ChannelObserver::new(event_tx);

        let mut runtime = match crate::runtime::AgentRuntime::with_mcp_pool(
            resolved,
            child_session_id.clone(),
            Arc::clone(&self.sessions),
            turn_llm,
            &spec.agent_name,
            self.depth + 1,
            crate::permission::deny_permission_sink(),
            observer,
            Some(child_cancel.clone()),
            spec.max_steps_override,
            self.engine_manager.clone(),
            self.workspace_engines.clone(),
            Arc::clone(&self.ide),
            Arc::clone(&self.mcp_pool),
        ) {
            Ok(r) => r,
            Err(e) => {
                abort_child(&self.sessions, &child_session_id);
                return ToolCallResult::error(format!("agent runtime init failed: {e}"));
            }
        };

        let turn_id = Uuid::new_v4().to_string();
        let step_max = runtime.agent_config.max_steps;
        let cancel = runtime.cancel_token();
        let turn_handle = TurnHandle {
            handle: None,
            rx: event_rx,
            cancel,
            turn_id: turn_id.clone(),
            step_max,
        };

        if let Err(e) = self.sessions.reserve_turn(
            &child_session_id,
            turn_id.clone(),
            step_max,
            &spec.agent_name,
            &project,
        ) {
            abort_child(&self.sessions, &child_session_id);
            return ToolCallResult::error(format!("reserve_turn failed: {e}"));
        }
        if let Err(e) = self
            .sessions
            .start_turn(
                &child_session_id,
                turn_handle,
                &spec.agent_name,
                &project,
                Arc::clone(&self.sessions),
            )
            .await
        {
            abort_child(&self.sessions, &child_session_id);
            return ToolCallResult::error(format!("start_turn failed: {e}"));
        }

        let _guard = ChildTurnGuard {
            sessions: Arc::clone(&self.sessions),
            child_id: child_session_id.clone(),
            turn_id: turn_id.clone(),
            cancel: child_cancel.clone(),
        };

        let result = tokio::select! {
            biased;
            _ = child_cancel.cancelled() => {
                Err(LitecodeError::Canceled)
            }
            result = runtime.run_with_turn(&spec.prompt, &turn_id, step_max) => result
        };
        let stats = std::mem::take(&mut runtime.turn_token_stats);
        drop(runtime);
        if !child_cancel.is_cancelled() && !matches!(&result, Err(LitecodeError::Canceled)) {
            for _ in 0..200 {
                if !self.sessions.is_turn_running(&child_session_id).await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        }

        Self::map_turn_result(result, stats, &child_session_id)
    }
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
                }
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
            None => "Delegate a task to a sub-agent and wait for its final output.".into(),
            Some(catalog) => format!(
                "Delegate a task to a sub-agent and wait for its final output. Available: {catalog}."
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
        // Pipeline timeout is the fallback; execute() cancels the child token on drop.
        Some(600)
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
