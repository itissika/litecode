//! Phase 2 acceptance checks: bounded reads/searches and the read/edit chain.

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

fn message(result: &ToolCallResult) -> String {
    result
        .data
        .as_ref()
        .and_then(|data| data.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[test]
fn phase2_read_limit_reports_more_in_message() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "a\nb\nc\nd\ne\n").unwrap();
    let result = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": "note.txt", "start_line": 1, "limit": 3}),
    );
    assert!(result.error.is_none(), "{}", result.content);
    let data = result.data.unwrap();
    assert_eq!(data["lines"].as_array().unwrap().len(), 3);
    assert_eq!(data["lines"][0], "a");
    assert_eq!(data["start_line"], 1);
    assert!(
        message_of(&data).contains("start_line=4"),
        "{}",
        message_of(&data)
    );
    assert!(data.get("page").is_none());
}

fn message_of(data: &Value) -> String {
    data.get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[test]
fn phase2_read_page_size_ignores_token_budget() {
    let dir = tempfile::tempdir().unwrap();
    let line = (0..80)
        .map(|i| format!("w{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let body: String = (0..200).map(|_| format!("{line}\n")).collect();
    std::fs::write(dir.path().join("big.txt"), body).unwrap();
    let result = safe(&ReadTool::default(), dir.path(), json!({"path": "big.txt"}));
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
fn phase2_glob_over_limit_notes_truncation_and_read_consumes_path() {
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
    assert!(data.get("base").is_none());
    assert!(data.get("page").is_none());
    assert_eq!(data["paths"].as_array().unwrap().len(), 2);
    assert!(
        message_of(&data).contains("Raise limit"),
        "{}",
        message_of(&data)
    );
    let path = data["paths"][0].as_str().unwrap().to_string();
    let read = safe(&ReadTool::default(), dir.path(), json!({"path": path}));
    assert!(read.error.is_none(), "{}", read.content);
    let text = read.data.unwrap()["lines"][0].as_str().unwrap().to_string();
    assert!(text.contains(&path), "read {path} returned {text}");
}

#[test]
fn phase2_grep_no_cursor_dialect() {
    let schema = GrepTool.schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(!props.contains_key("cursor"));
    assert!(!props.contains_key("include_ignored"));
    let data_schema = GrepTool.data_schema().unwrap();
    let dprops = data_schema["properties"].as_object().unwrap();
    assert!(dprops.contains_key("hits"));
    assert!(dprops.contains_key("message"));
    assert!(!dprops.contains_key("matches"));
    assert!(!dprops.contains_key("page"));
    assert!(!dprops.contains_key("searched"));
    assert!(!dprops.contains_key("reason"));
    assert!(!dprops.contains_key("base"));
}

#[test]
fn phase2_grep_limit_bounds_hits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let first = safe(&GrepTool, dir.path(), json!({"pattern": "e", "limit": 1}));
    let data = first.data.unwrap();
    assert_eq!(data["hits"].as_array().unwrap().len(), 1);
    assert_eq!(data["hits"][0]["line"], 1);
    assert!(
        message_of(&data).contains("Raise limit"),
        "{}",
        message_of(&data)
    );
    assert!(data.get("page").is_none());
}

#[test]
fn phase2_grep_message_no_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let result = safe(&GrepTool, dir.path(), json!({"pattern": "missing"}));
    assert!(message(&result).contains("No matches"), "{}", message(&result));
    assert!(result.data.unwrap()["hits"].as_array().unwrap().is_empty());
}

#[test]
fn phase2_grep_message_corpus_empty() {
    let dir = tempfile::tempdir().unwrap();
    let result = safe(&GrepTool, dir.path(), json!({"pattern": "anything"}));
    assert!(message(&result).contains("No files"), "{}", message(&result));
}

#[test]
fn phase2_grep_message_glob_empty() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "needle\n").unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "needle", "glob": "*.md"}),
    );
    assert!(
        message(&result).to_lowercase().contains("glob"),
        "{}",
        message(&result)
    );
}

