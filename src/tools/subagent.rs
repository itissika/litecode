use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::ResolvedConfig;
use crate::config::schema::AgentRole;
use crate::config::workspace::{set_runtime_paths, workspace_root_from_paths};
use crate::context_pipeline::Context;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::llm::LlmProvider;
use crate::runtime::ProviderRegistry;
use crate::runtime::TurnHandle;
use crate::runtime::llm_resolve::binding_for_agent;
use crate::runtime::observer::{ChannelObserver, InternalEnvelope, TurnTokenStats};
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

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

impl Tool for SubagentLaunchTool {
    fn name(&self) -> &str {
        "subagent_launch"
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // REV-9: capture the parent `call_id` and turn cancel from the execution
        // context and pass them into the blocking call path explicitly (never via
        // TLS).
        let mut tool = self.clone_for_call();
        tool.parent_call_id = execution.call_id.clone();
        tool.parent_cancel = execution.cancel.clone();
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || tool.call(input));
            match join.await {
                Ok(result) => result,
                Err(e) => ToolCallResult::error(format!("subagent task join failed: {e}")),
            }
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
        let agent_name = match crate::tool::require_nonempty_string(&input, "agent") {
            Ok(a) => a.to_string(),
            Err(e) => return ToolCallResult::error(e),
        };
        let prompt = match crate::tool::require_nonempty_string(&input, "prompt") {
            Ok(p) => p.to_string(),
            Err(e) => return ToolCallResult::error(e),
        };

        let resolved = self.resolved.clone();
        let parent = match resolved.agents().get(&self.parent_agent_id) {
            Some(p) => p,
            None => {
                return ToolCallResult::error(format!(
                    "parent primary agent '{}' not found in configuration",
                    self.parent_agent_id
                ));
            }
        };
        let allowed = parent.allowed_subagents.clone();
        let available = format_available_subagents(&resolved, &allowed);
        let profile = match resolved.agents().get(&agent_name) {
            Some(p) => p.clone(),
            None => {
                return match &available {
                    None => ToolCallResult::error(format!(
                        "agent '{}' not found. No subagents are configured for primary '{}'",
                        agent_name, self.parent_agent_id
                    )),
                    Some(catalog) => ToolCallResult::error(format!(
                        "agent '{agent_name}' not found. Available subagents: {catalog}"
                    )),
                };
            }
        };

        if profile.role != AgentRole::Subagent {
            return ToolCallResult::error(format!(
                "agent '{}' is not a subagent (role={:?}); only subagent role agents can be launched",
                agent_name, profile.role
            ));
        }

        if !parent.allowed_subagents.contains(&agent_name) {
            let allowed_list = available.as_deref().unwrap_or("none configured");
            return ToolCallResult::error(format!(
                "agent '{}' is not in primary '{}' allowed_subagents (allowed: {allowed_list})",
                agent_name, self.parent_agent_id
            ));
        }

        if self.parent_call_id.is_empty() {
            return ToolCallResult::error(
                "subagent_launch requires an active tool call_id (missing execution context)",
            );
        }
        let parent_call_id = self.parent_call_id.clone();

        let model_id_override: Option<String> = if let Some(model_id) = input["model"].as_str() {
            if !resolved.global().models.contains_key(model_id) {
                return ToolCallResult::error(format!(
                    "model '{}' is not a models registry id; use an id from the models table",
                    model_id
                ));
            }
            Some(model_id.to_string())
        } else {
            None
        };

        let max_steps_override = input["max_steps"].as_u64().map(|n| (n as u32).min(100));

        let sub_provider = self.provider.clone_for_isolated_runtime();
        let api_key = self.api_key.clone();
        let subagent_depth = self.depth + 1;
        let parent_cancel = self.parent_cancel.clone();
        let engine_manager = self.engine_manager.clone();
        let workspace_engines = self.workspace_engines.clone();
        let ide = Arc::clone(&self.ide);
        let workspace_paths = resolved.paths().clone();
        let agent_name_clone = agent_name.clone();
        let prompt_owned = prompt;
        let model_id_override_clone = model_id_override.clone();
        let sessions = Arc::clone(&self.sessions);
        let parent_session_id = self.parent_session_id.clone();
        let mcp_pool = Arc::clone(&self.mcp_pool);

        let (tx, rx) = std::sync::mpsc::channel::<(String, TurnTokenStats, String)>();

        let child_join = std::thread::spawn(move || {
            set_runtime_paths(workspace_paths.clone());
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send((
                        format!("runtime creation failed: {}", e),
                        TurnTokenStats::default(),
                        String::new(),
                    ));
                    return Ok::<String, crate::types::LitecodeError>(String::new());
                }
            };

            let project = workspace_root_from_paths(&workspace_paths)
                .to_string_lossy()
                .to_string();

            let seed_model = model_id_override_clone.as_deref().or_else(|| {
                resolved
                    .agents()
                    .get(&agent_name_clone)
                    .map(|p| p.model_ref.as_str())
                    .filter(|s| !s.is_empty())
            });

            let child_session_id = match sessions.open_child_session(
                &project,
                &agent_name_clone,
                seed_model,
                &parent_session_id,
                &parent_call_id,
            ) {
                Ok(id) => id,
                Err(e) => {
                    let _ = tx.send((
                        format!("child session creation failed: {}", e),
                        TurnTokenStats::default(),
                        String::new(),
                    ));
                    return Ok::<String, crate::types::LitecodeError>(String::new());
                }
            };

