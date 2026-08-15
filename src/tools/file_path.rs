//! Shared agent file-path UX for built-in tools (schema hint, risk warnings, errors).
//!
//! Path resolution stays in [`crate::workspace`]; sensitive-path policy stays in
//! [`crate::permission::sensitive`]. This module only composes user-facing text.

use std::path::Path;

use crate::permission::sensitive::is_sensitive_system_path;
use crate::types::ToolCallResult;
use crate::workspace::is_resolved_outside_workspace;

pub use crate::workspace::AGENT_FILE_PATH_HINT as FILE_PATH_SCHEMA_HINT;

/// Directory-as-file error shared by read (workspace + direct FS).
pub fn directory_not_file_message(path_display: &str) -> String {
    format!(
        "{path_display} is a directory, not a file. Use glob to list files, or pass a file path."
    )
}

/// Not-found guidance when no similar-file suggestions are available.
pub fn missing_file_hint() -> String {
    "verify the path exists".to_string()
}

/// Attach outside-workspace / sensitive-path warning after a successful edit/write.
/// Preserves an existing LSP Hint on `result`.
pub fn with_path_risk_warning(
    result: ToolCallResult,
    workspace_root: &Path,
    raw_path: &str,
    resolved: &Path,
    operation: &str,
) -> ToolCallResult {
    let mut parts: Vec<String> = Vec::new();
    if is_resolved_outside_workspace(workspace_root, resolved) {
        parts.push(format!(
            "you are {operation} outside the workspace ({})",
            resolved.display()
        ));
    }
    let resolved_display = resolved.to_string_lossy();
    if is_sensitive_system_path(raw_path) || is_sensitive_system_path(&resolved_display) {
        parts.push("target is a sensitive system location".into());
    }
    if parts.is_empty() {
        result
    } else {
        result.with_warning(parts.join("; "))
    }
}
