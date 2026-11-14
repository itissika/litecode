//! Shared helpers for Code-surface unified exec sessions.

use std::path::Path;
use std::time::Duration;

use crate::terminal::AgentJobRegistry;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// How long a just-exited session may keep draining its output pipe before a
/// caller snapshots it.
pub(crate) const READER_GRACE: Duration = Duration::from_millis(50);

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
