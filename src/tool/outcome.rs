//! Code-surface result contract.
//!
//! `Tool::execute` still returns [`ToolCallResult`] because direct tools share
//! that pipeline. A program sees [`program_result`]: success is the tool's
//! data, and a domain failure is a [`crate::ptc::HostError`].

use serde::Serialize;
use serde_json::Value;

use crate::ptc::HostError;
use crate::types::{ToolCallResult, ToolErrorKind, ToolSignalLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl Diagnostic {
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            details: None,
        }
    }

    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: DiagnosticSeverity::Info,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Program-facing error. Distinct from [`crate::types::ToolError`], which keeps
/// `kind` for the shared pipeline and Phase 0 fixtures.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Page {
    pub has_more: bool,
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

/// File identity. Phase 1 defines the shape; hash and bytes are filled in later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs: Option<String>,
    pub hash: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome<T> {
    pub data: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
    pub error: Option<ToolError>,
}

impl<T> ToolOutcome<T> {
    pub fn ok(data: T) -> Self {
        Self {
            data: Some(data),
            diagnostics: Vec::new(),
            error: None,
        }
    }

    pub fn failure(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        let message = message.into();
        Self {
            data: None,
            diagnostics: Vec::new(),
            error: Some(ToolError {
                code: code.into(),
                message,
                details: None,
                retryable,
            }),
        }
    }

    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self
    }

    pub fn with_error_details(mut self, details: Value) -> Self {
        if let Some(error) = self.error.as_mut() {
            error.details = Some(details);
        }
        self
    }
}

impl<T: Serialize> ToolOutcome<T> {
    /// Project onto the shared pipeline result. `content` is not the contract.
    pub fn into_call_result(self, content: impl Into<String>) -> ToolCallResult {
        let content = content.into();
        let ToolOutcome {
            data,
            diagnostics,
            error,
        } = self;
        let mut result = if let Some(error) = error {
            let mut result = ToolCallResult::error_with_kind(
                kind_for_code(&error.code),
                error.message,
                error.retryable,
            );
            result.content = content;
            result.error_code = Some(error.code);
            result.error_details = error.details;
            result
        } else {
            let mut result = ToolCallResult::ok(content);
            if let Some(data) = data {
                result.data = serde_json::to_value(data).ok();
            }
            result
        };
        result.diagnostics = diagnostics;
        result
    }
}

pub fn kind_for_code(code: &str) -> ToolErrorKind {
    match code {
        "invalid_input" | "invalid_regex" => ToolErrorKind::InvalidInput,
        "not_found" => ToolErrorKind::NotFound,
        "permission_denied" => ToolErrorKind::PermissionDenied,
        "conflict" | "stale" | "stale_cursor" => ToolErrorKind::Conflict,
        "unavailable" | "unsupported" => ToolErrorKind::Unavailable,
        "timeout" => ToolErrorKind::Timeout,
        "cancelled" => ToolErrorKind::Cancelled,
        "io" => ToolErrorKind::Io,
        "output_limit" => ToolErrorKind::OutputLimit,
        "invalid_output" => ToolErrorKind::InvalidOutput,
        _ => ToolErrorKind::Internal,
    }
}

/// Success result whose facts live in `data`. `content` stays empty.
pub fn ok_data(data: impl Serialize) -> ToolCallResult {
    let mut result = ToolCallResult::ok(String::new());
    result.data = Some(serde_json::to_value(data).unwrap_or(Value::Null));
    result
}

pub fn error_kind_code(kind: ToolErrorKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "internal".into())
}

/// Data the program receives, or the `ToolError` it should see.
///
/// `retryable` is true only when the code is `conflict` or `timeout` and the
/// pipeline marked the error retryable.
pub fn program_result(result: &ToolCallResult) -> Result<Value, HostError> {
    let failed = result.error.is_some() || result.level == ToolSignalLevel::Error;
    if !failed {
        return Ok(with_ok_flag(result.data.clone().unwrap_or(Value::Null)));
    }
    let code = program_error_code(result);
    if let Some(reason) = soft_failure_reason(&code) {
        // WP-210 G: the tool did not get the job done, but the program did
        // nothing wrong. Resolve `{ok:false, reason, message}` instead of
        // rejecting so one bad item does not end a batch.
        let message = result
            .error
            .as_ref()
            .map(|error| error.message.clone())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| result.content.clone());
        let mut map = serde_json::Map::new();
        map.insert("ok".into(), Value::Bool(false));
        map.insert("reason".into(), Value::String(reason.into()));
        map.insert("message".into(), Value::String(message));
        return Ok(Value::Object(map));
    }
    program_error(result, code)
}

