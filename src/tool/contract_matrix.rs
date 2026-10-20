//! Contract matrix. Each row asserts a live tool result.
//!
//! `file_not_searched` is not a §7.2 row. The live test probes it and records
//! whether that branch is reachable.

struct Row {
    tool: &'static str,
    scenario: &'static str,
    status: &'static str,
    data: &'static str,
    page: &'static str,
    diagnostics: &'static str,
    error_code: &'static str,
}

const MATRIX: &[Row] = &[
    Row {
        tool: "read",
        scenario: "正常文本",
        status: "ok",
        data: "file.hash/lines",
        page: "has_more",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "空文件",
        status: "ok",
        data: "lines=[]",
        page: "完成",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "显式窗口未读完",
        status: "ok",
        data: "lines",
        page: "has_more=true,next",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "单行超长",
        status: "ok",
        data: "lines[].line_truncated=true",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "start_line 非法",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "invalid_input",
    },
    Row {
        tool: "read",
        scenario: "未知参数",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "invalid_input",
    },
    Row {
        tool: "read",
        scenario: "上下文窗口",
        status: "ok",
        data: "lines=[]",
        page: "完成",
        diagnostics: "[]",
        error_code: "reason=in_context_window",
    },
    Row {
        tool: "read",
        scenario: "文件不存在",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "not_found",
    },
    Row {
        tool: "read",
        scenario: "图片/二进制",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "unsupported",
    },
    Row {
        tool: "glob",
        scenario: "正常",
        status: "ok",
        data: "base/paths",
        page: "next",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "glob",
        scenario: "超 limit",
        status: "ok",
        data: "paths",
        page: "has_more=true",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "grep",
        scenario: "有命中",
        status: "ok",
        data: "base/matches",
        page: "next",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "glob",
        scenario: "父目录 pattern",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "invalid_input",
    },
    Row {
        tool: "glob",
        scenario: "路径被排除",
        status: "ok",
        data: "paths=[]",
        page: "完成",
        diagnostics: "[]",
        error_code: "reason=path_excluded",
    },
    Row {
        tool: "grep",
        scenario: "零命中",
        status: "ok",
        data: "matches=[]",
        page: "完成",
        diagnostics: "[]",
        error_code: "reason=no_match",
    },
    Row {
        tool: "grep",
        scenario: "空白 glob",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "invalid_input",
    },
    Row {
        tool: "grep",
        scenario: "非法 regex",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "invalid_regex",
    },
    Row {
        tool: "grep",
        scenario: "literal 命中",
        status: "ok",
        data: "matches",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "grep",
        scenario: "cursor 过期",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "stale_cursor",
    },
    Row {
        tool: "edit",
        scenario: "全部成功",
        status: "ok",
        data: "file/blocks/applied",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "10 块 1 失败",
        status: "ok",
        data: "file/blocks/applied/failed",
        page: "—",
        diagnostics: "edit_block_failed",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "全部失败",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "edit_block_failed",
        error_code: "edit_failed + blocks",
    },
    Row {
        tool: "edit",
        scenario: "hash 过期",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "stale",
    },
    Row {
        tool: "edit",
        scenario: "LSP 错误",
        status: "ok",
        data: "file/blocks",
        page: "—",
        diagnostics: "lsp_error",
        error_code: "null",
    },
    Row {
        tool: "write",
        scenario: "成功",
        status: "ok",
        data: "file/created",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "write",
        scenario: "hash 过期",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "stale",
    },
    Row {
        tool: "write",
        scenario: "create_only 冲突",
        status: "error",
        data: "null",
        page: "—",
        diagnostics: "[]",
        error_code: "conflict",
    },
    Row {
        tool: "run_code",
        scenario: "正常",
        status: "ok",
        data: "calls/logs/result",
        page: "—",
        diagnostics: "聚合",
        error_code: "null",
    },
    Row {
        tool: "run_code",
        scenario: "嵌套部分成功",
        status: "ok",
        data: "calls[].outcome=partial",
        page: "—",
        diagnostics: "—",
        error_code: "—",
    },
    Row {
        tool: "run_code",
        scenario: "嵌套取消",
        status: "ok/error",
        data: "calls[].outcome=cancelled/unsettled",
        page: "—",
        diagnostics: "—",
        error_code: "—",
    },
];

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::{AgentProfile, AgentToolBinding, GlobalSettings};
use crate::config::ConfigManager;
use crate::context_pipeline::Context;
use crate::permission::{
    AskOutcome, AskPrompt, AskReply, PermissionAction, PermissionEngine, PermissionSink, ToolPolicy,
};
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tools::code::{CodeTool, CodeToolHost};
use crate::tools::lsp_feedback::arm_lsp_error_hook;
use crate::types::{FunctionToolCall, ToolCallResult, ToolSignalLevel};

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
        Arc::new(crate::tools::glob::GlobTool),
        Arc::new(crate::tools::grep::GrepTool),
        Arc::new(crate::tools::edit::EditTool::new()),
        Arc::new(crate::tools::write::WriteTool::new()),
    ]
}

