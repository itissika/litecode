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

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::index::{self, Content};

type Found = BTreeSet<(String, String, String)>;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn table() -> Value {
    let path = manifest().join("tests/fixtures/knowledge_refs/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("cases.json")).expect("cases json")
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// Divergence id when this case is a known backend divergence on this platform.
fn divergence(case: &Value) -> Option<String> {
    let mark = case.get("divergence")?;
    let id = mark["id"].as_str()?.to_string();
    match mark.get("platforms").and_then(Value::as_array) {
        Some(list) if !list.iter().any(|p| p.as_str() == Some(platform())) => None,
        _ => Some(id),
    }
}

fn case_id(case: &Value) -> &str {
    case["id"].as_str().unwrap_or("?")
}

// ---------------------------------------------------------------- extract

// ---------------------------------------------------------------- resolve

fn write_tree(root: &Path, files: Option<&Value>) {
    let Some(files) = files.and_then(Value::as_object) else {
        return;
    };
    for (rel, text) in files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text.as_str().expect("file text")).unwrap();
    }
}

fn node_markdown(node: &Value) -> String {
    let key = node["key"].as_str().expect("key");
    let status = node["status"].as_str().unwrap_or("enabled");
    let body = node["body"].as_str().unwrap_or("");
    format!("```node\nnode : {key}\nstatus : {status}\nsummary : {key}\n```\n{body}\n")
}

/// Builds the case workspace.
fn workspace(case: &Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write_tree(ws, case.get("files"));
    for rel in case["delete"].as_array().into_iter().flatten() {
        fs::remove_file(ws.join(rel.as_str().unwrap())).unwrap();
    }
    for rel in case["dirs"].as_array().into_iter().flatten() {
        fs::create_dir_all(ws.join(rel.as_str().unwrap())).unwrap();
    }
    for node in case["nodes"].as_array().into_iter().flatten() {
        let rel = node["path"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}.md", node["key"].as_str().unwrap()));
        let path = ws.join("knowledge").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, node_markdown(node)).unwrap();
    }
    dir
}

/// Issues in the snapshot: (node path, status, ref_key) for citation issues,
/// (node path, status, ref) for declaration issues.
fn found(content: &Content) -> Found {
    content
        .nodes
        .iter()
        .flat_map(|node| {
            node.issues.iter().map(move |issue| {
                let third = issue
                    .ref_key
                    .clone()
                    .or_else(|| issue.reference.clone())
                    .unwrap_or_default();
                (node.path.clone(), issue.code.clone(), third)
            })
        })
        .collect()
}

fn resolve_expected(case: &Value) -> Found {
    case["expect"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let third = item
                .get("ref_key")
                .or_else(|| item.get("ref"))
                .and_then(Value::as_str)
                .unwrap_or("");
            (
                item["node"].as_str().unwrap().to_string(),
                item["status"].as_str().unwrap().to_string(),
                third.to_string(),
            )
        })
        .collect()
}

/// `Ok` follows the spec, `Err` differs.
fn run_resolve(case: &Value) -> Result<(), String> {
    let dir = workspace(case);
    let content = index::compute_once(dir.path());
    let actual = found(&content);
    let wanted = resolve_expected(case);
    if actual != wanted {
        return Err(format!(
            "resolve {}: wanted {wanted:?} got {actual:?}",
            case_id(case)
        ));
    }
    // Every issue is the verdict of exactly one ref in the same node.
    for node in &content.nodes {
        for issue in &node.issues {
            if let Some(key) = &issue.ref_key {
                let hit = node.refs.iter().find(|r| &r.ref_key == key);
                match hit {
                    Some(r) if r.status == issue.code => {}
                    other => {
                        return Err(format!(
                            "resolve {}: issue {key:?} has ref {other:?}",
                            case_id(case)
                        ));
                    }
                }
            }
        }
        let keys: BTreeSet<_> = node.refs.iter().map(|r| &r.ref_key).collect();
        if keys.len() != node.refs.len() {
            return Err(format!("resolve {}: refs not deduped", case_id(case)));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- layout

// ---------------------------------------------------------------- tests

fn cases<'a>(table: &'a Value, section: &str) -> Vec<&'a Value> {
    table[section].as_array().expect(section).iter().collect()
}

#[test]
fn resolution_follows_the_spec() {
    let table = table();
    let failures: Vec<String> = cases(&table, "resolve")
        .into_iter()
        .filter(|case| divergence(case).is_none())
        .filter_map(|case| run_resolve(case).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

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

/// B5: a directory exists for file refs, and symbol/range refs on it are
/// `unreadable_file (directory)` in the index, the mention text and
/// `/symbol-refs`.
#[test]
fn directory_texts_match_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src/inner")).unwrap();
    node_file(
        ws,
        "[@ file=\"src\"] [@ file=\"src\" lines=\"1\"] [@ file=\"src\" symbol=\"fn a\"]",
    );
    let content = index::compute_once(ws);
    assert_eq!(ref_status(&content, "file:src").0, "ok");
    assert_eq!(
        ref_status(&content, "range:src\x001").1["reason"],
        "directory"
    );
    assert_eq!(ref_status(&content, "sym:src\0fn a").0, "unreadable_file");
    // WP-050 T8: directory reason already asserted; skip duplicate mention text.
    let hits = index::query_symbols(ws, &[("src".into(), Some("fn a".into()))]);
    assert_eq!(hits[0].status, "unreadable_file");
    assert!(hits[0].file_exists);
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
