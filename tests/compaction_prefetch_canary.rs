//! Canary for the default background compaction branch: it starts at the 0.8
//! prepare line, freezes its plan, and commits its own checkpoint. The main
//! turn keeps going while the branch is in flight. Set
//! LITECODE_COMPACT_PREFETCH=off to opt out.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::runtime::{build_runtime_with_provider, test_agent};
use litecode::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, OutputStatus,
};
use litecode::context_pipeline::compact::TOOL_RESULT_PRUNE_MARKER;
use litecode::llm::{LlmProvider, ModelRequest, RequestPurpose};
use litecode::platform_knobs::CompactionPrefetchMode;
use litecode::runtime::observer::{CompactionStage, CompactionTrigger, InternalEvent};
use litecode::session::working::WorkingPayload;
use litecode::types::{Item, Result, StreamEvents, item_text_preview, user_text};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
struct PrefetchCanaryProvider {
    main_calls: Arc<AtomicUsize>,
    compaction_calls: Arc<AtomicUsize>,
}

impl LlmProvider for PrefetchCanaryProvider {
    fn endpoint(&self) -> &str {
        "canary://compaction-prefetch"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            let summary = common::valid_summary_for_request(request);
            let items = vec![common::assistant_text_item(&summary, "msg_canary_summary")];
            for (index, item) in items.iter().enumerate() {
                common::recording_provider::emit_in_progress_added(
                    &mut on_event,
                    item,
                    index as u32,
                );
            }
            common::emit_completed_terminal(&mut on_event);
            return Box::pin(async move { Ok(items) });
        }

        let index = self.main_calls.fetch_add(1, Ordering::SeqCst);
        let items = if index == 0 {
            vec![Item::FunctionCall(FunctionToolCall {
                call_id: "call_sleep".into(),
                name: "bash".into(),
                arguments: "{\"command\":\"sleep 1\"}".into(),
                id: Some("item_call_sleep".into()),
                status: Some(OutputStatus::Completed),
                namespace: None,
            })]
        } else {
            vec![common::assistant_text_item(
                "canary done",
                "msg_canary_done",
            )]
        };
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[derive(Clone)]
struct GatedSummaryProvider {
    main_calls: Arc<AtomicUsize>,
    compaction_calls: Arc<AtomicUsize>,
    release: Arc<Notify>,
}

impl Default for GatedSummaryProvider {
    fn default() -> Self {
        Self {
            main_calls: Arc::new(AtomicUsize::new(0)),
            compaction_calls: Arc::new(AtomicUsize::new(0)),
            release: Arc::new(Notify::new()),
        }
    }
}

impl LlmProvider for GatedSummaryProvider {
    fn endpoint(&self) -> &str {
        "canary://gated-summary"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            let summary = common::valid_summary_for_request(request);
            let items = vec![common::assistant_text_item(&summary, "msg_gated_summary")];
            let release = Arc::clone(&self.release);
            return Box::pin(async move {
                release.notified().await;
                for (index, item) in items.iter().enumerate() {
                    common::recording_provider::emit_in_progress_added(
                        &mut on_event,
                        item,
                        index as u32,
                    );
                }
                common::emit_completed_terminal(&mut on_event);
                Ok(items)
            });
        }

        self.main_calls.fetch_add(1, Ordering::SeqCst);
        let items = vec![common::assistant_text_item(
            "gated final answer",
            "msg_gated_final",
        )];
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[derive(Clone, Default)]
struct FinalAnswerPrefetchProvider {
    main_calls: Arc<AtomicUsize>,
    compaction_calls: Arc<AtomicUsize>,
}

impl LlmProvider for FinalAnswerPrefetchProvider {
    fn endpoint(&self) -> &str {
        "canary://final-answer-prefetch"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            let summary = common::valid_summary_for_request(request);
            let items = vec![common::assistant_text_item(&summary, "msg_final_summary")];
            for (index, item) in items.iter().enumerate() {
                common::recording_provider::emit_in_progress_added(
                    &mut on_event,
                    item,
                    index as u32,
                );
            }
            common::emit_completed_terminal(&mut on_event);
            return Box::pin(async move { Ok(items) });
        }

