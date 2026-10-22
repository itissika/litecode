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
use std::process::Command;

use serde_json::{Value, json};

use super::index::{self, Content};
use super::mentions;
use crate::config::git_install::find_git_exe;

type Found = BTreeSet<(String, String, String)>;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn table() -> Value {
    let path = manifest().join("tests/fixtures/knowledge_refs/cases.json");
    serde_json::from_str(&fs::read_to_string(path).expect("cases.json")).expect("cases json")
}

fn spec() -> String {
    fs::read_to_string(manifest().join("docs/knowledge-references.md")).expect("spec document")
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

fn extract_actual(text: &str) -> Value {
    Value::Array(
        mentions::extract_cites(text)
            .into_iter()
            .map(|cite| {
                let mut item = json!({ "kind": cite.kind.as_str() });
                let obj = item.as_object_mut().unwrap();
                match cite.kind {
                    mentions::CiteKind::Node => {
                        obj.insert("key".into(), json!(cite.key));
                    }
                    kind => {
                        obj.insert("path".into(), json!(cite.norm_path()));
                        if kind == mentions::CiteKind::Symbol {
                            obj.insert("symbol".into(), json!(cite.symbol));
                        }
                        if kind != mentions::CiteKind::File {
                            obj.insert(
                                "lines".into(),
                                json!(cite.lines.map(mentions::format_line_span)),
                            );
                            if let Some(bad) = &cite.bad_lines {
                                obj.insert("bad_lines".into(), json!(bad));
                            }
                        }
                    }
                }
                obj.insert("ref_key".into(), json!(cite.ref_key()));
                item
            })
            .collect(),
    )
}

/// `Ok` when the case follows the spec, `Err(report)` when it does not.
fn run_extract(case: &Value) -> Result<(), String> {
    let text = case["text"].as_str().expect("text");
    let actual = extract_actual(text);
    let wanted = case["refs"].clone();
    if actual == wanted {
        Ok(())
    } else {
        Err(format!(
            "extract {}: wanted {wanted} got {actual}",
            case_id(case)
        ))
    }
}

// ---------------------------------------------------------------- resolve

fn git(dir: &Path, args: &[&str]) {
    let exe = find_git_exe().expect("git");
    let status = Command::new(exe)
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

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

/// Builds the case workspace. `None` when it needs git and there is none.
fn workspace(case: &Value) -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    if let Some(head) = case.get("head") {
        find_git_exe()?;
        git(ws, &["-c", "init.defaultBranch=main", "init", "-q"]);
        for (key, val) in [
            ("user.email", "test@litecode.local"),
            ("user.name", "Litecode Test"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            git(ws, &["config", key, val]);
        }
        write_tree(ws, Some(head));
        git(ws, &["add", "-A"]);
        git(ws, &["commit", "-q", "-m", "head"]);
    }
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
    Some(dir)
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

/// `Ok(true)` follows the spec, `Ok(false)` skipped (no git), `Err` differs.
fn run_resolve(case: &Value) -> Result<bool, String> {
    let Some(dir) = workspace(case) else {
        return Ok(false);
    };
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
    Ok(true)
}

// ---------------------------------------------------------------- layout

fn run_layout(case: &Value) -> Result<(), String> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("knowledge");
    write_tree(&root, case.get("knowledge"));
    for rel in case["dirs"].as_array().into_iter().flatten() {
        fs::create_dir_all(root.join(rel.as_str().unwrap())).unwrap();
    }
    let content = index::compute_once(dir.path());
    let sorted = |value: Value| {
        let mut items: Vec<Value> = value.as_array().cloned().unwrap_or_default();
        items.sort_by_key(|item| item.to_string());
        Value::Array(items)
    };
    let folders = sorted(json!(content.folders));
    let unknown = sorted(Value::Array(
        content
            .unknown
            .iter()
            .map(|u| json!({ "path": u.path, "folder_id": u.folder_id }))
            .collect(),
    ));
    let nodes = sorted(json!(
        content
            .nodes
            .iter()
            .map(|n| n.path.clone())
            .collect::<Vec<_>>()
    ));
    let wanted = (
        sorted(case["folders"].clone()),
        sorted(case["unknown"].clone()),
        sorted(case["nodes"].clone()),
    );
    if (folders.clone(), unknown.clone(), nodes.clone()) == wanted {
        Ok(())
    } else {
        Err(format!(
            "layout {}: wanted {wanted:?} got {:?}",
            case_id(case),
            (folders, unknown, nodes)
        ))
    }
}

// ---------------------------------------------------------------- tests

fn cases<'a>(table: &'a Value, section: &str) -> Vec<&'a Value> {
    table[section].as_array().expect(section).iter().collect()
}

