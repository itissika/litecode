//! Code-surface single-file replacement (`edit`).
//!
//! Contract (WP-198):
//! `tools.file.edit({ path, old_string, new_string }) -> ToolResult<EditData>`
//! Unique match required. Empty `new_string` deletes `old_string`. No
//! `replace_all`, `expected_hash`, multi-block `edits[]`, or structured
//! blocks/applied/failed report. Domain failures resolve as ToolFailure.

use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::file_path::coded_error;
use crate::types::{ToolCallResult, ToolErrorKind};

pub(crate) mod engine;
mod matcher;
pub(crate) mod planner;
pub(crate) mod result;

use self::engine::{Cancelled, EditEngine};
use self::planner::CANCELLED_MESSAGE;

#[cfg(test)]
mod python_indent;
#[cfg(test)]
mod tests;

pub use result::EditResult as EditResultData;

pub struct EditTool {
    engine: EditEngine,
}

impl EditTool {
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

    pub(crate) async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tools::file_path::require_code_path(&input) {
            Ok(s) => s.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        let spec = match planner::parse_spec(&input, "", &["path", "file_path"]) {
            Ok(spec) => spec,
            Err(e) => return coded_error("invalid_input", e),
        };
        if execution.cancel.is_cancelled() {
            return cancelled_edit();
        }
        match self
            .engine
            .apply_one(&execution, &raw_path, &spec, "replaced")
            .await
        {
            Ok(result) => result::present_edit(result),
            Err(Cancelled) => cancelled_edit(),
        }
    }
}

fn cancelled_edit() -> ToolCallResult {
    ToolCallResult::error_with_kind(ToolErrorKind::Cancelled, CANCELLED_MESSAGE, false)
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

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(30))
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
        planner::parse_spec(input, "", &["path", "file_path"])?;
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        _path_mode: crate::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> bool {
        input["new_string"].as_str().is_some_and(str::is_empty)
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
}

/// Re-export for apply_edits / tests that build results without the tool.
pub use result::{present_apply_edits, present_edit};
