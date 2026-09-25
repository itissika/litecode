//! Queued (pending) user messages: memory-only queue consumed at the request
//! seam, indistinguishable from an ordinary user message once injected.
mod common;

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::fake_deps::{assistant_text_item, function_call_item};
use common::{
    build_runtime_with_provider, build_runtime_with_provider_and_observer, test_agent,
};
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::runtime::observer::{InternalEvent, RuntimeObserver};
use litecode::session::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::{Item, Result, StreamEvents, item_text_preview};
use tokio_util::sync::CancellationToken;

/// Scripted provider that captures every request input and — while serving the
/// call at `queue_on` — enqueues one pending user message, standing in for a
/// human typing mid-turn.
#[derive(Clone)]
struct SteeringProvider {
    responses: Arc<Mutex<Vec<Vec<Item>>>>,
    index: Arc<AtomicUsize>,
    inputs: Arc<Mutex<Vec<Vec<Item>>>>,
    queue_on: usize,
    steer_text: String,
    target: Arc<Mutex<Option<(Arc<SessionManager>, String)>>>,
}

impl SteeringProvider {
    fn new(responses: Vec<Vec<Item>>, queue_on: usize, steer_text: &str) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            index: Arc::new(AtomicUsize::new(0)),
            inputs: Arc::new(Mutex::new(Vec::new())),
            queue_on,
            steer_text: steer_text.to_string(),
            target: Arc::new(Mutex::new(None)),
        }
    }

    fn attach(&self, sessions: Arc<SessionManager>, session_id: String) {
        *self.target.lock().unwrap() = Some((sessions, session_id));
    }

    fn inputs(&self) -> Vec<Vec<Item>> {
        self.inputs.lock().unwrap().clone()
    }

    fn next_items(&self, call: usize) -> Result<Vec<Item>> {
        self.responses
            .lock()
            .unwrap()
            .get(call)
            .cloned()
            .ok_or_else(|| {
                litecode::types::LitecodeError::Llm(format!(
                    "SteeringProvider: no response queued at index {call}"
                ))
            })
    }
}

impl LlmProvider for SteeringProvider {
    fn endpoint(&self) -> &str {
        "scripted://steering"
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
        if call == self.queue_on
            && let Some((sessions, session_id)) = self.target.lock().unwrap().clone()
        {
            sessions
                .enqueue_pending_message(&session_id, &self.steer_text)
                .expect("enqueue pending message");
        }
        let items = self.next_items(call);
        Box::pin(async move { items })
    }
}

fn user_rows(sessions: &SessionManager, session_id: &str) -> Vec<String> {
    sessions
        .data()
        .events_blocking(session_id)
        .expect("events")
        .into_iter()
        .filter(|event| event.event_type == EventType::ItemUser)
        .filter_map(|event| {
            litecode::session::event::item_from_event(&event)
                .ok()
                .map(|item| item_text_preview(&item))
        })
        .collect()
}

/// Records the two events the row-push contract is built from — `StepCommitted`
/// (the projection ships `buffer/item` on it, or on a seal restamp) and
/// `LlmRequestBuilt` (a request boundary) — with the number of durable user rows
/// at that moment, so a test can tell *when* a row became visible to clients.
#[derive(Default)]
struct SeamObserver {
    marks: Mutex<Vec<(&'static str, usize)>>,
    target: Mutex<Option<(Arc<SessionManager>, String)>>,
}

impl SeamObserver {
    fn attach(&self, sessions: Arc<SessionManager>, session_id: String) {
        *self.target.lock().unwrap() = Some((sessions, session_id));
    }

    /// Durable user rows seen by each commit announced between the first and the
    /// second request.
    fn commit_rows_between_requests(&self) -> Vec<usize> {
        let marks = self.marks.lock().unwrap();
        let first = marks
            .iter()
            .position(|(mark, _)| *mark == "request")
            .unwrap_or(0);
        let second = marks
            .iter()
            .rposition(|(mark, _)| *mark == "request")
            .unwrap_or(marks.len());
        marks[first..second]
            .iter()
            .filter(|(mark, _)| *mark == "commit")
            .map(|(_, rows)| *rows)
            .collect()
    }

    fn marks(&self) -> Vec<(&'static str, usize)> {
        self.marks.lock().unwrap().clone()
    }
}

impl RuntimeObserver for SeamObserver {
    fn on_internal(&self, event: InternalEvent) {
        let mark = match &event {
            InternalEvent::StepCommitted => "commit",
            InternalEvent::LlmRequestBuilt { .. } => "request",
            _ => return,
        };
        let rows = self
            .target
            .lock()
            .unwrap()
            .as_ref()
            .map(|(sessions, session_id)| user_rows(sessions, session_id).len())
            .unwrap_or(0);
        self.marks.lock().unwrap().push((mark, rows));
    }
}

/// The tool loop takes another step after the tool result: the queued message
/// must be injected at that seam, persisted as one ordinary user row, and be
/// visible to the very next request.
#[tokio::test(flavor = "current_thread")]
async fn queued_message_is_injected_at_the_tool_loop_seam() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();

