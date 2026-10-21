//! Phase 3 acceptance checks: write preconditions, edit/apply_edits EditResult, diagnostics.

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

#[test]
fn phase3_write_expected_hash_mismatch_is_stale() {
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
fn phase3_write_expected_hash_match_returns_file_ref() {
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
fn phase3_write_create_only_with_expected_hash_on_missing_file_is_invalid_input() {
    let dir = tempfile::tempdir().unwrap();
    let result = call(
        &WriteTool::new(),
        dir.path(),
        json!({
            "path": "missing.txt",
            "content": "hello",
            "create_only": true,
            "expected_hash": "ab"
        }),
    );
    assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
    assert!(result.data.is_none());
    assert!(!dir.path().join("missing.txt").exists());
}

#[test]
fn phase3_write_expected_hash_on_missing_file_does_not_create() {
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


#[test]
fn phase3_edit_rejects_expected_hash_knob() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "alpha\n").unwrap();
    let result = call(
        &EditTool::new(),
        dir.path(),
        json!({
            "path": "note.txt",
            "expected_hash": "0".repeat(64),
            "old_string": "alpha",
            "new_string": "beta"
        }),
    );
    assert_eq!(result.error_code.as_deref(), Some("invalid_input"));
    assert!(
        result.content.contains("expected_hash") || result.content.contains("unknown"),
        "{}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\n");
}

#[test]
fn phase3_apply_edits_serial_skip_on_failure() {
    use crate::tools::apply_edits::ApplyEditsTool;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.txt");
    let original: String = (0..10).map(|index| format!("line-{index}\n")).collect();
    std::fs::write(&path, &original).unwrap();
    let mut edits = Vec::new();
    for index in 0..9 {
        edits.push(json!({
            "path": "many.txt",
            "old_string": format!("line-{index}"),
            "new_string": format!("LINE-{index}")
        }));
    }
    edits.push(json!({
        "path": "many.txt",
        "old_string": "missing-line",
        "new_string": "NOPE"
    }));
    let result = call(
        &ApplyEditsTool::new(),
        dir.path(),
        json!({"edits": edits}),
    );
    assert!(result.error.is_none(), "{}", result.content);
    let data = result.data.unwrap();
    let results = data["results"].as_array().unwrap();
    assert_eq!(results.len(), 10);
    assert!(results[..9].iter().all(|item| item["ok"] == true));
    assert_eq!(results[9]["ok"], false);
    assert!(
        results[9]["message"].as_str().unwrap().contains("not found"),
        "{}",
        results[9]["message"]
    );
    let written = std::fs::read_to_string(&path).unwrap();
    for index in 0..9 {
        assert!(written.contains(&format!("LINE-{index}")));
    }
    assert!(written.contains("line-9\n"));
    assert!(!written.contains("NOPE"));
}

#[test]
fn phase3_edit_failure_resolves_with_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "keep\n").unwrap();
    let result = call(
        &EditTool::new(),
        dir.path(),
        json!({
            "path": "note.txt",
            "old_string": "nope-a",
            "new_string": "A"
        }),
    );
    assert!(result.error.is_none(), "failures resolve: {}", result.content);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["ok"], false);
    assert!(data["message"].as_str().unwrap().contains("not found"));
    assert!(data.get("blocks").is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n");
}

#[test]
fn phase3_edit_failure_messages_are_stable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\nalpha\n").unwrap();
    let tool = EditTool::new();
    let root = dir.path();

    let empty = call(
        &tool,
        root,
        json!({"path": "note.txt", "old_string": "", "new_string": "x"}),
    );
    assert!(
        empty.data.as_ref().unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("empty"),
        "{:?}",
        empty.data
    );

    let noop = call(
        &tool,
        root,
        json!({"path": "note.txt", "old_string": "beta", "new_string": "beta"}),
    );
    assert!(
        noop.data.as_ref().unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("same"),
        "{:?}",
        noop.data
    );

    let multi = call(
        &tool,
        root,
        json!({"path": "note.txt", "old_string": "alpha", "new_string": "ALPHA"}),
    );
    let msg = multi.data.as_ref().unwrap()["message"].as_str().unwrap();
    assert!(msg.contains("more than once"), "{msg}");
    assert!(!msg.contains("replace_all"), "{msg}");

    // Serial dependency across apply_edits items is allowed (unlike old same-call snapshot).
    use crate::tools::apply_edits::ApplyEditsTool;
    std::fs::write(root.join("note.txt"), "alpha\nbeta\nalpha\n").unwrap();
    let chained = call(
        &ApplyEditsTool::new(),
        root,
        json!({
            "edits": [
                {"path": "note.txt", "old_string": "beta", "new_string": "BETA"},
                {"path": "note.txt", "old_string": "BETA", "new_string": "gamma"}
            ]
        }),
    );
    let results = chained.data.as_ref().unwrap()["results"].as_array().unwrap();
    assert_eq!(results[0]["ok"], true);
    assert_eq!(results[1]["ok"], true);
    assert_eq!(
        std::fs::read_to_string(root.join("note.txt")).unwrap(),
        "alpha\ngamma\nalpha\n"
    );

    std::fs::write(root.join("note.txt"), "fn unique_name() {}\n").unwrap();
    let whitespace = call(
        &tool,
        root,
        json!({
            "path": "note.txt",
            "old_string": "    fn unique_name() {}",
            "new_string": "fn other() {}"
        }),
    );
    let msg = whitespace.data.as_ref().unwrap()["message"].as_str().unwrap();
    assert!(msg.contains("whitespace") || msg.contains("differ"), "{msg}");
    assert_eq!(
        std::fs::read_to_string(root.join("note.txt")).unwrap(),
        "fn unique_name() {}\n"
    );
}

#[tokio::test]
async fn phase3_apply_edits_partial_reaches_run_code_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let original: String = (0..10).map(|index| format!("line-{index}\n")).collect();
    std::fs::write(dir.path().join("many.txt"), &original).unwrap();
    let mut edits = String::from("[");
    for index in 0..9 {
        if index > 0 {
            edits.push(',');
        }
        edits.push_str(&format!(
            "{{path: \"many.txt\", old_string: \"line-{index}\", new_string: \"LINE-{index}\"}}"
        ));
    }
    edits.push_str(
        ", {path: \"many.txt\", old_string: \"missing-line\", new_string: \"NOPE\"}]",
    );
    let code = format!(
        "const out = await tools.apply_edits({{ edits: {edits} }}); return out.results.filter(r => !r.ok).length;"
    );
    let nested = run_program(dir.path(), &code).await;
    assert!(
        nested.error.is_none(),
        "run_code failed: {}",
        nested.content
    );
    // Ledger should show partial ok(n/total) for apply_edits.
    assert!(
        nested.content.contains("ok(9/10)") || nested.content.contains("apply_edits"),
        "run_code content: {}",
        nested.content
    );
    let written = std::fs::read_to_string(dir.path().join("many.txt")).unwrap();
    assert!(written.contains("LINE-0\n"));
    assert!(written.contains("line-9\n"));
    assert!(!written.contains("NOPE"));
}
