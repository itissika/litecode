//! Code-surface stop of a `run` job (`kill`).
//!
//! Killing only signals: the registry latches `tool_killed` and terminates the
//! process tree, while the supervisor still owns the exit. The tool then waits
//! for that exit, consumes the mailbox notice so the stop does not become a
//! natural-exit reminder, and reports `still_alive` only when the process
//! really outlived the reap wait. An unknown id or another session is
//! `not_found`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::terminal::AgentJobRegistry;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::command_job::{self, present_command};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// How long to wait for the supervisor to record the real exit code after the
/// tree kill. The wait is `tokio::time::sleep`, not the registry condvar.
const REAP_WAIT: Duration = Duration::from_secs(2);

pub struct KillTool {
    jobs: Arc<AgentJobRegistry>,
}

impl KillTool {
    pub fn new(jobs: Arc<AgentJobRegistry>) -> Self {
        Self { jobs }
    }

    /// Poll until the supervisor records the process exit. `Some(code)` means
    /// the process is gone; `None` means it outlived the reap wait (or the call
    /// was cancelled).
    async fn observed_exit(
        &self,
        job_id: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Option<Option<u32>> {
        let deadline = tokio::time::Instant::now() + REAP_WAIT;
        loop {
            match self.jobs.get(job_id) {
                Some((false, code, _, _)) => return Some(code),
                None => return None,
                Some((true, _, _, _)) => {}
            }
            if cancel.is_cancelled() || tokio::time::Instant::now() >= deadline {
                return match self.jobs.get(job_id) {
                    Some((false, code, _, _)) => Some(code),
                    _ => None,
                };
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn present(
        &self,
        execution: &ToolExecutionContext,
        job_id: &str,
        exit_code: Option<i32>,
        note: Option<&str>,
        killed: bool,
        still_alive: bool,
    ) -> ToolCallResult {
        let drain = command_job::exit_drain_note(&self.jobs, job_id, &execution.workspace_root);
        let mut notes: Vec<&str> = note.into_iter().collect();
        if killed {
            notes.push("process was killed");
        }
        if let Some(drain) = &drain {
            notes.push(drain);
        }
        let Some(output) = command_job::format_job_output(
            &self.jobs,
            job_id,
            &execution.workspace_root,
            exit_code,
            &notes,
        ) else {
            return coded_error("io", "kill lost the job logs");
        };
        if !still_alive {
            present_command(job_id, exit_code, output, None)
        } else {
            command_job::present_command_full(
                job_id,
                None,
                output,
                Some("kill signal sent but the process was still alive after the reap wait".into()),
                Some("still_alive"),
                None,
            )
        }
    }

    async fn run(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        let job_id = match crate::tool::require_nonempty_string_trimmed(&input, "id") {
            Ok(id) => id.to_string(),
            Err(error) => return coded_error("invalid_input", error),
        };
        if execution.cancel.is_cancelled() {
            return coded_error("cancelled", "kill cancelled before the signal");
        }
        if !command_job::owns_session(&self.jobs, &execution.session_id, &job_id) {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        }

        // `user_killed` stays false. The human click is `kill_from_ui`, which
        // leaves the mailbox notice. This path latches `tool_killed`; the notice
        // is consumed only once the exit is confirmed, so no natural-exit
        // reminder fires for a stop that landed.
        let Some(killed) = self.jobs.kill_registered(&job_id, false) else {
            return command_job::not_found(&self.jobs, &execution.session_id, &job_id);
        };
        if !killed.alive && !killed.reader_eof {
            // Draining: the shell was already gone and the tree kill closed
            // whatever descendant still held the pipes.
            self.jobs
                .wait_readers(&job_id, command_job::READER_GRACE)
                .await;
            self.jobs.take_notice(&execution.session_id, &job_id);
            return self.present(
                &execution,
                &job_id,
                command_job::code_i32(killed.exit_code),
                Some("process had already exited; remaining job processes were stopped"),
                true,
                false,
            );
        }
        if !killed.alive {
            // Sealed before the signal: report the exit that already happened.
            self.jobs.take_notice(&execution.session_id, &job_id);
            return self.present(
                &execution,
                &job_id,
                command_job::code_i32(killed.exit_code),
                Some("job had already exited"),
                false,
                false,
            );
        }
        match self.observed_exit(&job_id, &execution.cancel).await {
            Some(exit_code) => {
                self.jobs
                    .wait_readers(&job_id, command_job::READER_GRACE)
                    .await;
                self.jobs.take_notice(&execution.session_id, &job_id);
                self.present(
                    &execution,
                    &job_id,
                    command_job::code_i32(exit_code),
                    None,
                    true,
                    false,
                )
            }
            None => {
                // The stop did not land. Drop the latch so a later natural exit
                // is not reported as killed, and keep the notice for the seam.
                self.jobs.clear_tool_kill_if_alive(&job_id);
                self.present(&execution, &job_id, None, None, false, true)
            }
        }
    }
}

impl Tool for KillTool {
    fn name(&self) -> &str {
        "kill"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::tool::Tool;

    use crate::tools::command_job::{StopJob, pid_alive, sleep_shell, test_execution};
    use crate::tools::run::RunTool;

    fn tool() -> (KillTool, Arc<AgentJobRegistry>) {
        let jobs = Arc::new(AgentJobRegistry::new());
        (KillTool::new(Arc::clone(&jobs)), jobs)
    }

    async fn start_sleep(
        jobs: &Arc<AgentJobRegistry>,
        root: &std::path::Path,
        session: &str,
    ) -> (String, u32) {
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
        let job_id = data["id"].as_str().unwrap().to_string();
        let pid = jobs.job_pid(&job_id).expect("pid");
        (job_id, pid)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kill_stops_process_and_notes_output() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-kill";
        let (id, pid) = start_sleep(&jobs, dir.path(), session).await;
        let _stop = StopJob {
            jobs: Arc::clone(&jobs),
            id: id.clone(),
            session_id: session.into(),
        };
        assert!(pid_alive(pid));
        let started = Instant::now();
        let result = tool
            .execute(
                serde_json::json!({"id": id}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["id"].as_str().unwrap().len() > 0, true);
        assert_ne!(
            data.get("ok").and_then(|v| v.as_bool()),
            Some(false),
            "{data}"
        );
        assert!(
            data.get("reason").is_none() || data["reason"].is_null(),
            "{data}"
        );
        assert!(
            data["output"]
                .as_str()
                .unwrap()
                .contains("process was killed"),
            "{data}"
        );
        // Give the OS a moment; pid should die.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && pid_alive(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!pid_alive(pid), "pid should be dead");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kill_while_draining_stops_the_descendant() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-kill-drain";
        let started = RunTool::with_window(Arc::clone(&jobs), Duration::from_millis(500))
            .execute(
                serde_json::json!({"command": "(sleep 2; echo late > late.txt) & sleep 0"}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(started.error.is_none(), "{started:?}");
        let data = started.data.as_ref().expect("data");
        let id = data["id"].as_str().unwrap().to_string();
        assert!(
            jobs.readers_open(&id),
            "the descendant must still hold the pipe: {data}"
        );

        let result = tool
            .execute(
                serde_json::json!({"id": id}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert!(
            data["output"]
                .as_str()
                .unwrap()
                .contains("remaining job processes were stopped"),
            "{data}"
        );

        // The marker is written at 2s; the tree kill must reach the descendant
        // that outlived the shell long before that.
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !dir.path().join("late.txt").exists(),
            "a descendant survived the job tree kill"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kill_on_a_finished_job_reports_its_exit() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-kill-sealed";
        jobs.insert_finished_pipe_job(
            "bg_sealed01",
            session,
            "call_sealed",
            "echo hi",
            dir.path(),
            Some(0),
            false,
        )
        .expect("seed a finished job");

        let result = tool
            .execute(
                serde_json::json!({"id": "bg_sealed01"}),
                test_execution(dir.path(), session),
            )
            .await;
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["exit_code"], 0, "{data}");
        assert!(
            data.get("reason").is_none() || data["reason"].is_null(),
            "{data}"
        );
        assert!(
            data["output"]
                .as_str()
                .unwrap()
                .contains("job had already exited"),
            "{data}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn kill_without_a_signalable_process_reports_still_alive() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, jobs) = tool();
        let session = "sess-kill-nopid";
        // A registry row with no pid and no supervisor: the signal cannot land.
        jobs.insert_live_pipe_job("bg_nopid01", session, "call_nopid", "echo hi", dir.path())
            .expect("seed a live job");

        let result = tool
            .execute(
                serde_json::json!({"id": "bg_nopid01"}),
                test_execution(dir.path(), session),
            )
            .await;
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["reason"], "still_alive", "{data}");
        assert!(data["exit_code"].is_null(), "{data}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_id_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let (tool, _) = tool();
        let result = tool
            .execute(
                serde_json::json!({"id": "bg_missing"}),
                test_execution(dir.path(), "sess"),
            )
            .await;
        assert_eq!(result.error_code.as_deref(), Some("not_found"));
    }
}
