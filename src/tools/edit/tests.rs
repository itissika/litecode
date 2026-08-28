use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::EditTool;
use crate::types::ToolSignalLevel;

fn tool() -> EditTool {
    EditTool::new()
}

fn call_edits(path: &std::path::Path, edits: Value) -> crate::types::ToolCallResult {
    tool().call(serde_json::json!({
        "file_path": path.to_str().unwrap(),
        "edits": edits
    }))
}

fn one(old: &str, new: &str) -> Value {
    serde_json::json!([{ "old_string": old, "new_string": new }])
}

fn write_temp(name: &str, contents: impl AsRef<[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

#[test]
fn edit_tool_roundtrips_bom_and_matches_first_line() {
    let (_dir, path) = write_temp("bom.rs", "\u{feff}fn main() {}\n");
    let result = call_edits(&path, one("fn main() {}", "fn start() {}"));
    assert!(
        !result.content.starts_with("Error:"),
        "got: {}",
        result.content
    );
    let on_disk = std::fs::read(&path).unwrap();
    assert!(on_disk.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert_eq!(
        String::from_utf8(on_disk).unwrap(),
        "\u{feff}fn start() {}\n"
    );
}

#[test]
fn single_success_reports_summary_and_indexed_outcome() {
    let (_dir, path) = write_temp("a.rs", "fn main() {}\n");
    let result = call_edits(&path, one("fn main() {}", "fn start() {}"));
    assert!(result.content.contains("1 applied"), "{}", result.content);
    assert!(
        result.content.contains("[1] applied:"),
        "{}",
        result.content
    );
}

#[test]
fn edit_tool_rejects_nul_bytes() {
    let (_dir, path) = write_temp("bin.txt", b"ok\x00still-utf8");
    let result = call_edits(&path, one("ok", "no"));
    assert!(result.content.to_lowercase().contains("binary"));
    assert_eq!(std::fs::read(&path).unwrap(), b"ok\x00still-utf8");
}

#[test]
fn edit_tool_rejects_utf16() {
    let (_dir, path) = write_temp("u16.txt", [0xFF, 0xFE, b'A', 0x00]);
    let result = call_edits(&path, one("A", "B"));
    assert!(result.content.contains("UTF-16"));
}

#[test]
fn edit_tool_replaces_smart_quotes() {
    let (_dir, path) = write_temp("doc.md", "say \u{201C}hello\u{201D}\n");
    let result = call_edits(&path, one("\"hello\"", "\"hi\""));
    assert!(
        !result.content.starts_with("Error:"),
        "got: {}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "say \"hi\"\n");
}

#[test]
fn edit_tool_rejects_partial_em_dash() {
    let (_dir, path) = write_temp("dash.txt", "foo\u{2014}bar\n");
    let result = call_edits(&path, one("-", "="));
    assert!(
        result.content.contains("unicode_confusable"),
        "got: {}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo\u{2014}bar\n");
}

#[test]
fn edit_tool_wire_view_prefixes_and_indent() {
    let (_dir, path) = write_temp("a.rs", "    foo()\n");
    let result = call_edits(&path, one("     1:     foo()", "bar()"));
    assert!(result.content.starts_with("Error:"));
    assert!(result.content.contains("line-number prefixes"));
    assert!(
        result.content.contains("line 1") || result.content.contains("foo()"),
        "{}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "    foo()\n");
}

#[test]
fn edit_tool_wire_multiple_matches() {
    let (_dir, path) = write_temp("a.rs", "foo\nbar\nfoo\n");
    let result = call_edits(&path, one("foo", "FOO"));
    assert!(
        result.content.contains("multiple_exact"),
        "{}",
        result.content
    );
    assert!(result.content.contains("lines 1, 3"), "{}", result.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo\nbar\nfoo\n");
}

#[test]
fn edit_tool_wire_noop_and_empty_file() {
    let (_dir, path) = write_temp("a.rs", "fn main() {}\n");
    let noop = call_edits(&path, one("fn main() {}", "fn main() {}"));
    assert!(noop.content.contains("no_op"), "{}", noop.content);

    std::fs::write(&path, "").unwrap();
    let empty = call_edits(&path, one("hello", "world"));
    assert!(empty.content.contains("empty"), "{}", empty.content);
    assert!(!empty.content.contains("[1]"), "{}", empty.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
}

#[test]
fn edit_tool_partial_success_writes_once_and_warns() {
    let (_dir, path) = write_temp("a.rs", "alpha\nbeta\n");
    let result = call_edits(
        &path,
        serde_json::json!([
            {"old_string": "alpha", "new_string": "ALPHA"},
            {"old_string": "missing_token_zz", "new_string": "nope"}
        ]),
    );
    assert_eq!(result.level, ToolSignalLevel::Warning);
    assert!(
        result.content.contains("applied: exact"),
        "{}",
        result.content
    );
    assert!(result.content.contains("failed:"), "{}", result.content);
    assert!(
        result.content.contains("File was modified") || result.content.contains("File updated"),
        "{}",
        result.content
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "ALPHA\nbeta\n");
}

#[test]
fn edit_tool_all_fail_does_not_write() {
    let (_dir, path) = write_temp("a.rs", "keep\n");
    let result = call_edits(
        &path,
        serde_json::json!([
            {"old_string": "nope1", "new_string": "x"},
            {"old_string": "nope2", "new_string": "y"}
        ]),
    );
    assert!(result.content.starts_with("Error:"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n");
}

#[test]
fn edit_tool_schema_rejects_old_shape() {
    let err = tool()
        .validate_input(&serde_json::json!({
            "file_path": "a.rs",
            "old_string": "a",
            "new_string": "b"
        }))
        .unwrap_err();
    assert!(err.contains("edits"), "{err}");
}

#[test]
fn edit_tool_rejects_unknown_item_parameter() {
    let err = tool()
        .validate_input(&serde_json::json!({
            "file_path": "a.rs",
            "edits": [{
                "old_string": "a",
                "new_string": "b",
                "mode": "fuzzy"
            }]
        }))
        .unwrap_err();
    assert_eq!(err, "unknown parameter 'edits[0].mode'");
}

#[test]
fn edit_tool_is_destructive_on_any_empty_new_string() {
    assert!(tool().is_destructive(
        &serde_json::json!({
            "file_path": "a.rs",
            "edits": [
                {"old_string": "a", "new_string": "b"},
                {"old_string": "c", "new_string": ""}
            ]
        }),
        crate::workspace::ToolPathMode::All,
        std::path::Path::new("."),
    ));
    assert!(!tool().is_destructive(
        &serde_json::json!({
            "file_path": "a.rs",
            "edits": [{"old_string": "a", "new_string": "b"}]
        }),
        crate::workspace::ToolPathMode::All,
        std::path::Path::new("."),
    ));
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
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tool().call_for_execution(
            serde_json::json!({
                "file_path": path.to_str().unwrap(),
                "edits": [{"old_string": "fn main() {}", "new_string": "fn start() {}"}]
            }),
            execution,
        ));
    assert!(result.content.contains("cancelled"), "{}", result.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
}

#[test]
fn description_has_no_mode_knob() {
    let ctx = crate::context_pipeline::Context {
        cwd: std::path::PathBuf::from("."),
        workspace_paths: crate::config::resolved::WorkspacePaths::for_legacy_root(
            std::path::Path::new("."),
        ),
        agents_md: None,
        claude_md: None,
    };
    let text = tool().description(&ctx);
    assert!(!text.contains("mode"));
    assert!(text.contains("edits"));
    assert!(text.contains("cannot depend") || text.contains("before any of them apply"));
}

#[test]
fn virtual_session_path_is_read_only() {
    let result = tool().call(serde_json::json!({
        "file_path": ".litecode/sessions/01ABCDEF.md",
        "edits": [{ "old_string": "a", "new_string": "b" }]
    }));
    assert_eq!(result.level, ToolSignalLevel::Error);
    assert!(
        result
            .content
            .contains(crate::session::transcript_file::READ_ONLY_MSG),
        "{}",
        result.content
    );
}

#[test]
fn low_fuzzy_message_is_actionable_without_garbage_preview() {
    let (_dir, path) = write_temp("a.rs", "fn keep() {}\n");
    let result = call_edits(&path, one("totally_unrelated_identifier_xyz", "nope"));
    assert!(result.content.starts_with("Error:"));
    assert!(
        result
            .content
            .contains("No sufficiently similar region was found"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("unique context"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("score"), "{}", result.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn keep() {}\n");
}
