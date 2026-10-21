use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::edit::EditTool;
use crate::types::ToolSignalLevel;

fn tool() -> EditTool {
    EditTool::new()
}

fn call_edit(path: &std::path::Path, old: &str, new: &str) -> crate::types::ToolCallResult {
    tool().call(serde_json::json!({
        "path": path.to_str().unwrap(),
        "old_string": old,
        "new_string": new
    }))
}

fn write_temp(name: &str, contents: impl AsRef<[u8]>) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

#[test]
fn edit_declares_a_thirty_second_timeout() {
    assert_eq!(tool().timeout(), Some(std::time::Duration::from_secs(30)));
}

#[test]
fn edit_tool_roundtrips_bom_and_matches_first_line() {
    let (_dir, path) = write_temp("bom.rs", "\u{feff}fn main() {}\n");
    let result = call_edit(&path, "fn main() {}", "fn start() {}");
    assert!(result.error.is_none(), "got: {}", result.content);
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], true);
    let on_disk = std::fs::read(&path).unwrap();
    assert!(on_disk.starts_with(&[0xEF, 0xBB, 0xBF]));
    assert_eq!(
        String::from_utf8(on_disk).unwrap(),
        "\u{feff}fn start() {}\n"
    );
}

#[test]
fn single_success_reports_edit_result() {
    let (_dir, path) = write_temp("a.rs", "fn main() {}\n");
    let result = call_edit(&path, "fn main() {}", "fn start() {}");
    assert!(result.error.is_none());
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], true);
    assert_eq!(data["message"].as_str().unwrap().split('\n').next().unwrap(), "replaced");
    assert!(data.get("blocks").is_none());
    assert!(data.get("applied").is_none());
    assert!(data.get("file").is_none());
}

#[test]
fn edit_tool_rejects_nul_bytes() {
    let (_dir, path) = write_temp("bin.txt", b"ok\x00still-utf8");
    let result = call_edit(&path, "ok", "no");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().to_lowercase().contains("binary")
            || result.content.to_lowercase().contains("binary"),
        "{}",
        data["message"]
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"ok\x00still-utf8");
}

#[test]
fn edit_tool_rejects_utf16() {
    let (_dir, path) = write_temp("u16.txt", [0xFF, 0xFE, b'A', 0x00]);
    let result = call_edit(&path, "A", "B");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("UTF-16"),
        "{}",
        data["message"]
    );
}

#[test]
fn edit_tool_does_not_equate_curly_quotes() {
    let (_dir, path) = write_temp("doc.md", "say \u{201C}hello\u{201D}\n");
    let result = call_edit(&path, "\"hello\"", "\"hi\"");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("not found"),
        "{}",
        data["message"]
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "say \u{201C}hello\u{201D}\n"
    );
}

