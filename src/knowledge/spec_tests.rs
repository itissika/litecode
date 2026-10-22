//! Contract tests for `docs/knowledge-references.md`.
//!
//! The table lives in `tests/fixtures/knowledge_refs/cases.json` so the
//! frontend can run the same cases. Each case names the spec rule it locks.
//! A case marked `divergence` is behavior the backend does not follow yet:
//! it is listed in the doc and checked to still differ. None is listed today
//! (B1-B5 are fixed); `frontend` markers name the frontend gaps F1-F7, which
//! close once the frontend consumes `GET /api/workspace/knowledge`.
//!
//! Resolution runs through the same index the server keeps resident
//! (`index::compute_once`), so the table locks the snapshot the API serves.
//! Conditions JSON cannot express (10 MiB, binary, links) are the tests at
//! the bottom.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::index::{self, Content};








// ---------------------------------------------------------------- extract



// ---------------------------------------------------------------- resolve








// ---------------------------------------------------------------- layout


// ---------------------------------------------------------------- tests


// ------------------------------------------------- beyond the JSON table

fn node_file(ws: &Path, body: &str) {
    let path = ws.join("knowledge/a.md");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!("```node\nnode : a\nstatus : enabled\nsummary : a\n```\n{body}\n"),
    )
    .unwrap();
}

fn ref_status(content: &Content, ref_key: &str) -> (String, Value) {
    let hit = content
        .nodes
        .iter()
        .flat_map(|n| n.refs.iter())
        .find(|r| r.ref_key == ref_key)
        .unwrap_or_else(|| panic!("no ref {ref_key:?}"));
    (hit.status.to_string(), json!(hit.args))
}

fn mention_text(ws: &Path, text: &str) -> String {
    crate::reminder::mentions::build(ws, text, &Default::default())
        .map(|reminder| match reminder {
            crate::reminder::Reminder::Mentions(body) => body.text,
            _ => String::new(),
        })
        .unwrap_or_default()
}

/// B4 + KR-READ-1: one read policy. A file over 10 MiB exists (file refs are
/// fine) but symbol and range refs are `unreadable_file`, and the mention
/// snapshot says the same.
#[test]
fn oversized_and_binary_files_are_unreadable_everywhere() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src")).unwrap();
    let mut big = "fn alpha() {}\n".repeat(1);
    big.push_str(&"/".repeat(super::fsref::MAX_TEXT_BYTES as usize));
    fs::write(ws.join("src/big.rs"), &big).unwrap();
    fs::write(ws.join("src/bin.rs"), [0xff_u8, 0xfe, 0x00, 0x41]).unwrap();
    let at_cap = "a\n".repeat(super::fsref::MAX_TEXT_BYTES as usize / 2);
    fs::write(ws.join("src/cap.txt"), &at_cap).unwrap();
    node_file(
        ws,
        "[@ file=\"src/big.rs\"] [@ file=\"src/big.rs\" symbol=\"fn alpha\"] \
         [@ file=\"src/big.rs\" lines=\"1\"] [@ file=\"src/bin.rs\" lines=\"1\"] \
         [@ file=\"src/cap.txt\" lines=\"5\"]",
    );
    let content = index::compute_once(ws);
    assert_eq!(ref_status(&content, "file:src/big.rs").0, "ok");
    let (status, args) = ref_status(&content, "sym:src/big.rs\0fn alpha");
    assert_eq!(status, "unreadable_file");
    assert_eq!(args["reason"], "larger than 10 MiB");
    assert_eq!(
        ref_status(&content, "range:src/big.rs\x001").0,
        "unreadable_file"
    );
    let (status, args) = ref_status(&content, "range:src/bin.rs\x001");
    assert_eq!(status, "unreadable_file");
    assert_eq!(args["reason"], "not UTF-8 text");
    assert_eq!(
        ref_status(&content, "range:src/cap.txt\x005").0,
        "ok",
        "exactly 10 MiB is readable"
    );

    // WP-050 T8: keep one mention-text sample; reason asserts above cover branches.
    let text = mention_text(ws, "[@ file=\"src/big.rs\" symbol=\"fn alpha\"]");
    assert!(
        text.contains("is not a readable text file (larger than 10 MiB)"),
        "{text}"
    );
}

