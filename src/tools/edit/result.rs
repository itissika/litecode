//! Sole success/failure shape for `edit` / `apply_edits`.

use serde::Serialize;

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct EditSuccessData {
    path: String,
    message: String,
}

pub fn present_edit(result: EditResult) -> ToolCallResult {
    if result.ok {
        return ToolOutcome::ok(EditSuccessData {
            path: result.path,
            message: result.message,
        })
        .into_call_result("");
    }
    let code = result.reason.unwrap_or("io");
    crate::tools::file_path::coded_error(code, result.message)
}

/// Sole success shape for apply_edits: one entry per requested edit.
#[derive(Serialize)]
struct ApplyEditsData {
    results: Vec<EditResult>,
}

pub fn present_apply_edits(results: Vec<EditResult>) -> ToolCallResult {
    // Same as write success: empty content; batch facts in data.results[].message.
    ToolOutcome::ok(ApplyEditsData { results }).into_call_result("")
}

/// Drift samples for the two edit tools. apply_edits states use one item each:
/// a failure carries the optional reason, a success does not.
#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    vec![
        DriftCase {
            tool: "edit",
            // edit has no optional property, so both states use the same shape.
            all_optional_present: present_edit(EditResult::success("a.txt", "Edited a.txt.")),
            all_optional_absent: present_edit(EditResult::success("a.txt", "Edited a.txt.")),
        },
        DriftCase {
            tool: "apply_edits",
            all_optional_present: present_apply_edits(vec![
                EditResult::failure("a.txt", "No match.").because("no_match"),
            ]),
            all_optional_absent: present_apply_edits(vec![EditResult::success(
                "a.txt",
                "Edited a.txt.",
            )]),
        },
    ]
}