            // Immediate bind on the parent session so FE can subscribe before the
            // child turn finishes (and so reconnect buffer retains the binding).
            let _ = sessions.publish_internal(
                &parent_session_id,
                crate::runtime::observer::InternalEvent::SubagentBound {
                    call_id: parent_call_id.clone(),
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
                &agent_name_clone,
                model_id_override_clone.as_deref(),
                0,
            ) {
                Ok(mut binding) => {
                    binding.provider = Arc::from(sub_provider);
                    binding.api_key = api_key;
                    binding
                }
                Err(e) => {
                    abort_child(&sessions, &child_session_id);
                    let _ = tx.send((
                        format!("llm binding failed: {e}"),
                        TurnTokenStats::default(),
                        String::new(),
                    ));
                    return Ok::<String, crate::types::LitecodeError>(String::new());
                }
            };

            let (event_tx, event_rx) = mpsc::unbounded_channel::<InternalEnvelope>();
            let observer = ChannelObserver::new(event_tx);

            let mut runtime = match crate::runtime::AgentRuntime::with_mcp_pool(
                resolved,
                child_session_id.clone(),
                Arc::clone(&sessions),
                turn_llm,
                &agent_name_clone,
                subagent_depth,
                crate::permission::deny_permission_sink(),
                observer,
                Some(parent_cancel.clone()),
                max_steps_override,
                engine_manager,
                workspace_engines,
                ide,
                Arc::clone(&mcp_pool),
            ) {
                Ok(r) => r,
                Err(e) => {
                    abort_child(&sessions, &child_session_id);
                    let _ = tx.send((
                        format!("agent runtime init failed: {}", e),
                        TurnTokenStats::default(),
                        String::new(),
                    ));
                    return Ok::<String, crate::types::LitecodeError>(String::new());
                }
            };

            let turn_id = Uuid::new_v4().to_string();
            let step_max = runtime.agent_config.max_steps;
            let cancel = runtime.cancel_token();

            // REV-10: no fake `dummy_join` handle. The turn is driven inline and the
            // parent reclaims the actual OS thread via `child_join`.
            let turn_handle = TurnHandle {
                handle: None,
                rx: event_rx,
                cancel,
                turn_id: turn_id.clone(),
                step_max,
            };

            let run_result = rt.block_on(async {
                if let Err(e) = sessions.reserve_turn(
                    &child_session_id,
                    turn_id.clone(),
                    step_max,
                    &agent_name_clone,
                    &project,
                ) {
                    return (
                        Err(format!("reserve_turn failed: {e}")),
                        TurnTokenStats::default(),
                        true, // abort child
                    );
                }
                if let Err(e) = sessions
                    .start_turn(
                        &child_session_id,
                        turn_handle,
                        &agent_name_clone,
                        &project,
                        Arc::clone(&sessions),
                    )
                    .await
                {
                    return (
                        Err(format!("start_turn failed: {e}")),
                        TurnTokenStats::default(),
                        true, // abort child
                    );
                }
                let result = runtime
                    .run_with_turn(&prompt_owned, &turn_id, step_max)
                    .await
                    .map_err(|e| format!("agent error: {e}"));
                let stats = std::mem::take(&mut runtime.turn_token_stats);
                // Close observer channel so fanout can finish, then wait for it.
                drop(runtime);
                for _ in 0..200 {
                    if !sessions.is_turn_running(&child_session_id).await {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                (result, stats, false)
            });

            match run_result {
                (Ok(text), stats, _) => {
                    let _ = tx.send((text.clone(), stats, child_session_id));
                    Ok::<String, crate::types::LitecodeError>(text)
                }
                (Err(e), stats, abort) => {
                    if abort {
                        abort_child(&sessions, &child_session_id);
                        let _ = tx.send((e.clone(), stats, String::new()));
                    } else {
                        let _ = tx.send((e.clone(), stats, child_session_id));
                    }
                    Ok::<String, crate::types::LitecodeError>(e)
                }
            }
        });

        // Block until subagent completes or parent is cancelled. REV-10: on any
        // early return we join the child OS thread so it is explicitly reclaimed.
        loop {
            if self.parent_cancel.is_cancelled() {
                // Trigger the child's own cancel token (bound as parent_cancel) and
                // join the thread instead of leaking it.
                let _ = child_join.join();
                return ToolCallResult::error("subagent cancelled");
            }
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok((text, stats, child_session_id)) => {
                    let mut meta = serde_json::to_value(stats).unwrap_or_default();
                    if let Some(obj) = meta.as_object_mut()
                        && !child_session_id.is_empty()
                    {
                        obj.insert(
                            "child_session_id".into(),
                            serde_json::Value::String(child_session_id),
                        );
                    }
                    // Errors from the child thread are returned as content that starts with
                    // known prefixes; surface them as tool errors when clearly failed.
                    if text.starts_with("runtime creation failed:")
                        || text.starts_with("child session creation failed:")
                        || text.starts_with("llm binding failed:")
                        || text.starts_with("agent runtime init failed:")
                        || text.starts_with("start_turn failed:")
                        || text.starts_with("agent error:")
                    {
                        return ToolCallResult::error(text);
                    }
                    return ToolCallResult::ok_with_metadata(text, meta);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child_join.join();
                    return ToolCallResult::error("subagent thread exited unexpectedly");
                }
            }
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
        // Launches are scheduled by SessionManager, not ToolPipeline batches.
        false
    }

    fn timeout(&self) -> Option<u64> {
        // Long timeout matches typical subagent max_steps duration.
        // Parent cancellation handles user interruption.
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
