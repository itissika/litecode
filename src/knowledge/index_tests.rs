//! KnowledgeIndex tests: incremental == from scratch, HEAD via git2, the
//! per-turn read path, and the change-bus wiring.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use super::*;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
}

/// Writes then moves mtime forward, so a same-length edit still changes the
/// stamp whatever the filesystem's timestamp granularity.
fn edit(root: &Path, rel: &str, text: &str, tick: u64) {
    write(root, rel, text);
    let when = SystemTime::now() + Duration::from_secs(tick + 1);
    fs::File::options()
        .write(true)
        .open(root.join(rel))
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn node(key: &str, body: &str) -> String {
    format!("```node\nnode : {key}\nstatus : enabled\nsummary : {key}\n```\n{body}\n")
}

fn commit_all(dir: &Path) {
    let repo = git2::Repository::open(dir).unwrap_or_else(|_| {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_bool("core.autocrlf", false).unwrap();
        repo
    });
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.update_all(["*"].iter(), None).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = git2::Signature::now("Litecode Test", "test@litecode.local").unwrap();
    let parent = repo.head().ok().and_then(|head| head.peel_to_commit().ok());
    let parents: Vec<&git2::Commit<'_>> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, "c", &tree, &parents)
        .unwrap();
}

fn statuses(content: &Content) -> Vec<(String, String, String)> {
    let mut out: Vec<_> = content
        .nodes
        .iter()
        .flat_map(|node| {
            node.refs
                .iter()
                .map(|r| (node.path.clone(), r.ref_key.clone(), r.status.to_string()))
        })
        .collect();
    out.sort();
    out
}

fn status_of(content: &Content, ref_key: &str) -> String {
    content
        .nodes
        .iter()
        .flat_map(|node| node.refs.iter())
        .find(|r| r.ref_key == ref_key)
        .map(|r| r.status.to_string())
        .unwrap_or_else(|| "<absent>".into())
}

#[test]
fn drift_compares_head_with_the_worktree_through_git2() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(ws, "src/a.rs", "fn alpha() {\n    let x = 1;\n}\n");
    commit_all(ws);
    write(
        ws,
        "knowledge/n.md",
        &node("n", "[@ file=\"src/a.rs\" symbol=\"fn alpha\"]"),
    );
    let key = "sym:src/a.rs\0fn alpha";
    let mut engine = Engine::new(ws);
    assert_eq!(status_of(&engine.refresh(&Hint::full()), key), "ok");

    edit(ws, "src/a.rs", "fn alpha() {\n    let x = 2;\n}\n", 1);
    let mut hint = Hint::default();
    hint.paths.insert("src/a.rs".into());
    assert_eq!(status_of(&engine.refresh(&hint), key), "symbol_drift");

    commit_all(ws);
    let head = Hint {
        head: true,
        ..Hint::default()
    };
    let content = engine.refresh(&head);
    assert_eq!(status_of(&content, key), "ok", "committing clears drift");
    assert_eq!(content, Engine::new(ws).refresh(&Hint::full()));

    write(ws, "src/b.rs", "fn beta() {}\n");
    edit(
        ws,
        "knowledge/n.md",
        &node("n", "[@ file=\"src/b.rs\" symbol=\"fn beta\"]"),
        2,
    );
    let mut hint = Hint::default();
    hint.paths.insert("knowledge/n.md".into());
    assert_eq!(
        status_of(&engine.refresh(&hint), "sym:src/b.rs\0fn beta"),
        "symbol_drift",
        "a file absent from HEAD differs from the worktree"
    );

    let bare = tempfile::tempdir().unwrap();
    write(bare.path(), "src/a.rs", "fn alpha() {}\n");
    write(
        bare.path(),
        "knowledge/n.md",
        &node("n", "[@ file=\"src/a.rs\" symbol=\"fn alpha\"]"),
    );
    assert_eq!(
        status_of(&compute_once(bare.path()), key),
        "ok",
        "no repository means no drift"
    );
}

