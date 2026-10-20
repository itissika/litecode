//! Code-surface result contract.
//!
//! `Tool::execute` still returns [`ToolCallResult`] because direct tools share
//! that pipeline. Code-surface tools project this envelope onto it. The program
//! sees [`program_value`], not markdown.

use serde::Serialize;
use serde_json::{Value, json};

use crate::types::{ToolCallResult, ToolErrorKind, ToolSignalLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Ok,
    Partial,
    Error,
}

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
    pub status: OutcomeStatus,
    pub data: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
    pub page: Option<Page>,
    pub error: Option<ToolError>,
}

impl<T> ToolOutcome<T> {
    pub fn ok(data: T) -> Self {
        Self {
            status: OutcomeStatus::Ok,
            data: Some(data),
            diagnostics: Vec::new(),
            page: None,
            error: None,
        }
    }

    pub fn partial(data: T) -> Self {
        Self {
            status: OutcomeStatus::Partial,
            data: Some(data),
            diagnostics: Vec::new(),
            page: None,
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
            status: OutcomeStatus::Error,
            data: None,
            diagnostics: Vec::new(),
            page: None,
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

    pub fn with_page(mut self, page: Page) -> Self {
        self.page = Some(page);
        self
    }
}

impl<T: Serialize> ToolOutcome<T> {
    /// Project onto the shared pipeline result. `content` is not the contract.
    pub fn into_call_result(self, content: impl Into<String>) -> ToolCallResult {
        let content = content.into();
        let ToolOutcome {
            status,
            data,
            diagnostics,
            page,
            error,
        } = self;
        let mut result = match status {
            OutcomeStatus::Error => {
                let error = error.unwrap_or(ToolError {
                    code: "internal".into(),
                    message: content.clone(),
                    details: None,
                    retryable: false,
                });
                let mut result = ToolCallResult::error_with_kind(
                    kind_for_code(&error.code),
                    error.message,
                    error.retryable,
                );
                result.content = content;
                result.error_code = Some(error.code);
                result.error_details = error.details;
                result.outcome_status = Some(OutcomeStatus::Error);
                result
            }
            OutcomeStatus::Ok | OutcomeStatus::Partial => {
                let mut result = ToolCallResult::ok(content);
                if let Some(data) = data {
                    result.data = serde_json::to_value(data).ok();
                }
                result.outcome_status = Some(status);
                result
            }
        };
        result.diagnostics = diagnostics;
        result.page = page;
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
pub fn ok_data(data: impl Serialize, page: Option<Page>) -> ToolCallResult {
    let mut result = ToolCallResult::ok(String::new());
    result.data = Some(serde_json::to_value(data).unwrap_or(Value::Null));
    result.page = page;
    result.outcome_status = Some(OutcomeStatus::Ok);
    result
}

pub fn error_kind_code(kind: ToolErrorKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "internal".into())
}

/// JSON object the code program receives.
pub fn program_value(result: &ToolCallResult) -> Value {
    let status = if result.error.is_some()
        || result.level == ToolSignalLevel::Error
        || result.outcome_status == Some(OutcomeStatus::Error)
    {
        "error"
    } else if result.outcome_status == Some(OutcomeStatus::Partial) {
        "partial"
    } else {
        "ok"
    };
    let error = if status == "error" {
        contract_error(result)
    } else {
        None
    };
    let data = if status == "error" {
        Value::Null
    } else {
        result.data.clone().unwrap_or(Value::Null)
    };
    let mut value = json!({
        "status": status,
        "data": data,
        "diagnostics": result.diagnostics,
        "error": error,
    });
    if let Some(page) = &result.page {
        value["page"] = serde_json::to_value(page).unwrap_or(Value::Null);
    }
    value
}

fn contract_error(result: &ToolCallResult) -> Option<Value> {
    let error = result.error.as_ref()?;
    let code = result
        .error_code
        .clone()
        .unwrap_or_else(|| error_kind_code(error.kind));
    let mut value = json!({
        "code": code,
        "message": error.message,
        "retryable": error.retryable,
    });
    if let Some(details) = &result.error_details {
        value["details"] = details.clone();
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_keeps_partial_data_and_diagnostics() {
        let outcome = ToolOutcome::partial(json!({"applied": 1}))
            .with_diagnostic(
                Diagnostic::warning("edit_block_failed", "block 1 failed")
                    .with_details(json!({"index": 1})),
            );
        let result = outcome.into_call_result("body");
        let value = program_value(&result);
        assert_eq!(value["status"], "partial");
        assert_eq!(value["data"]["applied"], 1);
        assert!(value["error"].is_null());
        assert_eq!(value["diagnostics"][0]["code"], "edit_block_failed");
        assert_eq!(value["diagnostics"][0]["severity"], "warning");
    }

    #[test]
    fn invalid_regex_error_keeps_code_and_details() {
        let outcome = ToolOutcome::<serde_json::Value>::failure("invalid_regex", "bad pattern", true)
            .with_error_details(
            json!({"pattern": "(", "regex_error": "unclosed"}),
        );
        let result = outcome.into_call_result("bad pattern");
        let value = program_value(&result);
        assert_eq!(value["status"], "error");
        assert!(value["data"].is_null());
        assert_eq!(value["error"]["code"], "invalid_regex");
        assert_eq!(value["error"]["retryable"], true);
        assert_eq!(value["error"]["details"]["pattern"], "(");
    }

    #[test]
    fn file_ref_and_page_round_trip_required_fields() {
        let file = FileRef {
            path: "a.txt".into(),
            abs: None,
            hash: "abc".into(),
            bytes: 3,
        };
        let page = Page {
            has_more: true,
            next: Some("2".into()),
            total: Some(9),
        };
        let file_json = serde_json::to_value(file).unwrap();
        let page_json = serde_json::to_value(page).unwrap();
        assert_eq!(file_json["path"], "a.txt");
        assert!(file_json.get("abs").is_none());
        assert_eq!(file_json["hash"], "abc");
        assert_eq!(file_json["bytes"], 3);
        assert_eq!(page_json["has_more"], true);
        assert_eq!(page_json["next"], "2");
        assert_eq!(page_json["total"], 9);
    }
}
