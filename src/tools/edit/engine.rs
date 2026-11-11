//! Shared read/plan/write path for `edit` and `apply_edits`.

use std::sync::Arc;

use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::matcher;
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
    /// guidance in `message` (the failure envelope). Only cancellation is `Err`
    /// so a batch can abort.
    pub(crate) async fn apply_one(
        &self,
        execution: &ToolExecutionContext,
        raw_path: &str,
        spec: &EditSpec,
    ) -> Result<EditResult, Cancelled> {
        if crate::session::transcript_file::is_virtual_session_path(raw_path) {
            return Ok(EditResult::failure(
                raw_path,
                crate::session::transcript_file::READ_ONLY_MSG,
            )
            // `permission_denied` is the key in the soft-failure table; it
            // projects to the program as `reason: "permission"`.
            .because("permission_denied"));
        }
        let path = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => {
                // Input-class refusals stay a value: `invalid_input` is in the
                // soft-failure table, so the program sees `{ok:false}` instead of
                // the run ending.
                return Ok(EditResult::failure(raw_path, error.to_string()).because("invalid_input"));
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
                    let reason = if error.kind() == std::io::ErrorKind::NotFound {
                        "not_found"
                    } else {
                        "io"
                    };
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
                return Ok(
                    EditResult::failure(raw_path, planner::EMPTY_FILE_MESSAGE).because("empty")
                );
            }
            Err(PlanFail::Reject(reason)) => {
                let code = match reason.code() {
                    "no_match" | "multiple_matches" => "no_match",
                    _ => "invalid_input",
                };
                // A miss names its nearest line so the model fixes one typo
                // instead of re-reading the file. `multiple_matches` already
                // carries exact lines in its own text.
                let nearest = (reason.code() == "no_match")
                    .then(|| matcher::nearest_line(&content, &spec.old_string))
                    .flatten();
                return Ok(EditResult::failure(raw_path, reason.message())
                    .because(code)
                    .with_nearest_line(nearest));
            }
        };
        if execution.cancel.is_cancelled() {
            return Err(Cancelled);
        }
        if edited == content {
            return Ok(
                EditResult::failure(raw_path, "replacement does not change the file")
                    .because("invalid_input"),
            );
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

        // Same producer and same field as write: post-edit LSP diagnostics are
        // data, not prose folded into a message.
        let diagnostics = crate::tools::lsp_feedback::diagnostics_field(
            crate::tools::lsp_feedback::local_lsp_error_diagnostics(
                self.ide.as_ref().map(|ide| ide.engines.as_ref()),
                &path_display,
            )
            .await,
        );
        let risk = crate::tools::file_path::path_risk(&execution.workspace_root, raw_path, &path);
        Ok(EditResult::success(raw_path)
            .with_path_risk(risk)
            .with_diagnostics(diagnostics))
    }
}