        self.main_calls.fetch_add(1, Ordering::SeqCst);
        let items = vec![common::assistant_text_item(
            "final answer without a tool call",
            "msg_final_answer",
        )];
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[derive(Clone, Default)]
struct RetryPrefetchProvider {
    main_calls: Arc<AtomicUsize>,
    compaction_calls: Arc<AtomicUsize>,
    compaction_inputs: Arc<std::sync::Mutex<Vec<Vec<Item>>>>,
}

impl LlmProvider for RetryPrefetchProvider {
    fn endpoint(&self) -> &str {
        "canary://retry-prefetch"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            self.compaction_inputs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(request.input.clone());
            let text = if self.compaction_calls.load(Ordering::SeqCst) == 1 {
                String::new()
            } else {
                common::valid_summary_for_request(request)
            };
            let items = vec![common::assistant_text_item(&text, "msg_retry_summary")];
            for (index, item) in items.iter().enumerate() {
                common::recording_provider::emit_in_progress_added(
                    &mut on_event,
                    item,
                    index as u32,
                );
            }
            common::emit_completed_terminal(&mut on_event);
            return Box::pin(async move { Ok(items) });
        }

        self.main_calls.fetch_add(1, Ordering::SeqCst);
        let items = vec![common::assistant_text_item(
            "retry final answer",
            "msg_retry_final",
        )];
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[derive(Clone, Default)]
struct ParitySummaryProvider {
    compaction_inputs: Arc<std::sync::Mutex<Vec<Vec<Item>>>>,
}

impl LlmProvider for ParitySummaryProvider {
    fn endpoint(&self) -> &str {
        "canary://parity-summary"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_inputs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(request.input.clone());
            let text = common::valid_summary_for_request(request);
            let items = vec![common::assistant_text_item(&text, "msg_parity_summary")];
            for (index, item) in items.iter().enumerate() {
                common::recording_provider::emit_in_progress_added(
                    &mut on_event,
                    item,
                    index as u32,
                );
            }
            common::emit_completed_terminal(&mut on_event);
            return Box::pin(async move { Ok(items) });
        }

        let items = vec![common::assistant_text_item(
            "parity final answer",
            "msg_parity_final",
        )];
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[derive(Clone, Default)]
struct FailingSummaryProvider {
    main_calls: Arc<AtomicUsize>,
    compaction_calls: Arc<AtomicUsize>,
}

impl LlmProvider for FailingSummaryProvider {
    fn endpoint(&self) -> &str {
        "canary://failing-summary"
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
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        if request.purpose == RequestPurpose::Compaction {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            let items = vec![common::assistant_text_item("", "empty_summary")];
            for (index, item) in items.iter().enumerate() {
                common::recording_provider::emit_in_progress_added(
                    &mut on_event,
                    item,
                    index as u32,
                );
            }
            common::emit_completed_terminal(&mut on_event);
            return Box::pin(async move { Ok(items) });
        }
        self.main_calls.fetch_add(1, Ordering::SeqCst);
        let items = vec![common::assistant_text_item(
            "unexpected main",
            "unexpected_main",
        )];
        for (index, item) in items.iter().enumerate() {
            common::recording_provider::emit_in_progress_added(&mut on_event, item, index as u32);
        }
        common::emit_completed_terminal(&mut on_event);
        Box::pin(async move { Ok(items) })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn background_summary_commits_its_checkpoint() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(PrefetchCanaryProvider::default());
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["bash".into()], "auto", 5),
        provider.clone(),
    );
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    // ~105k estimated tokens: above the 0.8 prepare line (102.4k) and below the
    // hard send line (119.8k - 4k). Random letters avoid BPE runs, unlike a
    // repeated character, so the local estimate tracks the real token count.
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 1;
            let mut text = format!("seed-{i} ");
            for _ in 0..22_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");
    let result = runtime
        .run_with_turn("go".into(), "canary-turn", 5)
        .await
        .expect("turn succeeds");
    assert!(result.contains("canary done"), "{result}");
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        1,
        "the background summary must run exactly once"
    );
    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        2,
        "the turn keeps calling the model while the branch is in flight"
    );

