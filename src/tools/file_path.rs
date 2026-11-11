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
    format!("{path_display} is a directory, not a file")
}

/// Domain failure with a stable code. `content` and `error.message` are the same fact.
pub fn coded_error(code: &str, message: impl Into<String>) -> ToolCallResult {
    let message = message.into();
    crate::tool::outcome::ToolOutcome::<Value>::failure(code, message.clone(), false)
        .into_call_result(message)
}

pub fn tool_path_error(error: crate::workspace::ToolPathError) -> ToolCallResult {
    let code = match &error {
        crate::workspace::ToolPathError::Io(_) => "io",
        _ => "invalid_input",
    };
    coded_error(code, error.to_string())
}

pub fn workspace_error(error: crate::workspace::WorkspaceError) -> ToolCallResult {
    use crate::workspace::WorkspaceError as Kind;
    let code = match &error {
        Kind::NotFound(_) => "not_found",
        Kind::NotFile(_) | Kind::IsDir(_) | Kind::AlreadyExists(_) | Kind::InvalidMove(_) => {
            "invalid_input"
        }
        Kind::Binary | Kind::Utf16 | Kind::NotUtf8 | Kind::MediaUnsupported(_) => "unsupported",
        Kind::TooLarge | Kind::Io(_) | Kind::Sqlite(_) | Kind::RevealFailed(_) => "io",
        Kind::MediaFailed(_) => "internal",
        Kind::PreviewTooLarge
        | Kind::NotSqlite
        | Kind::UnknownTable
        | Kind::Sandbox(_)
        | Kind::Tree(_)
        | Kind::RevealUnsupported
        | Kind::PreviewOffset => "internal",
    };
    coded_error(code, error.to_string())
}

pub fn litecode_error(error: crate::types::LitecodeError) -> ToolCallResult {
    use crate::types::LitecodeError as Kind;
    let code = match &error {
        Kind::Io(_) => "io",
        Kind::SessionNotFound(_) => "not_found",
        Kind::ToolExecution(message) => tool_message_code(message),
        _ => "internal",
    };
    let message = match error {
        Kind::ToolExecution(message) => message,
        other => other.to_string(),
    };
    coded_error(code, message)
}

fn tool_message_code(message: &str) -> &'static str {
    if message.contains("invalid regular expression") {
        return "invalid_regex";
    }
    if message.starts_with("path does not exist")
        || message.contains("not found")
        || message.starts_with("File not found")
    {
        return "not_found";
    }
    if message.starts_with("parameter ")
        || message.contains("must be")
        || message.contains("must stay")
        || message.contains("expected")
        || message.starts_with("invalid ")
        || message.contains("SAFE mode")
        || message.contains("Unix-style")
        || message.contains("path must")
    {
        return "invalid_input";
    }
    "internal"
}

/// Structured path risk, attached to a write or an edit that landed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PathRisk {
    /// The target resolves outside the workspace root.
    pub outside_workspace: bool,
    /// The target is a sensitive system location.
    pub sensitive: bool,
    /// Absolute path the target resolved to, present when it is outside.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
}

/// The path risk of a write or edit, or `None` when there is none.
pub fn path_risk(workspace_root: &Path, raw_path: &str, resolved: &Path) -> Option<PathRisk> {
    let outside_workspace = is_resolved_outside_workspace(workspace_root, resolved);
    let resolved_display = resolved.to_string_lossy();
    let sensitive =
        is_sensitive_system_path(raw_path) || is_sensitive_system_path(&resolved_display);
    if !outside_workspace && !sensitive {
        return None;
    }
    Some(PathRisk {
        outside_workspace,
        sensitive,
        resolved: outside_workspace.then(|| resolved.display().to_string()),
    })
}

/// Code-mode file argument. The only name is `path`.
pub fn code_path(input: &Value) -> Option<&str> {
    input
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
}

#[cfg(test)]
mod tests {}

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
