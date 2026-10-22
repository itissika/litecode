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
        data: "path/lines/message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "空文件",
        status: "ok",
        data: "lines=[]",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "显式窗口未读完",
        status: "ok",
        data: "lines+message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "read",
        scenario: "单行超长",
        status: "ok",
        data: "lines string truncated",
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
        page: "—",
        diagnostics: "[]",
        error_code: "message=in_context",
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
        data: "paths/message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "glob",
        scenario: "超 limit",
        status: "ok",
        data: "paths+message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "grep",
        scenario: "有命中",
        status: "ok",
        data: "hits/message",
        page: "—",
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
        page: "—",
        diagnostics: "[]",
        error_code: "message=path_excluded",
    },
    Row {
        tool: "grep",
        scenario: "零命中",
        status: "ok",
        data: "hits=[]",
        page: "—",
        diagnostics: "[]",
        error_code: "message=no_match",
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
        data: "hits",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "全部成功",
        status: "ok",
        data: "ok/path/message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "apply_edits",
        scenario: "10 项 1 失败",
        status: "ok",
        data: "results",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "未找到",
        status: "ok",
        data: "ok=false/message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "多匹配",
        status: "ok",
        data: "ok=false/message",
        page: "—",
        diagnostics: "[]",
        error_code: "null",
    },
    Row {
        tool: "edit",
        scenario: "LSP 错误",
        status: "ok",
        data: "ok/path/message",
        page: "—",
        diagnostics: "[]",
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
        Arc::new(crate::tools::apply_edits::ApplyEditsTool::new()),
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
            deadline: None,
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
        "message=no_match" => {
            assert!(result.error_code.is_none(), "{scene}");
            let msg = result.data.as_ref().unwrap()["message"].as_str().unwrap_or("");
            assert!(msg.contains("No matches") || msg.contains("No matching"), "{scene}: {msg}");
            assert!(
                result.data.as_ref().unwrap()["hits"]
                    .as_array()
                    .is_some_and(|hits| hits.is_empty()),
                "{scene}"
            );
        }
        "message=path_excluded" => {
            assert!(result.error_code.is_none(), "{scene}");
            let msg = result.data.as_ref().unwrap()["message"]
                .as_str()
                .unwrap_or("")
                .to_lowercase();
            assert!(msg.contains("hidden") || msg.contains("filter"), "{scene}: {msg}");
        }
        "message=in_context" => {
            assert!(result.error_code.is_none(), "{scene}");
            let msg = result.data.as_ref().unwrap()["message"].as_str().unwrap_or("");
            assert!(
                msg.to_lowercase().contains("context"),
                "{scene}: {msg}"
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
            // Retired WP-198 dual-exit; keep arm for compile if referenced.
            assert!(
                result.data.as_ref().is_some_and(|d| d.get("ok") == Some(&serde_json::json!(false)))
                    || result.error_code.as_deref() == Some("edit_failed"),
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
            call_code(root, "read", json!({"path": "window.txt", "limit": 1})).await
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
        ("edit", "全部成功") => {
            write_file(root, "a.txt", "alpha\n");
            call_code(
                root,
                "edit",
                json!({"path": "a.txt", "old_string": "alpha", "new_string": "beta"}),
            )
            .await
        }
        ("apply_edits", "10 项 1 失败") => {
            let body: String = (0..10).map(|index| format!("line{index}\n")).collect();
            write_file(root, "blocks.txt", &body);
            let mut edits = Vec::new();
            for index in 0..9 {
                edits.push(json!({
                    "path": "blocks.txt",
                    "old_string": format!("line{index}"),
                    "new_string": format!("done{index}")
                }));
            }
            edits.push(json!({
                "path": "blocks.txt",
                "old_string": "missing-block",
                "new_string": "nope"
            }));
            call_code(root, "apply_edits", json!({"edits": edits})).await
        }
        ("edit", "未找到") => {
            write_file(root, "a.txt", "keep\n");
            call_code(
                root,
                "edit",
                json!({"path": "a.txt", "old_string": "nope1", "new_string": "x"}),
            )
            .await
        }
        ("edit", "多匹配") => {
            write_file(root, "a.txt", "alpha\nx\nalpha\n");
            call_code(
                root,
                "edit",
                json!({"path": "a.txt", "old_string": "alpha", "new_string": "BETA"}),
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
                    "old_string": "alpha",
                    "new_string": "beta"
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
                r#"const page = await tools.read({ path: "note.txt" }); return page.lines.length;"#,
            )
            .await
        }
        ("run_code", "嵌套部分成功") => {
            write_file(root, "a.txt", "alpha\n");
            run_program(
                root,
                r#"
                const out = await tools.apply_edits({
                  edits: [
                    { path: "a.txt", old_string: "alpha", new_string: "beta" },
                    { path: "a.txt", old_string: "missing", new_string: "nope" }
                  ]
                });
                return out.results.filter(r => !r.ok).length;
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
            assert!(data["path"].is_string());
            assert!(!data["lines"].as_array().unwrap().is_empty());
            assert!(data["lines"][0].is_string());
            assert!(data.get("file").is_none());
            assert!(data.get("page").is_none());
        }
        ("read", "空文件") => {
            assert!(result.data.as_ref().unwrap()["lines"].as_array().unwrap().is_empty());
        }
        ("read", "显式窗口未读完") => {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["lines"].as_array().unwrap().len(), 1);
            assert!(
                data["message"].as_str().unwrap().contains("start_line="),
                "{}",
                data["message"]
            );
        }
        ("read", "单行超长") => {
            let line = result.data.as_ref().unwrap()["lines"][0].as_str().unwrap();
            assert!(line.ends_with("...") || line.chars().count() <= 1500);
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
            assert!(data.get("base").is_none());
            assert!(data.get("page").is_none());
            assert!(!data["paths"].as_array().unwrap().is_empty());
            if row.scenario == "超 limit" {
                assert!(
                    data["message"].as_str().unwrap().contains("Raise limit"),
                    "{}",
                    data["message"]
                );
            }
        }
        ("grep", "有命中") | ("grep", "literal 命中") => {
            let data = result.data.as_ref().unwrap();
            assert!(data.get("base").is_none());
            assert!(data.get("matches").is_none());
            assert!(!data["hits"].as_array().unwrap().is_empty());
            assert!(data["hits"][0]["line"].as_u64().unwrap() >= 1);
            assert!(data["hits"][0]["text"].is_string());
        }
        ("grep", "零命中") => {
            assert!(result.data.as_ref().unwrap()["hits"].as_array().unwrap().is_empty());
        }
        ("edit", "全部成功") | ("edit", "LSP 错误") => {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["ok"], true);
            assert!(data["message"].is_string());
            assert!(data.get("blocks").is_none());
            assert!(data.get("applied").is_none());
        }
        ("apply_edits", "10 项 1 失败") => {
            let data = result.data.as_ref().unwrap();
            let results = data["results"].as_array().unwrap();
            assert_eq!(results.len(), 10);
            assert_eq!(results.iter().filter(|r| r["ok"] == true).count(), 9);
            assert_eq!(results[9]["ok"], false);
            let disk = std::fs::read_to_string(root.join("blocks.txt")).unwrap();
            assert!(disk.contains("done0"));
            assert!(!disk.contains("nope"));
        }
        ("edit", "未找到") | ("edit", "多匹配") => {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["ok"], false);
            assert!(data["message"].as_str().unwrap().len() > 0);
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
    assert_eq!(MATRIX.len(), 29);
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
    let msg = probed
        .data
        .as_ref()
        .and_then(|data| data.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    // Late-NUL probe: ripgrep still opens the file; grep reports empty hits.
    assert_eq!(status_of(&probed), "ok");
    assert!(
        probed
            .data
            .as_ref()
            .and_then(|d| d.get("hits"))
            .and_then(Value::as_array)
            .is_some_and(|h| h.is_empty()),
        "data={:?}",
        probed.data
    );
    assert!(
        msg.contains("No matches") || msg.contains("File could not"),
        "message={msg} data={:?}",
        probed.data
    );
    assert!(
        probed
            .diagnostics
            .iter()
            .all(|item| item.code != "file_not_searched"),
        "{:?}",
        probed.diagnostics
    );
}
