//! Continue an existing child session with another message (`subagent_send`).

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, ToolCallResult};

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

    async fn call_send(&self, input: Value) -> ToolCallResult {
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
                "subagent '{child_id}' is already running a turn. Its result will be delivered when that turn settles."
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
        let project = self
            .sessions
            .project(&child_id)
            .unwrap_or_else(|| self.sessions.project(&parent).unwrap_or_default());
        let mut opts =
            TurnOptions::agent(agent_id.clone(), self.sessions.session_model_id(&child_id));
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
                    "subagent '{child_id}' is already running a turn. Its result will be delivered when that turn settles."
                ));
            }
            Err(error) => {
                return ToolCallResult::error(format!("subagent_send failed: {error}"));
            }
        };
        let mut result = ToolCallResult::ok(status::format_started(&child_id, &turn_id, None));
        result.metadata =
            Some(serde_json::json!({ "child_session_id": child_id, "turn_id": turn_id }));
        result
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
                    "description": "Next assignment within this child session's established responsibility"
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
        Box::pin(async move { tool.call_send(input).await })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("subagent_send must be invoked via execute (async tool path)")
    }

    fn description(&self, _ctx: &Context) -> String {
        "Start another background turn in an idle child session. Requires its id and a next assignment that fits the session's established responsibility; fails while that child is running."
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
