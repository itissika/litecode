//! Hub wait / stop / mailbox contract without a nested LLM runtime.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::tool::Tool;
use crate::tool::trait_::ToolExecutionContext;
use crate::tools::subagent::status;
use crate::tools::subagent::{
    MAX_SUBAGENTS_PER_PARENT, SubagentHub, SubagentStopTool, SubagentWaitTool, WaitOutcome,
};
use crate::types::ToolSignalLevel;

fn exec_wait(hub: &Arc<SubagentHub>, sid: &str, input: serde_json::Value) -> String {
    let tool = SubagentWaitTool::new(Arc::clone(hub));
    tool.set_active_session(sid.to_string());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(tool.execute(
        input,
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::path::PathBuf::from("."),
            call_id: "wait_call".into(),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: sid.into(),
            session: None,
        },
    ))
    .content
}

#[test]
fn wait_timeout_lists_running() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "review this");
    let text = exec_wait(&hub, "p1", serde_json::json!({"id": "child-a", "sec": 1}));
    let jobs = hub.running("p1");
    assert_eq!(text, status::format_waited_status(&jobs));
    assert!(hub.is_alive("child-a"));
}

#[test]
fn wait_sees_finish() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
    hub.finish("child-a", true, false, "done".into());
    let text = exec_wait(&hub, "p1", serde_json::json!({"id": "child-a", "sec": 5}));
    assert!(text.contains("status: exited"));
    assert!(text.contains("child-a"));
    assert!(text.contains("done"));
}

#[test]
fn wait_cancel_does_not_stop() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
    let tool = SubagentWaitTool::new(Arc::clone(&hub));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(tool.execute(
        serde_json::json!({"id": "child-a", "sec": 30}),
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::path::PathBuf::from("."),
            call_id: "w".into(),
            cancel,
            output_limit: usize::MAX,
            session_id: "p1".into(),
            session: None,
        },
    ));
    assert_eq!(result.level, ToolSignalLevel::Error);
    assert!(result.content.contains("cancelled"));
    assert!(hub.is_alive("child-a"));
}

#[test]
fn stop_unknown_id() {
    let hub = Arc::new(SubagentHub::new());
    let tool = SubagentStopTool::new(Arc::clone(&hub));
    tool.set_active_session("p1".into());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(tool.execute(
        serde_json::json!({"id": "missing"}),
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::path::PathBuf::from("."),
            call_id: "s".into(),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: "p1".into(),
            session: None,
        },
    ));
    assert_eq!(result.level, ToolSignalLevel::Error);
    assert!(result.content.contains("not found"));
}

#[test]
fn stop_on_finished_reports_outcome_not_stopped() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
    hub.finish("child-a", true, false, "done".into());
    let tool = SubagentStopTool::new(Arc::clone(&hub));
    tool.set_active_session("p1".into());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = rt.block_on(tool.execute(
        serde_json::json!({"id": "child-a"}),
        ToolExecutionContext {
            path_mode: crate::workspace::ToolPathMode::All,
            workspace_root: std::path::PathBuf::from("."),
            call_id: "s".into(),
            cancel: CancellationToken::new(),
            output_limit: usize::MAX,
            session_id: "p1".into(),
            session: None,
        },
    ));
    assert_eq!(result.level, ToolSignalLevel::Ok);
    assert!(
        result.content.contains("status: already ended"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("outcome: completed"),
        "{}",
        result.content
    );
}

#[test]
fn parent_isolation() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "a");
    hub.insert_running_for_test("p2", "child-b", "reviewer", "b");
    let out = hub.wait(
        "p1",
        Some("child-b"),
        Some(Duration::from_millis(20)),
        &CancellationToken::new(),
        false,
    );
    assert!(matches!(out, WaitOutcome::UnknownId(_)));
    assert_eq!(hub.running("p1").len(), 1);
    assert_eq!(hub.running("p2").len(), 1);
}

#[test]
fn slot_cap_fail_closed() {
    let hub = SubagentHub::new();
    for i in 0..MAX_SUBAGENTS_PER_PARENT {
        hub.insert_running_for_test("p1", &format!("c{i}"), "reviewer", "x");
    }
    let overflow = hub.try_acquire_slot("p1").unwrap_err();
    assert!(overflow.contains("capacity exceeded"), "err: {overflow}");
    assert!(
        overflow.contains(&format!(
            "running: {MAX_SUBAGENTS_PER_PARENT}/{MAX_SUBAGENTS_PER_PARENT}"
        )),
        "capacity error must list the running children: {overflow}"
    );
}

#[test]
fn stop_then_wait_sees_exited() {
    let hub = Arc::new(SubagentHub::new());
    hub.insert_running_for_test("p1", "child-a", "reviewer", "go");
    let hub_finish = Arc::clone(&hub);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        hub_finish.finish("child-a", false, true, "stopped".into());
    });
    let notice = hub.stop("p1", "child-a").expect("stop");
    assert_eq!(notice.child_session_id, "child-a");
    let text = exec_wait(&hub, "p1", serde_json::json!({"id": "child-a", "sec": 1}));
    assert!(text.contains("status: exited") || text.contains("not found"));
}
