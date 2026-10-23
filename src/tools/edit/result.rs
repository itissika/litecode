//! Sole success/failure shape for `edit` / `apply_edits`.

use serde::Serialize;
use serde_json::Value;

use crate::tool::outcome::ToolOutcome;
use crate::types::ToolCallResult;

/// Wire result for one replacement attempt.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EditResult {
    pub ok: bool,
    pub path: String,
    pub message: String,
    /// Structured cause when `ok` is false (WP-210 G): one of
    /// not_found / no_match / invalid_args / empty / permission / io.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

impl EditResult {
    pub fn success(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            path: path.into(),
            message: message.into(),
            reason: None,
        }
    }

    pub fn failure(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            path: path.into(),
            message: message.into(),
            reason: Some("io"),
        }
    }

    pub fn because(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }
}

pub fn edit_result_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "ok": { "type": "boolean" },
            "path": { "type": "string" },
            "message": { "type": "string" },
            "reason": { "type": "string", "enum": ["not_found", "no_match", "invalid_args", "empty", "permission", "io"] }
        },
        "required": ["ok", "path", "message"]
    })
}

pub fn present_edit(result: EditResult) -> ToolCallResult {
    // Code surface: facts live in data (EditResult.message); content is not the contract.
    ToolOutcome::ok(result).into_call_result("")
}

pub fn present_apply_edits(results: Vec<EditResult>) -> ToolCallResult {
    // Same as write success: empty content; batch facts in data.results[].message.
    ToolOutcome::ok(serde_json::json!({ "results": results })).into_call_result("")
}
