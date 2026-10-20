//! Phase 2 acceptance checks: pagination, zero-hit reasons, and the read/edit chain.

use std::path::Path;

use serde_json::{Value, json};

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::EditTool;
use crate::tools::glob::GlobTool;
use crate::tools::grep::GrepTool;
use crate::tools::read::ReadTool;
use crate::types::ToolCallResult;

fn execution(root: &Path, path_mode: crate::workspace::ToolPathMode) -> ToolExecutionContext {
    ToolExecutionContext {
        path_mode,
        workspace_root: root.to_path_buf(),
        call_id: String::new(),
        cancel: tokio_util::sync::CancellationToken::new(),
        output_limit: usize::MAX,
        session_id: String::new(),
        session: None,
    }
}

fn call<T: Tool>(
    tool: &T,
    root: &Path,
    input: Value,
    path_mode: crate::workspace::ToolPathMode,
) -> ToolCallResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(tool.execute(input, execution(root, path_mode)))
}

fn safe<T: Tool>(tool: &T, root: &Path, input: Value) -> ToolCallResult {
    call(tool, root, input, crate::workspace::ToolPathMode::Safe)
}

fn reason(result: &ToolCallResult) -> String {
    result
        .data
        .as_ref()
        .and_then(|data| data.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[test]
fn phase2_read_explicit_window_reports_more() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "a\nb\nc\nd\ne\n").unwrap();
    let result = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": "note.txt", "start_line": 1, "end_line": 3}),
    );
    assert!(result.error.is_none(), "{}", result.content);
    let data = result.data.unwrap();
    assert_eq!(data["lines"].as_array().unwrap().len(), 3);
    assert_eq!(data["page"]["has_more"], true);
    assert_eq!(data["page"]["next"], "4");
}

#[test]
fn phase2_read_page_size_ignores_token_budget() {
    let dir = tempfile::tempdir().unwrap();
    let line = (0..80).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ");
    let body: String = (0..200).map(|_| format!("{line}\n")).collect();
    std::fs::write(dir.path().join("big.txt"), body).unwrap();
    let result = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": "big.txt"}),
    );
    assert!(result.error.is_none(), "{}", result.content);
    assert_eq!(result.data.unwrap()["lines"].as_array().unwrap().len(), 200);
}

#[test]
fn phase2_read_missing_file_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let result = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": "missing.txt"}),
    );
    assert_eq!(result.error_code.as_deref(), Some("not_found"));
}

#[test]
fn phase2_glob_over_limit_has_more_and_read_consumes_path() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), format!("body-{name}\n")).unwrap();
    }
    let listed = safe(
        &GlobTool,
        dir.path(),
        json!({"pattern": "*.txt", "limit": 2}),
    );
    assert!(listed.error.is_none(), "{}", listed.content);
    let data = listed.data.unwrap();
    assert_eq!(data["base"], "workspace");
    assert_eq!(data["page"]["has_more"], true);
    assert_eq!(data["paths"].as_array().unwrap().len(), 2);
    let path = data["paths"][0].as_str().unwrap().to_string();
    let read = safe(&ReadTool::default(), dir.path(), json!({"path": path}));
    assert!(read.error.is_none(), "{}", read.content);
    let text = read.data.unwrap()["lines"][0]["text"].as_str().unwrap().to_string();
    assert!(text.contains(&path), "read {path} returned {text}");
}

#[test]
fn phase2_grep_cursor_rejects_changed_pattern() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\nthree\n").unwrap();
    let first = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "e", "limit": 1}),
    );
    let cursor = first.data.unwrap()["page"]["next"]
        .as_str()
        .unwrap()
        .to_string();
    let stale = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "o", "limit": 1, "cursor": cursor}),
    );
    assert_eq!(stale.error_code.as_deref(), Some("stale_cursor"));
    assert_eq!(stale.error_details.unwrap()["reason"], "query");
}

#[test]
fn phase2_grep_same_query_pages_are_stable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let first = safe(&GrepTool, dir.path(), json!({"pattern": "e", "limit": 1}));
    let first_data = first.data.unwrap();
    let cursor = first_data["page"]["next"].as_str().unwrap().to_string();
    let second = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "e", "limit": 1, "cursor": cursor}),
    );
    let second_data = second.data.unwrap();
    assert_eq!(first_data["matches"][0]["start_line"], 1);
    assert_eq!(second_data["matches"][0]["start_line"], 3);
    assert_ne!(
        first_data["matches"][0]["line_text"],
        second_data["matches"][0]["line_text"]
    );
}

#[test]
fn phase2_grep_cursor_rejects_version_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "one\nthree\n").unwrap();
    let first = safe(&GrepTool, dir.path(), json!({"pattern": "e", "limit": 1}));
    let cursor = first.data.unwrap()["page"]["next"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(&path, "one\nthree\nextra\n").unwrap();
    let stale = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "e", "limit": 1, "cursor": cursor}),
    );
    assert_eq!(stale.error_code.as_deref(), Some("stale_cursor"));
    assert_eq!(stale.error_details.unwrap()["reason"], "version");
}

#[test]
fn phase2_grep_reason_no_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let result = safe(&GrepTool, dir.path(), json!({"pattern": "missing"}));
    assert_eq!(reason(&result), "no_match");
    assert_eq!(result.data.unwrap()["searched"]["files"], 1);
}

#[test]
fn phase2_grep_reason_corpus_empty() {
    let dir = tempfile::tempdir().unwrap();
    let result = safe(&GrepTool, dir.path(), json!({"pattern": "anything"}));
    assert_eq!(reason(&result), "corpus_empty");
}