async fn call_code(root: &Path, name: &str, args: Value) -> ToolCallResult {
    call_in_session(root, name, args, "p5-session", None).await
}

async fn call_in_session(
    root: &Path,
    name: &str,
    args: Value,
    session_id: &str,
    session: Option<crate::session::SessionDataReader>,
) -> ToolCallResult {
    let permission = allow_engine(root);
    let call = FunctionToolCall {
        arguments: args.to_string(),
        call_id: format!("p5-{name}"),
        name: name.into(),
        namespace: None,
        id: None,
        status: None,
    };
    execute_tool_call(
        &call,
        &tools(),
        &permission,
        &context(root),
        session_id,
        "default",
        crate::permission::deny_permission_sink().as_ref(),
        CancellationToken::new(),
        &root.join(".litecode"),
        usize::MAX,
        None,
        Arc::new(WorkspaceWriteLock::new()),
        session,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "p5".into(),
            },
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await
}

fn seed_session(root: &Path, text: &str) -> String {
    let db = root.join(".litecode").join("sessions.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let lease = crate::session::WorkspaceWriteLease::acquire(db.parent().unwrap()).unwrap();
    let data = crate::session::SessionData::open(&lease, &db).unwrap();
    let id = data
        .create_session(root.to_str().unwrap(), "default", None)
        .unwrap();
    data.insert_items(&id, &[crate::types::user_text(text)])
        .unwrap();
    id
}

async fn run_program(root: &Path, code: &str) -> ToolCallResult {
    let host = Arc::new(CodeToolHost::new(
        allow_engine(root),
        context(root),
        "default",
        crate::permission::deny_permission_sink(),
        root.join(".litecode"),
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ));
    let tool = CodeTool::new(tools(), host);
    let result = tool
        .execute(
            json!({ "code": code }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: "p5-run".into(),
                cancel: CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: "p5-session".into(),
                session: None,
            },
        )
        .await;
    assert_run_schema(&tool, &result);
    result
}

fn assert_run_schema(_tool: &CodeTool, result: &ToolCallResult) {
    if result.error.is_some() {
        return;
    }
    assert!(result.data.is_none(), "run_code success has no data");
    assert!(
        !result.content.contains("@@run_code_data"),
        "{}",
        result.content
    );
}

fn status_of(result: &ToolCallResult) -> &'static str {
    if result.error.is_some() || result.level == ToolSignalLevel::Error {
        "error"
    } else {
        "ok"
    }
}

fn data_page<'a>(result: &'a ToolCallResult) -> Option<&'a Value> {
    result.data.as_ref().and_then(|data| data.get("page"))
}