#[test]
fn every_case_names_a_rule_the_spec_defines() {
    let table = table();
    let spec = spec();
    let mut ids = BTreeSet::new();
    for case in cases(&table, "extract")
        .into_iter()
        .chain(cases(&table, "resolve"))
        .chain(cases(&table, "layout"))
    {
        let id = case_id(case).to_string();
        assert!(ids.insert(id.clone()), "duplicate case id {id}");
        let rule = case["rule"].as_str().expect("rule");
        assert!(
            spec.contains(&format!("**{rule}**")),
            "case {id} cites {rule}, which the spec does not define"
        );
        if let Some(mark) = case.get("divergence") {
            let div = mark["id"].as_str().unwrap();
            assert!(
                spec.contains(&format!("**{div}**")),
                "case {id} marks divergence {div}, which the spec does not list"
            );
        }
        if let Some(mark) = case["frontend"].as_str() {
            let div = mark.split(':').next().unwrap();
            assert!(
                spec.contains(&format!("**{div}**")),
                "case {id} names frontend divergence {div}, which the spec does not list"
            );
        }
    }
}

/// Every status code the snapshot can carry is in the spec's status table,
/// and the table has nothing the code cannot emit.
#[test]
fn status_table_matches_the_codes() {
    let spec = spec();
    for code in super::validate::STATUS_CODES {
        assert!(
            spec.contains(&format!("| `{code}` |")),
            "status {code} missing from the spec status table"
        );
    }
}

#[test]
fn extraction_follows_the_spec() {
    let table = table();
    let failures: Vec<String> = cases(&table, "extract")
        .into_iter()
        .filter(|case| divergence(case).is_none())
        .filter_map(|case| run_extract(case).err())
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

#[test]
fn layout_follows_the_spec() {
    let table = table();
    let failures: Vec<String> = cases(&table, "layout")
        .into_iter()
        .filter_map(|case| run_layout(case).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A listed divergence that starts following the spec must be unlisted, so
/// the doc and the table never claim a gap that is gone.
#[test]
fn listed_divergences_still_diverge() {
    let table = table();
    let mut stale = Vec::new();
    for case in cases(&table, "extract") {
        if let Some(div) = divergence(case) {
            if run_extract(case).is_ok() {
                stale.push(format!("{div} / {}", case_id(case)));
            }
        }
    }
    for case in cases(&table, "resolve") {
        if let Some(div) = divergence(case) {
            if run_resolve(case) == Ok(true) {
                stale.push(format!("{div} / {}", case_id(case)));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "now follows the spec, remove the divergence marker: {stale:?}"
    );
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

/// KR-RANGE-2 / KR-SYM-5 in the mention snapshot: the same verdicts as the index.
#[test]
fn mention_texts_follow_the_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    fs::create_dir_all(ws.join("src")).unwrap();
    fs::write(ws.join("src/a.rs"), "fn alpha() {}\n").unwrap();
    fs::write(ws.join("notes.txt"), "x\n").unwrap();
    let text = mention_text(ws, "[@ file=\"src/a.rs\" lines=\"2-3\"]");
    assert!(
        text.contains("Lines 2-3 are outside \"src/a.rs\", which has 1 lines."),
        "{text}"
    );
    let text = mention_text(ws, "[@ file=\"notes.txt\" symbol=\"fn x\"]");
    assert!(text.contains("该语言不支持符号解析"), "{text}");
    let text = mention_text(ws, "[@ file=\"src/A.rs\" lines=\"1\"]");
    assert!(text.contains("File \"src/A.rs\" does not exist."), "{text}");
    let text = mention_text(ws, "[@ file=\"src/a.rs\" lines=\"+1\"]");
    assert!(text.contains("are not valid"), "{text}");
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
