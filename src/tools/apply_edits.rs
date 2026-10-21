//! Code-surface serial batch replacements (`apply_edits`).
//!
//! Contract (WP-198):
//! `tools.apply_edits(edits: Array<{path,old_string,new_string}>) -> { results: EditResult[] }`
//! Serial; item failure skips and continues; `results` matches input length.
//! Companion of `edit` (injected when `edit` is bound). Shares the edit dial
//! for permission / path_mode.

use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::edit::engine::{Cancelled, EditEngine};
use crate::tools::edit::planner::{self, EditSpec, CANCELLED_MESSAGE};
use crate::tools::edit::result::present_apply_edits;
use crate::tools::edit::edit_result_schema;
use crate::tools::file_path::coded_error;
use crate::types::{ToolCallResult, ToolErrorKind};

pub struct ApplyEditsTool {
    engine: EditEngine,
}

impl ApplyEditsTool {
    pub fn new() -> Self {
        Self {
            engine: EditEngine::new(None),
        }
    }

    pub fn with_ide(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self {
            engine: EditEngine::new(Some(ide)),
        }
    }

    fn test_execution_context(&self) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: self
                .engine
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

    fn parse_items(input: &Value) -> Result<Vec<(String, EditSpec)>, String> {
        let Some(edits) = input.get("edits") else {
            return Err(crate::tool::missing_parameter("edits"));
        };
        let Some(items) = edits.as_array() else {
            return Err(crate::tool::expected_type("edits", "array", edits));
        };
        if items.is_empty() {
            return Err(crate::tool::must_be(
                "edits",
                "an array with at least 1 item(s)",
            ));
        }
        let mut out = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let prefix = format!("edits[{i}]");
            let path = match item.get("path").or_else(|| item.get("file_path")) {
                Some(value) => crate::tool::require_string_value(value, &format!("{prefix}.path"))?
                    .to_string(),
                None => {
                    return Err(crate::tool::missing_parameter(&format!("{prefix}.path")));
                }
            };
            if path.trim().is_empty() {
                return Err(crate::tool::must_be(&format!("{prefix}.path"), "a non-empty string"));
            }
            let spec = planner::parse_spec(item, &prefix, &["path", "file_path"])?;
            out.push((path, spec));
        }
        Ok(out)
    }

    pub(crate) async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let items = match Self::parse_items(&input) {
            Ok(items) => items,
            Err(e) => return coded_error("invalid_input", e),
        };
        if execution.cancel.is_cancelled() {
            return cancelled_apply();
        }
        let mut results = Vec::with_capacity(items.len());
        for (path, spec) in &items {
            if execution.cancel.is_cancelled() {
                return cancelled_apply();
            }
            match self.engine.apply_one(&execution, path, spec, "applied").await {
                Ok(result) => results.push(result),
                Err(Cancelled) => return cancelled_apply(),
            }
        }
        present_apply_edits(results)
    }
}

fn cancelled_apply() -> ToolCallResult {
    ToolCallResult::error_with_kind(
        ToolErrorKind::Cancelled,
        CANCELLED_MESSAGE,
        false,
    )
}

impl Default for ApplyEditsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ApplyEditsTool {
    fn name(&self) -> &str {
        "apply_edits"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Replacements to apply serially. A failed item is skipped; later items still run. results matches this array length.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                            },
                            "old_string": {
                                "type": "string",
                                "description": format!(
                                    "Exact file text to find. Do not include {}. Must match exactly one place.",
                                    crate::tool::FILE_LINE_PREFIX_HINT
                                )
                            },
                            "new_string": {
                                "type": "string",
                                "description": "Replacement text. Use an empty string to delete old_string."
                            }
                        },
                        "required": ["path", "old_string", "new_string"]
                    }
                }
            },
            "required": ["edits"]
        })
    }

    fn data_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
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
        }))
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(60))
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.call_for_execution(input, execution))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        Self::parse_items(input).map(|_| ())
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
        let keys = self.resource_keys(input, path_mode, workspace_root);
        if keys.is_empty() {
            crate::tool::resource_gate::ToolEffects::default()
        } else {
            // Lock the first file; resource_keys lists all for conflict detection.
            crate::tool::resource_gate::ToolEffects::write(keys[0].clone())
        }
    }

    fn resource_keys(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<ResourceKey> {
        let Some(items) = input.get("edits").and_then(Value::as_array) else {
            return vec![];
        };
        let mut keys = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let Some(path) = item
                .get("path")
                .or_else(|| item.get("file_path"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let key = crate::workspace::resolve_agent(workspace_root, path, path_mode)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.to_string());
            if seen.insert(key.clone()) {
                keys.push(ResourceKey::File(key));
            }
        }
        keys
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
        crate::tools::description_text(include_str!("descriptions/apply_edits.md"))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::Tool;

    fn tool() -> ApplyEditsTool {
        ApplyEditsTool::new()
    }

    fn write_temp(name: &str, contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    #[test]
    fn serial_skip_continues_results_same_len() {
        let (dir, path) = write_temp("a.txt", "alpha\nbeta\n");
        let rel = path.file_name().unwrap().to_str().unwrap();
        // Use absolute path via call with workspace = dir
        let execution = ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: dir.path().to_path_buf(),
            call_id: String::new(),
            cancel: CancellationToken::new(),
            output_limit: 8_000,
            session_id: String::new(),
            session: None,
        };
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(tool().call_for_execution(
                serde_json::json!({
                    "edits": [
                        {"path": rel, "old_string": "alpha", "new_string": "ALPHA"},
                        {"path": rel, "old_string": "missing", "new_string": "nope"},
                        {"path": rel, "old_string": "beta", "new_string": "BETA"}
                    ]
                }),
                execution,
            ));
        assert!(result.error.is_none(), "{}", result.content);
        let results = result.data.as_ref().unwrap()["results"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["ok"], true);
        assert_eq!(results[1]["ok"], false);
        assert_eq!(results[2]["ok"], true);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ALPHA\nBETA\n");
    }

    #[test]
    fn schema_rejects_retired_knobs() {
        let err = tool()
            .validate_input(&serde_json::json!({
                "edits": [{
                    "path": "a.txt",
                    "old_string": "a",
                    "new_string": "b",
                    "replace_all": true
                }]
            }))
            .unwrap_err();
        assert!(err.contains("replace_all"), "{err}");
    }

    #[test]
    fn description_mentions_serial_skip() {
        let ctx = crate::context_pipeline::Context {
            cwd: std::path::PathBuf::from("."),
            workspace_paths: crate::config::resolved::WorkspacePaths::for_legacy_root(
                std::path::Path::new("."),
            ),
            agents_md: None,
            claude_md: None,
        };
        let text = tool().description(&ctx);
        assert!(text.contains("serially") || text.contains("skipped"), "{text}");
        assert!(text.contains("no `replace_all`") || text.contains("There is no"), "{text}");
        assert!(text.contains("no `expected_hash`") || text.contains("There is no"), "{text}");
    }
}