#[test]
fn every_file_is_parsed_once_per_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(ws, "src/a.rs", "fn alpha() {}\nfn beta() {}\nmod m { fn gamma() {} }\n");
    commit_all(ws);
    let body = "[@ file=\"src/a.rs\" symbol=\"fn alpha\"] [@ file=\"src/a.rs\" symbol=\"fn beta\"] \
                [@ file=\"src/a.rs\" symbol=\"mod m › fn gamma\"] [@ file=\"src/a.rs\" lines=\"1-2\"]";
    write(ws, "knowledge/a.md", &node("a", body));
    write(ws, "knowledge/b.md", &node("b", body));
    let mut engine = Engine::new(ws);
    engine.refresh(&Hint::full());
    // One worktree parse and one HEAD parse for one file, however many refs.
    assert_eq!(engine.counters.ts_parses, 2);
    assert_eq!(engine.counters.head_blob_reads, 1);
    let before = engine.counters;
    engine.refresh(&Hint::full());
    assert_eq!(engine.counters.ts_parses, before.ts_parses, "unchanged: no parse");
    assert_eq!(engine.counters.file_reads, before.file_reads, "unchanged: no read");
    assert_eq!(engine.counters.head_blob_reads, before.head_blob_reads);
}

/// Deterministic xorshift, so a failure replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const SOURCES: [&str; 4] = ["src/a.rs", "src/b.rs", "lib/c.py", "notes.txt"];
const CHAINS: [&str; 4] = ["fn alpha", "fn beta", "class C › fn hi", "fn new"];

fn source_text(rng: &mut Rng) -> String {
    let mut out = String::new();
    for _ in 0..(1 + rng.pick(4)) {
        match rng.pick(5) {
            0 => out.push_str("fn alpha() {\n    let x = 1;\n}\n"),
            1 => out.push_str(&format!("fn beta() {{ let y = {}; }}\n", rng.pick(3))),
            2 => out.push_str("fn new() {}\n"),
            3 => out.push_str("class C:\n    def hi(self):\n        return 1\n"),
            _ => out.push_str(&format!("// pad {}\n", "x".repeat(rng.pick(20)))),
        }
    }
    out
}

fn node_body(rng: &mut Rng) -> String {
    let mut parts = Vec::new();
    for _ in 0..(1 + rng.pick(5)) {
        let file = SOURCES[rng.pick(SOURCES.len())];
        let part = match rng.pick(6) {
            0 => format!("[@ file=\"{file}\"]"),
            1 | 2 => format!(
                "[@ file=\"{file}\" symbol=\"{}\"]",
                CHAINS[rng.pick(CHAINS.len())]
            ),
            3 => format!("[@ file=\"{file}\" lines=\"{}-{}\"]", 1 + rng.pick(3), 3 + rng.pick(6)),
            4 => format!("[@ key=\"{}\"]", ["a", "b", "c", "gone"][rng.pick(4)]),
            _ => format!("[@ file=\"{}\" lines=\"+{}\"]", file.to_uppercase(), rng.pick(9)),
        };
        parts.push(part);
    }
    parts.join(" ")
}

#[test]
fn incremental_equals_from_scratch_over_random_edits() {
    for seed in [7_u64, 1234, 99_991] {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let mut rng = Rng(seed);
        for file in SOURCES {
            write(ws, file, &source_text(&mut rng));
        }
        commit_all(ws);
        for key in ["a", "b", "c"] {
            write(ws, &format!("knowledge/{key}.md"), &node(key, &node_body(&mut rng)));
        }
        let mut engine = Engine::new(ws);
        let mut tick = 0;
        assert_eq!(engine.refresh(&Hint::full()), compute_once(ws), "seed {seed} start");
        for step in 0..40 {
            tick += 1;
            let mut hint = Hint::default();
            match rng.pick(7) {
                0 | 1 => {
                    let file = SOURCES[rng.pick(SOURCES.len())];
                    let text = source_text(&mut rng);
                    edit(ws, file, &text, tick);
                    hint.paths.insert(file.into());
                }
                2 => {
                    let file = SOURCES[rng.pick(SOURCES.len())];
                    let _ = fs::remove_file(ws.join(file));
                    hint.paths.insert(file.into());
                }
                3 => {
                    let key = ["a", "b", "c", "d"][rng.pick(4)];
                    let rel = format!("knowledge/{key}.md");
                    edit(ws, &rel, &node(key, &node_body(&mut rng)), tick);
                    hint.paths.insert(rel);
                }
                4 => {
                    commit_all(ws);
                    hint.head = true;
                }
                5 => {
                    // Case-only rename: the bus reports both names.
                    let (from, to) = if ws.join("src/b.rs").exists() {
                        ("src/b.rs", "src/B.rs")
                    } else {
                        ("src/B.rs", "src/b.rs")
                    };
                    if ws.join(from).exists() {
                        fs::rename(ws.join(from), ws.join(to)).unwrap();
                    }
                    hint.paths.insert(from.into());
                    hint.paths.insert(to.into());
                }
                _ => {
                    let rel = "knowledge/sub/e.md";
                    edit(ws, rel, &node("e", &node_body(&mut rng)), tick);
                    hint.paths.insert(rel.into());
                }
            }
            let incremental = engine.refresh(&hint);
            let scratch = compute_once(ws);
            assert_eq!(
                statuses(&incremental),
                statuses(&scratch),
                "seed {seed} step {step} hint {hint:?}"
            );
            assert_eq!(incremental, scratch, "seed {seed} step {step}");
        }
    }
}

