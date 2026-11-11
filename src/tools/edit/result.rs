//! Sole success/failure shape for `edit` / `apply_edits`.

use serde::Serialize;

use crate::tool::outcome::{Diagnostic, ToolOutcome};
use crate::types::ToolCallResult;

/// Internal failure codes, projected to the six values the contract allows.
///
/// Every item failure leaves through this table: an internal name never reaches
/// a program, and an unknown one cannot terminate the run.
pub(crate) fn public_reason(internal: &str) -> &'static str {
    match internal {
        "not_found" => "not_found",
        "no_match" => "no_match",
        "empty" => "empty",
        "permission" | "permission_denied" => "permission",
        "io" => "io",
        "invalid_args" | "invalid_input" | "invalid_regex" | "empty_old_string"
        | "no_op" | "read_line_prefix" | "whitespace_only" => "invalid_args",
        "multiple_matches" => "no_match",
        _ => "io",
    }
}

/// Wire result for one replacement attempt.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EditResult {
    pub ok: bool,
    pub path: String,
    /// Failure explanation for the failure envelope. Never serialized: a batch
    /// item carries `reason`, and a success carries fields, not prose.
    #[serde(skip)]
    pub message: String,
    /// Internal cause when `ok` is false; item serialization projects it to
    /// one of not_found / no_match / invalid_args / empty / permission / io.
    /// Always on the wire: `null` when the item succeeded.
    pub reason: Option<&'static str>,
    /// Outside-workspace or sensitive-path target; absent when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_risk: Option<crate::tools::file_path::PathRisk>,
    /// 1-based line of the closest match when `old_string` was not found;
    /// `null` otherwise. Always on the wire: one item shape, no guessing.
    pub nearest_line: Option<u32>,
    /// Warnings recorded after the edit; absent when the edit was clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<Diagnostic>>,
}

impl EditResult {
    pub fn success(path: impl Into<String>) -> Self {
        Self {
            ok: true,
            path: path.into(),
            message: String::new(),
            reason: None,
            path_risk: None,
            nearest_line: None,
            diagnostics: None,
        }
    }

    /// Attach the structured path risk; the caller no longer builds prose.
    pub fn with_path_risk(mut self, risk: Option<crate::tools::file_path::PathRisk>) -> Self {
        self.path_risk = risk;
        self
    }

    pub fn with_nearest_line(mut self, nearest_line: Option<u32>) -> Self {
        self.nearest_line = nearest_line;
        self
    }

    pub fn with_diagnostics(mut self, diagnostics: Option<Vec<Diagnostic>>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn failure(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            path: path.into(),
            message: message.into(),
            reason: Some("io"),
            path_risk: None,
            nearest_line: None,
            diagnostics: None,
        }
    }

    /// Keep the internal table key: `present_edit` hands it to the failure
    /// envelope, and `present_apply_edits` projects it once for the wire.
    pub fn because(mut self, internal: &'static str) -> Self {
        self.reason = Some(internal);
        self
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct EditSuccessData {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_risk: Option<crate::tools::file_path::PathRisk>,
    /// Warnings recorded after the edit; absent when the edit was clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Vec<Diagnostic>>,
}

pub fn present_edit(result: EditResult) -> ToolCallResult {
    if result.ok {
        return ToolOutcome::ok(EditSuccessData {
            path: result.path,
            path_risk: result.path_risk,
            diagnostics: result.diagnostics,
        })
        .into_call_result("");
    }
    let code = result.reason.unwrap_or("io");
    crate::tools::file_path::coded_error(code, result.message)
}

/// Sole success shape for apply_edits: one entry per requested edit.
#[derive(Serialize)]
struct ApplyEditsData {
    results: Vec<serde_json::Value>,
}

pub fn present_apply_edits(results: Vec<EditResult>) -> ToolCallResult {
    // One projection, at the wire: an item reason is always one of the six
    // public values, so no internal name reaches a program and the run cannot
    // be terminated by an unknown one.
    let results: Vec<serde_json::Value> = results
        .into_iter()
        .map(|item| {
            let mut item = item;
            item.reason = item.reason.map(public_reason);
            serde_json::to_value(item).unwrap_or(serde_json::Value::Null)
        })
        .collect();
    // Same as write success: empty content; batch facts live in data.results[].
    ToolOutcome::ok(ApplyEditsData { results }).into_call_result("")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every internal failure name used anywhere in edit flows maps to one of
    /// the six public reasons: no internal name can reach a program, and no
    /// unknown name can terminate a run.
    #[test]
    fn every_internal_reason_projects_to_one_of_six() {
        const PUBLIC: [&str; 6] = [
            "not_found",
            "no_match",
            "invalid_args",
            "empty",
            "permission",
            "io",
        ];
        let internal = [
            "not_found",
            "no_match",
            "empty",
            "permission",
            "permission_denied",
            "io",
            "invalid_input",
            "invalid_args",
            "invalid_regex",
            "empty_old_string",
            "no_op",
            "read_line_prefix",
            "whitespace_only",
            "multiple_matches",
            "something_new",
        ];
        for code in internal {
            let public = public_reason(code);
            assert!(PUBLIC.contains(&public), "{code} -> {public}");
            // The tool keeps the internal key; the wire gets the public one.
            let item = EditResult::failure("a.txt", "x").because(code);
            assert_eq!(item.reason, Some(code), "the tool keeps the key");
            let wire = present_apply_edits(vec![item])
                .data
                .expect("data");
            assert_eq!(wire["results"][0]["reason"], public, "{code}");
        }
    }
}

/// Drift samples for the two edit tools. apply_edits states use one item each:
/// a failure carries the optional reason, a success does not. `diagnostics` is
/// always present (empty when there is nothing to report), so one item shape
/// can describe both a failure and a success.
#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;

    fn lsp_error() -> crate::tool::outcome::Diagnostic {
        crate::tool::outcome::Diagnostic::warning("lsp_error", "LSP reported errors in this file")
            .with_details(serde_json::json!({ "text": "error[E1]: boom" }))
    }

    fn outside() -> crate::tools::file_path::PathRisk {
        crate::tools::file_path::PathRisk {
            outside_workspace: true,
            sensitive: false,
            resolved: Some("C:/tmp/x.txt".into()),
        }
    }

    vec![
        DriftCase {
            tool: "edit",
            // Outside the workspace: the structured path risk appears.
            all_optional_present: present_edit(
                EditResult::success("a.txt")
                    .with_path_risk(Some(outside()))
                    .with_diagnostics(Some(vec![lsp_error()])),
            ),
            all_optional_absent: present_edit(EditResult::success("a.txt")),
        },
        DriftCase {
            tool: "apply_edits",
            // One item carries every key: `reason` is on the wire even when the
            // item succeeded (`null`), so a success can show the other two.
            all_optional_present: present_apply_edits(vec![
                EditResult::success("a.txt")
                    .with_path_risk(Some(outside()))
                    .with_diagnostics(Some(vec![lsp_error()])),
            ]),
            all_optional_absent: present_apply_edits(vec![EditResult::success("a.txt")]),
        },
    ]
}