fn assert_cells(result: &ToolCallResult, row: &Row) {
    let scene = format!("{}/{}", row.tool, row.scenario);
    assert!(!row.data.is_empty(), "{scene}");
    let actual = status_of(result);
    assert!(
        row.status.split('/').any(|part| part == actual),
        "{scene}: status {actual}, content {}, data {:?}",
        result.content,
        result.data
    );
    match row.diagnostics {
        "[]" => assert!(
            result.diagnostics.is_empty(),
            "{scene}: {:?}",
            result.diagnostics
        ),
        "—" | "聚合" => {}
        code => assert!(
            result.diagnostics.iter().any(|item| item.code == code),
            "{scene}: {:?}",
            result.diagnostics
        ),
    }
    match row.error_code {
        "null" => assert!(result.error_code.is_none(), "{scene}: {:?}", result.error_code),
        "—" => {}
        "reason=no_match" => {
            assert!(result.error_code.is_none(), "{scene}");
            assert_eq!(result.data.as_ref().unwrap()["reason"], "no_match", "{scene}");
        }
        "reason=path_excluded" => {
            assert!(result.error_code.is_none(), "{scene}");
            assert_eq!(
                result.data.as_ref().unwrap()["reason"],
                "path_excluded",
                "{scene}"
            );
        }
        "reason=in_context_window" => {
            assert!(result.error_code.is_none(), "{scene}");
            assert_eq!(
                result.data.as_ref().unwrap()["reason"],
                "in_context_window",
                "{scene}"
            );
            assert!(
                result.data.as_ref().unwrap()["lines"]
                    .as_array()
                    .is_some_and(|lines| lines.is_empty()),
                "{scene}"
            );
        }
        "invalid_regex" => {
            assert_eq!(result.error_code.as_deref(), Some("invalid_regex"), "{scene}");
            let details = result.error_details.as_ref().unwrap();
            assert!(details["pattern"].is_string(), "{scene}");
            assert!(details["regex_error"].is_string(), "{scene}");
            assert!(details.get("position").is_some(), "{scene}");
            assert!(details.get("suggested_pattern").is_none(), "{scene}");
            assert!(details.get("suggested_literal").is_none(), "{scene}");
        }
        "edit_failed + blocks" => {
            assert_eq!(result.error_code.as_deref(), Some("edit_failed"), "{scene}");
            assert!(
                result.error_details.as_ref().unwrap()["blocks"].is_array(),
                "{scene}"
            );
        }
        code => assert_eq!(result.error_code.as_deref(), Some(code), "{scene}"),
    }
    match row.page {
        "—" => {}
        "完成" => {
            let page = data_page(result).expect(&scene);
            assert_eq!(page["has_more"], false, "{scene}");
            assert!(
                page.get("next").is_none_or(|next| next.is_null()),
                "{scene}"
            );
        }
        "has_more" => {
            assert!(data_page(result).is_some(), "{scene}");
        }
        "has_more=true,next" | "has_more=true" | "next" => {
            let page = data_page(result).expect(&scene);
            if row.page.contains("has_more=true") {
                assert_eq!(page["has_more"], true, "{scene}");
            }
            if row.page.contains("next") {
                assert!(
                    page["next"].as_str().is_some_and(|next| !next.is_empty()),
                    "{scene}"
                );
            }
        }
        other => panic!("{scene}: unhandled page cell {other}"),
    }
    if row.tool == "run_code" {
        assert!(result.data.is_none(), "{scene}: run_code carries no data");
        if actual != "error" {
            assert!(!result.content.is_empty(), "{scene}: empty model view");
        }
    } else if actual != "error" {
        assert!(result.data.is_some(), "{scene}: success without data");
        assert!(result.content.is_empty(), "{scene}: content carries facts");
    } else {
        assert!(result.data.is_none(), "{scene}: error carried data");
    }
}

