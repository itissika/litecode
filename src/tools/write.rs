use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

use crate::types::{ToolCallResult, ToolErrorKind};

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::ResourceKey;
use crate::tools::lsp_feedback;

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
        created: bool,
        written: &[u8],
        execution: &ToolExecutionContext,
        raw_path: &str,
        resolved: &Path,
    ) -> ToolCallResult {
        let located = crate::tool::file_identity::locate_resolved(
            &execution.workspace_root,
            resolved,
            raw_path,
        );
        let file = crate::tool::file_identity::file_ref(located, written);
        let data = serde_json::json!({
            "file": file,
            "created": created,
        });
        let result = lsp_feedback::maybe_append_local_lsp_errors(
            self.ide.as_ref().map(|ide| ide.engines.as_ref()),
            &resolved.to_string_lossy(),
            crate::tool::outcome::ToolOutcome::ok(data).into_call_result(""),
        )
        .await;
        let mut result = crate::tools::file_path::with_path_risk_warning(
            result,
            &execution.workspace_root,
            raw_path,
            resolved,
            "writing",
        );
        if let Some(message) = diagnostic_message(&result.diagnostics)
            && let Some(data) = result.data.as_mut().and_then(Value::as_object_mut)
        {
            data.insert("message".into(), Value::String(message));
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
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: self.max_result_size(),
            session_id: String::new(),
            session: None,
        }
    }
}

fn diagnostic_message(diagnostics: &[crate::tool::outcome::Diagnostic]) -> Option<String> {
    let mut parts = Vec::new();
    for diagnostic in diagnostics {
        let mut text = diagnostic.message.clone();
        if diagnostic.code == "lsp_error"
            && let Some(block) = diagnostic
                .details
                .as_ref()
                .and_then(|details| details.get("text"))
                .and_then(Value::as_str)
        {
            text.push('\n');
            text.push_str(block);
        }
        if !text.trim().is_empty() {
            parts.push(text);
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
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
        if let Some(value) = input.get("expected_hash").filter(|value| !value.is_null())
            && !value.is_string()
        {
            return Err("parameter 'expected_hash' must be a string".into());
        }
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
        let create_only = input["create_only"].as_bool().unwrap_or(false);
        let expected_hash = input.get("expected_hash").and_then(Value::as_str);
        let workspace_relative = self
            .ide
            .as_ref()
            .and_then(|ide| ide.workspace.sandbox().rel_path(&resolved).ok());
        let current = match self.read_existing(&resolved, workspace_relative.as_deref()) {
            Ok(bytes) => bytes,
            Err(error) => return error,
        };
        if let Some(rejected) =
            write_precondition(&resolved, create_only, expected_hash, current.as_deref())
        {
            return rejected;
        }
        if execution.cancel.is_cancelled() {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Cancelled,
                "write cancelled before write; file was not modified",
                false,
            );
        }
        let created = current.is_none();
        if let (Some(ide), Some(relative)) = (&self.ide, workspace_relative) {
            return match ide.workspace.write_file(&relative, content) {
                Ok(_) => {
                    ide.apply_document_if_ready(&resolved, content).await;
                    self.finish_ok(created, content.as_bytes(), &execution, raw_path, &resolved)
                        .await
                }
                Err(error) => crate::tools::file_path::workspace_error(error),
            };
        }

        // ALL-mode absolute paths outside the workspace deliberately stay direct:
        // no workspace broadcast and no editor/LSP document sync.
        self.call_direct(&resolved, content, created, &execution, raw_path)
            .await
    }

    fn read_existing(
        &self,
        resolved: &Path,
        workspace_relative: Option<&str>,
    ) -> Result<Option<Vec<u8>>, ToolCallResult> {
        if let (Some(ide), Some(relative)) = (&self.ide, workspace_relative) {
            return match ide.workspace.read_file_bytes(relative) {
                Ok((_, bytes)) => Ok(Some(bytes)),
                Err(crate::workspace::WorkspaceError::NotFound(_)) => Ok(None),
                Err(error) => Err(crate::tools::file_path::workspace_error(error)),
            };
        }
        match std::fs::read(resolved) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(crate::tools::file_path::coded_error(
                "io",
                error.to_string(),
            )),
        }
    }

    async fn call_direct(
        &self,
        file_path: &Path,
        content: &str,
        created: bool,
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

        self.finish_ok(created, content.as_bytes(), execution, raw_path, file_path)
            .await
    }
}

fn write_precondition(
    resolved: &Path,
    create_only: bool,
    expected_hash: Option<&str>,
    current: Option<&[u8]>,
) -> Option<ToolCallResult> {
    if create_only && expected_hash.is_some() && current.is_none() {
        let message = "create_only and expected_hash were both set, but the file does not exist";
        return Some(
            crate::tool::outcome::ToolOutcome::<Value>::failure("invalid_input", message, false)
                .into_call_result(message),
        );
    }
    if let Some(expected) = expected_hash {
        let Some(bytes) = current else {
            let message = "file does not exist, so expected_hash cannot be checked";
            return Some(
                crate::tool::outcome::ToolOutcome::<Value>::failure("not_found", message, false)
                    .into_call_result(message),
            );
        };
        let current_hash = crate::tool::file_identity::sha256_hex(bytes);
        if expected != current_hash {
            let message = "file changed since it was read; expected_hash does not match";
            return Some(
                crate::tool::outcome::ToolOutcome::<Value>::failure("stale", message, true)
                    .with_error_details(serde_json::json!({ "current_hash": current_hash }))
                    .into_call_result(message),
            );
        }
    }
    if create_only && current.is_some() {
        let message = format!(
            "file already exists and create_only is set: {}. Omit create_only to overwrite, or choose a new path.",
            resolved.display()
        );
        return Some(
            crate::tool::outcome::ToolOutcome::<Value>::failure("conflict", message.clone(), false)
                .into_call_result(message),
        );
    }
    None
}

