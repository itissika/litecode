//! Layering guard: session stays a mechanical turn runtime; the hub is a
//! client of the same facts a UI reads (live stream + durable `turn/end`).
//!
//! - Tools may `reserve_turn` / `spawn_turn` / `start_turn` / `cancel_turn_sync`
//!   / `open_child_session` / `remove_session` (launch rollback only).
//! - Tools must not `finish_turn`, take the turn join handle, or keep a child
//!   concurrency cap.
//! - `src/session/` must not depend on `tools::subagent` or grow a second
//!   orchestrator (`start_session_turn`, job board, child-exit fanout).
//! - Runtime/protocol may hold a hub handle (idle + snapshot), like TerminalHub.

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

fn read_combined(dirs: &[&str]) -> String {
    let src = src_root();
    let mut files = Vec::new();
    for dir in dirs {
        rust_files(&src.join(dir), &mut files);
    }
    let mut combined = String::new();
    for file in files {
        combined.push_str(&fs::read_to_string(&file).unwrap_or_default());
        combined.push('\n');
    }
    combined
}

fn assert_absent(haystack: &str, needle: &str, where_: &str) {
    assert!(
        !haystack.contains(needle),
        "{where_} must not contain `{needle}`"
    );
}

#[test]
fn session_layer_does_not_absorb_the_hub() {
    let session = read_combined(&["session"]);
    for token in [
        "start_session_turn",
        "start_reserved_turn",
        "ChildTurnRegistry",
        "SubagentJobBoard",
        "emit_child_exit",
        "tools::subagent",
        "spawn_child_turn",
    ] {
        assert_absent(&session, token, "src/session/");
    }
}

#[test]
fn tool_layer_does_not_finish_or_join_turns() {
    let tools = read_combined(&["tools/subagent"]);
    assert_absent(&tools, ".finish_turn(", "tools/subagent");
    assert_absent(&tools, "handle.take()", "tools/subagent");
    assert_absent(&tools, "TurnHandle.handle", "tools/subagent");
    assert!(
        tools.contains("reserve_turn") && tools.contains("spawn_turn"),
        "tools/subagent must call the human reserve+spawn sequence"
    );
    assert_absent(&tools, "MAX_SUBAGENTS_PER_PARENT", "tools/subagent");
    assert_absent(&tools, "at_capacity", "tools/subagent");
}

#[test]
fn tool_turn_helper_uses_human_reserve_then_spawn() {
    let src = fs::read_to_string(src_root().join("tools/subagent/turn.rs")).unwrap();
    let reserve = src.find("reserve_turn").expect("tool helper reserves");
    let spawn = src
        .find("spawn_turn(")
        .expect("tool helper spawns");
    let start = src
        .find(".start_turn(")
        .expect("tool helper starts");
    assert!(
        reserve < spawn && spawn < start,
        "tools must reserve, then spawn_turn, then start_turn"
    );
    assert!(
        src.contains("release_turn_reservation"),
        "tool helper must release a reservation on spawn/start failure"
    );
}

#[test]
fn runtime_and_protocol_do_not_open_child_sessions() {
    let runtime = read_combined(&["runtime"]);
    let protocol = read_combined(&["client_protocol"]);
    for (name, src) in [("runtime", runtime.as_str()), ("protocol", protocol.as_str())] {
        assert_absent(src, "open_child_session", name);
    }
}

#[test]
fn tool_core_mailbox_drain_does_not_import_session_jobs() {
    let executor = fs::read_to_string(src_root().join("tool/executor.rs")).unwrap();
    let trait_ = fs::read_to_string(src_root().join("tool/trait_.rs")).unwrap();
    assert_absent(&executor, "session::jobs", "tool/executor.rs");
    assert!(
        trait_.contains("agent_subagents"),
        "tool trait must expose the hub for mailbox drain"
    );
}
