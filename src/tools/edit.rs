use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;
use crate::types::ToolCallResult;
use crate::workspace::text_codec;

mod feedback;
mod matcher;
mod planner;

#[cfg(test)]
mod tests;

use feedback::render_tool_result;
use planner::{
    RequestFail, SNAPSHOT_RULE, cancelled_message, empty_file_message, parse_edits, plan_edits,
};

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

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
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
                                "description": "Replace all exact occurrences (default: false). Never auto-replaces multiple fuzzy candidates."
                            }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["file_path", "edits"]
        })
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "file_path")?;
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

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        match input["file_path"].as_str() {
            Some(path) if !path.is_empty() => {
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
        format!(
            "Replace text in one file. Multiple files: multiple calls (one file_path each). Pass file_path and a non-empty edits array; each item is old_string, new_string, and optional replace_all (default false). Empty new_string deletes old_string.\n\
             Matching tries exact first (CRLF/LF and common typography such as smart quotes and dashes are equivalent), then a unique high-confidence line-aligned fuzzy match. replace_all applies every exact match and never auto-replaces multiple fuzzy candidates.\n\
             {SNAPSHOT_RULE}\n\
             Example (one edit): {{\"file_path\":\"src/a.rs\",\"edits\":[{{\"old_string\":\"fn start() {{}}\",\"new_string\":\"fn main() {{}}\"}}]}}\n\
             Example (several): {{\"file_path\":\"src/a.rs\",\"edits\":[{{\"old_string\":\"foo\",\"new_string\":\"bar\"}},{{\"old_string\":\"old_api(\",\"new_string\":\"new_api(\",\"replace_all\":true}}]}}"
        )
    }
}

impl EditTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tool::require_nonempty_string(&input, "file_path") {
            Ok(s) => s.to_string(),
            Err(e) => return ToolCallResult::error(e),
        };
        if crate::session::transcript_file::is_virtual_session_path(&raw_path) {
            return ToolCallResult::error(
                crate::session::transcript_file::READ_ONLY_MSG.to_string(),
            );
        }
        let blocks = match parse_edits(&input) {
            Ok(blocks) => blocks,
            Err(e) => return ToolCallResult::error(e),
        };
        let path = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            &raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        if execution.cancel.is_cancelled() {
            return ToolCallResult::error(cancelled_message());
        }
        let path_display = path.to_string_lossy().into_owned();
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&path).ok());
        let raw_bytes = match (&self.ide, &workspace_relative) {
            (Some(ide), Some(relative)) => match ide.workspace.read_file_bytes(relative) {
                Ok((_, bytes)) => bytes,
                Err(error) => return ToolCallResult::error(error.to_string()),
            },
            _ => match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    return ToolCallResult::error(format!("read {path_display}: {error}"));
                }
            },
        };
        let decoded = match text_codec::decode_utf8_bytes(&raw_bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                return ToolCallResult::error(text_codec::decode_error_for_path(
                    error,
                    &path_display,
                ));
            }
        };
        let content = decoded.text;
        let planned = match plan_edits(&content, &blocks, &execution.cancel) {
            Ok(planned) => planned,
            Err(RequestFail::Cancelled) => return ToolCallResult::error(cancelled_message()),
            Err(RequestFail::EmptyFile) => return ToolCallResult::error(empty_file_message()),
            Err(RequestFail::InvalidPlan(msg)) => {
                return ToolCallResult::error(format!("{msg}; file was not modified"));
            }
        };
        if execution.cancel.is_cancelled() {
            return ToolCallResult::error(cancelled_message());
        }
        let wrote = planned
            .edited_content()
            .is_some_and(|edited| edited != content);
        if wrote {
            if execution.cancel.is_cancelled() {
                return ToolCallResult::error(cancelled_message());
            }
            let edited = planned.edited_content().expect("wrote implies edited");
            let to_write = text_codec::reattach_utf8_bom(decoded.has_bom, edited);
            let write_err = match (&self.ide, &workspace_relative) {
                (Some(ide), Some(relative)) => ide
                    .workspace
                    .write_file(relative, &to_write)
                    .err()
                    .map(|e| e.to_string()),
                _ => crate::workspace::file_ops::atomic_write(&path, &to_write)
                    .err()
                    .map(|e| e.to_string()),
            };
            if let Some(error) = write_err {
                return ToolCallResult::error(error);
            }
            if let (Some(ide), Some(_)) = (&self.ide, &workspace_relative) {
                ide.apply_document_if_ready(&path, edited).await;
            }
        }
        let result = render_tool_result(&path_display, &planned, wrote, execution.output_limit);
        self.finish(result, wrote, &execution, &raw_path, &path, &path_display)
            .await
    }
}