fn write_file(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

async fn live_row(row: &Row) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let result = match (row.tool, row.scenario) {
        ("read", "正常文本") => {
            write_file(root, "note.txt", "alpha\nbeta\n");
            call_code(root, "read", json!({"path": "note.txt"})).await
        }
        ("read", "空文件") => {
            write_file(root, "empty.txt", "");
            call_code(root, "read", json!({"path": "empty.txt"})).await
        }
        ("read", "显式窗口未读完") => {
            write_file(root, "window.txt", "one\ntwo\nthree\n");
            call_code(root, "read", json!({"path": "window.txt", "end_line": 1})).await
        }
        ("read", "单行超长") => {
            write_file(root, "wide.txt", &format!("{}\n", "x".repeat(1600)));
            call_code(root, "read", json!({"path": "wide.txt"})).await
        }
        ("read", "start_line 非法") => {
            write_file(root, "a.txt", "alpha\n");
            call_code(root, "read", json!({"path": "a.txt", "start_line": 0})).await
        }
        ("read", "未知参数") => {
            write_file(root, "a.txt", "SENTINEL_NOT_READ\n");
            call_code(
                root,
                "read",
                json!({"path": "a.txt", "file_path": "a.txt"}),
            )
            .await
        }
        ("read", "上下文窗口") => {
            let sid = seed_session(root, "WINDOW_NEEDLE hidden");
            let path = crate::session::transcript_file::virtual_path_for(&sid);
            let reader =
                crate::session::SessionDataReader::open(&root.join(".litecode").join("sessions.db"));
            call_in_session(root, "read", json!({"path": path}), &sid, Some(reader)).await
        }
        ("read", "文件不存在") => call_code(root, "read", json!({"path": "missing.txt"})).await,
        ("read", "图片/二进制") => {
            std::fs::write(
                root.join("shot.png"),
                [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            )
            .unwrap();
            call_code(root, "read", json!({"path": "shot.png"})).await
        }
        ("glob", "正常") => {
            write_file(root, "a.txt", "a");
            write_file(root, "nested/b.txt", "b");
            call_code(root, "glob", json!({"pattern": "**/*.txt", "limit": 1})).await
        }
        ("glob", "超 limit") => {
            write_file(root, "a.txt", "a");
            write_file(root, "b.txt", "b");
            call_code(root, "glob", json!({"pattern": "*.txt", "limit": 1})).await
        }
        ("glob", "父目录 pattern") => {
            call_code(root, "glob", json!({"pattern": "../*"})).await
        }
        ("glob", "路径被排除") => {
            write_file(root, "node_modules/pkg/index.js", "export {}\n");
            call_code(
                root,
                "glob",
                json!({"pattern": "**/*", "path": "node_modules"}),
            )
            .await
        }
        ("grep", "有命中") => {
            write_file(root, "a.txt", "needle one\nneedle two\n");
            call_code(root, "grep", json!({"pattern": "needle", "limit": 1})).await
        }
        ("grep", "零命中") => {
            write_file(root, "a.txt", "present\n");
            call_code(root, "grep", json!({"pattern": "absent_token"})).await
        }
        ("grep", "空白 glob") => {
            write_file(root, "a.rs", "needle\n");
            call_code(root, "grep", json!({"pattern": "needle", "glob": "   "})).await
        }
        ("grep", "非法 regex") => call_code(root, "grep", json!({"pattern": "("})).await,
        ("grep", "literal 命中") => {
            write_file(root, "a.txt", "a.b\naxb\n");
            call_code(
                root,
                "grep",
                json!({"pattern": "a.b", "literal": true}),
            )
            .await
        }
        ("grep", "cursor 过期") => {
            write_file(root, "a.txt", "needle\n");
            call_code(
                root,
                "grep",
                json!({"pattern": "needle", "cursor": "not-a-cursor"}),
            )
            .await
        }
        ("edit", "全部成功") => {
            write_file(root, "a.txt", "alpha\n");
            call_code(
                root,
                "edit",
                json!({"path": "a.txt", "edits": [{"old_string": "alpha", "new_string": "beta"}]}),
            )
            .await
        }
        ("edit", "10 块 1 失败") => {
            let body: String = (0..10).map(|index| format!("line{index}\n")).collect();
            write_file(root, "blocks.txt", &body);
            let mut edits = Vec::new();
            for index in 0..9 {
                edits.push(json!({
                    "old_string": format!("line{index}"),
                    "new_string": format!("done{index}")
                }));
            }
            edits.push(json!({"old_string": "missing-block", "new_string": "nope"}));
            call_code(root, "edit", json!({"path": "blocks.txt", "edits": edits})).await
        }
        ("edit", "全部失败") => {
            write_file(root, "a.txt", "keep\n");
            call_code(
                root,
                "edit",
                json!({
                    "path": "a.txt",
                    "edits": [
                        {"old_string": "nope1", "new_string": "x"},
                        {"old_string": "nope2", "new_string": "y"}
                    ]
                }),
            )
            .await
        }
        ("edit", "hash 过期") => {
            write_file(root, "a.txt", "alpha\n");
            call_code(
                root,
                "edit",
                json!({
                    "path": "a.txt",
                    "expected_hash": "0000000000000000000000000000000000000000000000000000000000000000",
                    "edits": [{"old_string": "alpha", "new_string": "beta"}]
                }),
            )
            .await
        }
        ("edit", "LSP 错误") => {
            write_file(root, "hooked.rs", "alpha\n");
            let _guard = arm_lsp_error_hook("hooked.rs", "error: unused value");
            call_code(
                root,
                "edit",
                json!({
                    "path": "hooked.rs",
                    "edits": [{"old_string": "alpha", "new_string": "beta"}]
                }),
            )
            .await
        }
        ("write", "成功") => {
            call_code(
                root,
                "write",
                json!({"path": "fresh.txt", "content": "hello\n", "create_only": true}),
            )
            .await
        }
        ("write", "hash 过期") => {
            write_file(root, "fresh.txt", "hello\n");
            call_code(
                root,
                "write",
                json!({
                    "path": "fresh.txt",
                    "content": "changed\n",
                    "expected_hash": "0000000000000000000000000000000000000000000000000000000000000000"
                }),
            )
            .await
        }
        ("write", "create_only 冲突") => {
            write_file(root, "fresh.txt", "already\n");
            call_code(
                root,
                "write",
                json!({"path": "fresh.txt", "content": "new\n", "create_only": true}),
            )
            .await
        }
        ("run_code", "正常") => {
            write_file(root, "note.txt", "alpha\n");
            run_program(
                root,
                r#"const page = await tools.read({ path: "note.txt" }); return page.file.hash;"#,
            )
            .await
        }
        ("run_code", "嵌套部分成功") => {
            write_file(root, "a.txt", "alpha\n");
            run_program(
                root,
                r#"
                const edited = await tools.edit({
                  path: "a.txt",
                  edits: [
                    { old_string: "alpha", new_string: "beta" },
                    { old_string: "missing", new_string: "nope" }
                  ]
                });
                return edited.failed;
                "#,
            )
            .await
        }
        ("run_code", "嵌套取消") => nested_cancel(root).await,
        _ => panic!("no live assertion for {}/{}", row.tool, row.scenario),
    };
    assert_cells(&result, row);
    match (row.tool, row.scenario) {
        ("read", "正常文本") => {
            let data = result.data.as_ref().unwrap();
            assert!(data["file"]["hash"].as_str().unwrap().len() == 64);
            assert!(!data["lines"].as_array().unwrap().is_empty());
        }
        ("read", "空文件") => {
            assert!(result.data.as_ref().unwrap()["lines"].as_array().unwrap().is_empty());
        }
        ("read", "显式窗口未读完") => {
            assert_eq!(result.data.as_ref().unwrap()["lines"].as_array().unwrap().len(), 1);
        }
        ("read", "单行超长") => {
            assert_eq!(
                result.data.as_ref().unwrap()["lines"][0]["line_truncated"],
                true
            );
        }
        ("read", "start_line 非法") => {
            assert!(result.content.contains("start_line"), "{}", result.content);
            assert!(result.data.is_none());
        }
        ("read", "未知参数") => {
            assert!(result.content.contains("file_path"), "{}", result.content);
            assert!(result.data.is_none());
            assert!(!result.content.contains("SENTINEL_NOT_READ"), "{}", result.content);
        }
        ("grep", "空白 glob") => {
            assert!(
                result.content.contains("parameter 'glob' must not be blank"),
                "{}",
                result.content
            );
        }
        ("glob", "父目录 pattern") => {
            assert!(
                result.content.contains("reaches outside the search directory"),
                "{}",
                result.content
            );
        }
        ("glob", "正常") | ("glob", "超 limit") => {
            let data = result.data.as_ref().unwrap();
            assert!(data["base"].is_string());
            assert!(!data["paths"].as_array().unwrap().is_empty());
        }
        ("grep", "有命中") | ("grep", "literal 命中") => {
            let data = result.data.as_ref().unwrap();
            assert!(data["base"].is_string());
            assert!(!data["matches"].as_array().unwrap().is_empty());
        }
        ("grep", "零命中") => {
            assert!(result.data.as_ref().unwrap()["matches"].as_array().unwrap().is_empty());
        }
        ("edit", "全部成功") | ("edit", "LSP 错误") => {
            let data = result.data.as_ref().unwrap();
            assert!(data["file"]["hash"].is_string());
            assert!(data["blocks"].is_array());
            assert!(data["applied"].as_u64().unwrap() >= 1);
        }
        ("edit", "10 块 1 失败") => {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["applied"], 9);
            assert_eq!(data["failed"], 1);
            assert!(data["blocks"].is_array());
            let failed = data["blocks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|block| block["status"] == "failed")
                .expect("failed block");
            assert!(failed["reason"]["message"].is_string());
            let disk = std::fs::read_to_string(root.join("blocks.txt")).unwrap();
            assert!(disk.contains("done0"));
            assert!(!disk.contains("nope"));
        }
        ("edit", "hash 过期") => {
            assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(), "alpha\n");
        }
        ("write", "成功") => {
            let data = result.data.as_ref().unwrap();
            assert!(data["file"]["hash"].is_string());
            assert_eq!(data["created"], true);
        }
        ("write", "hash 过期") => {
            assert_eq!(std::fs::read_to_string(root.join("fresh.txt")).unwrap(), "hello\n");
        }
        ("run_code", "正常") => {
            assert!(result.content.contains("1. read "), "{}", result.content);
            assert!(result.content.contains(" ok"), "{}", result.content);
            assert!(result.content.contains("result:"), "{}", result.content);
        }
        ("run_code", "嵌套部分成功") => {
            assert!(result.content.contains("ok("), "{}", result.content);
        }
        ("run_code", "嵌套取消") => {
            assert!(
                result.content.contains(" cancelled") || result.content.contains(" unsettled"),
                "{}",
                result.content
            );
            assert!(!root.join("gone.txt").exists());
        }
        _ => {}
    }
}

