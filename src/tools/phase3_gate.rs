//! Phase 3 acceptance checks: write preconditions, partial edit blocks, diagnostics.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::GlobalSettings;
use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, deny_permission_sink};
use crate::tool::outcome::OutcomeStatus;
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
        usize::MAX,
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

fn block_code(result: &ToolCallResult, index: usize) -> String {
    let blocks = result
        .data
        .as_ref()
        .and_then(|data| data.get("blocks"))
        .or_else(|| {
            result
                .error_details
                .as_ref()
                .and_then(|details| details.get("blocks"))
        })
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("blocks missing: {result:?}"));
    blocks
        .iter()
        .find(|block| block["index"] == index)
        .unwrap_or_else(|| panic!("block {index} missing: {blocks:?}"))["reason"]["code"]
        .as_str()
        .unwrap_or("")
        .to_string()
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
fn phase3_edit_expected_hash_mismatch_is_stale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "alpha\n").unwrap();
    let result = call(
        &EditTool::new(),
        dir.path(),
        json!({
            "path": "note.txt",
            "expected_hash": "0".repeat(64),
            "edits": [{"old_string": "alpha", "new_string": "beta"}]
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
fn phase3_edit_ten_blocks_one_failure_is_partial() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.txt");
    let original: String = (0..10).map(|index| format!("line-{index}\n")).collect();
    std::fs::write(&path, &original).unwrap();
    let mut edits = Vec::new();
    for index in 0..9 {
        edits.push(json!({
            "old_string": format!("line-{index}"),
            "new_string": format!("LINE-{index}")
        }));
    }
    edits.push(json!({
        "old_string": "missing-line",
        "new_string": "NOPE"
    }));
    let result = call(
        &EditTool::new(),
        dir.path(),
        json!({"path": "many.txt", "edits": edits}),
    );
    assert_eq!(result.outcome_status, Some(OutcomeStatus::Partial));
    assert!(result.error.is_none());
    let data = result.data.unwrap();
    assert_eq!(data["applied"], 9);
    assert_eq!(data["failed"], 1);
    assert_eq!(data["blocks"].as_array().unwrap().len(), 10);
    assert_eq!(data["blocks"][9]["status"], "failed");
    assert_eq!(data["blocks"][9]["reason"]["code"], "no_match");
    assert_eq!(data["blocks"][9]["index"], 10);
    assert!(data["blocks"][0]["ranges"][0]["start_line"].is_number());
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|item| item.code == "edit_block_failed")
        .expect("diagnostic");
    assert_eq!(diagnostic.details.as_ref().unwrap()["index"], 10);
    assert_eq!(
        diagnostic.details.as_ref().unwrap()["reason"]["code"],
        "no_match"
    );
    let written = std::fs::read_to_string(&path).unwrap();
    for index in 0..9 {
        assert!(written.contains(&format!("LINE-{index}")));
        assert!(!written.contains(&format!("line-{index}\n")));
    }
    assert!(written.contains("line-9\n"));
    assert!(!written.contains("NOPE"));
}

#[test]
fn phase3_edit_all_fail_keeps_file_and_exposes_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "keep\n").unwrap();
    let result = call(
        &EditTool::new(),
        dir.path(),
        json!({
            "path": "note.txt",
            "edits": [
                {"old_string": "nope-a", "new_string": "A"},
                {"old_string": "nope-b", "new_string": "B"}
            ]
        }),
    );
    assert_eq!(result.outcome_status, Some(OutcomeStatus::Error));
    assert_eq!(result.error_code.as_deref(), Some("edit_failed"));
    assert!(result.data.is_none());
    let blocks = result.error_details.as_ref().unwrap()["blocks"]
        .as_array()
        .unwrap();
    assert_eq!(blocks.len(), 2);
    assert!(blocks.iter().all(|block| block["reason"]["code"] == "no_match"));
    assert!(
        result
            .diagnostics
            .iter()
            .filter(|item| item.code == "edit_block_failed")
            .count()
            == 2
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n");
}