/// Domain codes that resolve `{ok:false}` (WP-210 G, Q10 enum). Everything
/// else (conflict/stale, timeout, cancelled, unavailable, internal, …) still
/// rejects `ToolError`.
pub fn soft_failure_reason(code: &str) -> Option<&'static str> {
    match code {
        "not_found" => Some("not_found"),
        "invalid_input" | "invalid_regex" => Some("invalid_args"),
        "permission_denied" => Some("permission"),
        "io" => Some("io"),
        "no_match" => Some("no_match"),
        "empty" => Some("empty"),
        _ => None,
    }
}

/// Every resolved object carries a leading `ok`. Tools that already set it
/// (edit, webfetch, …) keep their own value.
fn with_ok_flag(data: Value) -> Value {
    match data {
        Value::Object(map) if !map.contains_key("ok") => {
            let mut out = serde_json::Map::with_capacity(map.len() + 1);
            out.insert("ok".into(), Value::Bool(true));
            out.extend(map);
            Value::Object(out)
        }
        other => other,
    }
}

fn program_error_code(result: &ToolCallResult) -> String {
    result.error_code.clone().unwrap_or_else(|| {
        result
            .error
            .as_ref()
            .map(|error| error_kind_code(error.kind))
            .unwrap_or_else(|| "internal".into())
    })
}

fn program_error(result: &ToolCallResult, code: String) -> Result<Value, HostError> {
    let message = result
        .error
        .as_ref()
        .map(|error| error.message.clone())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| result.content.clone());
    let retryable = matches!(code.as_str(), "conflict" | "timeout")
        && result
            .error
            .as_ref()
            .is_some_and(|error| error.retryable);
    Err(HostError::tool(
        code,
        message,
        result.error_details.clone(),
        retryable,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolErrorKind;

    fn coded(kind: ToolErrorKind, code: &str) -> ToolCallResult {
        let mut result = ToolCallResult::error_with_kind(kind, format!("{code} happened"), false);
        result.error_code = Some(code.to_string());
        result
    }

    #[test]
    fn domain_failures_resolve_ok_false_and_system_failures_reject() {
        // read missing file / grep bad regex / argument validation / permission / io.
        for (kind, code, reason) in [
            (ToolErrorKind::NotFound, "not_found", "not_found"),
            (ToolErrorKind::InvalidInput, "invalid_regex", "invalid_args"),
            (ToolErrorKind::InvalidInput, "invalid_input", "invalid_args"),
            (ToolErrorKind::PermissionDenied, "permission_denied", "permission"),
            (ToolErrorKind::Io, "io", "io"),
        ] {
            let value = program_result(&coded(kind, code)).expect("resolves, not rejects");
            assert_eq!(value["ok"], false, "{code}");
            assert_eq!(value["reason"], reason, "{code}");
            assert_eq!(value["message"], format!("{code} happened"));
        }
        // System failures still reject ToolError.
        for (kind, code) in [
            (ToolErrorKind::Conflict, "stale"),
            (ToolErrorKind::Timeout, "timeout"),
            (ToolErrorKind::Unavailable, "unavailable"),
            (ToolErrorKind::Internal, "internal"),
        ] {
            assert!(program_result(&coded(kind, code)).is_err(), "{code}");
        }
        // Success gains a leading ok:true; a tool's own ok (edit miss) is kept.
        let value = program_result(&ok_data(serde_json::json!({"paths": [], "message": "no match"}))).unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), r#"{"ok":true,"paths":[],"message":"no match"}"#);
        let value = program_result(&ok_data(serde_json::json!({"ok": false, "path": "a", "message": "old_string was not found", "reason": "no_match"}))).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["reason"], "no_match");
    }
}
