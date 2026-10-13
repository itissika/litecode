//! WP-098: child follow-up queue (queue / steer / stop_and_send) and the
//! red line that main-session step-seam injection is unchanged.
mod common;

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::fake_deps::{assistant_text_item, function_call_item};
use common::runtime::build_child_runtime_with_provider;
use common::{build_runtime_with_provider, test_agent};
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::session::manager::{CancelOutcome, FollowUpOutcome, SessionManager};
use litecode::session::{EventType, LifecycleEvent};
use litecode::types::{Item, Result, StopOrigin, StreamEvents, UserInput, item_text_preview};
use tokio_util::sync::CancellationToken;

type Action = Arc<dyn Fn(&SessionManager, &str) + Send + Sync>;

/// Scripted provider that runs `action` while serving call 0, standing in for
/// a parent/user acting mid-turn.
#[derive(Clone)]
struct ActingProvider {
    responses: Arc<Vec<Vec<Item>>>,
    index: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<Vec<Item>>>>,
    action: Action,
    target: Arc<Mutex<Option<(Arc<SessionManager>, String)>>>,
}

impl ActingProvider {
    fn new(responses: Vec<Vec<Item>>, action: Action) -> Self {
        Self {
            responses: Arc::new(responses),
            index: Arc::new(AtomicUsize::new(0)),
            inputs: Arc::new(Mutex::new(Vec::new())),
            action,
            target: Arc::new(Mutex::new(None)),
        }
    }
    fn attach(&self, sessions: Arc<SessionManager>, sid: String) {
        *self.target.lock().unwrap() = Some((sessions, sid));
    }
    fn inputs(&self) -> Vec<Vec<Item>> {
        self.inputs.lock().unwrap().clone()
    }
}

impl LlmProvider for ActingProvider {
    fn endpoint(&self) -> &str {
        "scripted://wp098"
    }
    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }
    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.inputs.lock().unwrap().push(request.input.clone());
        let call = self.index.fetch_add(1, Ordering::Relaxed);
        if call == 0
            && let Some((sessions, sid)) = self.target.lock().unwrap().clone()
        {
            (self.action)(&sessions, &sid);
        }
        let items =
            self.responses.get(call).cloned().ok_or_else(|| {
                litecode::types::LitecodeError::Llm(format!("no response at {call}"))
            });
        Box::pin(async move { items })
    }
}

fn tool_then_done() -> Vec<Vec<Item>> {
    vec![
        vec![function_call_item(
            "call_1",
            "read",
            r#"{"file_path":"probe.txt"}"#,
            "fc_1",
        )],
        vec![assistant_text_item("done", "msg_done")],
    ]
}

fn user_rows(sessions: &SessionManager, sid: &str) -> Vec<String> {
    sessions
        .data()
        .events_blocking(sid)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::ItemUser)
        .filter_map(|e| {
            litecode::session::event::item_from_event(&e)
                .ok()
                .map(|i| item_text_preview(&i))
        })
        .collect()
}

fn turn_ends(sessions: &SessionManager, sid: &str) -> Vec<serde_json::Value> {
    sessions
        .data()
        .events_blocking(sid)
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == EventType::TurnEnd)
        .map(|e| e.data)
        .collect()
}

fn in_request(input: &[Item], text: &str) -> bool {
    input.iter().any(|i| item_text_preview(i) == text)
}

async fn run_turn(
    runtime: &mut litecode::runtime::AgentRuntime,
    sessions: &SessionManager,
    dir: &std::path::Path,
) -> Result<String> {
    let sid = runtime.session_id.clone();
    sessions
        .begin_turn(
            &sid,
            "local-turn".into(),
            runtime.cancel_token(),
            10,
            "default",
            &dir.to_string_lossy(),
        )
        .expect("begin turn");
    runtime.run("first").await
}

/// RED LINE: a pending message on the MAIN session is still claimed at the next
/// step seam and answered in the same turn; the follow-up queue is untouched.
#[tokio::test(flavor = "current_thread")]
async fn runtime_main_session_pending_still_injects_at_seam() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();
    let provider = ActingProvider::new(
        tool_then_done(),
        Arc::new(|s, sid| {
            s.enqueue_pending_message(sid, "steer now").unwrap();
        }),
    );
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    assert!(
        sessions.parent_session_id_of(&sid).is_none(),
        "main session"
    );
    provider.attach(Arc::clone(&sessions), sid.clone());

    let text = run_turn(&mut runtime, &sessions, dir.path()).await.unwrap();
    assert_eq!(text, "done");
    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2);
    assert!(!in_request(&inputs[0], "steer now"));
    assert!(in_request(&inputs[1], "steer now"), "injected at the seam");
    assert_eq!(user_rows(&sessions, &sid), vec!["first", "steer now"]);
    assert_eq!(
        turn_ends(&sessions, &sid).len(),
        1,
        "same turn, no new turn"
    );
    assert!(!sessions.has_pending_messages(&sid));
    assert_eq!(sessions.follow_ups_len(&sid), 0);
    // The follow-up queue is child-only: the main session cannot use it.
    assert!(
        sessions
            .enqueue_follow_up(&sid, UserInput::text("x"))
            .is_err()
    );
}