    let rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set");
    assert!(
        rows.iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "the prepared summary must be committed: {rows:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn branch_commits_after_a_final_answer_without_a_next_boundary() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(GatedSummaryProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    let mut events = sessions.subscribe(&sid).expect("session event stream");
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 1;
            let mut text = format!("final-seed-{i} ");
            for _ in 0..20_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    let result = runtime
        .run_with_turn("go".into(), "final-answer-turn", 5)
        .await
        .expect("turn succeeds");
    assert!(result.contains("gated final answer"), "{result}");
    assert_eq!(provider.main_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        1,
        "the branch must be in flight while the main turn is running"
    );

    let mut saw_started = false;
    while let Ok(envelope) = events.try_recv() {
        if let InternalEvent::CompactionLifecycle { trigger, stage, .. } = envelope.event {
            if trigger == CompactionTrigger::Auto && stage == CompactionStage::Started {
                saw_started = true;
            }
        }
    }
    assert!(
        saw_started,
        "Started must be visible before the gated summary finishes"
    );
    let active = sessions
        .compaction_state_blocking(&sid)
        .active
        .expect("the background branch must own the session compaction state");
    assert_eq!(active.trigger, CompactionTrigger::Auto);

    provider.release.notify_one();

    // The main turn has no next boundary. The branch must finish and commit on
    // its own after the turn returns.
    let mut committed = false;
    for _ in 0..200 {
        if sessions
            .data()
            .working_set_blocking(&sid)
            .expect("working set")
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_)))
        {
            committed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        committed,
        "the independent branch must commit after the main turn ends"
    );

