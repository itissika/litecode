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

/// Internal error data carried by a failed [`ToolOutcome`]. The code surface
/// exposes it as a rejected ordinary JavaScript `Error`, not as a value.
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

    pub fn failure(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
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
        // `invalid_args` is the bridge's rejection code for an invalid input, so a
        // contract row labelled with it must not fall through to Internal.
        "invalid_input" | "invalid_args" | "invalid_regex" | "no_match" | "empty"
        | "ambiguous" => ToolErrorKind::InvalidInput,
        "not_found" => ToolErrorKind::NotFound,
        "permission" | "permission_denied" => ToolErrorKind::PermissionDenied,
        "conflict" | "stale" | "stale_cursor" | "still_alive" => ToolErrorKind::Conflict,
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

/// Data the program receives, or the `HostError` it sees as a rejected call.
///
/// `retryable` is true only when the code is `conflict` or `timeout` and the
/// pipeline marked the error retryable.
pub fn program_result(result: &ToolCallResult) -> Result<Value, HostError> {
    let failed = result.error.is_some() || result.level == ToolSignalLevel::Error;
    if !failed {
        return Ok(result.data.clone().unwrap_or(Value::Null));
    }
    let code = program_error_code(result);
    program_error(result, code)
}

/// The code carried by the rejected call.
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
        && result.error.as_ref().is_some_and(|error| error.retryable);
    Err(HostError::tool(
        code,
        message,
        result.error_details.clone(),
        retryable,
    ))
}

#[cfg(test)]
mod tests {
    /// The JS-facing name must map to the same kind as its Rust-side source.
    #[test]
    fn invalid_args_is_invalid_input() {
        assert_eq!(kind_for_code("invalid_args"), super::ToolErrorKind::InvalidInput);
    }

    use super::*;
    use crate::types::ToolErrorKind;

    fn coded(kind: ToolErrorKind, code: &str) -> ToolCallResult {
        let mut result = ToolCallResult::error_with_kind(kind, format!("{code} happened"), false);
        result.error_code = Some(code.to_string());
        result
    }

    #[test]
    fn every_failure_is_a_rejected_host_error() {
        use crate::ptc::HostErrorKind;

        // Domain failures, engine states, and system failures all reject. The
        // distinction is in `HostError.code`, not in the JavaScript shape.
        for (kind, code) in [
            (ToolErrorKind::NotFound, "not_found"),
            (ToolErrorKind::InvalidInput, "invalid_regex"),
            (ToolErrorKind::InvalidInput, "invalid_input"),
            (ToolErrorKind::PermissionDenied, "permission_denied"),
            (ToolErrorKind::Io, "io"),
            (ToolErrorKind::Conflict, "conflict"),
            (ToolErrorKind::Conflict, "stale"),
            (ToolErrorKind::Unavailable, "unavailable"),
            (ToolErrorKind::Conflict, "still_alive"),
            (ToolErrorKind::Conflict, "stale_cursor"),
            (ToolErrorKind::Timeout, "timeout"),
            (ToolErrorKind::Internal, "internal"),
        ] {
            let error = program_result(&coded(kind, code)).expect_err("must reject");
            match error.kind {
                HostErrorKind::Tool {
                    code: actual, ..
                } => assert_eq!(actual, code, "{code}"),
                other => panic!("{code}: {other:?}"),
            }
            assert_eq!(error.message, format!("{code} happened"), "{code}");
        }
    }

    #[test]
    fn success_is_the_raw_payload_with_no_platform_ok() {
        let value = program_result(&ok_data(
            serde_json::json!({"paths": [], "message": "no match"}),
        ))
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({"paths": [], "message": "no match"})
        );

        // `ok` is an ordinary business field on a successful payload; the host
        // neither adds it nor rewrites it.
        let value = program_result(&ok_data(
            serde_json::json!({"ok": false, "path": "a", "message": "payload ok"}),
        ))
        .unwrap();
        assert_eq!(value["ok"], false);
    }
}
