//! P0 contract freeze: a child turn's terminal fact as observed through the
//! agent-facing tool surface.
//!
//! Four `TurnEndReason` outcomes (`Completed` / `Cancelled` / `MaxSteps` /
//! `Error`) are stored as the durable `turn/end.reason` string. The wait/stop
//! text is the agent client's rendering of that fact (ok / stopped / max
//! steps reached), not a second invented verdict.

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::subagent_fixture::SubagentHarness;
use common::scripted_provider::HangProvider;
use common::{ScriptedProvider, function_call_item};
use litecode::types::ToolSignalLevel;

fn completed_provider() -> ScriptedProvider {
    ScriptedProvider::with_text("child finished cleanly")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_child_reports_output_and_clean_exit() {
    let harness = SubagentHarness::new(Arc::new(completed_provider()));
    let child = harness.launch_reviewer("call_ok", "do the thing").await;

    let waited = harness
        .wait(
            "call_wait_ok",
            serde_json::json!({ "id": child, "sec": 10 }),
        )
        .await;
    assert_eq!(waited.level, ToolSignalLevel::Ok, "{}", waited.content);
    assert!(
        waited.content.contains("status: exited"),
        "{}",
        waited.content
    );
    assert!(
        !waited.content.contains("stopped: true"),
        "a completed child must not report stopped: {}",
        waited.content
    );
    assert!(
        !waited.content.contains("ok: false"),
        "a completed child must not report ok: false: {}",
        waited.content
    );
    assert!(
        waited.content.contains("child finished cleanly"),
        "final text must reach the wait surface: {}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_child_reports_stopped() {
    let hang = HangProvider::until_cancel();
    let started = Arc::clone(&hang.started);
    let harness = SubagentHarness::new(Arc::new(hang));
    let child = harness.launch_reviewer("call_hang", "hang forever").await;

    harness.wait_for(|| started.load(Ordering::SeqCst), "child LLM start");

    let stopped = harness.stop("call_stop_hang", &child).await;
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(
        stopped.content.contains("stopping") || stopped.content.contains("already ended"),
        "{}",
        stopped.content
    );

    let waited = harness
        .wait(
            "call_wait_hang",
            serde_json::json!({ "id": child, "sec": 10 }),
        )
        .await;
    assert!(
        waited.content.contains("status: exited"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("stopped: true"),
        "a cancelled child must report stopped: true: {}",
        waited.content
    );
    assert!(
        !waited.content.contains("ok: false"),
        "stopped is reported instead of ok: false: {}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_steps_child_reports_failure_with_reason() {
    // The child keeps calling a tool, so the loop only ends at max_steps.
    let tool_call = || {
        vec![function_call_item(
            "call_read_loop",
            "read",
            r#"{"path":"missing.txt"}"#,
            "fc_read_loop",
        )]
    };
    let provider = ScriptedProvider::with_responses(vec![
        tool_call(),
        tool_call(),
        tool_call(),
        tool_call(),
    ]);
    let harness = SubagentHarness::with_subagent_max_steps(Arc::new(provider), 2);
    let child = harness.launch_reviewer("call_max", "loop forever").await;

    let waited = harness
        .wait(
            "call_wait_max",
            serde_json::json!({ "id": child, "sec": 10 }),
        )
        .await;
    assert!(
        waited.content.contains("status: exited"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("ok: false"),
        "max-steps is a failure outcome: {}",
        waited.content
    );
    assert!(
        waited.content.contains("max steps reached"),
        "failure reason must reach the wait surface: {}",
        waited.content
    );
    assert!(
        !waited.content.contains("stopped: true"),
        "max-steps is not a stop: {}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_error_child_reports_failure_with_reason() {
    // No queued response: the first model call fails.
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_responses(vec![])));
    let child = harness.launch_reviewer("call_err", "fail fast").await;

    let waited = harness
        .wait(
            "call_wait_err",
            serde_json::json!({ "id": child, "sec": 10 }),
        )
        .await;
    assert!(
        waited.content.contains("status: exited"),
        "{}",
        waited.content
    );
    assert!(
        waited.content.contains("ok: false"),
        "provider error is a failure outcome: {}",
        waited.content
    );
    assert!(
        waited.content.contains("no response queued"),
        "failure reason must reach the wait surface: {}",
        waited.content
    );
    assert!(
        !waited.content.contains("stopped: true"),
        "provider error is not a stop: {}",
        waited.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_after_completion_reports_outcome_not_stopped() {
    let harness = SubagentHarness::new(Arc::new(completed_provider()));
    let child = harness.launch_reviewer("call_done", "quick task").await;
    harness.wait_child_settled(&child);

    let stopped = harness.stop("call_stop_done", &child).await;
    assert_eq!(stopped.level, ToolSignalLevel::Ok, "{}", stopped.content);
    assert!(
        stopped.content.contains("status: already ended"),
        "{}",
        stopped.content
    );
    assert!(
        stopped.content.contains("outcome: completed"),
        "{}",
        stopped.content
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_exit_reminder_carries_outcome_and_preview() {
    let harness = SubagentHarness::new(Arc::new(ScriptedProvider::with_text("reminder body")));
    let child = harness.launch_reviewer("call_reminder", "produce a reminder").await;
    harness.wait_mailbox(&harness.parent_id);

    let notices = harness.jobs().take_mailbox(&harness.parent_id);
    assert_eq!(notices.len(), 1, "exactly one exit notice");
    let notice = &notices[0];
    assert_eq!(notice.child_session_id, child);
    assert_eq!(notice.agent_name, "reviewer");
    assert!(notice.ok && !notice.stopped, "{notice:?}");
    assert_eq!(notice.final_text, "reminder body");

    let reminder = litecode::tools::subagent::status::format_exit_reminder(&notices, &[]);
    assert!(reminder.starts_with("<system-reminder>"), "{reminder}");
    assert!(reminder.contains("finished."), "{reminder}");
    assert!(reminder.contains("output_preview: reminder body"), "{reminder}");
}
