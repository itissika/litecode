//! Snapshot barrier over child Session turns.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::session::live::LifecycleEvent;
use crate::session::manager::SessionManager;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::jobs::CompletionRef;
use super::status;

pub struct SubagentWaitTool {
    sessions: Arc<SessionManager>,
    cancel: CancellationToken,
    session_id: Mutex<String>,
}

impl SubagentWaitTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self {
            sessions,
            cancel: CancellationToken::new(),
            session_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    async fn call_wait(&self, input: Value) -> ToolCallResult {
        let parent = self.session_id();
        if parent.is_empty() {
            return ToolCallResult::error(
                "subagent_wait requires an active session execution context",
            );
        }

        // Subscribe before freezing the running-turn snapshot so no completion
        // can fall between selection and observation.
        let mut lifecycle = self.sessions.subscribe_lifecycle();
        let descendants: HashSet<String> = self
            .sessions
            .descendant_session_ids(&parent)
            .into_iter()
            .collect();
        let requested = input.get("ids").and_then(Value::as_array).map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        let explicit_ids = requested.is_some();
        let ids = requested.unwrap_or_else(|| descendants.iter().cloned().collect());
        let mut selected = HashMap::<String, String>::new();
        let mut completed = Vec::new();
        let mut skipped = Vec::<(String, String)>::new();
        for child_id in ids {
            if !descendants.contains(&child_id) {
                if explicit_ids {
                    skipped.push((child_id, "not a child of this session".into()));
                }
                continue;
            }
            if let Some(progress) = self.sessions.get_cached_progress(&child_id) {
                selected.insert(child_id, progress.turn_id);
                continue;
            }
            if !explicit_ids {
                continue;
            }
            match self.sessions.data().latest_turn_result_blocking(&child_id) {
                Ok(Some(result)) => completed.push(CompletionRef {
                    parent_session_id: parent.clone(),
                    child_session_id: child_id,
                    turn_id: result.turn_id,
                }),
                Ok(None) => skipped.push((child_id, "no completed turn".into())),
                Err(error) => skipped.push((child_id, format!("result unavailable: {error}"))),
            }
        }
        if selected.is_empty() && completed.is_empty() {
            return ToolCallResult::ok(status::format_wait_outcome(
                &self.sessions,
                &completed,
                &skipped,
            ));
        }
        let available = selected.len() + completed.len();
        let target = input
            .get("count")
            .and_then(Value::as_u64)
            .map(|count| (count as usize).clamp(1, available))
            .unwrap_or(available);

        loop {
            if completed.len() >= target {
                completed.truncate(target);
                return ToolCallResult::ok(status::format_wait_outcome(
                    &self.sessions,
                    &completed,
                    &skipped,
                ));
            }
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    return ToolCallResult::error("subagent_wait cancelled");
                }
                event = lifecycle.recv() => {
                    match event {
                        Ok(LifecycleEvent::TurnFinished { session_id, progress, .. }) => {
                            if selected.get(&session_id) == Some(&progress.turn_id) {
                                selected.remove(&session_id);
                                completed.push(CompletionRef {
                                    parent_session_id: parent.clone(),
                                    child_session_id: session_id,
                                    turn_id: progress.turn_id,
                                });
                            }
                        }
                        Ok(LifecycleEvent::SessionRemoved { session_id }) if selected.contains_key(&session_id) => {
                            return ToolCallResult::error(format!(
                                "subagent '{session_id}' was removed while waiting"
                            ));
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let settled = selected
                                .iter()
                                .filter_map(|(child_id, turn_id)| {
                                    self.sessions
                                        .data()
                                        .turn_result_blocking(child_id, turn_id)
                                        .ok()
                                        .map(|_| (child_id.clone(), turn_id.clone()))
                                })
                                .collect::<Vec<_>>();
                            for (child_id, turn_id) in settled {
                                selected.remove(&child_id);
                                completed.push(CompletionRef {
                                    parent_session_id: parent.clone(),
                                    child_session_id: child_id,
                                    turn_id,
                                });
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return ToolCallResult::error("session lifecycle stream closed while waiting");
                        }
                    }
                }
            }
        }
    }
}

impl Tool for SubagentWaitTool {
    fn name(&self) -> &str {
        "subagent_wait"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional child_session_ids. Omit to snapshot all currently running children."
                },
                "count": {
                    "type": "integer",
                    "description": "Number of selected child turns to await. Omit to await all selected turns."
                }
            }
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let tool = SubagentWaitTool {
            sessions: Arc::clone(&self.sessions),
            cancel: execution.cancel,
            session_id: Mutex::new(execution.session_id),
        };
        Box::pin(async move { tool.call_wait(input).await })
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error("subagent_wait must be invoked via execute (async tool path)")
    }

    fn description(&self, _ctx: &Context) -> String {
        "Wait for a snapshot of currently running child turns. ids selects which children; omit it to select all running children. Already-idle selected ids return their latest turn immediately; unknown ids are skipped. count is how many selected turns must settle; omit it to await all selected. Does not cancel children.".into()
    }

    fn timeout(&self) -> Option<u64> {
        None
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

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let object = input
            .as_object()
            .ok_or_else(|| crate::tool::expected_type("input", "object", input))?;
        if let Some(key) = object.keys().find(|key| *key != "ids" && *key != "count") {
            return Err(format!(
                "unsupported field '{key}'; subagent_wait accepts only ids and count"
            ));
        }
        if let Some(ids) = input.get("ids") {
            let ids = ids
                .as_array()
                .ok_or_else(|| crate::tool::expected_type("ids", "array", ids))?;
            if ids
                .iter()
                .any(|id| id.as_str().map(|value| value.is_empty()).unwrap_or(true))
            {
                return Err("ids must contain only non-empty strings".into());
            }
        }
        if let Some(count) = input.get("count")
            && count.as_u64().filter(|value| *value > 0).is_none()
        {
            return Err(crate::tool::must_be("count", "a positive integer"));
        }
        Ok(())
    }
}
