//! Subagent tools expose durable Session turn facts without a second outcome model.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use common::ScriptedProvider;
use common::scripted_provider::HangProvider;
use common::subagent_fixture::SubagentHarness;
use litecode::types::ToolSignalLevel;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_child_result_is_read_from_the_durable_turn() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text(
        "child finished cleanly",
    )));
    let child = harness.launch_reviewer("call_ok", "do the thing").await;
    harness.wait_child_settled(&child);

    let result = harness
        .sessions
        .data()
        .latest_turn_result_blocking(&child)
        .unwrap()
        .expect("turn result");
    assert_eq!(result.reason, "completed");
    assert_eq!(result.output, "child finished cleanly");
    assert!(result.transcript_path.ends_with(&format!("{child}.md")));
    assert!(result.start_line.is_some() && result.end_line.is_some());

    let listed = harness.list("call_list_idle").await;
    assert_eq!(listed.level, ToolSignalLevel::Ok, "{}", listed.content);
    assert!(listed.content.contains(&format!("- id: {child}")), "{}", listed.content);
    assert!(listed.content.contains("agent: reviewer"), "{}", listed.content);
    assert!(listed.content.contains("responsibility: test"), "{}", listed.content);
    assert!(listed.content.contains("last_send:"), "{}", listed.content);
    assert!(listed.content.contains("state: idle"), "{}", listed.content);
    assert!(listed.content.contains("reason: completed"), "{}", listed.content);
    assert!(!listed.content.contains("turn_age:"), "{}", listed.content);

    let waited = harness
        .wait(
            "call_wait_idle",
            serde_json::json!({ "ids": [child.clone(), "missing-id"] }),
        )
        .await;
    assert_eq!(waited.level, ToolSignalLevel::Ok, "{}", waited.content);
    assert!(waited.content.contains("status: settled"), "{}", waited.content);
    assert!(waited.content.contains("reason: completed"), "{}", waited.content);
    assert!(waited.content.contains("responsibility: test"), "{}", waited.content);
    assert!(waited.content.contains("skipped: 1"), "{}", waited.content);
    assert!(waited.content.contains("skipped_id: missing-id"), "{}", waited.content);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wait_freezes_a_child_turn_and_returns_its_session_result() {
    let hang = HangProvider::until_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_hang", "hang forever").await;
    harness.wait_for(|| started.load(Ordering::SeqCst), "child LLM start");

    let listed = harness.list("call_list_run").await;
    assert_eq!(listed.level, ToolSignalLevel::Ok, "{}", listed.content);
    assert!(listed.content.contains("state: running"), "{}", listed.content);
    assert!(listed.content.contains("turn_age:"), "{}", listed.content);
    assert!(listed.content.contains("step:"), "{}", listed.content);

    let (waited, stopped) = tokio::join!(
        harness.wait(
            "call_wait_hang",
            serde_json::json!({ "ids": [child.clone()] }),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            harness.stop("call_stop_hang", &child).await
        }
    );
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(stopped.content.contains("status: stop_requested"));
    assert_eq!(waited.level, ToolSignalLevel::Ok, "{}", waited.content);
    assert!(
        waited.content.contains("status: settled"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("reason: cancelled"),
        "{}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_after_completion_returns_the_last_durable_result() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("done")));
    let child = harness.launch_reviewer("call_done", "quick task").await;
    harness.wait_child_settled(&child);

    let stopped = harness.stop("call_stop_done", &child).await;
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(stopped.content.contains("status: already ended"));
    assert!(stopped.content.contains("reason: completed"));
    assert!(stopped.content.contains("responsibility: test"));
    assert!(stopped.content.contains("output:\ndone"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completion_reminder_resolves_the_reference_at_delivery_time() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("reminder body")));
    let child = harness
        .launch_reviewer("call_reminder", "produce a reminder")
        .await;
    harness.wait_mailbox(&harness.parent_id);

    let completions = harness
        .runtime
        .subagent_hub
        .take_completions(&harness.parent_id);
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].child_session_id, child);
    let reminder = litecode::tools::subagent::status::format_completion_reminder(
        &harness.sessions,
        &completions,
    );
    assert!(reminder.starts_with("<system-reminder>"));
    assert!(reminder.contains("source: subagent"));
    assert!(reminder.contains("reason: completed"));
    assert!(reminder.contains("responsibility: test"));
    assert!(reminder.contains("output:\nreminder body"));
}
