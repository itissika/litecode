//! Wait for background subagent jobs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::types::ToolCallResult;

use super::jobs::{SubagentJobBoard, WaitOutcome};
use super::status;

const MAX_WAIT_SECS: u64 = 600;

pub struct SubagentWaitTool {
    pub jobs: Arc<SubagentJobBoard>,
    cancel: CancellationToken,
    session_id: Mutex<String>,
    call_id: Mutex<String>,
}

impl SubagentWaitTool {
    pub fn new(jobs: Arc<SubagentJobBoard>) -> Self {
        Self {
            jobs,
            cancel: CancellationToken::new(),
            session_id: Mutex::new(String::new()),
            call_id: Mutex::new(String::new()),
        }
    }

    fn session_id(&self) -> String {
        self.session_id.lock().unwrap().clone()
    }

    fn call_id(&self) -> String {
        self.call_id.lock().unwrap().clone()
    }

    fn call_wait(&self, input: Value) -> ToolCallResult {
        let id = input["id"].as_str().filter(|s| !s.is_empty());
        let sec = input["sec"].as_u64();
        let sid = self.session_id();
        let call_id = self.call_id();
        let timeout = sec.map(Duration::from_secs);
        self.jobs.begin_wait(&sid, &call_id, id, timeout);
        let outcome = self.jobs.wait(&sid, id, timeout, &self.cancel, true);
        self.jobs.end_wait(&call_id);
        match outcome {
            WaitOutcome::Exited(notice) => {
                let jobs = self.jobs.running(&sid);
                ToolCallResult::ok(status::format_exited_status(&notice, &jobs))
            }
            WaitOutcome::TimedOut => {
                let jobs = self.jobs.running(&sid);
                ToolCallResult::ok(status::format_waited_status(&jobs))
            }
            WaitOutcome::Cancelled => ToolCallResult::error("subagent_wait cancelled"),
            WaitOutcome::UnknownId(unknown) => {
                let jobs = self.jobs.running(&sid);
                ToolCallResult::error(status::format_unknown_task(&unknown, &jobs))
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
                "id": {
                    "type": "string",
                    "description": "child_session_id to wait for. Omit to wait on sec, or until any child of this session exits."
                },
                "sec": {
                    "type": "integer",
                    "description": "Seconds to wait (1-600). With id: return when that child exits or time elapses. Without id: sleep, or return sooner if any child exits."
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
            jobs: Arc::clone(&self.jobs),
            cancel: execution.cancel.clone(),
            session_id: Mutex::new(execution.session_id.clone()),
            call_id: Mutex::new(execution.call_id.clone()),
        };
        Box::pin(async move { tool.call_wait(input) })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        self.call_wait(input)
    }

    fn description(&self, _ctx: &Context) -> String {
        "Wait until a given child exits, until any child of this session exits, or until sec elapses. Does not stop the child.".into()
    }

    fn timeout(&self) -> Option<u64> {
        None
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn set_active_session(&self, session_id: String) {
        *self.session_id.lock().unwrap() = session_id;
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        let id = input
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let sec = input.get("sec");
        if id.is_none() && sec.is_none() {
            return Err("missing required parameter 'id' or 'sec'".into());
        }
        if let Some(v) = sec {
            let n = v
                .as_u64()
                .ok_or_else(|| crate::tool::expected_type("sec", "integer", v))?;
            if !(1..=MAX_WAIT_SECS).contains(&n) {
                return Err(crate::tool::must_be("sec", "between 1 and 600"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_requires_id_or_sec() {
        let tool = SubagentWaitTool::new(Arc::new(SubagentJobBoard::new()));
        assert!(tool.validate_input(&serde_json::json!({})).is_err());
        assert!(
            tool.validate_input(&serde_json::json!({"id": "child-a"}))
                .is_ok()
        );
        assert!(tool.validate_input(&serde_json::json!({"sec": 2})).is_ok());
        assert!(tool.validate_input(&serde_json::json!({"sec": 0})).is_err());
    }
}
