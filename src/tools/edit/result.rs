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
}

impl EditResult {
    pub fn success(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: true,
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn failure(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            path: path.into(),
            message: message.into(),
        }
    }
}

pub fn edit_result_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "description": "One replacement outcome. Failures resolve with ok=false; guidance lives only in message.",
        "properties": {
            "ok": { "type": "boolean" },
            "path": { "type": "string" },
            "message": { "type": "string" }
        },
        "required": ["ok", "path", "message"]
    })
}

pub fn apply_edits_data_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Serial batch outcomes. results is the same length as the input edits array.",
        "properties": {
            "results": {
                "type": "array",
                "items": edit_result_schema()
            }
        },
        "required": ["results"]
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
