//! Shared agent file-path UX for built-in tools (schema hint, risk warnings, errors).
//!
//! Path resolution stays in [`crate::workspace`]; sensitive-path policy stays in
//! [`crate::permission::sensitive`]. This module only composes user-facing text.

use std::path::Path;

use serde_json::Value;

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
    let Some(msg) = path_risk_warning_text(workspace_root, raw_path, resolved, operation) else {
        return result;
    };
    match result
        .warning_status
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
    {
        Some(existing) => result.with_warning(format!("{existing}; {msg}")),
        None => result.with_warning(msg),
    }
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

/// Code-mode file argument. `path` is the current name; `file_path` is the
/// historical one still stored in old transcripts and direct unit-test calls.
pub fn code_path(input: &Value) -> Option<&str> {
    if let Some(path) = input.get("path").and_then(Value::as_str)
        && !path.is_empty()
    {
        return Some(path);
    }
    input
        .get("file_path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
}

pub fn require_code_path(input: &Value) -> Result<&str, String> {
    let path_empty = input
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(str::is_empty);
    let legacy_empty = input.get("path").is_none()
        && input
            .get("file_path")
            .and_then(Value::as_str)
            .is_some_and(str::is_empty);
    if path_empty || legacy_empty {
        return Err("parameter 'path' must be a non-empty string".to_string());
    }
    code_path(input).ok_or_else(|| "parameter 'path' is required".to_string())
}

/// Make `path` and `file_path` name the same file so schema uses `path` while
/// the sensitive-path floor and stored `file_path` rules still see their key.
pub fn alias_code_path(input: &mut Value) {
    let Some(obj) = input.as_object_mut() else {
        return;
    };
    if let Some(path) = obj.get("path").cloned() {
        obj.insert("file_path".to_string(), path);
    } else if let Some(legacy) = obj.get("file_path").cloned() {
        obj.insert("path".to_string(), legacy);
    }
}

/// Schema view of a code-mode file call. The model-facing parameter is `path`.
pub fn without_legacy_file_path(input: &Value) -> Value {
    let mut view = input.clone();
    if let Some(obj) = view.as_object_mut() {
        obj.remove("file_path");
    }
    view
}
