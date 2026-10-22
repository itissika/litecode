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

/// edit failure messages (merged from phase3_gate, WP-207).
#[cfg(test)]
#[allow(dead_code, unused_imports)]
mod failure_message_tests {
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
    fn edit_failure_resolves_with_message() {
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
    fn edit_failure_messages_are_stable() {
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
}