    let provider = SteeringProvider::new(
        vec![
            vec![function_call_item(
                "call_1",
                "read",
                r#"{"file_path":"probe.txt"}"#,
                "fc_1",
            )],
            vec![assistant_text_item("done", "msg_done")],
        ],
        0,
        "steer now",
    );
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let session_id = runtime.session_id.clone();
    provider.attach(Arc::clone(&sessions), session_id.clone());

    // Production turns reserve before the runtime starts; the inline `run` path
    // needs the same manager activity so the seam claim has an owner.
    let cancel = CancellationToken::new();
    sessions
        .begin_turn(
            &session_id,
            "local-turn".into(),
            cancel.clone(),
            10,
            "default",
            &dir.path().to_string_lossy(),
        )
        .expect("begin turn");

    let text = runtime.run("read probe").await.expect("turn completes");
    assert_eq!(text, "done");

    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2, "tool round + final answer");
    assert!(
        !inputs[0]
            .iter()
            .any(|item| item_text_preview(item) == "steer now"),
        "the queued message must not be in the request it raced with"
    );
    assert!(
        inputs[1]
            .iter()
            .any(|item| item_text_preview(item) == "steer now"),
        "the next request must carry the injected user message"
    );

    assert_eq!(
        user_rows(&sessions, &session_id),
        vec!["read probe".to_string(), "steer now".to_string()],
        "injection must persist exactly one ordinary user row, in order"
    );
    assert!(
        !sessions.has_pending_messages(&session_id),
        "the claim must clear the queue"
    );
}

/// The message can also arrive while the *final* answer streams. The stop check
/// must keep the turn alive so the model answers it in this turn rather than a
/// follow-up.
#[tokio::test(flavor = "current_thread")]
async fn queued_message_during_final_response_extends_the_turn() {
    let dir = tempfile::tempdir().expect("tempdir");

    let provider = SteeringProvider::new(
        vec![
            vec![assistant_text_item("first", "msg_first")],
            vec![assistant_text_item("second", "msg_second")],
        ],
        0,
        "steer while finishing",
    );
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec![], "readonly", 10),
        Arc::new(provider.clone()),
    );
    let sessions = Arc::clone(runtime.sessions());
    let session_id = runtime.session_id.clone();
    provider.attach(Arc::clone(&sessions), session_id.clone());

    let cancel = CancellationToken::new();
    sessions
        .begin_turn(
            &session_id,
            "local-turn".into(),
            cancel.clone(),
            10,
            "default",
            &dir.path().to_string_lossy(),
        )
        .expect("begin turn");

    let text = runtime.run("go").await.expect("turn completes");
    assert_eq!(text, "second", "the extended step owns the final text");

    let inputs = provider.inputs();
    assert_eq!(inputs.len(), 2, "the queued message must force one more request");
    assert!(
        inputs[1]
            .iter()
            .any(|item| item_text_preview(item) == "steer while finishing"),
        "the extra request must carry the injected user message"
    );
    assert_eq!(
        user_rows(&sessions, &session_id),
        vec!["go".to_string(), "steer while finishing".to_string()]
    );
    assert!(!sessions.has_pending_messages(&session_id));
}

/// The seam injection must announce its own commit. The projection only ships
/// `buffer/item` on `StepCommitted` (or a seal restamp), so without that
/// announcement the client keeps a claimed message parked at the tail — below
/// the answer that already consumed it — until the *next* request's step
/// commits, i.e. a whole response later.
#[tokio::test(flavor = "current_thread")]
async fn the_seam_injection_announces_its_own_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();

    let provider = SteeringProvider::new(
        vec![
            vec![function_call_item(
                "call_1",
                "read",
                r#"{"file_path":"probe.txt"}"#,
                "fc_1",
            )],
            vec![assistant_text_item("done", "msg_done")],
        ],
        0,
        "steer now",
    );
    let observer = Arc::new(SeamObserver::default());
    let mut runtime = build_runtime_with_provider_and_observer(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider.clone()),
        observer.clone(),
    );
    let sessions = Arc::clone(runtime.sessions());
    let session_id = runtime.session_id.clone();
    provider.attach(Arc::clone(&sessions), session_id.clone());
    observer.attach(Arc::clone(&sessions), session_id.clone());

    let cancel = CancellationToken::new();
    sessions
        .begin_turn(
            &session_id,
            "local-turn".into(),
            cancel.clone(),
            10,
            "default",
            &dir.path().to_string_lossy(),
        )
        .expect("begin turn");

    let text = runtime.run("read probe").await.expect("turn completes");
    assert_eq!(text, "done");
    assert_eq!(
        user_rows(&sessions, &session_id),
        vec!["read probe".to_string(), "steer now".to_string()],
        "the injection is durable before the request that carries it"
    );
    // Between the two requests the client must be told twice: commits that still
    // see only the first message (the step's own persistence) and then one that
    // already sees the injected second row. That last one is what makes the
    // projection ship it — before the request that consumed it, instead of a
    // whole response later.
    let window = observer.commit_rows_between_requests();
    assert!(
        window.contains(&1) && window.contains(&2),
        "expected the injection's announcement inside {window:?} (all marks: {:?})",
        observer.marks()
    );
}
