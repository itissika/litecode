use tokio_util::sync::CancellationToken;

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::EditTool;

fn tool() -> EditTool {
    EditTool::new()
}

fn call_edit(path: &std::path::Path, old: &str, new: &str) -> crate::types::ToolCallResult {
    tool().call(serde_json::json!({
        "path": path.to_str().unwrap(),
        "old_string": old,
        "new_string": new
    }))
}

fn write_temp(name: &str, contents: impl AsRef<[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

#[test]
fn edit_tool_rejects_nul_bytes() {
    let (_dir, path) = write_temp("bin.txt", b"ok\x00still-utf8");
    let result = call_edit(&path, "ok", "no");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().to_lowercase().contains("binary")
            || result.content.to_lowercase().contains("binary"),
        "{}",
        data["message"]
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"ok\x00still-utf8");
}

#[test]
fn edit_tool_rejects_utf16() {
    let (_dir, path) = write_temp("u16.txt", [0xFF, 0xFE, b'A', 0x00]);
    let result = call_edit(&path, "A", "B");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("UTF-16"),
        "{}",
        data["message"]
    );
}

#[test]
fn edit_tool_cancel_before_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let execution = ToolExecutionContext {
        path_mode: crate::workspace::ToolPathMode::All,
        workspace_root: dir.path().to_path_buf(),
        call_id: String::new(),
        cancel,
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
                "path": path.to_str().unwrap(),
                "old_string": "fn main() {}",
                "new_string": "fn start() {}"
            }),
            execution,
        ));
    assert!(result.content.contains("cancelled"), "{}", result.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
}

/// edit failure messages (merged from phase3_gate, WP-207).
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod failure_message_tests {
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
        let resolved = resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
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

}
