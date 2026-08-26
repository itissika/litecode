use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::context_pipeline::Context;
use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;
use crate::types::ToolCallResult;

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
        path: &str,
        msg: String,
        execution: &ToolExecutionContext,
        raw_path: &str,
        resolved: &Path,
    ) -> ToolCallResult {
        let result = lsp_feedback::maybe_append_local_lsp_errors(
            self.ide.as_ref().map(|ide| ide.engines.as_ref()),
            path,
            ToolCallResult::ok(msg),
        )
        .await;
        crate::tools::file_path::with_path_risk_warning(
            result,
            &execution.workspace_root,
            raw_path,
            resolved,
            "writing",
        )
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

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": crate::tools::file_path::FILE_PATH_SCHEMA_HINT
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "create_only": {
                    "type": "boolean",
                    "description": "If true, refuse to overwrite an existing file (default: false)"
                }
            },
            "required": ["file_path", "content"]
        })
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

    fn description(&self, _ctx: &Context) -> String {
        "Create or overwrite a file with the given content.".into()
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "file_path")?;
        crate::tool::require_string(input, "content")?;
        Ok(())
    }

    fn is_destructive(
        &self,
        input: &Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> bool {
        let Some(path) = input["file_path"].as_str() else {
            return false;
        };
        if path.is_empty() {
            return false;
        }
        crate::workspace::resolve_agent(workspace_root, path, path_mode)
            .map(|resolved| resolved.exists())
            .unwrap_or(false)
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
}

impl WriteTool {
    async fn call_for_execution(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> ToolCallResult {
        let raw_path = match crate::tool::require_nonempty_string(&input, "file_path") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };
        let content = match crate::tool::require_string(&input, "content") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };
        let resolved = match crate::workspace::resolve_agent(
            &execution.workspace_root,
            raw_path,
            execution.path_mode,
        ) {
            Ok(path) => path,
            Err(error) => return ToolCallResult::error(error.to_string()),
        };
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&resolved).ok());
        if let (Some(ide), Some(relative)) = (&self.ide, workspace_relative) {
            let existed = resolved.exists();
            if input["create_only"].as_bool().unwrap_or(false) && existed {
                return ToolCallResult::error(format!(
                    "file already exists and create_only is set: {}. Omit create_only to overwrite, or choose a new path.",
                    resolved.display()
                ));
            }
            return match ide.workspace.write_file(&relative, content) {
                Ok(_) => {
                    ide.apply_document_if_ready(&resolved, content).await;
                    self.finish_ok(
                        &resolved.to_string_lossy(),
                        format!(
                            "{}: {} ({} bytes)",
                            if existed { "Updated" } else { "Created" },
                            resolved.display(),
                            content.len()
                        ),
                        &execution,
                        raw_path,
                        &resolved,
                    )
                    .await
                }
                Err(error) => ToolCallResult::error(error.to_string()),
            };
        }

        // ALL-mode absolute paths outside the workspace deliberately stay direct:
        // no workspace broadcast and no editor/LSP document sync.
        self.call_direct(
            &resolved,
            content,
            input["create_only"].as_bool().unwrap_or(false),
            &execution,
            raw_path,
        )
        .await
    }

    async fn call_direct(
        &self,
        file_path: &Path,
        content: &str,
        create_only: bool,
        execution: &ToolExecutionContext,
        raw_path: &str,
    ) -> ToolCallResult {
        let path = file_path.to_string_lossy().into_owned();
        let is_new = !file_path.exists();

        if create_only && !is_new {
            return ToolCallResult::error(format!(
                "file already exists and create_only is set: {path}. Omit create_only to overwrite, or choose a new path."
            ));
        }

        if let Some(parent) = file_path.parent() {
            match std::fs::create_dir_all(parent) {
                Ok(()) => {}
                Err(e) => return ToolCallResult::error(e.to_string()),
            }
        }

        match crate::workspace::file_ops::atomic_write(file_path, content) {
            Ok(()) => {}
            Err(e) => return ToolCallResult::error(e.to_string()),
        }

        let status = if is_new { "Created" } else { "Updated" };
        self.finish_ok(
            &path,
            format!("{}: {} ({} bytes)", status, path, content.len()),
            execution,
            raw_path,
            file_path,
        )
        .await
    }
}