/// Check if the target path is a sensitive system location (workspace-only floor).
pub fn is_sensitive_write_path(path: &str) -> bool {
    crate::permission::is_sensitive_system_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_create_only_refuses_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("exists.txt");
        std::fs::write(&file_path, "existing").expect("write");

        let tool = WriteTool::new();
        let input = serde_json::json!({
            "path": file_path.to_str().expect("path"),
            "content": "new",
            "create_only": true
        });

        let result = tool.call(input);
        assert_eq!(result.error_code.as_deref(), Some("conflict"));
        assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "existing");
    }

    #[test]
    fn virtual_session_path_is_read_only() {
        let tool = WriteTool::new();
        let result = tool.call(serde_json::json!({
            "path": ".litecode/sessions/01ABCDEF.md",
            "content": "nope"
        }));
        assert_eq!(result.level, crate::types::ToolSignalLevel::Error);
        assert!(
            result
                .content
                .contains(crate::session::transcript_file::READ_ONLY_MSG),
            "{}",
            result.content
        );
    }
}

/// write expected_hash preconditions (merged from phase3_gate, WP-207).
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod expected_hash_tests {
    use std::path::Path;
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;

    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::GlobalSettings;
    use crate::context_pipeline::Context;
    use crate::permission::{PermissionEngine, deny_permission_sink};
    use crate::tool::trait_::{Tool, ToolExecutionContext};
    use crate::tool::write_lock::WorkspaceWriteLock;
    use crate::tools::code::{CodeTool, CodeToolHost};
    use crate::tools::edit::EditTool;
    use crate::tools::write::WriteTool;
    use crate::types::ToolCallResult;

    fn execution(root: &Path) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::Safe,
            workspace_root: root.to_path_buf(),
            call_id: String::new(),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: String::new(),
            session: None,
        }
    }

    fn call<T: Tool>(tool: &T, root: &Path, input: Value) -> ToolCallResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(tool.execute(input, execution(root)))
    }

    fn allow_engine(root: &Path) -> PermissionEngine {
        let resolved =
            resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
        PermissionEngine::resolver(resolved, "default", 0)
    }

    fn context(root: &Path) -> Context {
        Context {
            cwd: root.to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(root),
            agents_md: None,
            claude_md: None,
        }
    }

    fn tools() -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(crate::tools::read::ReadTool::default()),
            Arc::new(EditTool::new()),
            Arc::new(crate::tools::apply_edits::ApplyEditsTool::new()),
            Arc::new(WriteTool::new()),
        ]
    }

    async fn run_program(root: &Path, code: &str) -> ToolCallResult {
        let host = Arc::new(CodeToolHost::new(
            allow_engine(root),
            context(root),
            "default",
            deny_permission_sink(),
            root.join(".litecode"),
            Arc::new(WorkspaceWriteLock::new()),
            None,
        ));
        let tool = CodeTool::new(tools(), host);
        tool.execute(
            json!({ "code": code }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: "p3-run".into(),
                cancel: CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: "p3-session".into(),
                session: None,
            },
        )
        .await
    }

    #[test]
    fn write_expected_hash_mismatch_is_stale() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "alpha\n").unwrap();
        let result = call(
            &WriteTool::new(),
            dir.path(),
            json!({
                "path": "note.txt",
                "content": "beta\n",
                "expected_hash": "0".repeat(64)
            }),
        );
        assert_eq!(result.error_code.as_deref(), Some("stale"));
        assert!(result.data.is_none());
        assert_eq!(
            result.error_details.as_ref().unwrap()["current_hash"],
            crate::tool::file_identity::sha256_hex(b"alpha\n")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\n");
    }

    #[test]
    fn write_expected_hash_match_returns_file_ref() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "alpha\n").unwrap();
        let hash = crate::tool::file_identity::sha256_hex(b"alpha\n");
        let result = call(
            &WriteTool::new(),
            dir.path(),
            json!({
                "path": "note.txt",
                "content": "beta\n",
                "expected_hash": hash
            }),
        );
        assert!(result.error.is_none(), "{}", result.content);
        let data = result.data.unwrap();
        assert_eq!(data["created"], false);
        assert_eq!(data["file"]["path"], "note.txt");
        assert_eq!(
            data["file"]["hash"],
            crate::tool::file_identity::sha256_hex(b"beta\n")
        );
        assert_eq!(data["file"]["bytes"], 5);
        assert!(data["file"].get("abs").is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "beta\n");
    }

    #[test]
    fn write_expected_hash_on_missing_file_does_not_create() {
        let dir = tempfile::tempdir().unwrap();
        let result = call(
            &WriteTool::new(),
            dir.path(),
            json!({
                "path": "missing.txt",
                "content": "hello",
                "expected_hash": "ab"
            }),
        );
        assert_eq!(result.error_code.as_deref(), Some("not_found"));
        assert!(!dir.path().join("missing.txt").exists());
    }
}
