//! Shared helpers for Code-surface shell jobs (`run` / `wait` / `kill`).
//!
//! These helpers do not register tools. A wait goes through
//! [`AgentJobRegistry::wait_async`]. The blocking registry wait is not used.

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::terminal::{AgentJobRegistry, INLINE_FULL, TeeCapture, display_output_path};
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Sole success shape for `run` / `wait` / `kill`.
#[derive(Debug, Serialize)]
pub(crate) struct CommandData {
    pub id: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

pub(crate) fn command_data_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Shell command handle and observation. exit_code is null while the process is still running (or when no code is available). output is always present.",
        "properties": {
            "id": {
                "type": "string",
                "description": "Handle for later wait / kill in the same session."
            },
            "exit_code": {
                "type": ["integer", "null"],
                "description": "Process exit code when finished; null when still running or no code is available."
            },
            "output": {
                "type": "string",
                "description": "Combined process text plus any truncation note, on-disk spill path, or status line. Always present (may be empty)."
            }
        },
        "required": ["id", "exit_code", "output"]
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

pub(crate) fn code_i32(code: Option<u32>) -> Option<i32> {
    code.map(|code| code as i32)
}

/// Build `Command.output` from pipe streams plus optional status lines.
pub(crate) fn format_job_output(
    jobs: &AgentJobRegistry,
    job_id: &str,
    workspace_root: &Path,
    exit_code: Option<i32>,
    extra_notes: &[&str],
) -> Option<String> {
    let (stdout, stderr, stdout_path, stderr_path) = jobs.pipe_streams(job_id)?;
    let stdout_file = display_output_path(&stdout_path, workspace_root);
    let stderr_file = display_output_path(&stderr_path, workspace_root);
    Some(compose_output(
        &stdout,
        &stderr,
        Some(&stdout_file),
        Some(&stderr_file),
        exit_code,
        extra_notes,
    ))
}

/// Like [`format_job_output`], but may discard small attached logs after exit.
pub(crate) fn format_exited_output(
    jobs: &AgentJobRegistry,
    job_id: &str,
    workspace_root: &Path,
    exit_code: Option<i32>,
    extra_notes: &[&str],
) -> Option<String> {
    let (stdout, stderr, stdout_path, stderr_path) = jobs.pipe_streams(job_id)?;
    let need_file = stream_needs_file(&stdout) || stream_needs_file(&stderr);
    let (stdout_file, stderr_file) = if need_file {
        (
            Some(display_output_path(&stdout_path, workspace_root)),
            Some(display_output_path(&stderr_path, workspace_root)),
        )
    } else if jobs.discard_attached(job_id) {
        let _ = std::fs::remove_file(&stdout_path);
        let _ = std::fs::remove_file(&stderr_path);
        (None, None)
    } else {
        (None, None)
    };
    Some(compose_output(
        &stdout,
        &stderr,
        stdout_file.as_deref(),
        stderr_file.as_deref(),
        exit_code,
        extra_notes,
    ))
}

fn stream_needs_file(capture: &TeeCapture) -> bool {
    capture.frozen
        || capture.truncated_on_disk
        || capture.total_bytes > INLINE_FULL
}

fn stream_text(capture: &TeeCapture) -> String {
    let body = capture.snapshot();
    if capture.frozen && (!capture.head.is_empty() || !capture.tail.is_empty()) {
        // Keep head/tail readable when the in-memory window is frozen.
        let mut out = String::new();
        out.push_str(&capture.head);
        out.push_str("\n... (truncated)\n");
        out.push_str(&capture.tail);
        if out.trim().is_empty() { body } else { out }
    } else {
        body
    }
}

fn compose_output(
    stdout: &TeeCapture,
    stderr: &TeeCapture,
    stdout_file: Option<&str>,
    stderr_file: Option<&str>,
    exit_code: Option<i32>,
    extra_notes: &[&str],
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let out_text = stream_text(stdout);
    let err_text = stream_text(stderr);
    if !out_text.is_empty() {
        parts.push(out_text);
    }
    if !err_text.is_empty() {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push(format!("[stderr]\n{err_text}"));
    }

    let mut notes: Vec<String> = Vec::new();
    if stdout.frozen || stdout.truncated_on_disk {
        notes.push(format!(
            "stdout truncated ({} bytes{})",
            stdout.total_bytes,
            stdout_file
                .map(|p| format!("; full log: {p}"))
                .unwrap_or_default()
        ));
    } else if let Some(path) = stdout_file {
        if stdout.total_bytes > INLINE_FULL {
            notes.push(format!("stdout log: {path}"));
        }
    }
    if stderr.frozen || stderr.truncated_on_disk {
        notes.push(format!(
            "stderr truncated ({} bytes{})",
            stderr.total_bytes,
            stderr_file
                .map(|p| format!("; full log: {p}"))
                .unwrap_or_default()
        ));
    } else if let Some(path) = stderr_file {
        if stderr.total_bytes > INLINE_FULL {
            notes.push(format!("stderr log: {path}"));
        }
    }
    if exit_code.is_none() && parts.is_empty() && extra_notes.is_empty() {
        notes.push("still running; no output yet".into());
    }
    for note in extra_notes {
        if !note.is_empty() {
            notes.push((*note).to_string());
        }
    }
    for note in notes {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push(format!("[{note}]"));
    }
    parts.join("\n")
}

pub(crate) fn present_command(
    id: impl Into<String>,
    exit_code: Option<i32>,
    output: String,
) -> ToolCallResult {
    let id = id.into();
    let shown = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "null".into());
    crate::tool::outcome::ToolOutcome::ok(CommandData {
        id: id.clone(),
        exit_code,
        output,
    })
    .into_call_result(format!("id: {id}\nexit_code: {shown}\n"))
}

/// Drops the snapshot waiter registered for one `wait` call.
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
