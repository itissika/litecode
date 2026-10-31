//! KnowledgeIndex tests: incremental == from scratch, the per-turn read path,
//! and the change-bus wiring.

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

#[test]
fn every_file_is_parsed_once_per_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(
        ws,
        "src/a.rs",
        "fn alpha() {}\nfn beta() {}\nmod m { fn gamma() {} }\n",
    );
    let body = "[@ file=\"src/a.rs\" symbol=\"fn alpha\"] [@ file=\"src/a.rs\" symbol=\"fn beta\"] \
                [@ file=\"src/a.rs\" symbol=\"mod m › fn gamma\"] [@ file=\"src/a.rs\" lines=\"1-2\"]";
    write(ws, "knowledge/a.md", &node("a", body));
    write(ws, "knowledge/b.md", &node("b", body));
    let mut engine = Engine::new(ws);
    engine.refresh(&Hint::full());
    // One worktree parse for one file, however many refs.
    assert_eq!(engine.counters.ts_parses, 1);
    let before = engine.counters;
    engine.refresh(&Hint::full());
    assert_eq!(
        engine.counters.ts_parses, before.ts_parses,
        "unchanged: no parse"
    );
    assert_eq!(
        engine.counters.file_reads, before.file_reads,
        "unchanged: no read"
    );
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
            3 => format!(
                "[@ file=\"{file}\" lines=\"{}-{}\"]",
                1 + rng.pick(3),
                3 + rng.pick(6)
            ),
            4 => format!("[@ key=\"{}\"]", ["a", "b", "c", "gone"][rng.pick(4)]),
            _ => format!(
                "[@ file=\"{}\" lines=\"+{}\"]",
                file.to_uppercase(),
                rng.pick(9)
            ),
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
        for key in ["a", "b", "c"] {
            write(
                ws,
                &format!("knowledge/{key}.md"),
                &node(key, &node_body(&mut rng)),
            );
        }
        let mut engine = Engine::new(ws);
        let mut tick = 0;
        assert_eq!(
            engine.refresh(&Hint::full()),
            compute_once(ws),
            "seed {seed} start"
        );
        for step in 0..40 {
            tick += 1;
            let mut hint = Hint::default();
            match rng.pick(6) {
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
        &node(
            "n",
            "[@ file=\"src/a.rs\" symbol=\"fn alpha\"] [@ key=\"gone\"]",
        ),
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