struct BlockingAsk {
    entered: Arc<AtomicBool>,
    release: Arc<(Mutex<Option<AskOutcome>>, Condvar)>,
}

impl BlockingAsk {
    fn new() -> Self {
        Self {
            entered: Arc::new(AtomicBool::new(false)),
            release: Arc::new((Mutex::new(None), Condvar::new())),
        }
    }
}

impl PermissionSink for BlockingAsk {
    fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        self.entered.store(true, Ordering::SeqCst);
        let (lock, cv) = &*self.release;
        let mut slot = lock.lock().unwrap();
        loop {
            if cancel.is_cancelled() {
                return AskReply::from_outcome(AskOutcome::Aborted);
            }
            let (next, _) = cv
                .wait_timeout(slot, Duration::from_millis(20))
                .unwrap_or_else(|error| error.into_inner());
            slot = next;
        }
    }
}

fn ask_engine(root: &Path) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    let binding = AgentToolBinding {
        enabled: true,
        policy: ToolPolicy::with_default(PermissionAction::Ask),
        path_mode: Default::default(),
        last_applied_preset: None,
        allowed_tools: None,
    };
    let mut tools = std::collections::HashMap::new();
    tools.insert("write".to_string(), binding);
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools,
            ..Default::default()
        },
    );
    let resolved = ConfigManager::resolve_without_catalog(global, WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

async fn nested_cancel(root: &Path) -> ToolCallResult {
    let ask = Arc::new(BlockingAsk::new());
    let host = Arc::new(CodeToolHost::new(
        ask_engine(root),
        context(root),
        "default",
        Arc::clone(&ask) as Arc<dyn PermissionSink>,
        root.join(".litecode"),
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ));
    let tool = CodeTool::new(tools(), host);
    let cancel = CancellationToken::new();
    let cancel_run = cancel.clone();
    let workspace = root.to_path_buf();
    let run = tokio::spawn(async move {
        tool.execute(
            json!({
                "code": r#"await tools.write({ path: "gone.txt", content: "later" }); return "done";"#
            }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: workspace,
                call_id: "p5-cancel".into(),
                cancel: cancel_run,
                output_limit: usize::MAX,
                session_id: "p5-cancel".into(),
                session: None,
            },
        )
        .await
    });
    let start = std::time::Instant::now();
    while !ask.entered.load(Ordering::SeqCst) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("approval was never entered");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(8), run)
        .await
        .expect("cancelled run did not return")
        .unwrap();
    if result.error.is_none() {
        assert!(result.data.is_none(), "cancelled run_code carries no data");
        assert!(!result.content.is_empty(), "{}", result.content);
    }
    result
}

