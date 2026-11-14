use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::types::{ToolCallResult, ToolErrorKind};

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;

/// Success facts for write: the path plus reminder-only path risk and
/// language-server diagnostics.
#[derive(serde::Serialize)]
struct WriteData {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path_risk: Option<crate::tools::file_path::PathRisk>,
    /// Warnings recorded after the write; absent when the write was clean.
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostics: Option<Vec<crate::tool::outcome::Diagnostic>>,
}

pub struct WriteTool {
    ide: Option<Arc<crate::ide_base::IdeBaseHandle>>,
}

impl WriteTool {
    pub fn new() -> Self {
        Self { ide: None }
    }

    pub fn with_ide(ide: Arc<crate::ide_base::IdeBaseHandle>) -> Self {
        Self { ide: Some(ide) }
    }

    async fn finish_ok(
        &self,
        execution: &ToolExecutionContext,
        raw_path: &str,
        resolved: &Path,
    ) -> ToolCallResult {
        let path = crate::tool::file_identity::locate_resolved(
            &execution.workspace_root,
            resolved,
            raw_path,
        )
        .path;
        let data = WriteData {
            path,
            path_risk: crate::tools::file_path::path_risk(
                &execution.workspace_root,
                raw_path,
                resolved,
            ),
            diagnostics: lsp_feedback::diagnostics_field(
                lsp_feedback::local_lsp_error_diagnostics(
                    self.ide.as_ref().map(|ide| ide.engines.as_ref()),
                    &resolved.to_string_lossy(),
                )
                .await,
            ),
        };
        crate::tool::outcome::ToolOutcome::ok(data).into_call_result("")
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
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        }
    }
}

impl Default for WriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
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

    fn call_inner(&self, input: Value) -> ToolCallResult {
        // Unit-test helper only. Production Agent turns enter through execute().
        let execution = self.test_execution_context();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        rt.block_on(self.call_for_execution(input, execution))
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tools::file_path::require_code_path(input)?;
        crate::tool::require_string(input, "content")?;
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> bool {
        let Some(path) = crate::tools::file_path::code_path(input) else {
            return false;
        };
        crate::workspace::resolve_agent(workspace_root, path, path_mode)
            .map(|resolved| resolved.exists())
            .unwrap_or(false)
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
            None => vec![],
        }
    }
}

impl WriteTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tools::file_path::require_code_path(&input) {
            Ok(s) => s,
            Err(e) => return crate::tools::file_path::coded_error("invalid_input", e),
        };
        if crate::session::transcript_file::is_virtual_session_path(raw_path) {
            return crate::tools::file_path::coded_error(
                "invalid_input",
                crate::session::transcript_file::READ_ONLY_MSG,
            );
        }
        let content = match crate::tool::require_string(&input, "content") {
            Ok(s) => s,
            Err(e) => return crate::tools::file_path::coded_error("invalid_input", e),
        };
        let resolved = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => return crate::tools::file_path::tool_path_error(error),
        };
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&resolved).ok());
        if execution.cancel.is_cancelled() {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Cancelled,
                "write cancelled before write; file was not modified",
                false,
            );
        }
        if let (Some(ide), Some(relative)) = (&self.ide, workspace_relative) {
            return match ide.workspace.write_file(&relative, content) {
                Ok(_) => {
                    ide.apply_document_if_ready(&resolved, content).await;
                    self.finish_ok(&execution, raw_path, &resolved).await
                }
                Err(error) => crate::tools::file_path::workspace_error(error),
            };
        }

        // ALL-mode absolute paths outside the workspace deliberately stay direct:
        // no workspace broadcast and no editor/LSP document sync.
        self.call_direct(&resolved, content, &execution, raw_path).await
    }

    async fn call_direct(
        &self,
        file_path: &Path,
        content: &str,
        execution: &ToolExecutionContext,
        raw_path: &str,
    ) -> ToolCallResult {
        if let Some(parent) = file_path.parent() {
            match std::fs::create_dir_all(parent) {
                Ok(()) => {}
                Err(e) => return crate::tools::file_path::coded_error("io", e.to_string()),
            }
        }

        match crate::workspace::file_ops::atomic_write(file_path, content) {
            Ok(()) => {}
            Err(e) => return crate::tools::file_path::coded_error("io", e.to_string()),
        }

        self.finish_ok(execution, raw_path, file_path).await
    }
}

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    let tool = WriteTool::new();
    let workspace = tempfile::tempdir().expect("workspace root");
    let outside = tempfile::tempdir().expect("outside root");
    let run = |input: Value| {
        let mut execution = tool.test_execution_context();
        execution.workspace_root = workspace.path().to_path_buf();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(tool.call_for_execution(input, execution))
    };
    let _hook = crate::tools::lsp_feedback::arm_lsp_error_hook("outside.txt", "error[E1]: boom");
    vec![DriftCase {
        tool: "write",
        all_optional_present: run(serde_json::json!({
            "path": outside.path().join("outside.txt").to_string_lossy(),
            "content": "hello",
        })),
        all_optional_absent: run(serde_json::json!({
            "path": "inside.txt",
            "content": "hello",
        })),
    }]
}
