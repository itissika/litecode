//! Real JavaScript integration for the internal CodeTool vertical slice.

use std::sync::Arc;

use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{PermissionEngine, deny_permission_sink};
use litecode::tool::Tool;
use litecode::tool::executor::run_tool;
use litecode::tool::write_lock::process_write_lock;
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::read::ReadTool;
use litecode::types::{FunctionToolCall, ToolErrorKind, ToolSignalLevel};
use serde_json::{Value, json};

fn run_code_result(content: &str) -> Value {
    let rest = content
        .split_once("result:\n")
        .unwrap_or_else(|| panic!("missing result section in {content}"))
        .1;
    serde_json::from_str(rest.trim()).unwrap_or_else(|_| Value::String(rest.trim().to_string()))
}


#[tokio::test(flavor = "current_thread")]
async fn code_tool_captures_logs_and_enforces_output_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let resolved = ConfigManager::resolve_without_catalog(
        GlobalSettings::default(),
        WorkspaceState::new(root),
    );
    let permission = PermissionEngine::resolver(resolved, "default", 0);
    let ctx = Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let data_root = root.join(".litecode");
    std::fs::create_dir_all(&data_root).unwrap();

    let bundle: Vec<Arc<dyn Tool>> = vec![Arc::new(ReadTool::default())];
    let host = Arc::new(CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        deny_permission_sink(),
        data_root.clone(),
        process_write_lock(),
        None,
    ));
    let code: Arc<dyn Tool> = Arc::new(CodeTool::new(bundle, host));
    let tools = vec![Arc::clone(&code)];

    let logs_script = r#"
console.log("hello", 42);
console.error("warn");
return { ok: true };
"#;
    let call = FunctionToolCall {
        arguments: json!({ "code": logs_script }).to_string(),
        call_id: "code-logs-1".to_string(),
        namespace: None,
        name: "run_code".to_string(),
        id: None,
        status: None,
    };
    let result = run_tool(
        &call,
        &tools,
        &permission,
        &ctx,
        "code-runtime-session",
        "default",
        deny_permission_sink().as_ref(),
        tokio_util::sync::CancellationToken::new(),
        &data_root,
        32_768,
        None,
        process_write_lock(),
        None,
    )
    .await;
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    // WP-209: logs collapse to one line on success; kept (tail, 2k tok) on failure.
    assert!(
        result.content.contains("logs omitted (2 lines)"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("hello 42"), "{}", result.content);
    assert_eq!(run_code_result(&result.content), json!({ "ok": true }));

    let failing = FunctionToolCall {
        arguments: json!({ "code": "console.log(\"hello\", 42);\nconsole.error(\"warn\");\nthrow new Error(\"boom\");" }).to_string(),
        call_id: "code-logs-2".to_string(),
        namespace: None,
        name: "run_code".to_string(),
        id: None,
        status: None,
    };
    let result = run_tool(
        &failing,
        &tools,
        &permission,
        &ctx,
        "code-runtime-session",
        "default",
        deny_permission_sink().as_ref(),
        tokio_util::sync::CancellationToken::new(),
        &data_root,
        32_768,
        None,
        process_write_lock(),
        None,
    )
    .await;
    assert!(result.content.contains("[log] hello 42"), "{}", result.content);
    assert!(result.content.contains("[error] warn"), "{}", result.content);

    let oversized = format!(
        "return \"x\".repeat({});",
        litecode::tools::code::MAX_CODE_OUTPUT_BYTES + 1
    );
    let call = FunctionToolCall {
        arguments: json!({ "code": oversized }).to_string(),
        call_id: "code-output-limit-1".to_string(),
        namespace: None,
        name: "run_code".to_string(),
        id: None,
        status: None,
    };
    let result = run_tool(
        &call,
        &tools,
        &permission,
        &ctx,
        "code-runtime-session",
        "default",
        deny_permission_sink().as_ref(),
        tokio_util::sync::CancellationToken::new(),
        &data_root,
        32_768,
        None,
        process_write_lock(),
        None,
    )
    .await;
    assert_eq!(result.level, ToolSignalLevel::Error, "{}", result.content);
    assert_eq!(
        result.error.expect("typed output-limit result").kind,
        ToolErrorKind::OutputLimit
    );
    assert!(result.content.contains("output limit"), "{}", result.content);
}

#[tokio::test(flavor = "current_thread")]
async fn code_tool_has_no_process_filesystem_or_module_surface() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let resolved = ConfigManager::resolve_without_catalog(
        GlobalSettings::default(),
        WorkspaceState::new(root),
    );
    let permission = PermissionEngine::resolver(resolved, "default", 0);
    let ctx = Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let data_root = root.join(".litecode");
    std::fs::create_dir_all(&data_root).unwrap();

    let host = Arc::new(CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        deny_permission_sink(),
        data_root.clone(),
        process_write_lock(),
        None,
    ));
    let code: Arc<dyn Tool> = Arc::new(CodeTool::new(vec![Arc::new(ReadTool::default())], host));
    let tools = vec![Arc::clone(&code)];

    let script = r#"
let require_state = "threw";
try {
  require("fs");
  require_state = "loaded";
} catch (error) {
  require_state = String(error && error.message ? error.message : error);
}
return {
  process: typeof process,
  require_state,
};
"#;
    let call = FunctionToolCall {
        arguments: json!({ "code": script }).to_string(),
        call_id: "code-permission-1".to_string(),
        namespace: None,
        name: "run_code".to_string(),
        id: None,
        status: None,
    };
    let result = run_tool(
        &call,
        &tools,
        &permission,
        &ctx,
        "code-runtime-session",
        "default",
        deny_permission_sink().as_ref(),
        tokio_util::sync::CancellationToken::new(),
        &data_root,
        32_768,
        None,
        process_write_lock(),
        None,
    )
    .await;
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["process"], "undefined");
    let require_state = value["require_state"].as_str().unwrap_or("");
    assert_ne!(require_state, "loaded", "{require_state}");
    assert!(
        require_state.contains("require") || require_state.contains("defined"),
        "{require_state}"
    );
}
