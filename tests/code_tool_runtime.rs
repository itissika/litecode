//! Real JavaScript integration for the internal CodeTool vertical slice.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{PermissionEngine, deny_permission_sink};
use litecode::tool::Tool;
use litecode::tool::executor::run_tool;
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tool::write_lock::process_write_lock;
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
use litecode::tools::read::ReadTool;
use litecode::types::{FunctionToolCall, ToolCallResult, ToolErrorKind, ToolSignalLevel};
use serde_json::{Value, json};

fn run_code_result(content: &str) -> Value {
    let rest = content
        .split_once("result:\n")
        .unwrap_or_else(|| panic!("missing result section in {content}"))
        .1;
    serde_json::from_str(rest.trim())
        .unwrap_or_else(|_| Value::String(rest.trim().to_string()))
}

#[tokio::test(flavor = "current_thread")]
async fn code_tool_runs_real_bundle_with_data_only_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn needle_a() {}\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "fn needle_b() {}\n").unwrap();

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

    let bundle: Vec<Arc<dyn Tool>> = vec![
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(ReadTool::default()),
    ];
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

    let script = r#"
const matches = await tools.grep({ pattern: "needle", path: "src", limit: 1 });
if (!matches.matches) {
  throw new Error("grep failed: " + JSON.stringify(matches));
}
if (Object.prototype.hasOwnProperty.call(matches, "text")) {
  throw new Error("inner result must not expose text");
}
const file = await tools.read({
  path: matches.matches[0].path,
  start_line: 1,
  end_line: 1,
});
if (!file.file || !file.lines || file.lines.length !== 1) {
  throw new Error("read failed: " + JSON.stringify(file));
}
let badCode = "";
try {
  await tools.read({});
} catch (e) {
  badCode = e instanceof ToolError ? e.code : e.name;
}
let unsupportedCode = "";
try {
  await tools.read({ path: file.file.path }, { timeout_ms: 1 });
} catch (e) {
  unsupportedCode = e instanceof ToolError ? e.code : e.name;
}
return {
  path: file.file.path,
  line: file.lines[0].text,
  bad_code: badCode,
  unsupported_code: unsupportedCode,
  process_absent: typeof process === "undefined",
  require_absent: typeof require === "undefined",
  stats_mean: libs.stats.mean([1, 2, 3, 4]),
  csv: libs.csv.parse("a,b\n1,2\n"),
  diff: libs.text.diffLines("a\nb", "a\nc"),
  grouped: libs.data.countBy([{ kind: "a" }, { kind: "a" }, { kind: "b" }], "kind"),
};
"#;

    let call = FunctionToolCall {
        arguments: json!({ "code": script }).to_string(),
        call_id: "code-test-1".to_string(),
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
    assert!(result.data.is_none(), "{}", result.content);
    assert!(!result.content.contains("\nlogs:"), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["path"], "src/a.rs");
    assert_eq!(value["line"], "fn needle_a() {}");
    assert_eq!(value["bad_code"], "invalid_input");
    assert_eq!(value["unsupported_code"], "TypeError");
    assert_eq!(value["process_absent"], true);
    assert_eq!(value["require_absent"], true);
    assert_eq!(value["stats_mean"], 2.5);
    assert_eq!(value["csv"], json!([["a", "b"], ["1", "2"]]));
    assert_eq!(
        value["diff"],
        json!([
            { "kind": "equal", "text": "a" },
            { "kind": "delete", "text": "b" },
            { "kind": "insert", "text": "c" }
        ])
    );
    assert_eq!(value["grouped"], json!({ "a": 2, "b": 1 }));
    assert!(
        result.content.contains("\"path\": \"src/a.rs\""),
        "{}",
        result.content
    );
}

#[derive(Clone)]
struct ProbeTool {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

impl Tool for ProbeTool {
    fn name(&self) -> &str {
        "probe"
    }

