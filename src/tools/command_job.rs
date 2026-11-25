//! Shared helpers for shell command execution.

use std::time::Duration;

/// How long a just-exited session may keep draining its output pipe before a
/// caller snapshots it.
pub(crate) const READER_GRACE: Duration = Duration::from_millis(50);

#[cfg(all(test, unix))]
pub(crate) fn test_execution(
    root: &std::path::Path,
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