#[test]
fn edit_tool_wire_view_prefixes_and_indent() {
    let (_dir, path) = write_temp("a.rs", "    foo()\n");
    let prefixed = crate::tool::format_file_line(1, "    foo()");
    let result = call_edit(&path, prefixed.trim_end(), "bar()");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    let msg = data["message"].as_str().unwrap();
    assert!(msg.contains("line-number prefix") || msg.contains("prefix"), "{msg}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "    foo()\n");
}

#[test]
fn edit_tool_wire_multiple_matches() {
    let (_dir, path) = write_temp("a.rs", "foo\nbar\nfoo\n");
    let result = call_edit(&path, "foo", "FOO");
    let data = result.data.expect("edit data");
    assert_eq!(data["ok"], false);
    let msg = data["message"].as_str().unwrap();
    assert!(msg.contains("more than once"), "{msg}");
    assert!(msg.contains("1") && msg.contains("3"), "{msg}");
    assert!(!msg.contains("replace_all"), "{msg}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "foo\nbar\nfoo\n");
}

#[test]
fn edit_tool_wire_noop_and_empty_file() {
    let (_dir, path) = write_temp("a.rs", "fn main() {}\n");
    let noop = call_edit(&path, "fn main() {}", "fn main() {}");
    let data = noop.data.expect("data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("same"),
        "{}",
        data["message"]
    );

    std::fs::write(&path, "").unwrap();
    let empty = call_edit(&path, "hello", "world");
    let data = empty.data.expect("data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("empty"),
        "{}",
        data["message"]
    );
}

#[test]
fn edit_tool_schema_accepts_flat_shape_rejects_old() {
    tool()
        .validate_input(&serde_json::json!({
            "path": "a.rs",
            "old_string": "a",
            "new_string": "b"
        }))
        .unwrap();

    let err = tool()
        .validate_input(&serde_json::json!({
            "path": "a.rs",
            "edits": [{"old_string": "a", "new_string": "b"}]
        }))
        .unwrap_err();
    assert!(err.contains("edits"), "{err}");

    let err = tool()
        .validate_input(&serde_json::json!({
            "path": "a.rs",
            "old_string": "a",
            "new_string": "b",
            "replace_all": true
        }))
        .unwrap_err();
    assert!(err.contains("replace_all"), "{err}");

    let err = tool()
        .validate_input(&serde_json::json!({
            "path": "a.rs",
            "old_string": "a",
            "new_string": "b",
            "expected_hash": "abc"
        }))
        .unwrap_err();
    assert!(err.contains("expected_hash"), "{err}");
}

#[test]
fn edit_tool_is_destructive_on_empty_new_string() {
    assert!(tool().is_destructive(
        &serde_json::json!({
            "path": "a.rs",
            "old_string": "c",
            "new_string": ""
        }),
        crate::workspace::ToolPathMode::All,
        std::path::Path::new("."),
    ));
    assert!(!tool().is_destructive(
        &serde_json::json!({
            "path": "a.rs",
            "old_string": "a",
            "new_string": "b"
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
        session: None,
    };
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(tool().call_for_execution(
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "old_string": "fn main() {}",
                "new_string": "fn start() {}"
            }),
            execution,
        ));
    assert!(result.content.contains("cancelled"), "{}", result.content);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
}

#[test]
fn description_has_no_old_contract_knobs() {
    let ctx = crate::context_pipeline::Context {
        cwd: std::path::PathBuf::from("."),
        workspace_paths: crate::config::resolved::WorkspacePaths::for_legacy_root(
            std::path::Path::new("."),
        ),
        agents_md: None,
        claude_md: None,
    };
    let text = tool().description(&ctx);
    assert!(text.contains("no `replace_all`") || !text.contains("set replace_all"), "{text}");
    assert!(text.contains("no `expected_hash`") || text.contains("There is no `replace_all`"), "{text}");
    assert!(text.contains("EditResult"), "{text}");
    assert!(text.contains("old_string"));
    // Teaching may mention retired names only as "there is no …".
    assert!(text.contains("resolves") || text.contains("ok"), "{text}");
}

#[test]
fn virtual_session_path_is_read_only() {
    let result = tool().call(serde_json::json!({
        "path": ".litecode/sessions/01ABCDEF.md",
        "old_string": "a",
        "new_string": "b"
    }));
    // Resolves with ok:false (unified resolve) rather than ToolError throw.
    if let Some(data) = &result.data {
        assert_eq!(data["ok"], false);
        assert!(
            data["message"]
                .as_str()
                .unwrap()
                .contains(crate::session::transcript_file::READ_ONLY_MSG)
                || result
                    .content
                    .contains(crate::session::transcript_file::READ_ONLY_MSG),
            "{} / {}",
            data["message"],
            result.content
        );
    } else {
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(
            result
                .content
                .contains(crate::session::transcript_file::READ_ONLY_MSG),
            "{}",
            result.content
        );
    }
}

#[test]
fn unmatched_reports_no_match_without_applying() {
    let (_dir, path) = write_temp("a.rs", "fn keep() {}\n");
    let result = call_edit(&path, "totally_unrelated_identifier_xyz", "nope");
    let data = result.data.expect("data");
    assert_eq!(data["ok"], false);
    assert!(
        data["message"].as_str().unwrap().contains("not found"),
        "{}",
        data["message"]
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn keep() {}\n");
}

#[test]
fn empty_new_string_deletes_old() {
    let (_dir, path) = write_temp("a.rs", "keep\nremove\n");
    let result = call_edit(&path, "remove\n", "");
    let data = result.data.expect("data");
    assert_eq!(data["ok"], true);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep\n");
}