#[test]
fn phase2_grep_message_path_excluded() {
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
    let msg = message(&result).to_lowercase();
    assert!(msg.contains("hidden") || msg.contains("filter"), "{msg}");
}

#[test]
fn phase2_grep_message_binary_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    let result = safe(
        &GrepTool,
        dir.path(),
        json!({"pattern": "a", "path": "blob.bin"}),
    );
    assert!(
        message(&result).to_lowercase().contains("binary"),
        "{}",
        message(&result)
    );
}

#[test]
fn phase2_grep_invalid_regex_details() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "ModelRequest {\n").unwrap();
    let result = safe(&GrepTool, dir.path(), json!({"pattern": "ModelRequest {"}));
    assert_eq!(result.error_code.as_deref(), Some("invalid_regex"));
    let details = result.error_details.unwrap();
    assert_eq!(details["pattern"], "ModelRequest {");
    assert!(details["regex_error"].is_string());
    assert!(details["position"].is_number());
    assert!(details.get("suggested_literal").is_none());
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
    assert_eq!(data["hits"].as_array().unwrap().len(), 1);
    assert_eq!(data["hits"][0]["text"], "a.b");
}

#[test]
fn phase2_outside_workspace_paths_noted() {
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
    assert!(data.get("base").is_none());
    let path = data["paths"][0].as_str().unwrap().to_string();
    assert!(path.contains("note.txt"), "{path}");
    assert!(
        message_of(&data).to_lowercase().contains("absolute") || path.starts_with('/'),
        "{} / {path}",
        message_of(&data)
    );
    let read = call(
        &ReadTool::default(),
        workspace.path(),
        json!({"path": path}),
        crate::workspace::ToolPathMode::All,
    );
    assert!(read.error.is_none(), "{}", read.content);
    let file = read.data.unwrap();
    assert_eq!(file["lines"][0], "outside-body");
    assert!(file.get("file").is_none());
}

#[test]
fn phase2_chain_glob_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.txt"), "glob-chain\n").unwrap();
    let listed = safe(&GlobTool, dir.path(), json!({"pattern": "*.txt"}));
    let path = listed.data.unwrap()["paths"][0]
        .as_str()
        .unwrap()
        .to_string();
    let read = safe(&ReadTool::default(), dir.path(), json!({"path": path}));
    assert_eq!(read.data.unwrap()["lines"][0], "glob-chain");
}

#[test]
fn phase2_chain_grep_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "before\nneedle-chain\nafter\n").unwrap();
    let found = safe(&GrepTool, dir.path(), json!({"pattern": "needle-chain"}));
    let hit = &found.data.unwrap()["hits"][0];
    let path = hit["path"].as_str().unwrap().to_string();
    let line = hit["line"].as_u64().unwrap();
    let read = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": path, "start_line": line, "limit": 1}),
    );
    let data = read.data.unwrap();
    assert_eq!(data["lines"][0], "needle-chain");
    assert_eq!(data["start_line"], line);
    assert!(
        message_of(&data).contains(&format!("start_line={}", line + 1)),
        "{}",
        message_of(&data)
    );
}

#[test]
fn phase2_chain_read_edit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "alpha\n").unwrap();
    let read = safe(
        &ReadTool::default(),
        dir.path(),
        json!({"path": "note.txt"}),
    );
    assert!(read.error.is_none(), "{}", read.content);
    let edited = safe(
        &EditTool::default(),
        dir.path(),
        json!({
            "path": "note.txt",
            "old_string": "alpha",
            "new_string": "beta"
        }),
    );
    assert!(edited.error.is_none(), "{}", edited.content);
    let edited_data = edited.data.as_ref().unwrap();
    assert_eq!(edited_data["ok"], true);
    assert!(edited_data.get("blocks").is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "beta\n");
    let again = safe(
        &EditTool::default(),
        dir.path(),
        json!({
            "path": "note.txt",
            "old_string": "beta",
            "new_string": "gamma"
        }),
    );
    assert_eq!(again.data.as_ref().unwrap()["ok"], true);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "gamma\n");
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
