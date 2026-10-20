//! Shared agent file-path UX for built-in tools (schema hint, risk warnings, errors).
//!
//! Path resolution stays in [`crate::workspace`]; sensitive-path policy stays in
//! [`crate::permission::sensitive`]. This module only composes user-facing text.

use std::path::Path;

use serde_json::Value;

use crate::permission::sensitive::is_sensitive_system_path;
use crate::tool::outcome::Diagnostic;
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

/// Attach outside-workspace / sensitive-path diagnostics after a successful edit/write.
pub fn with_path_risk_warning(
    mut result: ToolCallResult,
    workspace_root: &Path,
    raw_path: &str,
    resolved: &Path,
    operation: &str,
) -> ToolCallResult {
    if is_resolved_outside_workspace(workspace_root, resolved) {
        result.diagnostics.push(
            Diagnostic::warning(
                "path_outside_workspace",
                format!(
                    "you are {operation} outside the workspace ({})",
                    resolved.display()
                ),
            )
            .with_details(serde_json::json!({
                "path": raw_path,
                "resolved": resolved.display().to_string(),
                "operation": operation,
            })),
        );
    }
    let resolved_display = resolved.to_string_lossy();
    if is_sensitive_system_path(raw_path) || is_sensitive_system_path(&resolved_display) {
        result.diagnostics.push(
            Diagnostic::warning(
                "sensitive_path",
                "target is a sensitive system location",
            )
            .with_details(serde_json::json!({ "path": raw_path })),
        );
    }
    result
}

/// Warning text for an outside-workspace or sensitive path, if any.
pub fn path_risk_warning_text(
    workspace_root: &Path,
    raw_path: &str,
    resolved: &Path,
    operation: &str,
) -> Option<String> {
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
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Code-mode file argument. The only name is `path`.
pub fn code_path(input: &Value) -> Option<&str> {
    input
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
}

pub fn require_code_path(input: &Value) -> Result<&str, String> {
    match input.get("path") {
        None => Err("parameter 'path' is required".to_string()),
        Some(Value::String(path)) if path.is_empty() => {
            Err("parameter 'path' must be a non-empty string".to_string())
        }
        Some(Value::String(path)) => Ok(path.as_str()),
        Some(_) => Err("parameter 'path' must be a non-empty string".to_string()),
    }
}
