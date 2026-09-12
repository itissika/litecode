//! Continue an existing child session with another message (`subagent_send`).

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, ToolCallResult};

use super::jobs::prompt_preview;
use super::status;
use super::turn::start_turn_like_human;

pub struct SubagentSendTool {
    runtime: RuntimeHandle,
    sessions: Arc<SessionManager>,
    depth: u32,
    session_id: Mutex<String>,
}

impl SubagentSendTool {
    pub fn new(runtime: RuntimeHandle, sessions: Arc<SessionManager>, depth: u32) -> Self {
        Self {
            runtime,
            sessions,
            depth,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    async fn call_send(&self, input: Value, call_id: &str) -> ToolCallResult {
        let child_id = match crate::tool::require_nonempty_string(&input, "id") {
            Ok(id) => id.to_string(),
            Err(e) => return ToolCallResult::error(e),
        };
        let message = match crate::tool::require_nonempty_string(&input, "message") {
            Ok(message) => message.to_string(),
            Err(e) => return ToolCallResult::error(e),
        };
        let parent = self.session_id();
        if parent.is_empty() {
            return ToolCallResult::error(
                "subagent_send requires an active session execution context",
            );
        }
        if call_id.is_empty() {
            return ToolCallResult::error(
                "subagent_send requires an active tool call_id (missing execution context)",
            );
        }
        let is_child = self
            .sessions
            .descendant_session_ids(&parent)
            .iter()
            .any(|id| id == &child_id);
        if !is_child {
            return ToolCallResult::error(format!(
                "subagent '{child_id}' is not a child session of this session"
            ));
        }
        if self.sessions.is_turn_running_blocking(&child_id) {
            return ToolCallResult::error(format!(
                "subagent '{child_id}' is already running a turn. Wait for it \
                 (subagent_wait) or stop it (subagent_stop), then send again."
            ));
        }
        if let Err(error) = self.sessions.ensure_entry(&child_id).await {
            return ToolCallResult::error(format!("subagent '{child_id}' is unavailable: {error}"));
        }
        let agent_id = match self.sessions.agent_id(&child_id) {
            Some(agent) if !agent.is_empty() => agent,
            _ => {
                return ToolCallResult::error(format!(
                    "subagent '{child_id}' has no agent profile recorded"
                ));
            }
        };
        let child_depth = self
            .sessions
            .reader()
            .meta_blocking(&child_id)
            .map(|meta| meta.subagent_depth)
            .unwrap_or(self.depth + 1);
        let project = self.sessions.project(&child_id).unwrap_or_else(|| {
            self.sessions.project(&parent).unwrap_or_default()
        });
        let preview = prompt_preview(&message);
        let Some(rx) = self.sessions.subscribe(&child_id) else {
            return ToolCallResult::error(format!(
                "subagent '{child_id}' has no event channel"
            ));
        };
        let mut opts = TurnOptions::agent(agent_id.clone(), self.sessions.session_model_id(&child_id));
        opts.depth = child_depth;
        let turn_id = match start_turn_like_human(
            &self.runtime,
            &self.sessions,
            &child_id,
            message,
            &agent_id,
            &project,
            opts,
        )
        .await
        {
            Ok(turn_id) => turn_id,
            Err(LitecodeError::AgentAlreadyRunning) => {
                return ToolCallResult::error(format!(
                    "subagent '{child_id}' is already running a turn. Wait for it \
                     (subagent_wait) or stop it (subagent_stop), then send again."
                ));
            }
            Err(error) => {
                return ToolCallResult::error(format!("subagent_send failed: {error}"));
            }
        };
        let hub = &self.runtime.subagent_hub;
        hub.jobs
            .register_child(&child_id, &parent, call_id, &agent_id, preview);
        hub.watch_child_exit(&child_id, call_id, rx);
        let jobs = hub.jobs.running(&parent);
        ToolCallResult::ok(status::format_sent_status(&child_id, &turn_id, &jobs))
    }
}

impl Tool for SubagentSendTool {
    fn name(&self) -> &str {
        "subagent_send"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "child_session_id"
                },
                "message": {
                    "type": "string",
                    "description": "Next assignment for this child"
                }
            },
            "required": ["id", "message"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentSendTool {
            runtime: self.runtime.clone(),
            sessions: Arc::clone(&self.sessions),
            depth: self.depth,
            session_id: Mutex::new(execution.session_id.clone()),
        };
        let call_id = execution.call_id.clone();
        Box::pin(async move { tool.call_send(input, &call_id).await })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("subagent_send must be invoked via execute (async tool path)")
    }

    fn description(&self, _ctx: &Context) -> String {
        "Send a message to an idle child session so it runs another turn in the background. \
         Fails if that child is already running a turn."
            .into()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}