/// steer on a child keeps today's interjection behaviour.
#[tokio::test(flavor = "current_thread")]
async fn runtime_steer_still_injects_at_step_seam() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();
    let provider = ActingProvider::new(
        tool_then_done(),
        Arc::new(|s, sid| {
            s.enqueue_user_input(sid, UserInput::text("steer child"))
                .unwrap();
        }),
    );
    let mut runtime = build_child_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    assert!(
        sessions.parent_session_id_of(&sid).is_some(),
        "child session"
    );
    provider.attach(Arc::clone(&sessions), sid.clone());

    run_turn(&mut runtime, &sessions, dir.path()).await.unwrap();
    let inputs = provider.inputs();
    assert!(in_request(&inputs[1], "steer child"));
    assert_eq!(user_rows(&sessions, &sid), vec!["first", "steer child"]);
    assert_eq!(turn_ends(&sessions, &sid).len(), 1);
}

/// queue: the message waits in the follow-up queue; the step seam never
/// claims it and the current turn ends normally.
#[tokio::test(flavor = "current_thread")]
async fn runtime_queue_not_drained_at_step_seam() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();
    let outcome = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&outcome);
    let provider = ActingProvider::new(
        tool_then_done(),
        Arc::new(move |s, sid| {
            *seen.lock().unwrap() = Some(
                s.enqueue_follow_up(sid, UserInput::text("queued msg"))
                    .unwrap(),
            );
        }),
    );
    let mut runtime = build_child_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    provider.attach(Arc::clone(&sessions), sid.clone());

    let text = run_turn(&mut runtime, &sessions, dir.path()).await.unwrap();
    assert_eq!(text, "done");
    assert_eq!(
        *outcome.lock().unwrap(),
        Some(FollowUpOutcome::Queued { position: 1 })
    );
    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2);
    assert!(!in_request(&inputs[1], "queued msg"), "not injected");
    assert_eq!(user_rows(&sessions, &sid), vec!["first"]);
    assert_eq!(turn_ends(&sessions, &sid).len(), 1);
    assert_eq!(turn_ends(&sessions, &sid)[0]["reason"], "completed");
    assert!(!sessions.has_pending_messages(&sid));
    assert_eq!(sessions.follow_ups_len(&sid), 1, "waits for its own turn");
    // Idle child: nothing is queued, the caller starts the turn itself.
    assert_eq!(
        sessions
            .enqueue_follow_up(&sid, UserInput::text("idle"))
            .unwrap(),
        FollowUpOutcome::Idle
    );
    assert_eq!(sessions.follow_ups_len(&sid), 1);
}

/// stop_and_send: one lock cancels the turn (cancelled_by=parent, persisted in
/// turn/end and carried on TurnFinished) and puts the message at the queue head.
#[tokio::test(flavor = "current_thread")]
async fn runtime_stop_and_send_cancels_then_runs_new_turn_no_notice() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();
    let outcome = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&outcome);
    let provider = ActingProvider::new(
        tool_then_done(),
        Arc::new(move |s, sid| {
            s.enqueue_follow_up(sid, UserInput::text("older")).unwrap();
            *seen.lock().unwrap() = Some(
                s.cancel_and_follow_up(sid, UserInput::text("urgent"), StopOrigin::Parent)
                    .unwrap(),
            );
        }),
    );
    let mut runtime = build_child_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let sid = runtime.session_id.clone();
    provider.attach(Arc::clone(&sessions), sid.clone());
    let mut lifecycle = sessions.subscribe_lifecycle();

    let _ = run_turn(&mut runtime, &sessions, dir.path()).await;
    assert_eq!(
        *outcome.lock().unwrap(),
        Some((
            CancelOutcome::Running,
            FollowUpOutcome::Queued { position: 1 }
        ))
    );
    let ends = turn_ends(&sessions, &sid);
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0]["reason"], "cancelled");
    assert_eq!(ends[0]["cancelled_by"], "parent");
    assert_eq!(
        sessions.data().latest_turn_end_blocking(&sid).unwrap(),
        Some((
            "local-turn".to_string(),
            "cancelled".to_string(),
            Some(StopOrigin::Parent)
        ))
    );
    let mut finished = None;
    while let Ok(ev) = lifecycle.try_recv() {
        if let LifecycleEvent::TurnFinished {
            session_id,
            reason,
            cancelled_by,
            ..
        } = ev
            && session_id == sid
        {
            finished = Some((reason, cancelled_by));
        }
    }
    assert_eq!(
        finished,
        Some((Some("cancelled".to_string()), Some(StopOrigin::Parent))),
        "TurnFinished carries the persisted cancelled_by"
    );
    assert_eq!(sessions.follow_ups_len(&sid), 2, "urgent ahead of older");
    assert!(!user_rows(&sessions, &sid).contains(&"urgent".to_string()));
}