async fn probe_file_not_searched(root: &Path) -> ToolCallResult {
    let mut body = vec![b'a'; 9000];
    body.push(0);
    body.extend(b"\nNEEDLE_AFTER_NUL\n");
    std::fs::write(root.join("late-nul.txt"), body).unwrap();
    call_code(
        root,
        "grep",
        json!({"pattern": "NEEDLE_AFTER_NUL", "path": "late-nul.txt"}),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phase5_contract_matrix_asserts_live_output() {
    assert_eq!(MATRIX.len(), 30);
    let mut seen = std::collections::BTreeSet::new();
    for row in MATRIX {
        assert!(
            seen.insert((row.tool, row.scenario)),
            "duplicate row {}/{}",
            row.tool,
            row.scenario
        );
        live_row(row).await;
    }

    let dir = tempfile::tempdir().unwrap();
    let probed = probe_file_not_searched(dir.path()).await;
    let reason = probed
        .data
        .as_ref()
        .and_then(|data| data.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("");
    // Unreachable from the late-NUL probe. `looks_binary` only scans the first
    // 8KiB, so a later NUL is not rejected up front. ripgrep still opens the
    // file (`scan.opened`), and grep then reports `no_match`. `file_not_searched`
    // requires a file-scoped candidate whose searcher never opens the file.
    assert_eq!(status_of(&probed), "ok");
    assert_eq!(reason, "no_match", "data={:?}", probed.data);
    assert!(
        probed
            .diagnostics
            .iter()
            .all(|item| item.code != "file_not_searched"),
        "{:?}",
        probed.diagnostics
    );
}
