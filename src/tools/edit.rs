use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;
use crate::types::{ToolCallResult, ToolErrorKind};
use crate::workspace::text_codec;

mod feedback;
mod matcher;
mod planner;

#[cfg(test)]
mod python_indent;
#[cfg(test)]
mod tests;

use feedback::report;
use planner::{
    RequestFail, SNAPSHOT_RULE, cancelled_message, empty_file_message, parse_edits, plan_edits,
};

fn cancelled_edit() -> ToolCallResult {
    ToolCallResult::error_with_kind(ToolErrorKind::Cancelled, cancelled_message(), false)
}

pub struct EditTool {
    ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

impl EditTool {
    pub fn new() -> Self {
        Self { ide: None }
    }

    pub fn with_ide(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide: Some(ide) }
    }

    async fn finish(
        &self,
        mut result: ToolCallResult,
        wrote: bool,
        execution: &ToolExecutionContext,
        raw_path: &str,
        resolved: &std::path::Path,
        path_display: &str,
    ) -> ToolCallResult {
        if wrote {
            result = lsp_feedback::maybe_append_local_lsp_errors(
                self.ide.as_ref().map(|ide| ide.engines.as_ref()),
                path_display,
                result,
            )
            .await;
            result = crate::tools::file_path::with_path_risk_warning(
                result,
                &execution.workspace_root,
                raw_path,
                resolved,
                "editing",
            );
        }
        result
    }

    fn test_execution_context(&self) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: self
                .ide
                .as_ref()
                .map(|ide| ide.workspace.sandbox().root().to_path_buf())
                .unwrap_or_else(crate::config::workspace::workspace_root_lap),
            call_id: String::new(),
            cancel: CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        }
    }
}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {
                    "type": "string",
                    "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                },
                "expected_hash": {
                    "type": "string",
                    "description": "sha256 of the file bytes from a previous read. Omit to skip the check. A mismatch rejects stale and does not write."
                },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Replacements to apply to this file.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "old_string": {
                                "type": "string",
                                "description": format!(
                                    "File text to find. Do not include {}. Must match one place unless replace_all is true; add surrounding lines if it appears more than once.",
                                    crate::tool::FILE_LINE_PREFIX_HINT
                                )
                            },
                            "new_string": {
                                "type": "string",
                                "description": "Replacement text. Use an empty string to delete old_string."
                            },
                            "replace_all": {
                                "type": "boolean",
                                "description": "Replace every exact occurrence (default: false)."
                            }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "description": "Blocks applied to one file. file is the identity after the successful blocks were written.",
            "properties": {
                "file": {
                    "type": "object",
                    "description": "File identity after the write. path is workspace-relative inside the workspace. Outside the workspace, path is the requested path and abs is absolute.",
                    "properties": {
                        "path": { "type": "string" },
                        "abs": { "type": "string" },
                        "hash": { "type": "string", "description": "sha256 hex of the file bytes." },
                        "bytes": { "type": "integer", "minimum": 0 }
                    },
                    "required": ["path", "hash", "bytes"]
                },
                "blocks": {
                    "type": "array",
                    "description": "One result per edit, in input order. Failed blocks do not roll back applied blocks.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "index": { "type": "integer", "minimum": 1, "description": "1-based index in edits." },
                            "status": { "type": "string", "enum": ["applied", "failed"] },
                            "ranges": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "start_line": { "type": "integer", "minimum": 1 },
                                        "end_line": { "type": "integer", "minimum": 1 }
                                    },
                                    "required": ["start_line", "end_line"]
                                }
                            },
                            "reason": {
                                "type": "object",
                                "description": "Present when status is failed.",
                                "properties": {
                                    "code": { "type": "string" },
                                    "message": { "type": "string" },
                                    "details": {
                                        "type": "object",
                                        "additionalProperties": false,
                                        "description": "Structured cause. Fields appear only for the reason that produced them.",
                                        "properties": {
                                            "kind": {
                                                "type": "string",
                                                "enum": ["identical_strings", "replacement_unchanged", "leading_trailing", "internal_spacing", "input"]
                                            },
                                            "lines": {
                                                "type": "array",
                                                "items": { "type": "integer", "minimum": 1 }
                                            },
                                            "excerpt": { "type": "string", "description": "One line of the file the whitespace difference pointed at." },
                                            "prior_index": { "type": "integer", "minimum": 1 },
                                            "other_index": { "type": "integer", "minimum": 1 },
                                            "other_range": {
                                                "type": "object",
                                                "additionalProperties": false,
                                                "properties": {
                                                    "start_line": { "type": "integer", "minimum": 1 },
                                                    "end_line": { "type": "integer", "minimum": 1 }
                                                },
                                                "required": ["start_line", "end_line"]
                                            }
                                        }
                                    }
                                },
                                "required": ["code", "message"]
                            }
                        },
                        "required": ["index", "status", "ranges"]
                    }
                },
                "applied": { "type": "integer", "minimum": 0, "description": "Number of blocks written." },
                "failed": { "type": "integer", "minimum": 0, "description": "Number of blocks not written." }
            },
            "required": ["file", "blocks", "applied", "failed"]
        }))
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tools::file_path::require_code_path(input)?;
        if let Some(value) = input.get("expected_hash").filter(|value| !value.is_null())
            && !value.is_string()
        {
            return Err("parameter 'expected_hash' must be a string".into());
        }
        parse_edits(input)?;
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> bool {
        input["edits"].as_array().is_some_and(|edits| {
            edits
                .iter()
                .any(|item| item["new_string"].as_str().is_some_and(str::is_empty))
        })
    }

    fn effects(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        match crate::tools::file_path::code_path(input) {
            Some(path) => crate::tool::resource_gate::ToolEffects::write(
                crate::tool::resource_gate::lock_key(workspace_root, path, path_mode),
            ),
            None => crate::tool::resource_gate::ToolEffects::default(),
        }
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        match crate::tools::file_path::code_path(input) {
            Some(path) => {
                let key = crate::workspace::resolve_agent(workspace_root, path, path_mode)
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| path.to_string());
                vec![ResourceKey::File(key)]
            }
            _ => vec![],
        }
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let execution = self.test_execution_context();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(self.call_for_execution(input, execution))
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/edit.md"))
            .replace("{{SNAPSHOT_RULE}}", SNAPSHOT_RULE)
    }
}

