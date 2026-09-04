//! Prompt-cache prefix stability inside one turn (no compact).
//!
//! Provider prompt cache is prefix-byte identity: `instructions` + `tools` +
//! `input[0..prev_len]`. If LiteCode rewrites already-sent Items (persist
//! roundtrip, stream `persist_item` seal, job-exit overlay), the next step
//! reports a cache miss. These tests try to turn that red.

mod common;

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::fake_deps::{assistant_text_item, function_call_item};
use common::{build_runtime_with_provider, test_agent};
use litecode::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, MessageItem, OutputItem,
    OutputStatus, ResponseOutputItemAddedEvent, ResponseStreamEvent,
};
use litecode::config::TurnGuard;
use litecode::config::workspace::set_runtime_paths;
use litecode::context_pipeline::{Context, ContextPipeline, ProviderPromptBaseline};
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::session::manager::SessionManager;
use litecode::session::task_state::TaskReminders;
use litecode::session::{WorkingRow, project_items};
use litecode::types::{Item, Result, StreamEvents, item_text_preview, user_text};
use tokio_util::sync::CancellationToken;

fn item_json(item: &Item) -> serde_json::Value {
    serde_json::to_value(item).expect("serialize Item")
}

fn items_json(items: &[Item]) -> Vec<serde_json::Value> {
    items.iter().map(item_json).collect()
}

fn fco(call_id: &str, text: &str) -> Item {
    Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: call_id.into(),
        output: FunctionCallOutput::Text(text.into()),
        id: None,
        status: None,
    })
}

fn assert_json_prefix(prev: &[serde_json::Value], next: &[serde_json::Value], label: &str) {
    assert!(
        next.len() >= prev.len(),
        "{label}: next input shrank ({} → {})",
        prev.len(),
        next.len()
    );
    for (i, (a, b)) in prev.iter().zip(next.iter()).enumerate() {
        assert_eq!(
            a, b,
            "{label}: input[{i}] JSON changed — this busts prompt-cache prefix\nprev={a}\nnext={b}"
        );
    }
}

// ── recording provider (product path: stream added → persist_item) ───────────

#[derive(Clone, Debug)]
struct CapturedLlmRequest {
    instructions: String,
    tools: Vec<serde_json::Value>,
    input: Vec<serde_json::Value>,
}

fn capture_request(request: &ModelRequest) -> CapturedLlmRequest {
    CapturedLlmRequest {
        instructions: request.instructions.clone(),
        tools: request
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect(),
        input: items_json(&request.input),
    }
}

fn assert_request_prefix(prev: &CapturedLlmRequest, next: &CapturedLlmRequest, step: usize) {
    assert_eq!(
        prev.instructions, next.instructions,
        "step {step}: instructions changed (full cache miss)"
    );
    assert_eq!(
        prev.tools, next.tools,
        "step {step}: tools[] changed (full cache miss)"
    );
    assert_json_prefix(&prev.input, &next.input, &format!("step {step} input"));
}

/// Queued Items, plus `output_item.added` (InProgress) so `call_model` hits
/// `sessions.persist_item` before the completed Items are returned.
#[derive(Clone)]
struct RecordingProvider {
    responses: Arc<Mutex<Vec<Vec<Item>>>>,
    index: Arc<AtomicUsize>,
    captured: Arc<Mutex<Vec<CapturedLlmRequest>>>,
}

impl RecordingProvider {
    fn new(responses: Vec<Vec<Item>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            index: Arc::new(AtomicUsize::new(0)),
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<CapturedLlmRequest> {
        self.captured.lock().expect("captured").clone()
    }

    fn next_items(&self) -> Result<Vec<Item>> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        let guard = self.responses.lock().expect("responses");
        guard.get(idx).cloned().ok_or_else(|| {
            litecode::types::LitecodeError::Llm(format!(
                "RecordingProvider: no response queued at index {idx}"
            ))
        })
    }
}

fn emit_in_progress_added(
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
    item: &Item,
    output_index: u32,
) {
    let Some(cb) = on_event else {
        return;
    };
    let output = match item {
        Item::FunctionCall(fc) => {
            let mut fc = fc.clone();
            fc.arguments.clear();
            fc.status = Some(OutputStatus::InProgress);
            OutputItem::FunctionCall(fc)
        }
        Item::Message(MessageItem::Output(msg)) => {
            let mut msg = msg.clone();
            msg.status = OutputStatus::InProgress;
            OutputItem::Message(msg)
        }
        _ => return,
    };
    cb(ResponseStreamEvent::ResponseOutputItemAdded(
        ResponseOutputItemAddedEvent {
            sequence_number: u64::from(output_index) + 1,
            output_index,
            item: output,
        },
    ));
}