/// First call hangs until cancelled; every later call answers with text.
#[derive(Clone)]
struct HangThenText {
    calls: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<Vec<Item>>>>,
}

impl LlmProvider for HangThenText {
    fn endpoint(&self) -> &str {
        "scripted://hang-then-text"
    }
    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }
    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.inputs.lock().unwrap().push(request.input.clone());
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let cancel = cancel.clone();
        Box::pin(async move {
            if call == 0 {
                cancel.cancelled().await;
                return Err(litecode::types::LitecodeError::Canceled);
            }
            Ok(vec![assistant_text_item("follow-up done", "msg_fu")])
        })
    }
}

/// End-to-end: stop_and_send on a running child cancels the old turn
/// (cancelled_by=parent, no wake, no notice), the follow-up turn auto-starts
/// with that message, and only its normal end wakes the parent with exactly
/// one inbox result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_and_send_cancels_then_runs_follow_up_without_waking_parent() {
    use common::subagent_fixture::SubagentHarness;
    let provider = HangThenText {
        calls: Arc::new(AtomicUsize::new(0)),
        inputs: Arc::new(Mutex::new(Vec::new())),
    };
    let harness = SubagentHarness::new(Arc::new(provider.clone()));
    let sessions = Arc::clone(&harness.sessions);
    let parent = harness.parent_id.clone();
    let hub = Arc::clone(&harness.runtime.subagent_hub);
    let wakes = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&wakes);
    hub.set_exit_handler(Arc::new(move |c| {
        seen.lock().unwrap().push(c.turn_id.clone());
    }));
    litecode::runtime::pending_auto_turn::install_pending_flush(
        Arc::new(std::sync::RwLock::new(harness.runtime.clone())),
        Arc::clone(&sessions),
        harness.dir.path().to_path_buf(),
    );
    let mut lifecycle = sessions.subscribe_lifecycle();

    let child = harness.launch_reviewer("call_launch", "work").await;
    harness.wait_for(
        || provider.calls.load(Ordering::SeqCst) >= 1,
        "child turn running in the model call",
    );
    assert!(sessions.is_turn_running_blocking(&child));

    let out = sessions
        .cancel_and_follow_up(&child, UserInput::text("urgent"), StopOrigin::Parent)
        .unwrap();
    assert_eq!(
        out,
        (
            CancelOutcome::Running,
            FollowUpOutcome::Queued { position: 1 }
        )
    );

    // Wait for the follow-up turn to have run and ended.
    harness.wait_for(
        || {
            provider.calls.load(Ordering::SeqCst) >= 2
                && !sessions.is_turn_running_blocking(&child)
                && turn_ends(&sessions, &child).len() >= 2
        },
        "follow-up turn finished",
    );
    harness.wait_for(|| !wakes.lock().unwrap().is_empty(), "parent wake");

    // Stored turn/end: old cancelled by parent, new completed.
    let ends = turn_ends(&sessions, &child);
    assert_eq!(ends.len(), 2, "{ends:?}");
    assert_eq!(ends[0]["reason"], "cancelled");
    assert_eq!(ends[0]["cancelled_by"], "parent");
    assert_eq!(ends[1]["reason"], "completed");
    assert!(ends[1].get("cancelled_by").is_none());
    let old_turn = ends[0]["turn"].as_str().unwrap().to_string();
    let new_turn = ends[1]["turn"].as_str().unwrap().to_string();

    // Live TurnFinished carries the same values.
    let mut finished = Vec::new();
    while let Ok(ev) = lifecycle.try_recv() {
        if let LifecycleEvent::TurnFinished {
            session_id,
            progress,
            reason,
            cancelled_by,
        } = ev
            && session_id == child
        {
            finished.push((progress.turn_id, reason, cancelled_by));
        }
    }
    assert_eq!(
        finished,
        vec![
            (
                old_turn.clone(),
                Some("cancelled".to_string()),
                Some(StopOrigin::Parent)
            ),
            (new_turn.clone(), Some("completed".to_string()), None),
        ]
    );

    // The follow-up turn ran the queued message.
    assert!(
        user_rows(&sessions, &child).contains(&"urgent".to_string()),
        "follow-up persisted as a user row"
    );
    let inputs = provider.inputs.lock().unwrap().clone();
    assert!(in_request(&inputs[1], "urgent"));
    assert_eq!(sessions.follow_ups_len(&child), 0);

    // Only the normal completion woke the parent; no notice; one result.
    assert_eq!(*wakes.lock().unwrap(), vec![new_turn.clone()]);
    assert!(
        !hub.has_notices(&parent),
        "stop_and_send writes no ParentNotice"
    );
    let inbox = hub.take_completions(&parent);
    assert_eq!(inbox.len(), 1, "{inbox:?}");
    assert_eq!(inbox[0].child_session_id, child);
    assert_eq!(inbox[0].turn_id, new_turn);
    assert!(!inbox[0].cancelled);
}
