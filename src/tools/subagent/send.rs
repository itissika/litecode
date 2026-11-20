//! Continue an existing child session with another message (`subagent_send`).

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::runtime::{RuntimeHandle, TurnOptions};
use crate::session::SessionService;
use crate::session::manager::FollowUpOutcome;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::{LitecodeError, ToolCallResult};

use super::status;
use super::turn::start_turn_like_human;

pub struct SubagentSendTool {
    runtime: Option<RuntimeHandle>,
    service: Option<Arc<SessionService>>,
    depth: u32,
    session_id: Mutex<String>,
}

impl SubagentSendTool {
    pub fn new(runtime: RuntimeHandle, service: Arc<SessionService>, depth: u32) -> Self {
        Self {
            runtime: Some(runtime),
            service: Some(service),
            depth,
            session_id: Mutex::new(String::new()),
        }
    }

    pub fn cold() -> Self {
        Self {
            runtime: None,
            service: None,
            depth: 0,
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    async fn call_send(&self, input: Value) -> ToolCallResult {
        let (Some(runtime), Some(service)) = (self.runtime.as_ref(), self.service.as_ref()) else {
            return ToolCallResult::error("subagent.send requires an active runtime");
        };
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
        let is_child = service
            .descendant_session_ids(&parent)
            .iter()
            .any(|id| id == &child_id);
        if !is_child {
            return ToolCallResult::error(format!(
                "subagent '{child_id}' is not a child session of this session"
            ));
        }
        let mode = match SendMode::parse(&input) {
            Ok(mode) => mode,
            Err(e) => return ToolCallResult::error(e),
        };
        if let Err(error) = service.ensure_entry(&child_id).await {
            return ToolCallResult::error(format!("subagent '{child_id}' is unavailable: {error}"));
        }
        // WP-100: route the message by mode. A busy child never gets a second
        // turn; an idle child (FollowUpOutcome::Idle) gets a turn started here.
        if let Some(result) = self.deliver_to_busy(&child_id, &message, mode) {
            return result;
        }
        let agent_id = match service.agent_id(&child_id) {
            Some(agent) if !agent.is_empty() => agent,
            _ => {
                return ToolCallResult::error(format!(
                    "subagent '{child_id}' has no agent profile recorded"
                ));
            }
        };
        let project = service
            .project(&child_id)
            .unwrap_or_else(|| service.project(&parent).unwrap_or_default());
        // Identity only — model / tier / context mode come from the child's own
        // session row inside the session turn operation.
        let opts = TurnOptions::child(agent_id.clone());
        let retry_message = message.clone();
        let turn_id = match start_turn_like_human(
            runtime,
            service,
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
            // reservation. Deliver per mode instead of failing.
            Err(LitecodeError::AgentAlreadyRunning) => {
                return self
                    .deliver_to_busy(&child_id, &retry_message, mode)
                    .unwrap_or_else(|| {
                        ToolCallResult::error(format!(
                            "subagent '{child_id}' changed state during send; retry"
                        ))
                    });
            }
            Err(error) => {
                return ToolCallResult::error(format!("subagent_send failed: {error}"));
            }
        };
        let mut text = status::format_started(&child_id, &turn_id, None);
        text.push_str("status: started\n");
        let mut result = ToolCallResult::ok(text.clone()).with_data(serde_json::json!({
            "child_session_id": child_id,
            "mode": mode.as_str(),
            "status": "started",
            "turn_id": turn_id,
            "message": text,
        }));
        result.metadata = Some(serde_json::json!({
            "child_session_id": child_id,
            "turn_id": turn_id,
            "mode": mode.as_str(),
            "status": "started",
        }));
        result
    }

    /// `Some(result)` when the child was busy and the message was handled by
    /// `mode`; `None` when the child is idle and the caller must start a turn.
    fn deliver_to_busy(
        &self,
        child_id: &str,
        message: &str,
        mode: SendMode,
    ) -> Option<ToolCallResult> {
        let input = crate::types::UserInput::text(message.to_string());
        let meta = |status: &str, extra: Value| {
            let mut m = serde_json::json!({
                "child_session_id": child_id,
                "mode": mode.as_str(),
                "status": status,
            });
            if let (Some(obj), Value::Object(more)) = (m.as_object_mut(), extra) {
                obj.extend(more);
            }
            m
        };
        let Some(service) = self.service.as_ref() else {
            return Some(ToolCallResult::error(
                "subagent.send requires an active session service",
            ));
        };
        match mode {
            SendMode::Queue => match service.enqueue_follow_up(child_id, input) {
                Ok(FollowUpOutcome::Idle) => None,
                Ok(FollowUpOutcome::Queued { position }) => {
                    let msg = status::format_queued(child_id, position);
                    let mut r = ToolCallResult::ok(msg.clone()).with_data(serde_json::json!({
                        "child_session_id": child_id,
                        "mode": mode.as_str(),
                        "status": "queued",
                        "position": position,
                        "message": msg,
                    }));
                    r.metadata = Some(meta("queued", serde_json::json!({ "position": position })));
                    Some(r)
                }
                Err(error) => Some(ToolCallResult::error(format!(
                    "subagent '{child_id}' message could not be queued: {error}"
                ))),
            },
            SendMode::Steer => {
                if !service.is_turn_running_blocking(child_id) {
                    return None;
                }
                match service.enqueue_user_input(child_id, input) {
                    Ok(pending) => {
                        let msg = status::format_steered(child_id, &pending.id);
                        let mut r =
                            ToolCallResult::ok(msg.clone()).with_data(serde_json::json!({
                                "child_session_id": child_id,
                                "mode": mode.as_str(),
                                "status": "steered",
                                "pending_id": pending.id,
                                "message": msg,
                            }));
                        r.metadata = Some(meta(
                            "steered",
                            serde_json::json!({ "pending_id": pending.id }),
                        ));
                        Some(r)
                    }
                    Err(error) => Some(ToolCallResult::error(format!(
                        "subagent '{child_id}' message could not be steered: {error}"
                    ))),
                }
            }
            SendMode::StopAndSend => match service.cancel_and_follow_up(
                child_id,
                input,
                crate::types::StopOrigin::Parent,
            ) {
                Ok((_, FollowUpOutcome::Idle)) => None,
                Ok((_, FollowUpOutcome::Queued { .. })) => {
                    let msg = status::format_stopped_and_queued(child_id);
                    let mut r = ToolCallResult::ok(msg.clone()).with_data(serde_json::json!({
                        "child_session_id": child_id,
                        "mode": mode.as_str(),
                        "status": "stopped_and_queued",
                        "message": msg,
                    }));
                    r.metadata = Some(meta("stopped_and_queued", serde_json::json!({})));
                    Some(r)
                }
                Err(error) => Some(ToolCallResult::error(format!(
                    "subagent '{child_id}' could not be stopped and sent: {error}"
                ))),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendMode {
    Queue,
    Steer,
    StopAndSend,
}

impl SendMode {
    fn parse(input: &Value) -> Result<Self, String> {
        match input.get("mode") {
            None | Some(Value::Null) => Ok(Self::Queue),
            Some(Value::String(s)) => match s.as_str() {
                "queue" => Ok(Self::Queue),
                "steer" => Ok(Self::Steer),
                "stop_and_send" => Ok(Self::StopAndSend),
                other => Err(format!(
                    "unknown mode '{other}'; expected one of queue, steer, stop_and_send"
                )),
            },
            Some(_) => Err("mode must be a string: queue, steer, or stop_and_send".into()),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Steer => "steer",
            Self::StopAndSend => "stop_and_send",
        }
    }
}

impl Tool for SubagentSendTool {
    fn name(&self) -> &str {
        "subagent.send"
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentSendTool {
            runtime: self.runtime.clone(),
            service: self.service.clone(),
            depth: self.depth,
            session_id: Mutex::new(execution.session_id.clone()),
        };
        Box::pin(async move { tool.call_send(input).await })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("subagent_send must be invoked via execute (async tool path)")
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        SendMode::parse(input).map(|_| ())
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }
}

#[cfg(test)]
mod tests {}