    fn surface(&self) -> litecode::tool::ToolSurface {
        litecode::tool::ToolSurface::Code
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer" }
            },
            "required": ["id"]
        })
    }

    fn data_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "id": { "type": "integer" }
            },
            "required": ["id"]
        }))
    }

    fn description(&self, _ctx: &Context) -> String {
        "Concurrency probe tool for the code runtime test.".to_string()
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(5))
    }

    fn is_concurrency_safe(&self, _input: &serde_json::Value) -> bool {
        true
    }

    fn execute(
        &self,
        input: serde_json::Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let id = input["id"].as_u64().unwrap_or(0);
        let active = Arc::clone(&self.active);
        let max_active = Arc::clone(&self.max_active);
        let cancel = execution.cancel.clone();
        Box::pin(async move {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(current, Ordering::SeqCst);
            let outcome = tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    ToolCallResult::ok("probe").with_data(json!({ "id": id }))
                }
                _ = cancel.cancelled() => ToolCallResult::error_with_kind(
                    ToolErrorKind::Cancelled,
                    "probe cancelled",
                    false,
                ),
            };
            active.fetch_sub(1, Ordering::SeqCst);
            outcome
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn code_tool_parallelizes_safe_calls_and_rejects_a_second_argument() {
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

    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let bundle: Vec<Arc<dyn Tool>> = vec![Arc::new(ProbeTool {
        active: Arc::clone(&active),
        max_active: Arc::clone(&max_active),
    })];
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

    let parallel_script = r#"
const results = await Promise.all([
  tools.probe({ id: 1 }),
  tools.probe({ id: 2 }),
  tools.probe({ id: 3 }),
]);
if (results.some((result) => !result || result.id == null)) {
  throw new Error("parallel probe failed: " + JSON.stringify(results));
}
return results.map((result) => result.id);
"#;
    let call = FunctionToolCall {
        arguments: json!({ "code": parallel_script }).to_string(),
        call_id: "code-parallel-1".to_string(),
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
    assert!(
        max_active.load(Ordering::SeqCst) >= 2,
        "Promise.all should overlap safe calls, max_active={}",
        max_active.load(Ordering::SeqCst)
    );

    let timeout_script = r#"
try {
  await tools.probe({ id: 9 }, { timeout_ms: 1 });
  return "resolved";
} catch (e) {
  return e instanceof ToolError ? e.code : e.name;
}
"#;
    let call = FunctionToolCall {
        arguments: json!({ "code": timeout_script, "timeout_ms": 5_000 }).to_string(),
        call_id: "code-timeout-1".to_string(),
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
    assert!(result.data.is_none(), "{}", result.content);
    assert!(
        result.content.contains("result:\nTypeError"),
        "{}",
        result.content
    );

    let runaway_script = "while (true) {}";
    let call = FunctionToolCall {
        arguments: json!({ "code": runaway_script, "timeout_ms": 500 }).to_string(),
        call_id: "code-run-timeout-1".to_string(),
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
        result.error.as_ref().map(|error| error.kind),
        Some(ToolErrorKind::Timeout),
        "{}",
        result.content
    );
    assert!(result.content.contains("run_code timed out after 500ms"));
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
    assert!(result.data.is_none(), "{}", result.content);
    assert!(result.content.contains("[log] hello 42"), "{}", result.content);
    assert!(result.content.contains("[error] warn"), "{}", result.content);
    assert_eq!(run_code_result(&result.content), json!({ "ok": true }));
    assert!(result.content.contains("hello 42"), "{}", result.content);

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
    assert!(result.content.contains("exceeded"), "{}", result.content);
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
    assert!(result.data.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["process"], "undefined");
    let require_state = value["require_state"].as_str().unwrap_or("");
    assert_ne!(require_state, "loaded", "{require_state}");
    assert!(
        require_state.contains("require") || require_state.contains("defined"),
        "{require_state}"
    );
}
