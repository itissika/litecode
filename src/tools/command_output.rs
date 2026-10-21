//! Code-surface read of a `run_command` job (`command_output`).
//!
//! Not a Direct tool. A positive `wait_sec` uses [`AgentJobRegistry::wait_async`]
//! and registers a snapshot waiter. The cancel token is only for that wait.
//! It is not given to the supervisor or the pipe readers, and it does not kill
//! the job.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::terminal::{AgentJobRegistry, WaitOutcome};
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job::{self, EndWait, JobLogs, StreamView};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

const MAX_WAIT_SECS: u64 = 120;

pub struct CommandOutputTool {
    jobs: Arc<AgentJobRegistry>,
}

impl CommandOutputTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    fn parse_wait_sec(input: &Value) -> Result<u64, String> {
        match input.get("wait_sec").filter(|value| !value.is_null()) {
            None => Ok(0),
            Some(value) => {
                let seconds = value
                    .as_u64()
                    .ok_or_else(|| crate::tool::expected_type("wait_sec", "integer", value))?;
                if seconds > MAX_WAIT_SECS {
                    return Err(crate::tool::must_be(
                        "wait_sec",
                        &format!("between 0 and {MAX_WAIT_SECS}"),
                    ));
                }
                Ok(seconds)
            }
        }
    }

    fn captured(
        &self,
        execution: &ToolExecutionContext,
        job_id: &str,
    ) -> Result<JobLogs, ToolCallResult> {
        command_job::job_logs(&self.jobs, job_id, &execution.workspace_root)
            .ok_or_else(|| coded_error("io", "command_output lost the job logs"))
    }

    fn present_running(&self, execution: &ToolExecutionContext, job_id: &str) -> ToolCallResult {
        let logs = match self.captured(execution, job_id) {
            Ok(logs) => logs,
            Err(error) => return error,
        };
        ToolOutcome::ok(OutputData::Running {
            job_id: job_id.to_string(),
            stdout: logs.stdout,
            stderr: logs.stderr,
            output_file: logs.output_file,
            stderr_file: logs.stderr_file,
        })
        .into_call_result(format!("kind: running\njob_id: {job_id}\n"))
    }

    fn present_exited(
        &self,
        execution: &ToolExecutionContext,
        job_id: &str,
        exit_code: Option<i32>,
    ) -> ToolCallResult {
        let logs = match self.captured(execution, job_id) {
            Ok(logs) => logs,
            Err(error) => return error,
        };
        let shown = exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "null".into());
        ToolOutcome::ok(OutputData::Exited {
            job_id: job_id.to_string(),
            exit_code,
            stdout: logs.stdout,
            stderr: logs.stderr,
            output_file: logs.output_file,
            stderr_file: logs.stderr_file,
        })
        .into_call_result(format!(
            "kind: exited\njob_id: {job_id}\nexit_code: {shown}\n"
        ))
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let job_id = match crate::tool::require_nonempty_string_trimmed(&input, "job_id") {
            Ok(id) => id.to_string(),
            Err(error) => return coded_error("invalid_input", error),
        };
        let wait_sec = match Self::parse_wait_sec(&input) {
            Ok(seconds) => seconds,
            Err(error) => return coded_error("invalid_input", error),
        };
        if !command_job::owns_session(&self.jobs, &execution.session_id, &job_id) {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        }

        // `wait_sec: 0` is a snapshot. It still goes through `wait_async` so an
        // already-exited job consumes its notice, but it does not register a waiter.
        let timeout = Duration::from_secs(wait_sec);
        let _waiter = if wait_sec > 0 {
            Some(EndWait::begin(
                &self.jobs,
                &execution.session_id,
                &execution.call_id,
                &job_id,
                timeout,
            ))
        } else {
            None
        };
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
            WaitOutcome::Exited(notice) => {
                self.present_exited(&execution, &job_id, command_job::code_i32(notice.exit_code))
            }
            WaitOutcome::TimedOut => self.present_running(&execution, &job_id),
            WaitOutcome::Cancelled => coded_error("cancelled", "command_output wait cancelled"),
            WaitOutcome::UnknownId(_) => {
                command_job::not_found(&self.jobs, &execution.session_id, &job_id)
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OutputData {
    Running {
        job_id: String,
        stdout: StreamView,
        stderr: StreamView,
        output_file: String,
        stderr_file: String,
    },
    Exited {
        job_id: String,
        exit_code: Option<i32>,
        stdout: StreamView,
        stderr: StreamView,
        output_file: String,
        stderr_file: String,
    },
}

impl Tool for CommandOutputTool {
    fn name(&self) -> &str {
        "command_output"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "job_id from a background or promoted run_command in this session."
                },
                "wait_sec": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": MAX_WAIT_SECS,
                    "description": "Seconds to wait for the process to exit. 0 returns the current snapshot."
                }
            },
            "required": ["job_id"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        let streams = command_job::stream_schema();
        Some(serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "wait_sec elapsed and the process is still running. The job was not killed.",
                    "properties": {
                        "kind": { "const": "running" },
                        "job_id": { "type": "string" },
                        "stdout": streams.clone(),
                        "stderr": streams.clone(),
                        "output_file": { "type": "string" },
                        "stderr_file": { "type": "string" }
                    },
                    "required": ["kind", "job_id", "stdout", "stderr", "output_file", "stderr_file"]
                },
                {
                    "type": "object",
                    "additionalProperties": false,
                    "description": "The process has exited. This consumes the exit notice.",
                    "properties": {
                        "kind": { "const": "exited" },
                        "job_id": { "type": "string" },
                        "exit_code": { "type": ["integer", "null"] },
                        "stdout": streams.clone(),
                        "stderr": streams,
                        "output_file": { "type": "string" },
                        "stderr_file": { "type": "string" }
                    },
                    "required": ["kind", "job_id", "exit_code", "stdout", "stderr", "output_file", "stderr_file"]
                }
            ]
        }))
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/command_output.md"))
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
        crate::tool::require_nonempty_string_trimmed(input, "job_id")?;
        Self::parse_wait_sec(input)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use crate::tool::Tool;
    use crate::tool::code_manifest::{render_code_tool_description, validate_code_contract};
    use crate::tool::{ToolSurface, project_tools};
    use crate::tools::command_job::{
        StopJob, echo_then_sleep, pid_alive, sleep_shell, test_execution,
    };
    use crate::tools::run_command::RunCommandTool;

    fn tool() -> (CommandOutputTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (CommandOutputTool::new(Arc::clone(&jobs)), jobs)
    }

    #[test]
    fn surface_schema_and_admission() {
        let (tool, _) = tool();
        assert_eq!(tool.surface(), ToolSurface::Code);
        assert!(tool.timeout().is_none());
        assert!(tool.is_cancellable());
        let description = tool.description(&crate::context_pipeline::Context {
            cwd: std::path::PathBuf::new(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(std::path::Path::new(
                ".",
            )),
            agents_md: None,
            claude_md: None,
        });
        assert!(!description.contains("wait_shell"));
        assert!(!description.contains("kill_shell"));
        assert!(validate_code_contract(&tool).is_ok());
        let rendered = render_code_tool_description(&[Arc::new(tool)]).expect("sdk");
        assert!(rendered.contains("command_output"));
        assert!(rendered.contains("running"));
        assert!(rendered.contains("exited"));
        assert!(!rendered.contains("wait_shell"));
        let projected = project_tools(vec![Arc::new(CommandOutputTool::new(Arc::new(
            AgentJobRegistry::new(),
        )))]);
        assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
        assert_eq!(projected.whitelist, vec!["command_output".to_string()]);
    }

    #[test]
    fn permission_is_fixed_allow_without_command_policy() {
        use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
        use crate::config::schema::GlobalSettings;
        use crate::config::schema::PermissionSurface;
        use crate::permission::floor::check_floor;
        use crate::permission::{
            BindingPathMode, MatchContext, PermissionAction, PermissionEngine, permission_surface,
        };

        assert_eq!(
            permission_surface("command_output"),
            PermissionSurface::Fixed
        );
        let root = std::path::Path::new(".");
        let ctx = MatchContext {
            workspace_root: root,
            path_mode: BindingPathMode::Unrestricted,
        };
        assert!(
            check_floor(
                "command_output",
                &serde_json::json!({"job_id": "bg_a", "command": "cargo build"}),
                &ctx
            )
            .is_none()
        );
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
        let engine = PermissionEngine::resolver(resolved, "default", 0);
        let decision = engine.evaluate_tool(
            "command_output",
            &serde_json::json!({"job_id": "bg_a", "command": "cargo build"}),
            root,
        );
        assert_eq!(decision.action, PermissionAction::Allow);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_sec_zero_reads_running_without_blocking() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        let runner = RunCommandTool::new(Arc::clone(&jobs));
        let started = runner
            .execute(
                serde_json::json!({
                    "command": echo_then_sleep("hello_out", 30),
                    "run_in_background": true
                }),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let started_data = started.data.unwrap();
        assert_eq!(started_data["kind"], "background", "{started_data}");
        let job_id = started_data["job_id"].as_str().unwrap().to_string();
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: job_id.clone(),
            session_id: "sess".into(),
        };
        let pid = jobs.job_pid(&job_id).expect("pid");
        let tool = CommandOutputTool::new(Arc::clone(&jobs));
        let mut data = serde_json::Value::Null;
        for _ in 0..10 {
            let started_at = Instant::now();
            let result = tool
                .execute(
                    serde_json::json!({"job_id": job_id, "wait_sec": 0}),
                    test_execution(dir.path(), "sess"),
                )
                .await;
            assert!(
                started_at.elapsed() < Duration::from_millis(500),
                "wait_sec 0 blocked for {:?}",
                started_at.elapsed()
            );
            assert!(result.error.is_none(), "{result:?}");
            data = result.data.unwrap();
            assert_eq!(data["kind"], "running");
            if data["stdout"]["head"]
                .as_str()
                .unwrap()
                .contains("hello_out")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(data["job_id"], job_id);
        assert!(
            data["stdout"]["head"]
                .as_str()
                .unwrap()
                .contains("hello_out"),
            "{data}"
        );
        let output_file = data["output_file"].as_str().unwrap();
        assert!(output_file.contains(".litecode/run_command/"));
        assert!(output_file.contains(&job_id));
        assert!(!output_file.contains("parent:code"));
        assert!(jobs.wire_snapshot("sess").waits.is_empty());
        assert!(pid_alive(pid), "snapshot must not kill the job");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn exited_read_consumes_notice_without_a_reminder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        jobs.insert_finished_pipe_job(
            "bg_exited1",
            "sess",
            "parent:code:0",
            "echo hi",
            dir.path(),
            Some(3),
            false,
        )
        .unwrap();
        assert!(jobs.mailbox_pending("sess"));
        let result = tool
            .execute(
                serde_json::json!({"job_id": "bg_exited1", "wait_sec": 0}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 3);
        assert_eq!(data["job_id"], "bg_exited1");
        assert!(!result.content.contains("<system-reminder>"));
        assert!(!jobs.mailbox_pending("sess"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_until_exit_consumes_notice() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        jobs.insert_live_pipe_job("bg_wait001", "sess", "parent:code:0", "sleep", dir.path())
            .unwrap();
        let jobs2 = Arc::clone(&jobs);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            jobs2.finish("bg_wait001", Some(4));
        });
        let result = tool
            .execute(
                serde_json::json!({"job_id": "bg_wait001", "wait_sec": 5}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["kind"], "exited");
        assert_eq!(data["exit_code"], 4);
        assert!(!jobs.mailbox_pending("sess"));
        assert!(jobs.wire_snapshot("sess").waits.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_during_wait_does_not_kill() {
        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        let runner = RunCommandTool::new(Arc::clone(&jobs));
        let started = runner
            .execute(
                serde_json::json!({
                    "command": sleep_shell(30),
                    "run_in_background": true
                }),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let job_id = started.data.as_ref().unwrap()["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: job_id.clone(),
            session_id: "sess".into(),
        };
        let pid = jobs.job_pid(&job_id).expect("pid");
        let seen = Arc::new(AtomicBool::new(false));
        let seen2 = Arc::clone(&seen);
        let jobs2 = Arc::clone(&jobs);
        let job_for_wait = job_id.clone();
        let mut exec = test_execution(dir.path(), "sess");
        exec.call_id = "wait-call".into();
        let cancel = exec.cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let snap = jobs2.wire_snapshot("sess");
            if snap
                .waits
                .iter()
                .any(|wait| wait.watching_id.as_deref() == Some(job_for_wait.as_str()))
            {
                seen2.store(true, Ordering::SeqCst);
            }
            cancel.cancel();
        });
        let tool = CommandOutputTool::new(Arc::clone(&jobs));
        let result = tool
            .execute(serde_json::json!({"job_id": job_id, "wait_sec": 30}), exec)
            .await;
        assert_eq!(
            result.error_code.as_deref(),
            Some("cancelled"),
            "{result:?}"
        );
        assert!(result.data.is_none());
        assert!(
            seen.load(Ordering::SeqCst),
            "wait did not register a waiter"
        );
        assert!(pid_alive(pid), "cancelling the wait killed the job");
        assert!(jobs.get(&job_id).unwrap().0, "job row was sealed");
        assert!(!jobs.mailbox_pending("sess"));
        assert!(jobs.wire_snapshot("sess").waits.is_empty());
    }

    #[test]
    fn other_session_is_not_found_and_does_not_leak() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        let secret = jobs
            .insert_live_pipe_job("bg_secret1", "other", "c", "echo", dir.path())
            .unwrap();
        std::fs::write(&secret, b"SECRET_OUTPUT").unwrap();
        jobs.insert_live_pipe_job("bg_mine001", "sess", "c", "sleep", dir.path())
            .unwrap();
        jobs.insert_live_pipe_job("bg_third01", "third", "c", "sleep", dir.path())
            .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(tool.run(
            serde_json::json!({"job_id": "bg_secret1", "wait_sec": 0}),
            test_execution(dir.path(), "sess"),
        ));
        assert_eq!(
            result.error_code.as_deref(),
            Some("not_found"),
            "{result:?}"
        );
        assert!(result.data.is_none());
        assert!(result.content.contains("bg_mine001"), "{}", result.content);
        assert!(!result.content.contains("bg_third01"), "{}", result.content);
        assert!(
            !result.content.contains("SECRET_OUTPUT"),
            "{}",
            result.content
        );
        assert!(jobs.get("bg_secret1").unwrap().0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_yields_so_a_sibling_can_run() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (tool, jobs) = tool();
        jobs.insert_live_pipe_job("bg_yield01", "sess", "c", "sleep", dir.path())
            .unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = Arc::clone(&flag);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            flag2.store(true, Ordering::SeqCst);
        });
        let result = tool
            .execute(
                serde_json::json!({"job_id": "bg_yield01", "wait_sec": 1}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        assert_eq!(result.data.unwrap()["kind"], "running");
        assert!(
            flag.load(Ordering::SeqCst),
            "sibling must run during wait_async"
        );
        assert!(jobs.get("bg_yield01").unwrap().0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_call_is_routed_and_does_not_run() {
        use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
        use crate::config::schema::GlobalSettings;
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tool::Tool;
        use crate::tool::executor::run_tool;
        use crate::tool::write_lock::WorkspaceWriteLock;
        use crate::types::FunctionToolCall;

        let dir = tempfile::tempdir().expect("tempdir");
        let jobs = Arc::new(AgentJobRegistry::new());
        jobs.insert_live_pipe_job("bg_direct1", "sess", "c", "sleep", dir.path())
            .unwrap();
        let tool = Arc::new(CommandOutputTool::new(Arc::clone(&jobs)));
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(dir.path()));
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let call = FunctionToolCall {
            arguments: r#"{"job_id":"bg_direct1","wait_sec":30}"#.into(),
            call_id: "call_1".into(),
            name: "command_output".into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let tools: Vec<Arc<dyn Tool>> = vec![tool];
        let sink = deny_permission_sink();
        let result = run_tool(
            &call,
            &tools,
            &permission,
            &ctx,
            "sess",
            "default",
            sink.as_ref(),
            tokio_util::sync::CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        assert!(
            result
                .content
                .contains("only available as tools.command_output inside run_code"),
            "{}",
            result.content
        );
        assert!(
            jobs.get("bg_direct1").unwrap().0,
            "direct call ran the tool"
        );
    }
}