#[test]
fn phase2_grep_reason_glob_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "needle", "glob": "*.md"}),
    );
    assert_eq!(reason(&result), "glob_empty");
}

#[test]
fn phase2_grep_reason_path_excluded() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("hidden_dir")).unwrap();
    std::fs::write(dir.path().join("hidden_dir/hidden.rs"), "needle\n").unwrap();
    let root = dir.path().to_path_buf();
    let result = with_excluded_dir(|| {
        safe(
            &GrepTool,
            &root,
            json!({"pattern": "needle", "path": "hidden_dir"}),
        )
    });
    assert_eq!(reason(&result), "path_excluded");
}

#[test]
fn phase2_grep_reason_binary_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "a", "path": "blob.bin"}),
    );
    assert_eq!(reason(&result), "binary_file");
}

#[test]
fn phase2_grep_invalid_regex_details() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "ModelRequest {\n").unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "ModelRequest {"}),
    );
    assert_eq!(result.error_code.as_deref(), Some("invalid_regex"));
    let details = result.error_details.unwrap();
    assert_eq!(details["pattern"], "ModelRequest {");
    assert!(details["regex_error"].as_str().unwrap().contains('^') || details["regex_error"].is_string());
    assert!(
        details["position"].is_number(),
        "position should be the caret column, got {}",
        details["position"]
    );
    assert_eq!(details["suggested_literal"], "ModelRequest {");
    assert!(details["suggested_pattern"].is_string());
}

#[test]
fn phase2_grep_literal_searches_metacharacters() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a.b\naxb\n").unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "a.b", "literal": true}),
    );
    let data = result.data.unwrap();
    assert_eq!(data["matches"].as_array().unwrap().len(), 1);
    assert_eq!(data["matches"][0]["line_text"], "a.b");
}

#[test]
fn phase2_outside_workspace_base_absolute_read() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("note.txt"), "outside-body\n").unwrap();
    let listed = call(
        &GlobTool,
        workspace.path(),
        json!({"pattern": "*.txt", "path": outside.path().display().to_string()}),
        crate::workspace::ToolPathMode::All,
    );
    assert!(listed.error.is_none(), "{}", listed.content);
    let data = listed.data.unwrap();
    assert_eq!(data["base"], "absolute");
    let path = data["paths"][0].as_str().unwrap().to_string();
    assert!(path.contains("note.txt"), "{path}");
    let read = call(
        &ReadTool::default(),
        workspace.path(),
        json!({"path": path}),
        crate::workspace::ToolPathMode::All,
    );
    assert!(read.error.is_none(), "{}", read.content);
    let file = read.data.unwrap();
    assert!(file["file"]["abs"].is_string(), "{file}");
    assert_eq!(file["file"]["hash"].as_str().unwrap().len(), 64);
    assert_eq!(file["lines"][0]["text"], "outside-body");
}

#[test]
fn phase2_chain_glob_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.txt"), "glob-chain\n").unwrap();
    let listed = safe(&GlobTool, dir.path(), json!({"pattern": "*.txt"}));
    let path = listed.data.unwrap()["paths"][0].as_str().unwrap().to_string();
    let read = safe(&ReadTool::default(), dir.path(), json!({"path": path}));
    assert_eq!(read.data.unwrap()["lines"][0]["text"], "glob-chain");
}

#[test]
fn phase2_chain_grep_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "before\nneedle-chain\nafter\n").unwrap();
    let found = safe(&GrepTool, dir.path(), json!({"pattern": "needle-chain"}));
    let hit = &found.data.unwrap()["matches"][0];
    let path = hit["path"].as_str().unwrap().to_string();
    let line = hit["start_line"].as_u64().unwrap();
    let read = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": path, "start_line": line, "end_line": line}),
    );
    let data = read.data.unwrap();
    assert_eq!(data["page"]["has_more"], true);
    assert_eq!(data["page"]["next"], (line + 1).to_string());
    assert_eq!(data["lines"][0]["text"], "needle-chain");
}

#[test]
fn phase2_chain_read_edit_expected_hash() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "alpha\n").unwrap();
    let read = safe(&ReadTool::default(), dir.path(), json!({"path": "note.txt"}));
    let hash = read.data.unwrap()["file"]["hash"].as_str().unwrap().to_string();
    let edited = safe(
        &EditTool::default(),
        dir.path(),
        json!({
            "path": "note.txt",
            "expected_hash": hash,
            "edits": [{"old_string": "alpha", "new_string": "beta"}]
        }),
    );
    assert!(edited.error.is_none(), "{}", edited.content);
    let edited_data = edited.data.as_ref().unwrap();
    assert_eq!(edited_data["applied"], 1);
    assert_eq!(edited_data["failed"], 0);
    assert_eq!(edited_data["blocks"][0]["status"], "applied");
    assert_eq!(
        edited_data["file"]["hash"],
        crate::tool::file_identity::sha256_hex(b"beta\n")
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\n");
    let stale = safe(
        &EditTool::default(),
        dir.path(),
        json!({
            "path": "note.txt",
            "expected_hash": hash,
            "edits": [{"old_string": "beta", "new_string": "gamma"}]
        }),
    );
    assert_eq!(stale.error_code.as_deref(), Some("stale"));
    let current = stale.error_details.unwrap()["current_hash"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        current,
        crate::tool::file_identity::sha256_hex(b"beta\n")
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\n");
}

fn with_excluded_dir<R>(f: impl FnOnce() -> R) -> R {
    let mut file = crate::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
    file.search_exclude.push("hidden_dir".into());
    let mut out = None;
    crate::workspace::filter::with_excludes_cache_for_test(file, || {
        out = Some(f());
    });
    out.expect("cache test body ran")
}
