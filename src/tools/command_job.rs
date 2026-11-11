//! Shared helpers for Code-surface shell jobs (`run` / `wait` / `kill`).
//!
//! These helpers do not register tools. A wait goes through
//! [`AgentJobRegistry::wait_async`]. The blocking registry wait is not used.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use crate::terminal::{AgentJobRegistry, INLINE_FULL, TeeCapture, display_output_path};
use crate::tool::clamp::Clamp;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Sole success shape for `run` / `wait` / `kill`.
#[derive(Debug, Serialize)]
pub(crate) struct CommandData {
    pub id: String,
    pub exit_code: Option<i32>,
    /// Stdout exactly as the process wrote it; notes live in fields instead.
    pub output: String,
    /// Stderr exactly as the process wrote it. Empty when there was none.
    pub stderr: String,
    /// Shell that ran the command, e.g. `bash (MINGW64)`.
    pub shell: String,
    /// This call's time budget ran out while the process kept running.
    pub timed_out: bool,
    /// The process exited but a pipe is still open, so the streams can grow.
    pub draining: bool,
    /// The in-memory window froze this stream; the full text is in `output_log`.
    pub output_truncated: bool,
    /// The same fact for stderr.
    pub stderr_truncated: bool,
    /// The shell may rewrite POSIX-looking arguments such as `cmd /c`.
    pub path_conversion: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_log: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_log: Option<String>,
    /// Set when wait observes a tool-killed (or UI-killed) exit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub killed: Option<bool>,
    /// Input parameters adjusted to a legal value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clamped: Option<Vec<Clamp>>,
}

/// Both streams of one job, plus the facts about how they were captured.
pub(crate) struct JobOutput {
    pub stdout: String,
    pub stderr: String,
    pub draining: bool,
    pub output_truncated: bool,
    pub stderr_truncated: bool,
    pub output_log: Option<String>,
    pub stderr_log: Option<String>,
}

