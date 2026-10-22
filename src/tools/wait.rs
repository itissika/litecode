//! Code-surface wait on a `run` job (`wait`).
//!
//! `sec` is required. Values above [`MAX_WAIT_SECS`] are silently clamped.
//! A positive wait registers a snapshot waiter. The cancel token is only for
//! that wait; it does not kill the job.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::{AgentJobRegistry, WaitOutcome};
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job::{self, EndWait, present_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Built-in wait ceiling (seconds). Larger model values are clamped quietly.
pub const MAX_WAIT_SECS: u64 = 300;

pub struct WaitTool {
    jobs: Arc<AgentJobRegistry>,
}

impl WaitTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    /// Parse required `sec` and silently clamp to [`MAX_WAIT_SECS`].
    fn parse_sec(input: &Value) -> Result<u64, String> {
        let value = input
            .get("sec")
            .filter(|value| !value.is_null())
            .ok_or_else(|| crate::tool::must_be("sec", "a positive integer"))?;
        let seconds = value
            .as_u64()
            .ok_or_else(|| crate::tool::expected_type("sec", "integer", value))?;
        if seconds < 1 {
            return Err(crate::tool::must_be("sec", "a positive integer (>= 1)"));
        }
        Ok(seconds.min(MAX_WAIT_SECS))
    }

    fn present(
        &self,
        execution: &ToolExecutionContext,
        job_id: &str,
        exit_code: Option<i32>,
        note: Option<&str>,
    ) -> ToolCallResult {
        let notes: Vec<&str> = note.into_iter().collect();
        let Some(output) = command_job::format_job_output(
            &self.jobs,
            job_id,
            &execution.workspace_root,
            exit_code,
            &notes,
        ) else {
            return coded_error("io", "wait lost the job logs");
        };
        present_command(job_id, exit_code, output)
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let job_id = match crate::tool::require_nonempty_string_trimmed(&input, "id") {
            Ok(id) => id.to_string(),
            Err(error) => return coded_error("invalid_input", error),
        };
        let sec = match Self::parse_sec(&input) {
            Ok(seconds) => seconds,
            Err(error) => return coded_error("invalid_input", error),
        };
        if !command_job::owns_session(&self.jobs, &execution.session_id, &job_id) {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        }

        let timeout = Duration::from_secs(sec);
        let _waiter = EndWait::begin(
            &self.jobs,
            &execution.session_id,
            &execution.call_id,
            &job_id,
            timeout,
        );
        let outcome = self
            .jobs
            .wait_async(
                &execution.session_id,
                &job_id,
                Some(timeout),
                &execution.cancel,
            )
            .await;
        drop(_waiter);

        match outcome {
            WaitOutcome::Exited(notice) => self.present(
                &execution,
                &job_id,
                command_job::code_i32(notice.exit_code),
                None,
            ),
            WaitOutcome::TimedOut => self.present(
                &execution,
                &job_id,
                None,
                Some("wait budget elapsed; process still running"),
            ),
            WaitOutcome::Cancelled => coded_error("cancelled", "wait cancelled"),
            WaitOutcome::UnknownId(_) => {
                command_job::not_found(&self.jobs, &execution.session_id, &job_id)
            }
        }
    }
}

impl Tool for WaitTool {
    fn name(&self) -> &str {
        "wait"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Command.id from tools.run in this session."
                },
                "sec": {
                    "type": "integer",
                    "minimum": 1,
                    "description": format!(
                        "Seconds to block waiting for exit. Required. Values above {MAX_WAIT_SECS} are silently clamped to {MAX_WAIT_SECS}."
                    )
                }
            },
            "required": ["id", "sec"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(command_job::command_data_schema())
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/wait.md"))
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.run(input, execution))
    }

    fn timeout(&self) -> Option<Duration> {
        None
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string_trimmed(input, "id")?;
        Self::parse_sec(input)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::tool::Tool;
    
    
    use crate::tools::command_job::{StopJob, echo_then_sleep, sleep_shell, test_execution};
    use crate::tools::run::RunTool;

    fn tool() -> (WaitTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (WaitTool::new(Arc::clone(&jobs)), jobs)
    }

    async fn start_sleep(
        jobs: &Arc<AgentJobRegistry>,
        root: &std::path::Path,
        session: &str,
    ) -> String {
        let runner = RunTool::with_window(Arc::clone(jobs), Duration::from_millis(500));
        let started = runner
            .execute(
                serde_json::json!({"command": sleep_shell(30)}),
                test_execution(root, session),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let data = started.data.as_ref().unwrap();
        assert!(data["exit_code"].is_null(), "{data}");
        data["id"].as_str().unwrap().to_string()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_until_exit_sets_code() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-wait-exit";
        let runner = RunTool::with_window(Arc::clone(&jobs), Duration::from_millis(500));
        let started = runner
            .execute(
                serde_json::json!({"command": echo_then_sleep("ready", 1)}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let id = started.data.as_ref().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        let result = tool
            .execute(
                serde_json::json!({"id": id, "sec": 10}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["exit_code"], 0);
        assert!(data["output"].as_str().unwrap().contains("ready"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_timeout_keeps_null_exit() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-wait-to";
        let id = start_sleep(&jobs, dir.path(), session).await;
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        let started = Instant::now();
        let result = tool
            .execute(
                serde_json::json!({"id": id, "sec": 1}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert!(data["exit_code"].is_null(), "{data}");
        assert!(
            data["output"]
                .as_str()
                .unwrap()
                .contains("wait budget elapsed"),
            "{data}"
        );
    }

}
