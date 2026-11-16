//! Sole success/failure shape for `edit` / `apply_edits`.

use serde::Serialize;

use crate::tool::outcome::{Diagnostic, ToolOutcome};
use crate::types::ToolCallResult;

/// Internal failure codes, projected to the seven values the contract allows.
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
        // Two matches are not a miss: the model has to add context, not fix a
        // typo, so the code must not collapse into no_match.
        "multiple_matches" | "ambiguous" => "ambiguous",
        _ => "io",
    }
}

/// Wire result for one replacement attempt.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EditResult {
    pub ok: bool,
    pub path: String,
    /// Internal cause when `ok` is false; item serialization projects it to
    /// one of ambiguous / not_found / no_match / invalid_args / empty /
    /// permission / io. Always on the wire: `null` when the item succeeded.
    pub reason: Option<&'static str>,
    /// Failure explanation for the failure envelope, serialized as `detail`:
    /// the batch table carries the "why" next to the machine reason. Empty on
    /// success, so a successful row leaves the cell blank.
    #[serde(rename = "detail")]
    pub message: String,
    /// Outside-workspace or sensitive-path target; absent when there is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_risk: Option<crate::tools::file_path::PathRisk>,
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
            diagnostics: None,
        }
    }

    /// Attach the structured path risk; the caller no longer builds prose.
    pub fn with_path_risk(mut self, risk: Option<crate::tools::file_path::PathRisk>) -> Self {
        self.path_risk = risk;
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
    results: Vec<EditSuccessData>,
}

pub fn present_apply_edits(results: Vec<EditResult>) -> ToolCallResult {
    let failed: Vec<(usize, &EditResult)> = results
        .iter()
        .enumerate()
        .filter(|(_, item)| !item.ok)
        .collect();
    if failed.is_empty() {
        let items: Vec<EditSuccessData> = results
            .into_iter()
            .map(|item| EditSuccessData {
                path: item.path,
                path_risk: item.path_risk,
                diagnostics: item.diagnostics,
            })
            .collect();
        return ToolOutcome::ok(ApplyEditsData { results: items }).into_call_result("");
    }

    // Serial execution already wrote matching items to disk; reject so callers
    // stop dependent steps while preserving applied edits and reporting all
    // failures clearly using the seven public reason codes.
    let total = results.len();
    let applied = total - failed.len();
    let first_code = public_reason(failed[0].1.reason.unwrap_or("io"));
    let failures_summary: String = failed
        .iter()
        .map(|(index, item)| {
            let code = public_reason(item.reason.unwrap_or("io"));
            format!("#{} ({}) [{}]: {}", index + 1, item.path, code, item.message)
        })
        .collect::<Vec<_>>()
        .join("; ");
    let message = if applied > 0 {
        format!(
            "applied {applied}/{total} edits (valid edits saved on disk); failed: {failures_summary}"
        )
    } else {
        format!("applied 0/{total} edits; failed: {failures_summary}")
    };
    let detail_items: Vec<serde_json::Value> = results
        .iter()
        .enumerate()
        .map(|(index, item)| {
            serde_json::json!({
                "index": index + 1,
                "ok": item.ok,
                "path": item.path,
                "reason": item.reason.map(public_reason),
                "detail": item.message,
            })
        })
        .collect();
    let diagnostics: Vec<Diagnostic> = results
        .iter()
        .filter(|item| item.ok)
        .flat_map(|item| item.diagnostics.iter().flatten().cloned())
        .collect();
    let mut outcome: ToolOutcome<()> = ToolOutcome::failure(first_code, message.clone(), false)
        .with_error_details(serde_json::json!({
            "applied": applied,
            "total": total,
            "results": detail_items,
        }));
    for diagnostic in diagnostics {
        outcome = outcome.with_diagnostic(diagnostic);
    }
    outcome.into_call_result(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every internal failure name used anywhere in edit flows maps to one of
    /// the seven public reasons: no internal name can reach a program, and no
    /// unknown name can terminate a run.
    #[test]
    fn every_internal_reason_projects_to_one_of_seven() {
        const PUBLIC: [&str; 7] = [
            "not_found",
            "no_match",
            "ambiguous",
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
            "ambiguous",
            "something_new",
        ];
        for code in internal {
            let public = public_reason(code);
            assert!(PUBLIC.contains(&public), "{code} -> {public}");
            // The tool keeps the internal key; failure output gets the public one.
            let item = EditResult::failure("a.txt", "x").because(code);
            assert_eq!(item.reason, Some(code), "the tool keeps the key");
            let wire = present_apply_edits(vec![item]);
            assert_eq!(wire.error_code.as_deref(), Some(public), "{code}");
            let details = wire.error_details.expect("error_details");
            assert_eq!(details["results"][0]["reason"], public, "{code}");
        }
    }
}

/// Drift samples for apply_edits success items (`path_risk` and `diagnostics`
/// move to metadata in `result_projection`).
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