impl EditTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tools::file_path::require_code_path(&input) {
            Ok(s) => s.to_string(),
            Err(e) => return crate::tools::file_path::coded_error("invalid_input", e),
        };
        if crate::session::transcript_file::is_virtual_session_path(&raw_path) {
            return crate::tools::file_path::coded_error(
                "invalid_input",
                crate::session::transcript_file::READ_ONLY_MSG,
            );
        }
        let blocks = match parse_edits(&input) {
            Ok(blocks) => blocks,
            Err(e) => return crate::tools::file_path::coded_error("invalid_input", e),
        };
        let path = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            &raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => return crate::tools::file_path::tool_path_error(error),
        };
        if execution.cancel.is_cancelled() {
            return cancelled_edit();
        }
        let path_display = path.to_string_lossy().into_owned();
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&path).ok());
        let raw_bytes = match (&self.ide, &workspace_relative) {
            (Some(ide), Some(relative)) => match ide.workspace.read_file_bytes(relative) {
                Ok((_, bytes)) => bytes,
                Err(error) => return crate::tools::file_path::workspace_error(error),
            },
            _ => match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return crate::tools::file_path::coded_error(
                        "not_found",
                        format!("read {path_display}: {error}"),
                    );
                }
                Err(error) => {
                    return crate::tools::file_path::coded_error(
                        "io",
                        format!("read {path_display}: {error}"),
                    );
                }
            },
        };
        if let Some(expected) = input.get("expected_hash").and_then(Value::as_str) {
            let current = crate::tool::file_identity::sha256_hex(&raw_bytes);
            if expected != current {
                let message = "file changed since it was read; expected_hash does not match";
                return crate::tool::outcome::ToolOutcome::<Value>::failure(
                    "stale",
                    message,
                    false,
                )
                .with_error_details(serde_json::json!({ "current_hash": current }))
                .into_call_result(message);
            }
        }
        let decoded = match text_codec::decode_utf8_bytes(&raw_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                return crate::tools::file_path::coded_error(
                    "unsupported",
                    text_codec::decode_error_for_path(error, &path_display),
                );
            }
        };
        let content = decoded.text;
        let planned = match plan_edits(&content, &blocks, &execution.cancel) {
            Ok(planned) => planned,
            Err(RequestFail::Cancelled) => return cancelled_edit(),
            Err(RequestFail::EmptyFile) => {
                return crate::tools::file_path::coded_error("invalid_input", empty_file_message());
            }
            Err(RequestFail::InvalidPlan(msg)) => {
                return crate::tools::file_path::coded_error(
                    "invalid_input",
                    format!("{msg}; file was not modified"),
                );
            }
        };
        if execution.cancel.is_cancelled() {
            return cancelled_edit();
        }
        let edit_report = report(&planned);
        if edit_report.applied == 0 {
            let message = "no edits applied; file was not modified";
            let mut outcome = crate::tool::outcome::ToolOutcome::<Value>::failure(
                "edit_failed",
                message,
                false,
            )
            .with_error_details(serde_json::json!({ "blocks": edit_report.blocks }));
            for diagnostic in edit_report.diagnostics {
                outcome = outcome.with_diagnostic(diagnostic);
            }
            return outcome.into_call_result(message);
        }
        let wrote = planned
            .edited_content()
            .is_some_and(|edited| edited != content);
        let written_bytes = if wrote {
            if execution.cancel.is_cancelled() {
                return cancelled_edit();
            }
            let edited = planned.edited_content().expect("wrote implies edited");
            let to_write = text_codec::reattach_utf8_bom(decoded.has_bom, edited);
            match (&self.ide, &workspace_relative) {
                (Some(ide), Some(relative)) => {
                    if let Err(error) = ide.workspace.write_file(relative, &to_write) {
                        return crate::tools::file_path::workspace_error(error);
                    }
                }
                _ => {
                    if let Err(error) = crate::workspace::file_ops::atomic_write(&path, &to_write) {
                        return crate::tools::file_path::coded_error("io", error.to_string());
                    }
                }
            }
            if let (Some(ide), Some(_)) = (&self.ide, &workspace_relative) {
                ide.apply_document_if_ready(&path, edited).await;
            }
            to_write.into_bytes()
        } else {
            raw_bytes
        };
        let located =
            crate::tool::file_identity::locate_resolved(&execution.workspace_root, &path, &raw_path);
        let file = crate::tool::file_identity::file_ref(located, &written_bytes);
        let data = serde_json::json!({
            "file": file,
            "blocks": edit_report.blocks,
            "applied": edit_report.applied,
            "failed": edit_report.failed,
        });
        let mut outcome = crate::tool::outcome::ToolOutcome::ok(data);
        for diagnostic in edit_report.diagnostics {
            outcome = outcome.with_diagnostic(diagnostic);
        }
        let result = outcome.into_call_result("");
        self.finish(result, wrote, &execution, &raw_path, &path, &path_display)
            .await
    }
}