#[test]
fn phase3_edit_failure_codes_are_stable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "alpha\nbeta\nalpha\n").unwrap();
    let tool = EditTool::new();
    let root = dir.path();

    let empty = call(
        &tool,
        root,
        json!({"path": "note.txt", "edits": [{"old_string": "", "new_string": "x"}]}),
    );
    assert_eq!(block_code(&empty, 1), "empty_old_string");

    let noop = call(
        &tool,
        root,
        json!({"path": "note.txt", "edits": [{"old_string": "beta", "new_string": "beta"}]}),
    );
    assert_eq!(block_code(&noop, 1), "no_op");

    let multi = call(
        &tool,
        root,
        json!({"path": "note.txt", "edits": [{"old_string": "alpha", "new_string": "ALPHA"}]}),
    );
    assert_eq!(block_code(&multi, 1), "multiple_matches");

    let chained = call(
        &tool,
        root,
        json!({
            "path": "note.txt",
            "edits": [
                {"old_string": "beta", "new_string": "BETA"},
                {"old_string": "BETA", "new_string": "gamma"}
            ]
        }),
    );
    assert_eq!(chained.outcome_status, Some(OutcomeStatus::Partial));
    assert_eq!(block_code(&chained, 2), "chained");
    assert_eq!(std::fs::read_to_string(root.join("note.txt")).unwrap(), "alpha\nBETA\nalpha\n");

    std::fs::write(root.join("note.txt"), "alpha\nbeta\n").unwrap();
    let overlap = call(
        &tool,
        root,
        json!({
            "path": "note.txt",
            "edits": [
                {"old_string": "alpha\nbeta", "new_string": "AB"},
                {"old_string": "beta", "new_string": "BETA"}
            ]
        }),
    );
    assert_eq!(block_code(&overlap, 2), "overlap");
    assert!(!overlap.error_details.is_some() || overlap.data.is_some());
    let overlap_block = overlap.data.as_ref().unwrap()["blocks"][1].clone();
    assert!(!overlap_block["ranges"].as_array().unwrap().is_empty());

    std::fs::write(root.join("note.txt"), "fn unique_name() {}\n").unwrap();
    let whitespace = call(
        &tool,
        root,
        json!({
            "path": "note.txt",
            "edits": [{
                "old_string": "    fn unique_name() {}",
                "new_string": "fn other() {}"
            }]
        }),
    );
    assert_eq!(block_code(&whitespace, 1), "whitespace_only");
    let ranges = whitespace.error_details.as_ref().unwrap()["blocks"][0]["ranges"]
        .as_array()
        .unwrap();
    assert_eq!(ranges[0]["start_line"], 1);
    assert_eq!(std::fs::read_to_string(root.join("note.txt")).unwrap(), "fn unique_name() {}\n");
}

#[tokio::test]
async fn phase3_edit_block_failed_reaches_run_code_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let original: String = (0..10).map(|index| format!("line-{index}\n")).collect();
    std::fs::write(dir.path().join("many.txt"), &original).unwrap();
    let mut edits = String::from("[");
    for index in 0..9 {
        if index > 0 {
            edits.push(',');
        }
        edits.push_str(&format!(
            "{{old_string: \"line-{index}\", new_string: \"LINE-{index}\"}}"
        ));
    }
    edits.push_str(", {old_string: \"missing-line\", new_string: \"NOPE\"}]");
    let code = format!(
        "await tools.edit({{ path: \"many.txt\", edits: {edits} }}); return null;"
    );
    let nested = run_program(dir.path(), &code).await;
    assert!(nested.error.is_none(), "run_code failed: {}", nested.content);
    assert!(nested.data.is_none(), "run_code no longer returns data");
    assert!(
        nested.content.contains("edit_block_failed")
            && nested.content.contains("edit block 10 failed: no_match"),
        "run_code content: {}",
        nested.content
    );
    let written = std::fs::read_to_string(dir.path().join("many.txt")).unwrap();
    assert!(written.contains("LINE-0\n"));
    assert!(written.contains("line-9\n"));
    assert!(!written.contains("NOPE"));
}