/// Check if the target path is a sensitive system location (workspace-only floor).
pub fn is_sensitive_write_path(path: &str) -> bool {
    crate::permission::is_sensitive_system_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_create_new() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("new.txt");

        let tool = WriteTool::new();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "content": "hello world"
        });

        let result = tool.call(input);
        assert!(
            result.content.contains("Created"),
            "should report Created, got: {}",
            result.content
        );

        let content = std::fs::read_to_string(&file_path).expect("read back");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_write_update_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("existing.txt");
        std::fs::write(&file_path, "old content").expect("write");

        let tool = WriteTool::new();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "content": "new content"
        });

        let result = tool.call(input);
        assert!(
            result.content.contains("Updated"),
            "should report Updated, got: {}",
            result.content
        );

        let content = std::fs::read_to_string(&file_path).expect("read back");
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_write_create_only_refuses_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("exists.txt");
        std::fs::write(&file_path, "existing").expect("write");

        let tool = WriteTool::new();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "content": "new",
            "create_only": true
        });

        let result = tool.call(input);
        assert!(
            result.content.contains("create_only"),
            "error should mention create_only, got: {}",
            result.content
        );
    }

    #[test]
    fn test_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("a/b/c/deep.txt");

        let tool = WriteTool::new();
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "content": "deep content"
        });

        let _ = tool.call(input);
        let content = std::fs::read_to_string(&file_path).expect("read back");
        assert_eq!(content, "deep content");
    }

    #[test]
    fn test_write_atomic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("atomic.txt");

        let tool = WriteTool::new();
        let expected = "atomic write content\nsecond line\n";
        let input = serde_json::json!({
            "file_path": file_path.to_str().expect("path"),
            "content": expected
        });

        let _ = tool.call(input);

        let content = std::fs::read_to_string(&file_path).expect("read back");
        assert_eq!(content, expected, "read-back should match written content");
    }

    #[test]
    fn test_validate_input() {
        let tool = WriteTool::new();

        // Missing file_path
        assert!(
            tool.validate_input(&serde_json::json!({"content": "x"}))
                .is_err()
        );

        // Empty file_path
        assert!(
            tool.validate_input(&serde_json::json!({"file_path": "", "content": "x"}))
                .is_err()
        );

        // Missing content
        assert!(
            tool.validate_input(&serde_json::json!({"file_path": "/tmp/x"}))
                .is_err()
        );

        // Valid
        assert!(
            tool.validate_input(&serde_json::json!({"file_path": "/tmp/x", "content": "hello"}))
                .is_ok()
        );
    }

    #[test]
    fn test_is_destructive() {
        let dir = tempfile::tempdir().expect("tempdir");
        let existing = dir.path().join("existing.txt");
        std::fs::write(&existing, "x").expect("write");

        let tool = WriteTool::new();

        // Overwriting existing = destructive
        assert!(tool.is_destructive(
            &serde_json::json!({
                "file_path": "existing.txt",
                "content": "new"
            }),
            crate::workspace::ToolPathMode::All,
            dir.path(),
        ));

        // Creating new = not destructive
        assert!(!tool.is_destructive(
            &serde_json::json!({
                "file_path": "missing.txt",
                "content": "new"
            }),
            crate::workspace::ToolPathMode::All,
            dir.path(),
        ));
    }

    #[test]
    fn test_sensitive_path_detection() {
        assert!(is_sensitive_write_path("/etc/passwd"));
        assert!(is_sensitive_write_path("/boot/vmlinuz"));
        assert!(is_sensitive_write_path("/usr/bin/python3"));
        assert!(!is_sensitive_write_path("/home/user/project/main.rs"));
        assert!(!is_sensitive_write_path("/tmp/test.txt"));
    }

    #[tokio::test]
    async fn ide_write_emits_workspace_change_for_inner_path() {
        let root = tempfile::tempdir().expect("tempdir");
        let workspace =
            crate::workspace::WorkspaceService::new(root.path().to_path_buf()).expect("workspace");
        let engines = std::sync::Arc::new(crate::engines::WorkspaceEngines::new());
        let hub = std::sync::Arc::new(crate::terminal::TerminalHub::new());
        crate::terminal::install_hub(Arc::clone(&hub));
        let ide =
            crate::ide_base::IdeBaseHandle::new(std::sync::Arc::clone(&workspace), engines, hub);
        let mut rx = workspace.subscribe_changes();
        let tool = WriteTool::with_ide(std::sync::Arc::clone(&ide));
        let result = tool
            .execute(
                serde_json::json!({
                    "file_path": "inner.txt",
                    "content": "from agent"
                }),
                crate::tool::trait_::ToolExecutionContext {
                    path_mode: crate::workspace::ToolPathMode::All,
                    workspace_root: root.path().to_path_buf(),
                    call_id: "write-1".into(),
                    cancel: tokio_util::sync::CancellationToken::new(),
                    output_limit: 8_000,
                    session_id: String::new(),
                },
            )
            .await;
        assert!(result.content.contains("Created"), "{}", result.content);
        let change = rx.try_recv().expect("workspace change");
        assert!(change.paths.iter().any(|p| p.contains("inner.txt")));
        assert_eq!(
            std::fs::read_to_string(root.path().join("inner.txt")).unwrap(),
            "from agent"
        );
    }

    #[tokio::test]
    async fn external_all_write_skips_workspace_change() {
        let root = tempfile::tempdir().expect("workspace");
        let external = tempfile::tempdir().expect("external");
        let workspace =
            crate::workspace::WorkspaceService::new(root.path().to_path_buf()).expect("workspace");
        let engines = std::sync::Arc::new(crate::engines::WorkspaceEngines::new());
        let hub = std::sync::Arc::new(crate::terminal::TerminalHub::new());
        crate::terminal::install_hub(Arc::clone(&hub));
        let ide =
            crate::ide_base::IdeBaseHandle::new(std::sync::Arc::clone(&workspace), engines, hub);
        let mut rx = workspace.subscribe_changes();
        let external_file = external.path().join("outside.txt");
        let tool = WriteTool::with_ide(std::sync::Arc::clone(&ide));
        let result = tool
            .execute(
                serde_json::json!({
                    "file_path": external_file.to_string_lossy(),
                    "content": "external"
                }),
                crate::tool::trait_::ToolExecutionContext {
                    path_mode: crate::workspace::ToolPathMode::All,
                    workspace_root: root.path().to_path_buf(),
                    call_id: "write-ext".into(),
                    cancel: tokio_util::sync::CancellationToken::new(),
                    output_limit: 8_000,
                    session_id: String::new(),
                },
            )
            .await;
        assert!(
            result.content.contains("Created") || result.content.contains("Updated"),
            "{}",
            result.content
        );
        assert!(
            rx.try_recv().is_err(),
            "external ALL must not emit workspace change"
        );
        assert_eq!(std::fs::read_to_string(&external_file).unwrap(), "external");
    }
}
