//! P5 acceptance: `subagent_send` continues an existing child session through
//! the same reserve → spawn_turn → start_turn sequence a human message uses.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::ScriptedProvider;
use common::scripted_provider::HangProvider;
use common::subagent_fixture::SubagentHarness;
use litecode::session::event::EventType;
use litecode::tool::Tool;
use litecode::types::ToolSignalLevel;

fn turn_starts(harness: &SubagentHarness, child_id: &str) -> usize {
    harness
        .sessions
        .data()
        .events_blocking(child_id)
        .expect("child events")
        .iter()
        .filter(|event| event.event_type == EventType::TurnStart)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_to_idle_child_runs_another_turn() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_texts(&[
        "first turn",
        "second turn",
    ])));
    let child = harness.launch_reviewer("call_launch", "first").await;
    harness.wait_child_settled(&child);

    let sent = harness.send("call_send", &child, "please continue").await;
    assert_eq!(sent.level, ToolSignalLevel::Ok, "{}", sent.content);
    assert!(sent.content.contains("status: running"), "{}", sent.content);
    assert!(sent.content.contains(&child), "{}", sent.content);

    let waited = harness
        .wait(
            "call_wait_send",
            serde_json::json!({ "ids": [child.clone()] }),
        )
        .await;
    assert!(
        waited.content.contains("status: settled"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("second turn"),
        "the wait must surface this turn's outcome: {}",
        waited.content
    );
    assert!(!waited.content.contains("first turn"), "{}", waited.content);
    assert_eq!(turn_starts(&harness, &child), 2, "child owns both turns");

    // The parent session stays untouched: the message went to the child log.
    let parent_events = harness
        .sessions
        .data()
        .events_blocking(&harness.parent_id)
        .expect("parent events");
    assert!(
        !parent_events
            .iter()
            .any(|event| { event.data.to_string().contains("please continue") }),
        "child messages must not leak into the parent log"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_to_busy_child_fails_without_starting_a_turn() {
    let hang = HangProvider::ignore_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_busy", "hang").await;
    harness.wait_for(|| started.load(Ordering::SeqCst), "child LLM start");

    let sent = harness.send("call_send_busy", &child, "again").await;
    assert_eq!(sent.level, ToolSignalLevel::Error, "{}", sent.content);
    assert!(sent.content.contains("already running"), "{}", sent.content);
    assert_eq!(turn_starts(&harness, &child), 1, "no second turn may start");
    assert!(
        harness.sessions.is_turn_running_blocking(&child),
        "a rejected send must not stop the first running turn"
    );
    assert!(
        !harness.runtime.subagent_hub.has_pending(&harness.parent_id),
        "a rejected send must not settle the first turn's exit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_rejects_unknown_and_foreign_ids() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("ok")));

    let unknown = harness.send("call_unknown", "01NOTACHILD", "hello").await;
    assert_eq!(unknown.level, ToolSignalLevel::Error);
    assert!(
        unknown.content.contains("not a child session"),
        "{}",
        unknown.content
    );

    // The parent session itself is not a child of itself.
    let self_send = harness.send("call_self", &harness.parent_id, "hello").await;
    assert_eq!(self_send.level, ToolSignalLevel::Error);
    assert!(
        self_send.content.contains("not a child session"),
        "{}",
        self_send.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_then_stop_cancels_the_new_turn() {
    let hang = HangProvider::until_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_first", "first").await;
    harness.wait_for(|| started.load(Ordering::SeqCst), "first turn start");
    harness.stop("call_stop_first", &child).await;
    harness.wait_child_settled(&child);

    let sent = harness.send("call_send", &child, "continue").await;
    assert_eq!(sent.level, ToolSignalLevel::Ok, "{}", sent.content);
    harness.wait_for(|| turn_starts(&harness, &child) == 2, "second turn start");

    let stopped = harness.stop("call_stop", &child).await;
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);

    let waited = harness
        .wait(
            "call_wait_stopped",
            serde_json::json!({ "ids": [child.clone()] }),
        )
        .await;
    assert!(
        waited.content.contains("reason: cancelled"),
        "{}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_sends_only_one_wins() {
    let hang = HangProvider::until_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_first", "first").await;
    harness.wait_for(|| started.load(Ordering::SeqCst), "first turn start");
    harness.stop("call_stop_first", &child).await;
    harness.wait_child_settled(&child);

    let first = harness.send("call_send_a", &child, "a");
    let second = harness.send("call_send_b", &child, "b");
    let (first, second) = tokio::join!(first, second);
    let results = [&first, &second];
    let ok = results
        .iter()
        .filter(|result| result.level == ToolSignalLevel::Ok)
        .count();
    let busy = results
        .iter()
        .filter(|result| result.content.contains("already running"))
        .count();
    assert_eq!(ok, 1, "exactly one send may win: {results:?}");
    assert_eq!(busy, 1, "the other must report the busy child: {results:?}");
    // Clean up the winner's hanging turn.
    harness.stop("call_stop_winner", &child).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn send_schema_requires_id_and_message() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("ok")));
    let tool = harness.send_tool();
    assert_eq!(
        tool.schema()["required"],
        serde_json::json!(["id", "message"])
    );
    assert_eq!(tool.name(), "subagent_send");
}