/// Facts the call site knows and the job does not.
#[derive(Default)]
pub(crate) struct CallFacts {
    pub killed: Option<bool>,
    pub timed_out: bool,
    pub path_conversion: bool,
    pub clamped: Vec<Clamp>,
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

/// How long a just-exited job may keep draining its pipes before `run` /
/// `wait` / `kill` return. A descendant that inherited the pipe can hold it
/// far longer; the tools then return the exit-time snapshot plus the log path.
pub(crate) const READER_GRACE: Duration = Duration::from_millis(50);

/// Build the streams plus their facts from pipe streams and exit state.
pub(crate) fn format_job_output(
    jobs: &AgentJobRegistry,
    job_id: &str,
    workspace_root: &Path,
) -> Option<JobOutput> {
    let (stdout, stderr, stdout_path, stderr_path) = jobs.pipe_streams(job_id)?;
    let mut output = compose_streams(
        &stdout,
        &stderr,
        Some(&display_output_path(&stdout_path, workspace_root)),
        Some(&display_output_path(&stderr_path, workspace_root)),
    );
    output.draining = jobs.readers_open(job_id);
    Some(output)
}

/// Like [`format_job_output`], but may discard small attached logs after exit.
pub(crate) fn format_exited_output(
    jobs: &AgentJobRegistry,
    job_id: &str,
    workspace_root: &Path,
) -> Option<JobOutput> {
    let (stdout, stderr, stdout_path, stderr_path) = jobs.pipe_streams(job_id)?;
    // A job whose shell exited while a descendant still holds the pipe is
    // draining: its log can still grow, so keep both files and say so.
    let draining = jobs.readers_open(job_id);
    let need_file = draining || stream_needs_file(&stdout) || stream_needs_file(&stderr);
    let (stdout_file, stderr_file) = if need_file {
        (
            Some(display_output_path(&stdout_path, workspace_root)),
            Some(display_output_path(&stderr_path, workspace_root)),
        )
    } else {
        if jobs.discard_attached(job_id) {
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
        }
        (None, None)
    };
    let mut output = compose_streams(
        &stdout,
        &stderr,
        stdout_file.as_deref(),
        stderr_file.as_deref(),
    );
    output.draining = draining;
    Some(output)
}

fn stream_needs_file(capture: &TeeCapture) -> bool {
    capture.frozen || capture.truncated_on_disk || capture.total_bytes > INLINE_FULL
}

fn stream_text(capture: &TeeCapture) -> String {
    let body = capture.snapshot();
    if capture.frozen && (!capture.head.is_empty() || !capture.tail.is_empty()) {
        // The marker stays inside the text: without it the head and the tail
        // would read as one continuous stream. It is never appended at the end.
        let mut out = String::new();
        out.push_str(&capture.head);
        out.push_str("\n... (truncated)\n");
        out.push_str(&capture.tail);
        if out.trim().is_empty() { body } else { out }
    } else {
        body
    }
}

fn compose_streams(
    stdout: &TeeCapture,
    stderr: &TeeCapture,
    stdout_file: Option<&str>,
    stderr_file: Option<&str>,
) -> JobOutput {
    JobOutput {
        stdout: stream_text(stdout),
        stderr: stream_text(stderr),
        draining: false,
        output_truncated: stdout.frozen || stdout.truncated_on_disk,
        stderr_truncated: stderr.frozen || stderr.truncated_on_disk,
        output_log: stdout_file.map(str::to_string),
        stderr_log: stderr_file.map(str::to_string),
    }
}

/// Killed-command presentation shared by wait.
///
/// `killed` is only set when the exit was a kill. Command payloads do not
/// carry an `ok` field; the ToolSuccess envelope owns success.
pub(crate) fn present_command(
    id: impl Into<String>,
    exit_code: Option<i32>,
    output: JobOutput,
    facts: CallFacts,
) -> ToolCallResult {
    let id = id.into();
    let shown = exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "null".into());
    crate::tool::outcome::ToolOutcome::ok(CommandData {
        id: id.clone(),
        exit_code,
        output: output.stdout,
        stderr: output.stderr,
        shell: crate::terminal::decode::shell_identity(),
        timed_out: facts.timed_out,
        draining: output.draining,
        output_truncated: output.output_truncated,
        stderr_truncated: output.stderr_truncated,
        path_conversion: facts.path_conversion,
        output_log: output.output_log,
        stderr_log: output.stderr_log,
        killed: facts.killed,
        clamped: (!facts.clamped.is_empty()).then_some(facts.clamped),
    })
    .into_call_result(format!("id: {id}\nexit_code: {shown}\n"))
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let output = || JobOutput {
        stdout: "hello\n".into(),
        stderr: String::new(),
        draining: false,
        output_truncated: false,
        stderr_truncated: false,
        output_log: None,
        stderr_log: None,
    };
    // run, wait, and kill share CommandData. Their own paths set killed,
    // timed_out, and clamped only sometimes, so the full state is a shape
    // check, not an output every tool can produce.
    let full = || {
        present_command(
            "01JCOMMANDJOB",
            Some(0),
            JobOutput {
                draining: true,
                output_truncated: true,
                stderr_truncated: true,
                output_log: Some("logs/out.log".into()),
                stderr_log: Some("logs/err.log".into()),
                ..output()
            },
            CallFacts {
                killed: Some(true),
                timed_out: true,
                path_conversion: true,
                clamped: vec![Clamp {
                    parameter: "sec".into(),
                    requested: serde_json::json!(600),
                    applied: serde_json::json!(300),
                }],
            },
        )
    };
    let bare = || present_command("01JCOMMANDJOB", Some(0), output(), CallFacts::default());
    ["run", "wait", "kill"]
        .into_iter()
        .map(|tool| DriftCase {
            tool,
            all_optional_present: full(),
            all_optional_absent: bare(),
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn job_out() -> JobOutput {
        JobOutput {
            stdout: "out".into(),
            stderr: "err".into(),
            draining: false,
            output_truncated: false,
            stderr_truncated: false,
            output_log: None,
            stderr_log: None,
        }
    }


    /// The cut marker sits between the head and the tail; it is never appended
    /// after the text, which would read as an extra trailing line.
    #[test]
    fn a_cut_stream_keeps_its_marker_between_head_and_tail() {
        let capture = crate::terminal::TeeCapture {
            path: std::path::PathBuf::new(),
            head: "first\n".into(),
            tail: "last\n".into(),
            frozen: true,
            total_bytes: 4096,
            truncated_on_disk: false,
        };
        let text = stream_text(&capture);
        assert_eq!(text, "first\n\n... (truncated)\nlast\n");
        assert!(text.contains("\n... (truncated)\n"), "{text}");
        assert!(!text.trim_end().ends_with("(truncated)"), "{text}");
    }
    #[test]
    fn a_path_conversion_flag_is_data_not_prose() {
        let result = present_command(
            "bg_test",
            Some(0),
            job_out(),
            CallFacts {
                path_conversion: true,
                ..CallFacts::default()
            },
        );
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().expect("data");
        assert!(data.get("ok").is_none(), "{data}");
        assert!(data.get("reason").is_none(), "{data}");
        assert!(data.get("message").is_none(), "{data}");
        assert_eq!(data["exit_code"], 0, "{data}");
        assert_eq!(data["path_conversion"], true, "{data}");
        assert_eq!(data["output"], "out");
        assert_eq!(data["stderr"], "err");
        assert!(!data["shell"].as_str().unwrap().is_empty());
    }

    #[test]
    fn a_timed_out_command_carries_state_fields_only() {
        let result = present_command(
            "bg_stuck",
            None,
            job_out(),
            CallFacts {
                timed_out: true,
                ..CallFacts::default()
            },
        );
        assert!(result.error.is_none(), "{result:?}");
        let data = result.data.as_ref().expect("data");
        assert!(data["ok"].is_null(), "{data}");
        assert!(data.get("message").is_none(), "{data}");
        assert!(data["exit_code"].is_null(), "{data}");
        assert_eq!(data["timed_out"], true, "{data}");
        assert!(
            data.get("killed").is_none() || data["killed"].is_null(),
            "{data}"
        );
        assert_eq!(data["output"], "out");
        assert_eq!(data["stderr"], "err");
    }
}