/// KR-SYM-6: a symbol citation takes no `lines`. In the snapshot it is
/// `invalid_lines` with `args.reason` naming the rule, its `lines` field is
/// `null` and its key has no lines; a range keeps its span and its own reason.
#[test]
fn lines_on_a_symbol_are_invalid_lines_in_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(ws.join("src/a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
    node_file(
        ws,
        "[@ file=\"src/a.rs\" symbol=\"fn alpha\" lines=\"1\"] [@ file=\"src/a.rs\" symbol=\"fn beta\"] \
         [@ file=\"src/a.rs\" lines=\"2\"] [@ file=\"src/a.rs\" lines=\"x\"]",
    );
    let content = index::compute_once(ws);
    let refs = &content.nodes[0].refs;
    let alpha = refs
        .iter()
        .find(|r| r.ref_key == "sym:src/a.rs\0fn alpha")
        .expect("symbol key has no lines");
    assert_eq!(alpha.status, "invalid_lines");
    assert_eq!(alpha.lines, None);
    assert_eq!(
        alpha.args["reason"],
        super::validate::INVALID_LINES_ON_SYMBOL
    );
    assert_eq!(alpha.args["lines"], "1");
    // WP-050 T8: reason covers this branch; skip duplicate message/mention text.
    let beta = refs
        .iter()
        .find(|r| r.ref_key == "sym:src/a.rs\0fn beta")
        .unwrap();
    assert_eq!((beta.status, beta.lines.clone()), ("ok", None));
    let range = refs
        .iter()
        .find(|r| r.ref_key == "range:src/a.rs\x002")
        .unwrap();
    assert_eq!((range.status, range.lines.as_deref()), ("ok", Some("2")));
    let (status, args) = ref_status(&content, "range:src/a.rs\0x");
    assert_eq!(status, "invalid_lines");
    assert_eq!(args["reason"], super::validate::INVALID_LINES_FORMAT);
}

/// KR-LINK-1: links are followed inside the workspace and are an error when
/// they leave it. Skipped where the OS refuses to create links (Windows
/// without Developer Mode).
#[test]
fn links_resolve_inside_the_workspace_only() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.rs"), "fn alpha() {}\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(ws.join("src/real.rs"), "fn alpha() {}\n").unwrap();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(outside.path().join("secret.rs"), ws.join("src/out.rs"))
        .and_then(|_| std::os::unix::fs::symlink(ws.join("src/real.rs"), ws.join("src/in.rs")));
    #[cfg(windows)]
    let made =
        std::os::windows::fs::symlink_file(outside.path().join("secret.rs"), ws.join("src/out.rs"))
            .and_then(|_| {
                std::os::windows::fs::symlink_file(ws.join("src/real.rs"), ws.join("src/in.rs"))
            });
    if made.is_err() {
        // Windows without Developer Mode: a directory junction needs no
        // privilege and must follow the same rule.
        #[cfg(windows)]
        {
            let made = Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(ws.join("src").join("outdir"))
                .arg(outside.path())
                .output()
                .is_ok_and(|out| out.status.success());
            if made {
                node_file(
                    ws,
                    "[@ file=\"src/outdir/secret.rs\" lines=\"1\"] [@ file=\"src/outdir\"]",
                );
                let content = index::compute_once(ws);
                assert_eq!(
                    ref_status(&content, "range:src/outdir/secret.rs\x001").0,
                    "symlink_escape"
                );
                assert_eq!(ref_status(&content, "file:src/outdir").0, "symlink_escape");
                return;
            }
        }
        eprintln!("skipped: cannot create links here");
        return;
    }
    node_file(
        ws,
        "[@ file=\"src/out.rs\"] [@ file=\"src/out.rs\" symbol=\"fn alpha\"] [@ file=\"src/in.rs\" symbol=\"fn alpha\"]",
    );
    let content = index::compute_once(ws);
    assert_eq!(ref_status(&content, "file:src/out.rs").0, "symlink_escape");
    assert_eq!(
        ref_status(&content, "sym:src/out.rs\0fn alpha").0,
        "symlink_escape"
    );
    assert_eq!(ref_status(&content, "sym:src/in.rs\0fn alpha").0, "ok");
    let text = mention_text(ws, "[@ file=\"src/out.rs\" lines=\"1\"]");
    assert!(text.contains("resolves outside the workspace"), "{text}");
}