    let mut saw_succeeded = false;
    while let Ok(envelope) = events.try_recv() {
        if let InternalEvent::CompactionLifecycle { trigger, stage, .. } = envelope.event {
            if trigger == CompactionTrigger::Auto && stage == CompactionStage::Succeeded {
                saw_succeeded = true;
            }
        }
    }
    assert!(saw_succeeded, "the branch must emit Succeeded after commit");
    assert!(
        sessions.compaction_state_blocking(&sid).active.is_none(),
        "Succeeded must clear the one compaction state"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn branch_announces_the_anchor_before_the_summary_and_persists_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(GatedSummaryProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    let mut events = sessions.subscribe(&sid).expect("session event stream");
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 1;
            let mut text = format!("anchor-seed-{i} ");
            for _ in 0..20_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");
    let last_seed = sessions.entry_wire_seq_cursor(&sid).0 as u64;

    let result = runtime
        .run_with_turn("go".into(), "anchor-turn", 5)
        .await
        .expect("turn succeeds");
    assert!(result.contains("gated final answer"), "{result}");

    // The anchor is known while the summary is still gated: a reader can place
    // the line before the checkpoint row exists.
    let mut started_at = None;
    while let Ok(envelope) = events.try_recv() {
        if let InternalEvent::CompactionLifecycle {
            trigger,
            stage: CompactionStage::Started,
            at,
            ..
        } = envelope.event
        {
            if trigger == CompactionTrigger::Auto {
                started_at = at;
            }
        }
    }
    let at = started_at.expect("Started must carry the frozen anchor");
    assert!(
        at >= last_seed,
        "anchor {at} must be at or after the last seeded row {last_seed}"
    );
    let active = sessions
        .compaction_state_blocking(&sid)
        .active
        .expect("the emitted Started must have entered the session state");
    assert_eq!(active.at, Some(at));

    provider.release.notify_one();
    let mut committed_seq = None;
    for _ in 0..200 {
        let (_, next) = sessions.entry_wire_seq_cursor(&sid);
        if let Ok(rows) = sessions.entry_load_events_range(&sid, 0, next) {
            if let Some(event) = rows
                .iter()
                .find(|event| event.event_type == litecode::session::EventType::Compacted)
            {
                committed_seq = Some(event.seq);
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let checkpoint_seq = committed_seq.expect("the branch must commit a checkpoint");

    let (_, next) = sessions.entry_wire_seq_cursor(&sid);
    let rows = sessions
        .entry_load_events_range(&sid, 0, next)
        .expect("event log");
    let checkpoint = rows
        .iter()
        .find(|event| event.seq == checkpoint_seq)
        .expect("checkpoint row");
    let body: litecode::session::CompactedBody =
        serde_json::from_value(checkpoint.data.clone()).expect("checkpoint body");
    assert_eq!(
        body.at,
        Some(at),
        "the durable row must keep the anchor the branch announced"
    );
    assert!(
        checkpoint_seq > at,
        "the checkpoint row lands at commit ({checkpoint_seq}), after its anchor ({at})"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn committed_branch_is_loaded_by_the_next_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(FinalAnswerPrefetchProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 201;
            let mut text = format!("next-turn-seed-{i} ");
            for _ in 0..20_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    runtime
        .run_with_turn("first".into(), "next-turn-1", 5)
        .await
        .expect("first turn succeeds");
    let mut saw_checkpoint = false;
    for _ in 0..200 {
        if sessions
            .data()
            .working_set_blocking(&sid)
            .expect("working set")
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_)))
        {
            saw_checkpoint = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        saw_checkpoint,
        "the branch must commit before the next turn"
    );
    // Let the branch return its outcome so the next turn exercises the
    // finished-branch settlement path, not the in-flight take-ready path.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    runtime
        .run_with_turn("second".into(), "next-turn-2", 5)
        .await
        .expect("the next turn loads the committed checkpoint");
    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        2,
        "the next turn must send a main request"
    );
    let rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set after the next turn");
    assert!(
        rows.iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "the checkpoint remains the durable prefix"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn branch_retries_the_same_prefix_and_cut() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(RetryPrefetchProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 1;
            let mut text = format!("retry-seed-{i} ");
            for _ in 0..20_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    runtime
        .run_with_turn("go".into(), "retry-turn", 5)
        .await
        .expect("turn succeeds");
    let mut saw_checkpoint = false;
    for _ in 0..200 {
        if sessions
            .data()
            .working_set_blocking(&sid)
            .expect("working set")
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_)))
        {
            saw_checkpoint = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        saw_checkpoint,
        "the retried plan must commit its checkpoint"
    );
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        2,
        "one failed attempt plus one retry"
    );
    let inputs = provider
        .compaction_inputs
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(inputs.len(), 2);
    assert_eq!(
        inputs[0], inputs[1],
        "the retry must use the exact same prefix, not a replanned cut"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn branch_summary_input_matches_foreground_compaction() {
    let seed: Vec<_> = (0..9)
        .map(|i| {
            let mut state = i as u64 + 101;
            let mut text = format!("parity-seed-{i} ");
            for _ in 0..20_500 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();

    let off_dir = tempfile::tempdir().expect("off tempdir");
    let off_provider = Arc::new(ParitySummaryProvider::default());
    let mut off = build_runtime_with_provider(
        off_dir.path(),
        test_agent(vec![], "auto", 5),
        off_provider.clone(),
    );
    off.set_compaction_prefetch_mode(CompactionPrefetchMode::Off);
    let off_sid = off.session_id.clone();
    let off_sessions = Arc::clone(off.sessions());
    off_sessions
        .insert_detail_rows(&off_sid, &seed)
        .expect("off seed rows");
    off.run_with_turn("go".into(), "parity-off", 5)
        .await
        .expect("foreground turn succeeds");

    let on_dir = tempfile::tempdir().expect("on tempdir");
    let on_provider = Arc::new(ParitySummaryProvider::default());
    let mut on = build_runtime_with_provider(
        on_dir.path(),
        test_agent(vec![], "auto", 5),
        on_provider.clone(),
    );
    on.set_compaction_prefetch_mode(CompactionPrefetchMode::On);
    let on_sid = on.session_id.clone();
    let on_sessions = Arc::clone(on.sessions());
    on_sessions
        .insert_detail_rows(&on_sid, &seed)
        .expect("on seed rows");
    on.run_with_turn("go".into(), "parity-on", 5)
        .await
        .expect("branch turn succeeds");
    for _ in 0..200 {
        if on_sessions
            .data()
            .working_set_blocking(&on_sid)
            .expect("working set")
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_)))
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let off_inputs = off_provider
        .compaction_inputs
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let on_inputs = on_provider
        .compaction_inputs
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    assert_eq!(off_inputs.len(), 1);
    assert_eq!(on_inputs.len(), 1);
    let off_text: Vec<_> = off_inputs[0].iter().map(item_text_preview).collect();
    let on_text: Vec<_> = on_inputs[0].iter().map(item_text_preview).collect();
    assert_eq!(
        off_text, on_text,
        "the background branch must summarize the same request as the foreground path"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hard_stop_failure_uses_both_summary_attempts_and_stops_the_turn() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(FailingSummaryProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    // ~160k estimated tokens: already over the hard send line at step one, so
    // the runtime must use the synchronous path, not start a background task.
    let seed: Vec<_> = (0..10)
        .map(|i| {
            let mut state = i as u64 + 1;
            let mut text = format!("hard-stop-{i} ");
            for _ in 0..30_000 {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push((b'a' + ((state >> 33) % 26) as u8) as char);
            }
            user_text(text)
        })
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    let result = runtime
        .run_with_turn("go".into(), "hard-stop-turn", 5)
        .await;
    assert!(result.is_err(), "compaction failure must fail the turn");
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        2,
        "the synchronous path owns both summary attempts"
    );
    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        0,
        "no main request may be sent after a failed hard-stop compaction"
    );
}

fn seeded_text(prefix: &str, index: u64, chars: usize) -> String {
    let mut state = index.wrapping_mul(6364136223846793005).wrapping_add(1);
    let mut text = format!("{prefix}-{index} ");
    for _ in 0..chars {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        text.push((b'a' + ((state >> 33) % 26) as u8) as char);
    }
    text
}

fn tool_output(call_id: &str, text: String) -> Item {
    Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: call_id.into(),
        output: FunctionCallOutput::Text(text),
        id: Some(format!("output-{call_id}")),
        status: Some(OutputStatus::Completed),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn background_prune_at_the_prepare_line_skips_the_summary_branch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(FinalAnswerPrefetchProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    // Padding stays protected; the oversized tool result is the only eligible
    // tail, so one prune must relieve the pressure without a summary request.
    let mut seed: Vec<_> = (0..8)
        .map(|i| user_text(seeded_text("prune-seed", i, 20_000)))
        .collect();
    seed.push(Item::FunctionCall(FunctionToolCall {
        call_id: "call_prune".into(),
        name: "read".into(),
        arguments: "{}".into(),
        id: Some("item_call_prune".into()),
        status: Some(OutputStatus::Completed),
        namespace: None,
    }));
    seed.push(tool_output(
        "call_prune",
        seeded_text("huge-tool-output", 77, 100_000),
    ));
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    let result = runtime
        .run_with_turn("go".into(), "prune-turn", 5)
        .await
        .expect("turn succeeds");
    assert!(
        result.contains("final answer without a tool call"),
        "{result}"
    );
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        0,
        "a worthwhile prune must skip the summary request"
    );
    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        1,
        "the main turn must continue after the prune"
    );

    let rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set");
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "a prune-only relief must not write a compacted checkpoint: {rows:?}"
    );
    assert!(
        rows.iter().filter_map(|row| row.item()).any(|item| {
            matches!(
                item,
                Item::FunctionCallOutput(output)
                    if matches!(
                        &output.output,
                        FunctionCallOutput::Text(text)
                            if text.contains(TOOL_RESULT_PRUNE_MARKER)
                    )
            )
        }),
        "the oversized tool result must be replaced by the prune marker: {rows:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hard_stop_waits_for_an_in_flight_branch_instead_of_replanning() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider = Arc::new(GatedSummaryProvider::default());
    let mut runtime =
        build_runtime_with_provider(dir.path(), test_agent(vec![], "auto", 5), provider.clone());
    runtime.set_compaction_prefetch_mode(CompactionPrefetchMode::On);

    let sid = runtime.session_id.clone();
    let sessions = Arc::clone(runtime.sessions());
    let seed: Vec<_> = (0..9)
        .map(|i| user_text(seeded_text("wait-seed", i, 20_500)))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).expect("seed rows");

    runtime
        .run_with_turn("first".into(), "wait-turn-1", 5)
        .await
        .expect("first turn succeeds");
    for _ in 0..200 {
        if provider.compaction_calls.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        1,
        "the branch must be blocked inside its summary request"
    );

    // Push the surface over the hard line while the prepared branch is still
    // in flight. The next boundary must await that branch, not start a second
    // summary with a fresh cut.
    sessions
        .insert_detail_rows(&sid, &[user_text(seeded_text("hard-pressure", 99, 60_000))])
        .expect("pressure row");
    let release = Arc::clone(&provider.release);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        release.notify_one();
    });

    let result = runtime
        .run_with_turn("second".into(), "wait-turn-2", 5)
        .await
        .expect("hard-stop turn waits and succeeds");
    assert!(result.contains("gated final answer"), "{result}");
    assert_eq!(
        provider.compaction_calls.load(Ordering::SeqCst),
        1,
        "the hard line must consume the in-flight prepared branch exactly once"
    );
    assert_eq!(
        provider.main_calls.load(Ordering::SeqCst),
        2,
        "the main turn must continue after the branch commits"
    );

    let rows = sessions
        .data()
        .working_set_blocking(&sid)
        .expect("working set");
    assert!(
        rows.iter()
            .any(|row| matches!(row.payload, WorkingPayload::Checkpoint(_))),
        "the in-flight branch must commit its checkpoint: {rows:?}"
    );
    assert!(
        rows.iter()
            .filter_map(|row| row.item())
            .any(|item| item_text_preview(item).contains("hard-pressure")),
        "rows appended after the frozen prepare must remain verbatim: {rows:?}"
    );
}
