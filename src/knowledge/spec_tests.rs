//! Contract tests for knowledge references: extract, resolve, layout.
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
use super::mentions;

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

fn run_extract(case: &Value) -> Result<(), String> {
    let text = case["text"].as_str().expect("text");
    let actual: Vec<String> = mentions::extract_cites(text)
        .iter()
        .map(|cite| cite.ref_key())
        .collect();
    let wanted: Vec<String> = case["refs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| item["ref_key"].as_str().expect("ref_key").to_string())
        .collect();
    if actual != wanted {
        return Err(format!(
            "extract {}: wanted {wanted:?} got {actual:?}",
            case_id(case)
        ));
    }
    Ok(())
}

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

fn run_layout(case: &Value) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("knowledge");
    for (rel, body) in case["knowledge"].as_object().expect("knowledge") {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body.as_str().expect("knowledge text")).unwrap();
    }
    for rel in case["dirs"].as_array().into_iter().flatten() {
        fs::create_dir_all(root.join(rel.as_str().unwrap())).unwrap();
    }

    let content = index::compute_once(dir.path());
    let folders: Vec<Value> = content
        .folders
        .iter()
        .map(|folder| {
            json!({
                "id": folder.id,
                "name": folder.name,
                "parent_id": folder.parent_id,
            })
        })
        .collect();
    if Value::Array(folders.clone()) != case["folders"] {
        return Err(format!(
            "layout {}: folders wanted {:?} got {folders:?}",
            case_id(case),
            case["folders"]
        ));
    }

    // Walk order is filesystem order; the contract is the set, not the order.
    let by_path = |a: &Value, b: &Value| a["path"].as_str().cmp(&b["path"].as_str());
    let mut unknown: Vec<Value> = content
        .unknown
        .iter()
        .map(|file| json!({"path": file.path, "folder_id": file.folder_id}))
        .collect();
    let mut wanted_unknown: Vec<Value> = case["unknown"].as_array().cloned().unwrap_or_default();
    unknown.sort_by(by_path);
    wanted_unknown.sort_by(by_path);
    if unknown != wanted_unknown {
        return Err(format!(
            "layout {}: unknown wanted {wanted_unknown:?} got {unknown:?}",
            case_id(case)
        ));
    }

    let mut nodes: Vec<&str> = content.nodes.iter().map(|node| node.path.as_str()).collect();
    let mut wanted_nodes: Vec<&str> = case["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|path| path.as_str().expect("node path"))
        .collect();
    nodes.sort();
    wanted_nodes.sort();
    if nodes != wanted_nodes {
        return Err(format!(
            "layout {}: nodes wanted {wanted_nodes:?} got {nodes:?}",
            case_id(case)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------- tests

fn cases<'a>(table: &'a Value, section: &str) -> Vec<&'a Value> {
    table[section].as_array().expect(section).iter().collect()
}

#[test]
fn extraction_follows_the_spec() {
    let table = table();
    let failures: Vec<String> = cases(&table, "extract")
        .into_iter()
        .filter_map(|case| run_extract(case).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn layout_follows_the_spec() {
    let table = table();
    let failures: Vec<String> = cases(&table, "layout")
        .into_iter()
        .filter_map(|case| run_layout(case).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
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
        "[big](src/big.rs) [alpha](src/big.rs#alpha) \
         [big:1](src/big.rs#L1) [bin:1](src/bin.rs#L1) \
         [cap:5](src/cap.txt#L5)",
    );
    let content = index::compute_once(ws);
    assert_eq!(ref_status(&content, "path:src/big.rs").0, "ok");
    let (status, args) = ref_status(&content, "sym:src/big.rs\0alpha");
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
}

/// B5: a directory exists for file refs, and symbol/range refs on it are
/// `unreadable_file (directory)` in the index and `/symbol-refs`.
#[test]
fn directory_texts_match_the_status() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src/inner")).unwrap();
    node_file(
        ws,
        "[src](src) [src:1](src#L1) [a](src#a)",
    );
    let content = index::compute_once(ws);
    assert_eq!(ref_status(&content, "path:src").0, "ok");
    assert_eq!(
        ref_status(&content, "range:src\x001").1["reason"],
        "directory"
    );
    assert_eq!(ref_status(&content, "sym:src\0a").0, "unreadable_file");
    let hits = index::query_symbols(ws, &[("src".into(), Some("a".into()))]);
    assert_eq!(hits[0].status, "unreadable_file");
    assert!(hits[0].file_exists);
}

/// Private root (`.litecode/knowledge`) resolves logical `knowledge/...` paths
/// (both node folding and line ranges) and tolerates `.litecode/knowledge/...`.
/// When a public `knowledge/` root exists, `.litecode/knowledge/...` is never folded.
#[test]
fn private_and_public_roots_fold_correctly() {
    let priv_dir = tempfile::tempdir().unwrap();
    let ws = priv_dir.path();
    fs::create_dir_all(ws.join(".litecode/knowledge/arch")).unwrap();
    fs::write(
        ws.join(".litecode/knowledge/arch/seq.md"),
        "```node\nnode : seq\nstatus : enabled\nsummary : seq\n```\nline one\n",
    )
    .unwrap();
    fs::write(
        ws.join(".litecode/knowledge/a.md"),
        "```node\nnode : a\nstatus : enabled\nsummary : a\n```\n\
         [seq](knowledge/arch/seq.md) [disk](.litecode/knowledge/arch/seq.md) \
         [line](knowledge/arch/seq.md#L1)\n",
    )
    .unwrap();
    let content = index::compute_once(ws);
    assert_eq!(content.visibility, "private");
    let a = content.nodes.iter().find(|n| n.key == "a").unwrap();
    assert_eq!(a.relations, vec!["seq"]);
    assert!(a.issues.is_empty(), "{:?}", a.issues);
    let logical = a
        .refs
        .iter()
        .find(|r| r.ref_key == "path:knowledge/arch/seq.md")
        .unwrap();
    assert_eq!((logical.kind, logical.status), ("node", "ok"));
    let disk = a
        .refs
        .iter()
        .find(|r| r.ref_key == "path:.litecode/knowledge/arch/seq.md")
        .unwrap();
    assert_eq!((disk.kind, disk.status), ("node", "ok"));
    let range = a
        .refs
        .iter()
        .find(|r| r.ref_key == "range:knowledge/arch/seq.md\x001")
        .unwrap();
    assert_eq!((range.kind, range.status), ("range", "ok"));

    // Public root hides `.litecode/knowledge/...` from node folding.
    let pub_dir = tempfile::tempdir().unwrap();
    let ws = pub_dir.path();
    fs::create_dir_all(ws.join("knowledge")).unwrap();
    fs::create_dir_all(ws.join(".litecode/knowledge")).unwrap();
    fs::write(
        ws.join(".litecode/knowledge/secret.md"),
        "```node\nnode : secret\nstatus : enabled\nsummary : secret\n```\n",
    )
    .unwrap();
    fs::write(
        ws.join("knowledge/a.md"),
        "```node\nnode : a\nstatus : enabled\nsummary : a\n```\n[s](.litecode/knowledge/secret.md)\n",
    )
    .unwrap();
    let content = index::compute_once(ws);
    assert_eq!(content.visibility, "public");
    let a = content.nodes.iter().find(|n| n.key == "a").unwrap();
    assert!(a.relations.is_empty());
    let r = a
        .refs
        .iter()
        .find(|r| r.ref_key == "path:.litecode/knowledge/secret.md")
        .unwrap();
    assert_eq!(r.kind, "file");
}

/// Filename mismatch is an Error and prevents the bad node from being cited.
#[test]
fn filename_mismatch_is_error_and_unciteable() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("knowledge")).unwrap();
    fs::write(
        ws.join("knowledge/wrong.md"),
        "```node\nnode : right\nstatus : enabled\nsummary : bad\n```\n",
    )
    .unwrap();
    fs::write(
        ws.join("knowledge/a.md"),
        "```node\nnode : a\nstatus : enabled\nsummary : a\n```\n[bad](knowledge/wrong.md)\n",
    )
    .unwrap();
    let content = index::compute_once(ws);
    let wrong = content.nodes.iter().find(|n| n.path == "wrong.md").unwrap();
    assert_eq!(wrong.severity, Some(super::validate::Severity::Error));
    assert!(wrong.issues.iter().any(|i| i.code == "filename_mismatch" && i.severity == super::validate::Severity::Error));
    let a = content.nodes.iter().find(|n| n.path == "a.md").unwrap();
    assert!(a.relations.is_empty());
    let cite = a.refs.iter().find(|r| r.ref_key == "path:knowledge/wrong.md").unwrap();
    assert_eq!(cite.kind, "node");
    assert_eq!(cite.status, "dangling_relation");
    assert_eq!(cite.severity, Some(super::validate::Severity::Error));
    assert_eq!(cite.args["reason"], "filename_mismatch");
}
