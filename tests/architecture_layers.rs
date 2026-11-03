//! Layering guard: session stays a mechanical turn runtime; the hub is a
//! client of the same facts a UI reads (live stream + durable `turn/end`).
//!
//! - UI/subagent production paths use `SessionService` business operations;
//!   support plumbing is isolated in `session::service::support`.
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

fn production_part(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
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
        tools.contains("start_turn("),
        "tools/subagent must call the SessionService turn operation"
    );
    assert_absent(&tools, "MAX_SUBAGENTS_PER_PARENT", "tools/subagent");
    assert_absent(&tools, "at_capacity", "tools/subagent");
}

#[test]
fn tool_turn_helper_uses_session_business_start() {
    let src = fs::read_to_string(src_root().join("tools/subagent/turn.rs")).unwrap();
    assert!(
        src.contains(".start_turn("),
        "tool helper must call the SessionService turn operation"
    );
    assert_absent(&src, "reserve_turn", "tools/subagent/turn.rs");
    assert_absent(&src, "spawn_turn", "tools/subagent/turn.rs");
    assert_absent(&src, "release_turn_reservation", "tools/subagent/turn.rs");
}

#[test]
fn runtime_and_protocol_do_not_open_child_sessions() {
    let runtime = read_combined(&["runtime"]);
    let protocol = read_combined(&["client_protocol"]);
    for (name, src) in [
        ("runtime", runtime.as_str()),
        ("protocol", protocol.as_str()),
    ] {
        assert_absent(src, "open_child_session", name);
    }
}

#[test]
fn session_service_business_and_support_surfaces_are_separate() {
    let service = src_root().join("session/service");
    for module in ["lifecycle", "conversation", "permissions", "observation"] {
        assert!(
            service.join(format!("{module}.rs")).is_file(),
            "missing business module `{module}`"
        );
    }
    assert!(
        service.join("support.rs").is_file(),
        "missing internal support module"
    );

    let mut business = String::new();
    for module in ["lifecycle", "conversation", "permissions", "observation"] {
        business.push_str(&fs::read_to_string(service.join(format!("{module}.rs"))).unwrap());
        business.push('\n');
    }
    for support_only in [
        "fn build_runtime",
        "fn spawn_compaction_worker",
        "fn gc_stale_empty_sessions",
        "fn shutdown_cleanup",
        "fn permission_route",
        "fn entry_load_events_range",
        "fn mark_startup_settled",
    ] {
        assert_absent(&business, support_only, "SessionService business modules");
    }

    let support = fs::read_to_string(service.join("support.rs")).unwrap();
    for required in [
        "fn build_runtime",
        "fn spawn_compaction_worker",
        "fn gc_stale_empty_sessions",
        "fn shutdown_cleanup",
        "fn permission_route",
        "fn entry_load_events_range",
        "fn mark_startup_settled",
    ] {
        assert!(
            support.contains(required),
            "support surface must own `{required}`"
        );
    }
}

#[test]
fn production_ui_and_subagent_entrypoints_use_the_service() {
    let connection = fs::read_to_string(src_root().join("client_protocol/connection.rs")).unwrap();
    assert_absent(
        production_part(&connection),
        "SessionManager",
        "client_protocol/connection.rs",
    );

    for path in [
        "tools/subagent/turn.rs",
        "tools/subagent/launch.rs",
        "tools/subagent/send.rs",
        "tools/subagent/stop.rs",
        "runtime/subagent_auto_turn.rs",
        "runtime/bash_auto_turn.rs",
        "runtime/custom_auto_turn.rs",
        "runtime/pending_auto_turn.rs",
    ] {
        let source = fs::read_to_string(src_root().join(path)).unwrap();
        assert_absent(production_part(&source), "SessionManager", path);
    }

    let cli = fs::read_to_string(src_root().join("cli/cli.rs")).unwrap();
    assert_absent(&cli, "with_turn_guard", "cli/cli.rs");
    assert!(
        cli.contains("with_service"),
        "CLI must enter through the service"
    );
}

#[test]
fn tool_core_does_not_own_subagent_completion_delivery() {
    let executor = fs::read_to_string(src_root().join("tool/executor.rs")).unwrap();
    let trait_ = fs::read_to_string(src_root().join("tool/trait_.rs")).unwrap();
    assert_absent(&executor, "session::jobs", "tool/executor.rs");
    assert_absent(&executor, "agent_subagents", "tool/executor.rs");
    assert_absent(&trait_, "agent_subagents", "tool/trait_.rs");
}
