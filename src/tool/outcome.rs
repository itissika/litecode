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
        return Ok(result.data.clone().unwrap_or(Value::Null));
    }
    let code = result.error_code.clone().unwrap_or_else(|| {
        result
            .error
            .as_ref()
            .map(|error| error_kind_code(error.kind))
            .unwrap_or_else(|| "internal".into())
    });
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
    use crate::ptc::HostErrorKind;
    use serde_json::json;

    #[test]
    fn success_resolves_data_and_keeps_diagnostics_off_the_value() {
        let outcome = ToolOutcome::ok(json!({"applied": 9, "failed": 1})).with_diagnostic(
            Diagnostic::warning("edit_block_failed", "block 1 failed")
                .with_details(json!({"index": 1})),
        );
        let result = outcome.into_call_result("body");
        let value = program_result(&result).expect("success");
        assert_eq!(value["applied"], 9);
        assert_eq!(value["failed"], 1);
        assert!(value.get("status").is_none());
        assert_eq!(result.diagnostics[0].code, "edit_block_failed");
    }

    #[test]
    fn invalid_regex_rejects_with_code_and_details_and_is_not_retryable() {
        let outcome = ToolOutcome::<serde_json::Value>::failure("invalid_regex", "bad pattern", true)
            .with_error_details(json!({"pattern": "(", "regex_error": "unclosed"}));
        let result = outcome.into_call_result("bad pattern");
        let error = program_result(&result).expect_err("domain failure");
        let HostErrorKind::Tool {
            code,
            details,
            retryable,
        } = error.kind
        else {
            panic!("expected a tool error, got {error:?}");
        };
        assert_eq!(code, "invalid_regex");
        assert!(!retryable);
        assert_eq!(error.message, "bad pattern");
        assert_eq!(details.unwrap()["pattern"], "(");
    }

    #[test]
    fn retryable_is_only_conflict_and_timeout() {
        let timeout = ToolOutcome::<serde_json::Value>::failure("timeout", "slow", true)
            .into_call_result("slow");
        let conflict = ToolOutcome::<serde_json::Value>::failure("conflict", "busy", true)
            .into_call_result("busy");
        let stale = ToolOutcome::<serde_json::Value>::failure("stale", "changed", true)
            .into_call_result("changed");
        let quiet = ToolOutcome::<serde_json::Value>::failure("timeout", "slow", false)
            .into_call_result("slow");
        assert!(tool_retryable(&timeout));
        assert!(tool_retryable(&conflict));
        assert!(!tool_retryable(&stale));
        assert!(!tool_retryable(&quiet));
    }

    fn tool_retryable(result: &ToolCallResult) -> bool {
        match program_result(result) {
            Err(error) => matches!(
                error.kind,
                HostErrorKind::Tool { retryable: true, .. }
            ),
            Ok(_) => false,
        }
    }

}
