//! Phase 1 checks: diagnostics, unsupported media, and the structured ledger.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::GlobalSettings;
use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, deny_permission_sink};
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tools::code::{CodeTool, CodeToolHost};
use crate::tools::lsp_feedback::arm_lsp_error_hook;
use crate::types::{FunctionToolCall, ToolCallResult};

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

fn code_origin() -> ToolCallOrigin {
    ToolCallOrigin {
        caller: ToolCaller::Code {
            parent_call_id: "p1".into(),
        },
        code_names: Arc::from([]),
        file_note: None,
        permission_sink: None,
        plan_touch: None,
    }
}

fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(crate::tools::read::ReadTool::default()),
        Arc::new(crate::tools::glob::GlobTool),
        Arc::new(crate::tools::grep::GrepTool),
        Arc::new(crate::tools::edit::EditTool::new()),
        Arc::new(crate::tools::write::WriteTool::new()),
    ]
}

async fn call_code(root: &Path, name: &str, args: Value) -> ToolCallResult {
    let permission = allow_engine(root);
    let call = FunctionToolCall {
        arguments: args.to_string(),
        call_id: format!("p1-{name}"),
        name: name.into(),
        namespace: None,
        id: None,
        status: None,
    };
    let sink = deny_permission_sink();
    execute_tool_call(
        &call,
        &tools(),
        &permission,
        &context(root),
        "p1-session",
        "default",
        sink.as_ref(),
        CancellationToken::new(),
        &root.join(".litecode"),
        usize::MAX,
        None,
        Arc::new(WorkspaceWriteLock::new()),
        None,
        code_origin(),
    )
    .await
}

fn host(root: &Path) -> Arc<CodeToolHost> {
    Arc::new(CodeToolHost::new(
        allow_engine(root),
        context(root),
        "default",
        deny_permission_sink(),
        root.join(".litecode"),
        usize::MAX,
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ))
}

async fn run_program(root: &Path, code: &str) -> ToolCallResult {
    let tool = CodeTool::new(tools(), host(root));
    tool.execute(
        json!({ "code": code }),
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: root.to_path_buf(),
            call_id: "p1-run".into(),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: "p1-session".into(),
            session: None,
        },
    )
    .await
}

fn has_code(result: &ToolCallResult, code: &str) -> bool {
    result.diagnostics.iter().any(|diagnostic| diagnostic.code == code)
}

#[tokio::test]
async fn edit_lsp_error_reaches_outcome_and_run_code_data() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hooked.rs"), "alpha\n").unwrap();
    let _guard = arm_lsp_error_hook("hooked.rs", "error: unused value");

    let direct = call_code(
        dir.path(),
        "edit",
        json!({
            "path": "hooked.rs",
            "edits": [{ "old_string": "alpha", "new_string": "beta" }]
        }),
    )
    .await;
    assert!(
        direct.error.is_none(),
        "edit failed: {}",
        direct.content
    );
    assert!(
        has_code(&direct, "lsp_error"),
        "direct diagnostics: {:?}",
        direct.diagnostics
    );

    let nested = run_program(
        dir.path(),
        r#"await tools.edit({ path: "hooked.rs", edits: [{ old_string: "beta", new_string: "gamma" }] }); return null;"#,
    )
    .await;
    assert!(nested.error.is_none(), "run_code failed: {}", nested.content);
    assert!(
        nested.content.contains("lsp_error"),
        "run_code content: {}",
        nested.content
    );
    assert!(nested.data.is_none(), "run_code no longer returns data");
}

#[tokio::test]
async fn write_outside_workspace_reports_path_outside_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("note.txt");
    let result = call_code(
        workspace.path(),
        "write",
        json!({
            "path": target.to_string_lossy(),
            "content": "hello"
        }),
    )
    .await;
    assert!(
        has_code(&result, "path_outside_workspace"),
        "diagnostics={:?} content={}",
        result.diagnostics,
        result.content
    );
    assert!(target.is_file(), "write should still commit the file");
}

#[tokio::test]
async fn unknown_parameter_is_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let result = call_code(
        dir.path(),
        "read",
        json!({ "path": "a.txt", "nope": true }),
    )
    .await;
    assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
    assert!(result.content.contains("nope"), "{}", result.content);
    assert!(result.data.is_none());
}

#[tokio::test]
async fn read_image_is_unsupported_without_parts() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = [
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00,
    ];
    std::fs::write(dir.path().join("pic.png"), bytes).unwrap();
    let result = call_code(dir.path(), "read", json!({ "path": "pic.png" })).await;
    assert_eq!(result.error_code.as_deref(), Some("unsupported"));
    assert!(result.parts.is_empty(), "image read must not return media parts");
    assert!(result.data.is_none());
    let details = result.error_details.as_ref().expect("error details");
    assert_eq!(details["mime"], "image/png");
    assert_eq!(details["path"], "pic.png");
    assert_eq!(details["bytes"], bytes.len() as u64);
}

#[tokio::test]
async fn run_code_calls_match_the_text_ledger_and_logs_keep_level() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let result = run_program(
        dir.path(),
        r#"
        const out = await tools.read({ path: "a.txt" });
        console.info("seen");
        console.warn("careful");
        return out.lines[0].text;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "run_code failed: {}", result.content);
    assert!(result.data.is_none(), "run_code no longer returns data");
    assert!(
        result.content.contains("1. read ") && result.content.contains(" ok"),
        "ledger line missing from {}",
        result.content
    );
    assert!(result.content.contains("path=a.txt") || result.content.contains("a.txt"), "{}", result.content);
    assert!(result.content.contains("[info] seen"), "{}", result.content);
    assert!(result.content.contains("[warn] careful"), "{}", result.content);
    assert!(result.content.contains("hello"), "{}", result.content);
    assert!(!result.content.contains("@@run_code_data"), "{}", result.content);
}

#[tokio::test]
async fn uncaught_tool_error_keeps_the_outer_code() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        r#"await tools.read({ path: "missing.txt" });"#,
    )
    .await;
    assert_eq!(result.error_code.as_deref(), Some("not_found"));
    assert!(
        result
            .content
            .contains("uncaught ToolError not_found from call #1 read path=missing.txt"),
        "{}",
        result.content
    );
    assert!(result.content.contains("not_found:"), "{}", result.content);
}