#[test]
fn the_turn_reads_the_snapshot_only() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(ws, "src/a.rs", "fn alpha() {}\n");
    write(
        ws,
        "knowledge/n.md",
        &node("n", "[@ file=\"src/a.rs\" symbol=\"fn alpha\"] [@ key=\"gone\"]"),
    );
    let index = start(ws);
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !index.snapshot().content.ready {
        assert!(std::time::Instant::now() < deadline, "index never ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut rx = index.subscribe();
    let counters = index.with_engine(|engine| engine.counters);
    let note = turn_note(ws).expect("note");
    assert_eq!(note.class, status::Class::Error);
    for _ in 0..20 {
        turn_note(ws);
    }
    assert_eq!(
        index.with_engine(|engine| engine.counters).refreshes,
        counters.refreshes,
        "a turn never refreshes, reads, stats or parses"
    );

    let version = index.snapshot().version;
    edit(ws, "knowledge/n.md", &node("n", "fine"), 1);
    index.notify_paths(&["knowledge/n.md".into()]);
    let fresh = index.wait_fresh(Duration::from_secs(10));
    assert!(fresh);
    let snapshot = index.snapshot();
    assert!(snapshot.version > version);
    assert_eq!(turn_note(ws).unwrap().class, status::Class::Ok);
    let pushed = rx.try_recv().expect("knowledge/changed");
    assert_eq!(pushed.paths, vec!["n.md".to_string()]);

    // Unrelated paths are dropped before they can make a turn wait.
    index.notify_paths(&["target/debug/x.o".into()]);
    assert!(index.wait_fresh(Duration::ZERO));
    unregister(ws);
}

#[test]
fn snapshot_wire_shape() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(ws, "src/a.rs", "fn alpha() {}\n");
    write(
        ws,
        "knowledge/内核/seq.md",
        &node("seq", "[@ file=\"src\\a.rs\"] [@ key=\"other\"]"),
    );
    write(ws, "knowledge/内核/readme.md", "no declaration\n");
    fs::create_dir_all(ws.join("knowledge/empty")).unwrap();
    let snapshot = Snapshot {
        version: 3,
        epoch: "e".into(),
        content: Arc::new(compute_once(ws)),
    };
    let json = snapshot.to_json();
    assert_eq!(json["version"], 3);
    assert_eq!(json["ready"], true);
    assert_eq!(json["root"], "knowledge");
    assert_eq!(json["visibility"], "public");
    assert_eq!(json["folders"][0]["id"], "empty");
    assert_eq!(json["folders"][1]["id"], "内核");
    assert_eq!(json["unknown"][0]["path"], "内核/readme.md");
    assert_eq!(json["unknown"][0]["folder_id"], "内核");
    let seq = &json["nodes"][0];
    assert_eq!(seq["path"], "内核/seq.md");
    assert_eq!(seq["severity"], "error");
    assert_eq!(seq["refs"][0]["ref_key"], "file:src/a.rs");
    assert_eq!(seq["refs"][0]["raw"], "[@ file=\"src\\a.rs\"]");
    assert_eq!(seq["refs"][0]["status"], "ok");
    assert_eq!(seq["refs"][1]["status"], "dangling_relation");
    assert_eq!(seq["refs"][1]["args"]["key"], "other");
    assert_eq!(seq["issues"][0]["code"], "dangling_relation");
    assert_eq!(seq["issues"][0]["ref_key"], "node:other");
}