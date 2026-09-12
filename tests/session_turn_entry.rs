//! Characterization: the session layer must not grow a second turn orchestrator
//! or absorb the subagent job registry. Tools call the same primitives humans
//! use (`reserve_turn` → `spawn_turn` → `start_turn`).

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn src_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read_src(rel: &str) -> String {
    fs::read_to_string(src_root().join(rel)).unwrap_or_else(|_| panic!("missing src/{rel}"))
}

fn assert_absent(haystack: &str, needle: &str, where_: &str) {
    assert!(
        !haystack.contains(needle),
        "{where_} must not contain `{needle}`"
    );
}

#[test]
fn session_layer_has_no_subagent_orchestrator() {
    let mut files = Vec::new();
    rust_files(&src_root().join("session"), &mut files);
    let mut combined = String::new();
    for file in &files {
        combined.push_str(&fs::read_to_string(file).unwrap_or_default());
        combined.push('\n');
    }
    for token in [
        "start_session_turn",
        "start_reserved_turn",
        "ChildTurnRegistry",
        "SubagentJobBoard",
        "emit_child_exit",
        "spawn_child_turn",
        "ReservedTurn",
    ] {
        assert_absent(&combined, token, "src/session/");
    }
}

#[test]
fn fanout_does_not_specialize_child_exits() {
    let manager = read_src("session/manager.rs");
    assert_absent(&manager, "emit_child_exit", "fanout_turn");
    assert_absent(&manager, "subagent worker panicked", "fanout_turn");
}

#[test]
fn bash_idle_uses_human_turn_order_and_releases_on_failure() {
    let src = read_src("runtime/bash_auto_turn.rs");
    assert_absent(&src, "apply_non_engine", "bash idle");
    assert_absent(&src, "start_reserved_turn", "bash idle");
    assert_absent(&src, "idle_turn::", "bash idle");

    let reserve = src
        .find("reserve_turn")
        .expect("bash idle reserves before spawn");
    let spawn = src
        .find("spawn_turn(")
        .expect("bash idle calls spawn_turn");
    let start = src
        .find(".start_turn(")
        .expect("bash idle calls start_turn");
    assert!(
        reserve < spawn && spawn < start,
        "bash idle must reserve, then spawn_turn, then start_turn (got reserve={reserve} spawn={spawn} start={start})"
    );
    assert!(
        src.contains("release_turn_reservation"),
        "bash idle must release a reservation on spawn/start failure"
    );
}

#[test]
fn session_module_does_not_declare_jobs_or_turn_entry() {
    let session_mod = read_src("session/mod.rs");
    assert_absent(&session_mod, "pub mod jobs", "session/mod.rs");
    assert_absent(&session_mod, "pub mod turn_entry", "session/mod.rs");
}
