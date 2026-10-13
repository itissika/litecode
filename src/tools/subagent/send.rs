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
        // K2: a running child gets the message queued into its pending inbox;
        // pending_auto_turn / the step seam deliver it at the next opportunity.
        if self.sessions.is_turn_running_blocking(&child_id) {
            return self.enqueue(&child_id, message);
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
        let project = self
            .sessions
            .project(&child_id)
            .unwrap_or_else(|| self.sessions.project(&parent).unwrap_or_default());
        // Identity only — model / tier / context mode come from the child's own
        // session row inside `spawn_turn`.
        let opts = TurnOptions::child(agent_id.clone());
        let retry_message = message.clone();
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
            // Lost the race: the child started a turn between the check and the
            // reservation. Queue instead of failing.
            Err(LitecodeError::AgentAlreadyRunning) => {
                return self.enqueue(&child_id, retry_message);
            }
            Err(error) => {
                return ToolCallResult::error(format!("subagent_send failed: {error}"));
            }
        };
        let mut text = status::format_started(&child_id, &turn_id, None);
        text.push_str("queued: false\n");
        let mut result = ToolCallResult::ok(text);
        result.metadata = Some(serde_json::json!({
            "child_session_id": child_id,
            "turn_id": turn_id,
            "queued": false,
        }));
        result
    }

    fn enqueue(&self, child_id: &str, message: String) -> ToolCallResult {
        match self
            .sessions
            .enqueue_user_input(child_id, crate::types::UserInput::text(message))
        {
            Ok(pending) => {
                let mut result = ToolCallResult::ok(status::format_queued(child_id, &pending.id));
                result.metadata = Some(serde_json::json!({
                    "child_session_id": child_id,
                    "queued": true,
                    "pending_id": pending.id,
                }));
                result
            }
            Err(error) => ToolCallResult::error(format!(
                "subagent '{child_id}' message could not be queued: {error}"
            )),
        }
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
        crate::tools::description_text(include_str!("../descriptions/subagent_send.md"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TurnGuard;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::{AgentProfile, AgentRole, GlobalSettings};
    use crate::engines::WorkspaceEngines;
    use crate::ide_base::IdeBaseHandle;
    use crate::optional::EngineManager;
    use crate::workspace::WorkspaceService;
    use std::sync::atomic::AtomicU64;

    fn test_runtime(root: &std::path::Path) -> (RuntimeHandle, Arc<SessionManager>) {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role: AgentRole::Primary,
                model_ref: "default".into(),
                ..Default::default()
            },
        );
        let workspace_state = WorkspaceState::new(root);
        let resolved = resolve_without_catalog(global, workspace_state.clone());
        let workspace = WorkspaceService::new(root.to_path_buf()).unwrap();
        let engines = Arc::new(WorkspaceEngines::new());
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let ide = IdeBaseHandle::new(workspace, Arc::clone(&engines), Arc::clone(&hub));
        let runtime = RuntimeHandle::new(
            resolved,
            "default".into(),
            workspace_state,
            Arc::new(EngineManager::new()),
            engines,
            ide,
            Arc::new(AtomicU64::new(0)),
            root.join("global.db"),
        );
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            root.join("sessions.db").to_string_lossy().to_string(),
        ));
        (runtime, sessions)
    }

    #[tokio::test]
    async fn enqueues_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let parent = sessions
            .open_session("/proj", "default", None)
            .await
            .unwrap();
        let child = sessions
            .open_child_session("/proj", "reviewer", None, &parent, "call_1")
            .unwrap();
        sessions
            .reserve_turn(&child, "t1".into(), 10, "reviewer", "/proj")
            .unwrap();

        let tool = SubagentSendTool::new(runtime, Arc::clone(&sessions), 0);
        tool.set_active_session(parent.clone());
        let result = tool
            .call_send(serde_json::json!({"id": child, "message": "next step"}))
            .await;

        assert_eq!(
            result.level,
            crate::types::ToolSignalLevel::Ok,
            "{}",
            result.content
        );
        assert!(
            result.content.contains("queued: true"),
            "{}",
            result.content
        );
        let meta = result.metadata.clone().unwrap();
        assert_eq!(meta["queued"], serde_json::json!(true));
        let pending = sessions.pending_messages_snapshot(&child);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "next step");
        assert_eq!(meta["pending_id"], serde_json::json!(pending[0].id));
    }

    #[test]
    fn schema_has_no_mode_parameter() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, sessions) = test_runtime(dir.path());
        let tool = SubagentSendTool::new(runtime, sessions, 0);
        let schema = tool.schema();
        assert!(schema["properties"].get("mode").is_none());
        assert_eq!(schema["required"], serde_json::json!(["id", "message"]));
    }
}
