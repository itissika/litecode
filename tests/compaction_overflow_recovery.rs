//! PR-5: provider-confirmed context overflow gets at most one forced
//! compaction recovery, and only when the durable surface advances.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{build_runtime_with_provider, test_agent};
use litecode::client_protocol::observer::InternalEvent;
use litecode::llm::{LlmProvider, ModelRequest, RequestPurpose};
use litecode::runtime::observer::{CompactionStage, CompactionTrigger};
use litecode::session::EventType;
use litecode::types::{Item, LitecodeError, StreamEvents, assistant_text, user_text};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
enum Mode {
    RecoverOnce,
    OverflowAlways,
    MaxTokensParameterError,
}

#[derive(Clone)]
struct OverflowProvider {
    mode: Mode,
    calls: Arc<AtomicUsize>,
}

impl OverflowProvider {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl LlmProvider for OverflowProvider {
    fn endpoint(&self) -> &str {
        "overflow://test"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>, LitecodeError>> + Send + 'a>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let is_compaction = request.purpose == RequestPurpose::Compaction;
        let overflow = || {
            LitecodeError::Llm(
                "HTTP 400 context_length_exceeded: prompt is too long for the model".into(),
            )
        };
        match (self.mode, call, is_compaction) {
            (Mode::RecoverOnce, 0, false) | (Mode::OverflowAlways, 0, false) => {
                Box::pin(async move { Err(overflow()) })
            }
            (_, 1, true) => {
                common::emit_completed_terminal(&mut on_event);
                let text = common::valid_summary_for_request(request);
                Box::pin(async move { Ok(vec![assistant_text(&text)]) })
            }
            (Mode::RecoverOnce, 2, false) => {
                common::emit_completed_terminal(&mut on_event);
                Box::pin(async move { Ok(vec![assistant_text("recovered after compaction")]) })
            }
            (Mode::OverflowAlways, _, false) => Box::pin(async move { Err(overflow()) }),
            (Mode::MaxTokensParameterError, _, false) => Box::pin(async move {
                Err(LitecodeError::Llm(
                    "HTTP 400 too many tokens requested for the max_tokens parameter".into(),
                ))
            }),
            _ => Box::pin(async move {
                Err(LitecodeError::Llm(format!(
                    "unexpected provider call {call} (compaction={is_compaction})"
                )))
            }),
        }
    }
}

fn seed_large_transcript(sessions: &litecode::session::manager::SessionManager, sid: &str) {
    let seed: Vec<_> = (0..30)
        .map(|i| user_text(format!("overflow-seed-{i} {}", "word ".repeat(800))))
        .collect();
    sessions
        .insert_detail_rows(sid, &seed)
        .expect("seed transcript");
}

#[tokio::test(flavor = "current_thread")]
async fn provider_overflow_compacts_and_retries_the_main_request_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(OverflowProvider::new(Mode::RecoverOnce));
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider.clone(),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    seed_large_transcript(&sessions, &sid);
    let mut events = sessions.subscribe(&sid).expect("subscribe");
    let before = sessions.data().revision_blocking(&sid).unwrap();

    let result = runtime
        .run("continue after the provider overflow")
        .await
        .expect("one forced compaction recovery should complete the turn");
    assert_eq!(result, "recovered after compaction");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        3,
        "main overflow, one compaction call, then exactly one main retry"
    );
    assert!(
        sessions.data().revision_blocking(&sid).unwrap() > before,
        "the checkpoint must advance the durable surface"
    );
    assert!(
        sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted))
    );
    let mut started_telemetry = None;
    let mut succeeded_telemetry = None;
    while let Ok(envelope) = events.try_recv() {
        if let InternalEvent::CompactionLifecycle {
            trigger: CompactionTrigger::Overflow,
            stage,
            telemetry,
            ..
        } = envelope.event
        {
            match stage {
                CompactionStage::Started => started_telemetry = Some(telemetry),
                CompactionStage::Succeeded => succeeded_telemetry = Some(telemetry),
                CompactionStage::Failed => panic!("forced recovery must not fail compaction"),
            }
        }
    }
    let started =
        started_telemetry.expect("forced recovery must emit an Overflow/Started lifecycle");
    assert!(
        started.tokens_before.is_some()
            && started.tokens_pruned.is_some()
            && started.tokens_after.is_some(),
        "Overflow/Started must carry measured token fields: {started:?}"
    );
    assert_eq!(
        started.overflow_retry,
        Some(1),
        "Overflow/Started must identify the overflow recovery trigger"
    );
    let succeeded =
        succeeded_telemetry.expect("forced recovery must emit an Overflow/Succeeded lifecycle");
    assert!(
        succeeded.tokens_before.is_some()
            && succeeded.tokens_pruned.is_some()
            && succeeded.tokens_after.is_some(),
        "Overflow/Succeeded must carry measured token fields: {succeeded:?}"
    );
    assert!(
        succeeded.summary_attempts.is_some(),
        "Overflow/Succeeded must carry the summary attempt count: {succeeded:?}"
    );
    assert_eq!(
        succeeded.overflow_retry,
        Some(1),
        "Overflow/Succeeded must report the recovery retry"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn overflow_without_a_compactable_cut_returns_the_original_error_without_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(OverflowProvider::new(Mode::RecoverOnce));
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider.clone(),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    sessions
        .insert_detail_rows(&sid, &[user_text("one indivisible request")])
        .expect("seed transcript");

    let error = runtime
        .run("trigger provider overflow")
        .await
        .expect_err("no durable advance means no main retry");
    assert!(
        error.to_string().contains("context_length_exceeded"),
        "the original provider error must surface, got {error}"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "without surface advance there must be no retry bill"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn max_tokens_parameter_error_does_not_force_compaction_or_retry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(OverflowProvider::new(Mode::MaxTokensParameterError));
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider.clone(),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    seed_large_transcript(&sessions, &sid);
    let mut events = sessions.subscribe(&sid).expect("subscribe");

    let error = runtime
        .run("request an impossible max_tokens value")
        .await
        .expect_err("max_tokens parameter error is not context overflow");
    assert!(
        error
            .to_string()
            .contains("too many tokens requested for the max_tokens parameter"),
        "original parameter error must surface, got {error}"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "no compaction request or main retry may be billed"
    );
    assert!(
        !sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted)),
        "a parameter error must not create a checkpoint"
    );
    let mut saw_overflow_lifecycle = false;
    while let Ok(envelope) = events.try_recv() {
        if matches!(
            envelope.event,
            InternalEvent::CompactionLifecycle {
                trigger: CompactionTrigger::Overflow,
                ..
            }
        ) {
            saw_overflow_lifecycle = true;
        }
    }
    assert!(
        !saw_overflow_lifecycle,
        "a max_tokens parameter error must not start overflow compaction"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_second_overflow_is_returned_without_a_third_main_attempt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(OverflowProvider::new(Mode::OverflowAlways));
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider.clone(),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    seed_large_transcript(&sessions, &sid);

    let error = runtime
        .run("overflow again after recovery")
        .await
        .expect_err("the single recovery budget must be exhausted");
    assert!(
        error.to_string().contains("context_length_exceeded"),
        "second overflow returns the original provider error, got {error}"
    );
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        3,
        "only one main retry is allowed: overflow + compaction + overflow"
    );
    assert!(
        sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted))
    );
}
