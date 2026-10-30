//! Shared read/plan/write path for `edit` and `apply_edits`.

use std::sync::Arc;

use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::planner::{self, EditSpec, PlanFail};
use crate::tools::edit::result::EditResult;
use crate::workspace::text_codec;

pub(crate) struct EditEngine {
    pub ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

pub(crate) struct Cancelled;

impl EditEngine {
    pub(crate) fn new(ide: Option<Arc<crate::ide_base::IdeBaseHandle>>) -> Self {
        Self { ide }
    }

    /// Attempt one replacement. Match/IO/path failures become typed failures with
    /// guidance in `message`. Only cancellation is `Err` so a batch can abort.
    pub(crate) async fn apply_one(
        &self,
        execution: &ToolExecutionContext,
        raw_path: &str,
        spec: &EditSpec,
        success_label: &str,
    ) -> Result<EditResult, Cancelled> {
        if crate::session::transcript_file::is_virtual_session_path(raw_path) {
            return Ok(EditResult::failure(
                raw_path,
                crate::session::transcript_file::READ_ONLY_MSG,
            )
            .because("permission"));
        }
        let path = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => {
                return Ok(EditResult::failure(raw_path, error.to_string()).because("invalid_args"));
            }
        };
        if execution.cancel.is_cancelled() {
            return Err(Cancelled);
        }
        let path_display = path.to_string_lossy().into_owned();
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&path).ok());
        let raw_bytes = match (&self.ide, &workspace_relative) {
            (Some(ide), Some(relative)) => match ide.workspace.read_file_bytes(relative) {
                Ok((_, bytes)) => bytes,
                Err(error) => {
                    let reason = if path.exists() { "io" } else { "not_found" };
                    return Ok(EditResult::failure(
                        raw_path,
                        format!("read {path_display}: {error}"),
                    )
                    .because(reason));
                }
            },
            _ => match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let reason = if error.kind() == std::io::ErrorKind::NotFound { "not_found" } else { "io" };
                    return Ok(EditResult::failure(
                        raw_path,
                        format!("read {path_display}: {error}"),
                    )
                    .because(reason));
                }
            },
        };
        let decoded = match text_codec::decode_utf8_bytes(&raw_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                return Ok(EditResult::failure(
                    raw_path,
                    text_codec::decode_error_for_path(error, &path_display),
                ));
            }
        };
        let content = decoded.text;
        let edited = match planner::plan_one(&content, spec, &execution.cancel) {
            Ok(edited) => edited,
            Err(PlanFail::Cancelled) => return Err(Cancelled),
            Err(PlanFail::EmptyFile) => {
                return Ok(EditResult::failure(raw_path, planner::EMPTY_FILE_MESSAGE).because("empty"));
            }
            Err(PlanFail::Reject(reason)) => {
                let code = match reason.code() {
                    "no_match" | "multiple_matches" => "no_match",
                    _ => "invalid_args",
                };
                return Ok(EditResult::failure(raw_path, reason.message()).because(code));
            }
        };
        if execution.cancel.is_cancelled() {
            return Err(Cancelled);
        }
        if edited == content {
            return Ok(EditResult::failure(
                raw_path,
                "replacement does not change the file",
            )
            .because("invalid_args"));
        }
        let to_write = text_codec::reattach_utf8_bom(decoded.has_bom, &edited);
        match (&self.ide, &workspace_relative) {
            (Some(ide), Some(relative)) => {
                if let Err(error) = ide.workspace.write_file(relative, &to_write) {
                    return Ok(EditResult::failure(
                        raw_path,
                        format!("write {path_display}: {error}"),
                    ));
                }
            }
            _ => {
                if let Err(error) = crate::workspace::file_ops::atomic_write(&path, &to_write) {
                    return Ok(EditResult::failure(
                        raw_path,
                        format!("write {path_display}: {error}"),
                    ));
                }
            }
        }
        if let (Some(ide), Some(_)) = (&self.ide, &workspace_relative) {
            ide.apply_document_if_ready(&path, &edited).await;
        }

        let mut message = if spec.new_string.is_empty() {
            "deleted".to_string()
        } else {
            success_label.to_string()
        };

        // Fold LSP / path-risk notes into message (no structured details tree).
        // lsp_feedback attaches `lsp_error` as diagnostics (content stays empty
        // until finalize); pull those into message for the EditResult contract.
        if let Some(engines) = self.ide.as_ref().map(|ide| ide.engines.as_ref()) {
            let stub = crate::types::ToolCallResult::ok(String::new());
            let annotated = crate::tools::lsp_feedback::maybe_append_local_lsp_errors(
                Some(engines),
                &path_display,
                stub,
            )
            .await;
            for diag in &annotated.diagnostics {
                if diag.code != "lsp_error" {
                    continue;
                }
                message.push('\n');
                message.push_str(&diag.message);
                if let Some(text) = diag
                    .details
                    .as_ref()
                    .and_then(|d| d.get("text"))
                    .and_then(|t| t.as_str())
                {
                    message.push('\n');
                    message.push_str(text);
                }
            }
            let finalized = annotated.finalize_signals();
            if !finalized.content.is_empty() {
                message.push('\n');
                message.push_str(finalized.content.trim());
            }
        }
        let risk = crate::tools::file_path::with_path_risk_warning(
            crate::types::ToolCallResult::ok(String::new()),
            &execution.workspace_root,
            raw_path,
            &path,
            "editing",
        );
        if !risk.content.is_empty() {
            message.push('\n');
            message.push_str(risk.content.trim());
        }

        Ok(EditResult::success(raw_path, message))
    }
}
