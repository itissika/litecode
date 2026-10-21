//! Shared read/kill helpers for the code-surface command jobs.
//!
//! These tools do not spawn a process and do not put `call_id` in a path.
//! A wait goes through [`AgentJobRegistry::wait_async`]. The blocking registry
//! wait is not used.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::terminal::{AgentJobRegistry, TeeCapture, display_output_path};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

#[derive(Debug, Serialize)]
pub(crate) struct StreamView {
    head: String,
    tail: String,
    bytes: u64,
    truncated: bool,
}

pub(crate) fn stream_view(capture: &TeeCapture) -> StreamView {
    StreamView {
        head: capture.head.clone(),
        tail: capture.tail.clone(),
        bytes: capture.total_bytes as u64,
        truncated: capture.frozen || capture.truncated_on_disk,
    }
}

pub(crate) fn stream_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "head": { "type": "string" },
            "tail": { "type": "string" },
            "bytes": { "type": "integer", "minimum": 0 },
            "truncated": { "type": "boolean" }
        },
        "required": ["head", "tail", "bytes", "truncated"]
    })
}

pub(crate) fn owns_session(jobs: &AgentJobRegistry, session_id: &str, job_id: &str) -> bool {
    jobs.get(job_id)
        .is_some_and(|(_, _, owner, _)| owner == session_id)
}

/// `not_found` for an unknown id or another session. The text may list this
/// session's still-running ids. It does not list any other session.
pub(crate) fn not_found(jobs: &AgentJobRegistry, session_id: &str, job_id: &str) -> ToolCallResult {
    let running = jobs.running(session_id);
    let listed = running
        .iter()
        .map(|job| job.id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let message = if listed.is_empty() {
        format!("job '{job_id}' was not found in this session")
    } else {
        format!("job '{job_id}' was not found in this session. running: {listed}")
    };
    coded_error("not_found", message)
}

pub(crate) struct JobLogs {
    pub stdout: StreamView,
    pub stderr: StreamView,
    pub output_file: String,
    pub stderr_file: String,
}

pub(crate) fn job_logs(
    jobs: &AgentJobRegistry,
    job_id: &str,
    workspace_root: &Path,
) -> Option<JobLogs> {
    let (stdout, stderr, stdout_path, stderr_path) = jobs.pipe_streams(job_id)?;
    Some(JobLogs {
        stdout: stream_view(&stdout),
        stderr: stream_view(&stderr),
        output_file: display_output_path(&stdout_path, workspace_root),
        stderr_file: display_output_path(&stderr_path, workspace_root),
    })
}

pub(crate) fn code_i32(code: Option<u32>) -> Option<i32> {
    code.map(|code| code as i32)
}

/// Drops the `BashJobsSnapshot` waiter registered for one `command_output` call.
pub(crate) struct EndWait {
    jobs: Arc<AgentJobRegistry>,
    call_id: String,
}

impl EndWait {
    pub(crate) fn begin(
        jobs: &Arc<AgentJobRegistry>,
        session_id: &str,
        call_id: &str,
        job_id: &str,
        timeout: std::time::Duration,
    ) -> Self {
        jobs.begin_wait(session_id, call_id, Some(job_id), Some(timeout));
        Self {
            jobs: Arc::clone(jobs),
            call_id: call_id.to_string(),
        }
    }
}

impl Drop for EndWait {
    fn drop(&mut self) {
        self.jobs.end_wait(&self.call_id);
    }
}

#[cfg(test)]
pub(crate) fn test_execution(
    root: &Path,
    session_id: &str,
) -> crate::tool::trait_::ToolExecutionContext {
    crate::tool::trait_::ToolExecutionContext {
        path_mode: crate::workspace::ToolPathMode::All,
        workspace_root: root.to_path_buf(),
        call_id: "parent:code:0".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: usize::MAX,
        session_id: session_id.to_string(),
        session: None,
    }
}

#[cfg(test)]
pub(crate) fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            CloseHandle(handle);
            true
        }
    }
}

#[cfg(test)]
pub(crate) fn sleep_shell(seconds: u64) -> String {
    #[cfg(windows)]
    {
        if crate::config::git_install::find_git_bash().is_some() {
            format!("sleep {seconds}")
        } else {
            format!("Start-Sleep -Seconds {seconds}")
        }
    }
    #[cfg(not(windows))]
    {
        format!("sleep {seconds}")
    }
}

#[cfg(test)]
pub(crate) struct StopJob {
    pub jobs: Arc<AgentJobRegistry>,
    pub id: String,
    pub session_id: String,
}

#[cfg(test)]
impl Drop for StopJob {
    fn drop(&mut self) {
        if self.id.is_empty() {
            return;
        }
        let _ = self.jobs.kill_registered(&self.id, false);
        self.jobs.take_notice(&self.session_id, &self.id);
    }
}

#[cfg(test)]
pub(crate) fn echo_then_sleep(text: &str, seconds: u64) -> String {
    #[cfg(windows)]
    {
        if crate::config::git_install::find_git_bash().is_some() {
            format!("echo {text}; sleep {seconds}")
        } else {
            format!("Write-Output {text}; Start-Sleep -Seconds {seconds}")
        }
    }
    #[cfg(not(windows))]
    {
        format!("echo {text}; sleep {seconds}")
    }
}