impl LlmProvider for RecordingProvider {
    fn endpoint(&self) -> &str {
        "recording://turn-prefix"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.captured
            .lock()
            .expect("captured")
            .push(capture_request(request));
        let items = self.next_items();
        Box::pin(async move { items })
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.captured
            .lock()
            .expect("captured")
            .push(capture_request(request));
        let items = self.next_items();
        Box::pin(async move {
            let items = items?;
            for (i, item) in items.iter().enumerate() {
                emit_in_progress_added(&mut on_event, item, i as u32);
            }
            Ok(items)
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tool_loop_llm_request_prefix_json_is_stable() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();

    let provider = RecordingProvider::new(vec![
        vec![function_call_item(
            "call_1",
            "read",
            r#"{"file_path":"probe.txt"}"#,
            "fc_1",
        )],
        vec![function_call_item(
            "call_2",
            "read",
            r#"{"file_path":"probe.txt"}"#,
            "fc_2",
        )],
        vec![assistant_text_item("done", "msg_done")],
    ]);
    let captured_handle = provider.clone();
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 10),
        Arc::new(provider),
    );

    let text = runtime
        .run("read probe")
        .await
        .expect("turn completes without compact");
    assert_eq!(text, "done");

    let captured = captured_handle.captured();
    assert_eq!(
        captured.len(),
        3,
        "expected 3 LLM requests (two tool rounds + final text), got {}",
        captured.len()
    );
    assert_request_prefix(&captured[0], &captured[1], 1);
    assert_request_prefix(&captured[1], &captured[2], 2);
}

// ── persist / working-set JSON identity ─────────────────────────────────────

fn light_workspace(dir: &std::path::Path) -> litecode::config::WorkspaceState {
    let litecode_dir = dir.join(".litecode");
    std::fs::create_dir_all(litecode_dir.join("logs")).expect("logs dir");
    std::fs::create_dir_all(litecode_dir.join("plan")).expect("plan dir");
    let sessions_db = litecode_dir.join("sessions.db");
    if !sessions_db.exists() {
        std::fs::File::create(&sessions_db).expect("sessions.db");
    }
    let workspace_id = "test-workspace-id".to_string();
    litecode::config::WorkspaceState {
        workspace_root: dir.to_path_buf(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: litecode::config::WorkspacePaths::for_workspace(dir, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    }
}

fn test_context(cwd: &std::path::Path) -> Context {
    let workspace_id =
        litecode::config::peek_workspace_id(cwd).unwrap_or_else(|| "test-workspace-id".to_string());
    let paths = litecode::config::WorkspacePaths::for_workspace(cwd, &workspace_id);
    Context {
        cwd: cwd.to_path_buf(),
        workspace_paths: paths,
        agents_md: None,
        claude_md: None,
    }
}

fn test_sessions(db_path: &str) -> Arc<SessionManager> {
    Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.to_string(),
    ))
}

fn setup_session(dir: &std::path::Path) -> (String, Arc<SessionManager>) {
    let ws = light_workspace(dir);
    set_runtime_paths(ws.paths.clone());
    let db_path = ws.paths.sessions_db.to_string_lossy().to_string();
    let sessions = test_sessions(&db_path);
    let sid = sessions
        .open_session_sync("/proj", "default", Some("test-model"))
        .expect("open");
    (sid, sessions)
}

#[test]
fn persist_roundtrip_preserves_item_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_session(dir.path());
    let original = vec![
        user_text("hello"),
        function_call_item("c1", "read", r#"{"file_path":"a.rs"}"#, "fc_1"),
        fco("c1", "file contents"),
        assistant_text_item("ok", "msg_1"),
    ];
    sessions.insert_detail_rows(&sid, &original).unwrap();
    let loaded: Vec<Item> = sessions
        .data()
        .working_set_blocking(&sid)
        .unwrap()
        .into_iter()
        .map(|row| row.item)
        .collect();
    assert_eq!(
        items_json(&original),
        items_json(&loaded),
        "working-set reload changed Item JSON (Responses wire prefix would miss)"
    );
}

// ── prepare_step overlay ────────────────────────────────────────────────────

fn test_model() -> litecode::config::schema::ModelDefinition {
    litecode::config::schema::ModelDefinition {
        id: "test-model".into(),
        adapter_id: litecode::config::schema::ADAPTER_OPENAI_RESPONSES.into(),
        provider_ref: "main".into(),
        label: "Test".into(),
        config: litecode::config::schema::ModelAdapterConfig {
            api_model_id: "m".into(),
            context_window: 128_000,
            max_tokens: 1024,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: vec![litecode::config::schema::ModelCapability::Text],
        },
    }
}

async fn prepare_snapshot(
    pipeline: &ContextPipeline,
    sessions: &Arc<SessionManager>,
    sid: &str,
    turn: &mut Vec<WorkingRow>,
    step: u64,
) -> Vec<serde_json::Value> {
    let provider = common::ScriptedProvider::with_text("unused");
    let cancel = CancellationToken::new();
    let mut items = project_items(turn);
    pipeline
        .prepare_step(
            sessions,
            sid,
            &provider,
            "key",
            "m",
            "system",
            1024,
            &ProviderPromptBaseline::default(),
            &mut items,
            step,
            &cancel,
            &TaskReminders::default(),
            &test_model(),
        )
        .await
        .expect("prepare_step");
    *turn = pipeline.working_set();
    items_json(
        &pipeline
            .prepared_view()
            .expect("prepared view")
            .items,
    )
}

fn in_progress_fc(call_id: &str, name: &str, id: &str) -> Item {
    Item::FunctionCall(FunctionToolCall {
        arguments: String::new(),
        call_id: call_id.into(),
        namespace: None,
        name: name.into(),
        id: Some(id.into()),
        status: Some(OutputStatus::InProgress),
    })
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_after_stream_persist_seal_keeps_prior_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_session(dir.path());
    sessions
        .insert_detail_rows(&sid, &[user_text("hist-a"), user_text("hist-b")])
        .unwrap();

    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(10_000, ctx, dir.path().to_path_buf());
    let mut turn = pipeline
        .begin_turn_with_id(&sessions, &sid, Some("t1".into()))
        .unwrap();
    turn.push(WorkingRow::pending(user_text("go")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let first = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 1).await;

    sessions
        .persist_item(&sid, &in_progress_fc("c1", "read", "fc_1"))
        .expect("stream persist_item");
    turn.push(WorkingRow::pending(function_call_item(
        "c1",
        "read",
        r#"{"file_path":"a.rs"}"#,
        "fc_1",
    )));
    let seal = pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();
    assert!(
        !seal.sealed_seqs.is_empty(),
        "completed FunctionCall must seal the in_progress row, got {seal:?}"
    );
    turn.push(WorkingRow::pending(fco("c1", "ok")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let second = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 2).await;
    assert_json_prefix(&first, &second, "after first tool round");

    sessions
        .persist_item(&sid, &in_progress_fc("c2", "read", "fc_2"))
        .expect("stream persist_item 2");
    turn.push(WorkingRow::pending(function_call_item(
        "c2",
        "read",
        r#"{"file_path":"b.rs"}"#,
        "fc_2",
    )));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();
    turn.push(WorkingRow::pending(fco("c2", "ok2")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let third = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 3).await;
    assert_json_prefix(&second, &third, "after second tool round");
}

#[tokio::test(flavor = "current_thread")]
async fn job_exit_mid_turn_does_not_rewrite_already_sent_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_session(dir.path());
    sessions
        .insert_detail_rows(&sid, &[user_text("hist")])
        .unwrap();

    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(10_000, ctx, dir.path().to_path_buf());
    let mut turn = pipeline
        .begin_turn_with_id(&sessions, &sid, Some("t1".into()))
        .unwrap();
    turn.push(WorkingRow::pending(user_text("go")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let _ = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 1).await;
    turn.push(WorkingRow::pending(function_call_item(
        "c1",
        "read",
        "{}",
        "fc_1",
    )));
    turn.push(WorkingRow::pending(fco("c1", "ok")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let after_tools = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 2).await;

    sessions
        .append_job_exit(
            &sid,
            &user_text("<system-reminder>\nBackground bash bg-1 exited with code 0.\n</system-reminder>"),
        )
        .expect("append_job_exit while turn pipeline is live");

    let after_exit = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 3).await;
    assert_json_prefix(
        &after_tools,
        &after_exit,
        "job_exit overlay must not rewrite already-sent prefix",
    );
    assert!(
        after_exit.len() > after_tools.len(),
        "job_exit should append a new item (len {} → {}), not rewrite in place",
        after_tools.len(),
        after_exit.len()
    );
    let last = item_text_preview(
        &serde_json::from_value::<Item>(after_exit.last().cloned().unwrap()).unwrap(),
    );
    assert!(
        last.contains("Background bash"),
        "job_exit reminder should land at the tail, got {last:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn hanging_call_pad_is_stable_across_prepare_steps() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (sid, sessions) = setup_session(dir.path());
    let hanging = function_call_item("hanging", "read", "{}", "fc_hang");
    sessions
        .insert_detail_rows(&sid, &[user_text("ask"), hanging])
        .unwrap();

    let ctx = test_context(dir.path());
    let pipeline = ContextPipeline::new(10_000, ctx, dir.path().to_path_buf());
    let mut turn = pipeline.begin_turn(&sessions, &sid).unwrap();
    let first = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 1).await;
    assert!(
        first.iter().any(|v| v["call_id"] == "hanging"
            && v["type"] == "function_call_output"),
        "first LLM view must pad the hanging call: {first:?}"
    );

    turn.push(WorkingRow::pending(function_call_item(
        "c_new",
        "read",
        "{}",
        "fc_new",
    )));
    turn.push(WorkingRow::pending(fco("c_new", "fresh")));
    pipeline.commit_step(&sessions, &sid, &mut turn).unwrap();

    let second = prepare_snapshot(&pipeline, &sessions, &sid, &mut turn, 2).await;
    assert_json_prefix(&first, &second, "hanging-call pad");
}
