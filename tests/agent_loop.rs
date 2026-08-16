//! Product-path agent loop regressions on authority `Item` / `Transcript`.
//!
//! Restored spirit of `docs/phase1-removed-tests/integration/agent_loop.rs`
//! without Message/ContentBlock/StreamOutput.

mod common;

use common::fake_deps::{FakeAgentDeps, assistant_text_item, function_call_item};
use litecode::agent;
use litecode::types::{Item, item_text_preview, user_text};

#[tokio::test(flavor = "current_thread")]
async fn single_text_response_stops_loop() {
    let mut deps = FakeAgentDeps::with_text_response("done");
    let mut transcript = vec![user_text("hi")];

    let outcome = agent::run(&mut deps, &mut transcript).await;
    match outcome {
        litecode::agent::TurnOutcome::Completed { final_text } => {
            assert_eq!(final_text, "done");
        }
        other => panic!("expected Completed, got {other:?}"),
    }
    assert!(
        transcript
            .iter()
            .any(|item| { matches!(item, Item::Message(_)) && item_text_preview(item) == "done" }),
        "transcript must contain assistant text Item: {transcript:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn function_call_then_text_grows_transcript() {
    let mut deps = FakeAgentDeps::with_responses(vec![
        vec![function_call_item(
            "call_1",
            "read",
            r#"{"path":"x"}"#,
            "fc_1",
        )],
        vec![assistant_text_item("final", "msg_final")],
    ]);
    let mut transcript = vec![user_text("go")];
    let before = transcript.len();

    let outcome = agent::run(&mut deps, &mut transcript).await;
    match outcome {
        litecode::agent::TurnOutcome::Completed { final_text } => {
            assert_eq!(final_text, "final");
        }
        other => panic!("expected Completed, got {other:?}"),
    }
    assert!(
        transcript.len() > before + 1,
        "tool round must grow transcript (before={before}, after={})",
        transcript.len()
    );
    assert!(
        transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCall(_))),
        "FunctionCall Item must remain in transcript"
    );
    assert!(
        transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(_))),
        "execute_tools must append FunctionCallOutput"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn tool_step_persists_function_call_before_output() {
    let mut deps = FakeAgentDeps::with_responses(vec![
        vec![function_call_item(
            "call_1",
            "read",
            r#"{"path":"x"}"#,
            "fc_1",
        )],
        vec![assistant_text_item("final", "msg_final")],
    ]);
    let mut transcript = vec![user_text("go")];

    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );

    let log = deps.persist_log.borrow();
    assert!(
        log.len() >= 2,
        "tool step must persist at least twice (call then output): {log:?}"
    );
    // First persist after model tool output: FunctionCall present, no FunctionCallOutput yet.
    assert!(
        log[0].types.iter().any(|t| *t == "function_call"),
        "first persist must include function_call: {log:?}"
    );
    assert!(
        !log[0].types.iter().any(|t| *t == "function_call_output"),
        "first persist must not yet include function_call_output: {log:?}"
    );
    // Second persist after execute_tools: FunctionCallOutput present.
    assert!(
        log[1].types.iter().any(|t| *t == "function_call_output"),
        "second persist must include function_call_output: {log:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn max_steps_with_perpetual_tools() {
    // Enough perpetual tool responses; max_steps=1 ⇒ MaxSteps on the second step.
    let mut deps = FakeAgentDeps::with_responses(
        (0..5)
            .map(|i| {
                vec![function_call_item(
                    &format!("call_{i}"),
                    "read",
                    "{}",
                    &format!("fc_{i}"),
                )]
            })
            .collect(),
    );
    deps.max_steps = 1;
    deps.stop_on_text = false;

    let mut transcript = vec![user_text("loop")];
    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::MaxSteps { .. }),
        "expected MaxSteps, got {outcome:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_breaks_loop() {
    let mut deps = FakeAgentDeps::with_text_response("never returned");
    deps.cancelled.set(true);
    let mut transcript = vec![user_text("stop")];

    let outcome = agent::run(&mut deps, &mut transcript).await;
    match outcome {
        litecode::agent::TurnOutcome::Cancelled { final_text } => {
            assert!(final_text.is_empty());
        }
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert!(
        deps.persist_log.borrow().is_empty(),
        "cancel before call_model must not persist a model row"
    );
    assert!(
        !transcript
            .iter()
            .any(|item| matches!(item, Item::Message(_))
                && item_text_preview(item) == "never returned"),
        "cancel before stream must not leave a model Item: {transcript:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_after_model_persists_output() {
    let mut deps = FakeAgentDeps::with_text_response("partial");
    deps.cancel_after_model = true;
    let mut transcript = vec![user_text("hi")];

    let outcome = agent::run(&mut deps, &mut transcript).await;
    match outcome {
        litecode::agent::TurnOutcome::Cancelled { final_text } => {
            assert_eq!(final_text, "partial");
        }
        other => panic!("expected Cancelled, got {other:?}"),
    }
    assert!(
        transcript
            .iter()
            .any(|item| matches!(item, Item::Message(_)) && item_text_preview(item) == "partial"),
        "cancelled after model must keep sealed Item: {transcript:?}"
    );
    assert!(
        !deps.persist_log.borrow().is_empty(),
        "sealed model output must be persisted"
    );
    assert_eq!(deps.execute_calls.get(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_after_stream_skips_tool_execution() {
    let mut deps = FakeAgentDeps::with_responses(vec![vec![function_call_item(
        "call_1", "read", "{}", "fc_1",
    )]]);
    deps.cancel_after_model = true;
    let mut transcript = vec![user_text("go")];
    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::Cancelled { .. }),
        "revert/cancel after stream must interrupt before tools, got {outcome:?}"
    );
    assert_eq!(deps.execute_calls.get(), 0);
    assert!(
        transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(_))),
        "open FunctionCall still in the working set must be sealed interrupted"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn incomplete_function_call_is_not_executed() {
    let mut deps = FakeAgentDeps::with_responses(vec![vec![Item::FunctionCall(
        litecode::authority::responses::FunctionToolCall {
            call_id: "call_1".into(),
            name: "read".into(),
            arguments: "{\"path\":\"x\"".into(),
            id: Some("fc_1".into()),
            status: Some(litecode::authority::responses::OutputStatus::Incomplete),
            namespace: None,
        },
    )]]);
    let mut transcript = vec![user_text("go")];
    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::Cancelled { .. }),
        "incomplete FC must stop the turn, got {outcome:?}"
    );
    assert_eq!(deps.execute_calls.get(), 0);
    assert!(
        transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCall(_))),
        "FunctionCall must remain"
    );
    assert!(
        transcript
            .iter()
            .any(|i| matches!(i, Item::FunctionCallOutput(_))),
        "interrupted FunctionCallOutput must be appended"
    );
    assert!(
        !transcript.iter().any(|i| {
            matches!(i, Item::FunctionCallOutput(out) if match &out.output {
                litecode::authority::responses::FunctionCallOutput::Text(s) => s == "fake result",
                _ => false,
            })
        }),
        "must not execute the incomplete call"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn persist_failure_rolls_back_working_set() {
    let mut deps = FakeAgentDeps::with_text_response("gone");
    deps.persist_fail = true;
    let mut transcript = vec![user_text("hi")];
    let before = transcript.len();
    let outcome = agent::run(&mut deps, &mut transcript).await;
    assert!(
        matches!(outcome, litecode::agent::TurnOutcome::Error(_)),
        "expected Error, got {outcome:?}"
    );
    assert_eq!(
        transcript.len(),
        before,
        "persist failure must not leave model output in the working set: {transcript:?}"
    );
}
