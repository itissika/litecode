//! PR-1: a compaction summary is only checkpointed after its real finish
//! status passes the persistence gate. Incomplete, empty, and tool-call
//! summaries must leave the session log and in-memory rows untouched.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::SessionDataFixture;
use common::recording_provider::RecordingProvider;
use litecode::authority::responses::{MessageItem, OutputStatus};
use litecode::context_pipeline::{
    BudgetPolicy, CompactPolicy, ProviderPromptBaseline, SummaryFinish,
};
use litecode::llm::{CompactionRequestContext, LlmProvider, RequestPurpose, provider_from_model};
use litecode::platform_knobs::ThinkingSpec;
use litecode::provider_catalog::ProviderCatalog;
use litecode::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use litecode::session::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::{Item, item_text_preview, user_text};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn assistant_with_status(text: &str, id: &str, status: OutputStatus) -> Item {
    let Item::Message(MessageItem::Output(mut message)) = common::assistant_text_item(text, id)
    else {
        panic!("fixture assistant item must be an output message");
    };
    message.status = status;
    Item::Message(MessageItem::Output(message))
}

fn row_snapshot(rows: &[litecode::session::working::WorkingRow]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{:?}", row.payload))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_summary_finish_never_writes_a_compacted_checkpoint() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<_> = (0..12)
        .map(|i| litecode::types::user_text(format!("summary-gate-seed-{i}")))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();

    let truncated =
        assistant_with_status("partial summary", "msg_truncated", OutputStatus::Incomplete);
    let empty = Vec::new();
    let tool_call = vec![
        common::assistant_text_item("looks like text", "msg_tool_text"),
        common::function_call_item("call-summary", "read", "{}", "fc-summary"),
    ];
    let provider = Arc::new(RecordingProvider::new(vec![
        vec![truncated.clone()],
        vec![truncated],
        empty.clone(),
        empty,
        tool_call.clone(),
        tool_call,
    ]));
    let captured = Arc::clone(&provider);
    let provider_for_request: Arc<dyn LlmProvider> = provider;
    let request = common::test_compaction_request(
        provider_for_request,
        "main system / instructions",
        Vec::new(),
        &sid,
    );
    let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
    let baseline = ProviderPromptBaseline::default();
    let cancel = CancellationToken::new();
    let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
    let before = row_snapshot(&rows);

    for (index, label) in ["truncated/incomplete", "empty", "tool-call"]
        .into_iter()
        .enumerate()
    {
        let error = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &baseline,
            &mut rows,
            &cancel,
            CompactionTrigger::Auto,
            Some(&format!("summary-gate-{index}")),
            &sessions.data_root_path(),
        )
        .await
        .expect_err(label);
        assert!(
            error.is_compaction_failure(),
            "{label} must fail the summary gate, got {error}"
        );
        assert_eq!(
            error.compaction_failure_kind(),
            Some(litecode::types::CompactionFailureKind::SummaryFinish),
            "{label} must classify as a finish failure"
        );
        assert_eq!(
            row_snapshot(&rows),
            before,
            "{label} must restore the in-memory working-set snapshot"
        );
        let events = sessions.data().events_blocking(&sid).unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, EventType::Compacted)),
            "{label} must not write a Compacted event"
        );
    }

    let requests = captured.captured();
    assert_eq!(
        requests.len(),
        6,
        "each rejected summary reached the provider twice"
    );
    for request in requests {
        assert_eq!(request.purpose, RequestPurpose::Compaction);
        assert_eq!(
            request.max_output_tokens, 8192,
            "summary cap must clamp to the primary model max_output"
        );
    }
    assert!(
        before
            .iter()
            .any(|text| text.contains("summary-gate-seed-0")),
        "fixture rows are still present"
    );
    let persisted_previews: Vec<_> = sessions
        .data()
        .working_set_blocking(&sid)
        .unwrap()
        .iter()
        .map(|row| item_text_preview(&row.payload.model_item()))
        .collect();
    assert!(
        persisted_previews
            .iter()
            .any(|text| text.contains("summary-gate-seed-0")),
        "durable rows must still contain the discarded-prefix source"
    );
}

async fn assert_rejected_quality_summary(
    anchor: &str,
    summary: String,
    label: &str,
    expected_failure: &str,
) {
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(litecode::config::TurnGuard::new()),
        String::new(),
    ));
    let sid = sessions
        .open_session_sync("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF))
        .expect("open session");
    let mut seed: Vec<_> = (0..11)
        .map(|i| user_text(format!("{label}-seed-{i}-{}", "word ".repeat(200))))
        .collect();
    seed.push(user_text(anchor));
    seed.push(user_text(format!("{label}-kept-tail")));
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
    let before = row_snapshot(&rows);

    let response = vec![common::assistant_text_item(
        &summary,
        "msg_rejected_summary",
    )];
    let recording = Arc::new(RecordingProvider::new(vec![response.clone(), response]));
    let captured = Arc::clone(&recording);
    let provider: Arc<dyn LlmProvider> = recording;
    let request =
        common::test_compaction_request(provider, "main system / instructions", Vec::new(), &sid);
    let Some(mut lifecycle) = sessions.subscribe(&sid) else {
        panic!("lifecycle subscription");
    };

    let error = CompactPolicy::compact_transcript(
        &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
        &sessions,
        &sid,
        &request,
        litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
        &ProviderPromptBaseline::default(),
        &mut rows,
        &CancellationToken::new(),
        CompactionTrigger::Auto,
        Some(label),
        &sessions.data_root_path(),
    )
    .await
    .expect_err("rejected summary must not be accepted");
    assert_eq!(
        error.compaction_failure_kind(),
        Some(litecode::types::CompactionFailureKind::Quality),
        "{label}: quality gate must use the structured Quality subtype: {error:?}"
    );
    assert!(
        error.to_string().contains(expected_failure),
        "{label}: expected {expected_failure} in rejection, got {error}"
    );
    let mut lifecycle_fail_kind = None;
    let mut failed_telemetry = None;
    while let Ok(envelope) = lifecycle.try_recv() {
        if let InternalEvent::CompactionLifecycle {
            stage: CompactionStage::Failed,
            fail_kind,
            telemetry,
            ..
        } = envelope.event
        {
            lifecycle_fail_kind = fail_kind;
            failed_telemetry = Some(telemetry);
        }
    }
    assert_eq!(
        lifecycle_fail_kind,
        Some(CompactionFailKind::Quality),
        "{label}: lifecycle Failed event must retain the structured quality subtype"
    );
    let telemetry = failed_telemetry.expect("quality failure telemetry");
    assert!(
        telemetry.tokens_before.is_some()
            && telemetry.tokens_pruned.is_some()
            && telemetry.tokens_after.is_some(),
        "{label}: quality failure must retain measured token fields: {telemetry:?}"
    );
    assert_eq!(
        telemetry.summary_attempts,
        Some(2),
        "{label}: quality failure must report both total attempts"
    );
    assert!(
        telemetry.review_score.is_some(),
        "{label}: quality failure must retain the final review score: {telemetry:?}"
    );
    assert_eq!(
        telemetry.overflow_retry,
        Some(0),
        "{label}: non-overflow failure must report zero overflow retries"
    );
    assert_eq!(
        row_snapshot(&rows),
        before,
        "{label}: rejected quality summary must restore in-memory rows"
    );
    assert_eq!(
        row_snapshot(&sessions.data().working_set_blocking(&sid).unwrap()),
        before,
        "{label}: durable rows must remain unchanged"
    );
    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted)),
        "{label}: a quality-rejected summary must not write a Compacted event"
    );
    let requests = captured.captured();
    assert_eq!(
        requests.len(),
        2,
        "{label}: quality failures get exactly one retry"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn placeholder_summary_is_a_quality_failure_without_checkpoint_or_row_mutation() {
    let anchor = "quality-placeholder-anchor";
    let weak = valid_summary(anchor)
        .replace(
            "- Validate the terminal summary before checkpoint persistence",
            "- TBD",
        )
        .replace(
            "- Continue the main request after the accepted checkpoint",
            "- see above",
        );
    assert_rejected_quality_summary(anchor, weak, "quality-placeholder", "NonActionableWork").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prefixed_placeholder_summary_is_a_quality_failure_without_checkpoint() {
    let anchor = "quality-prefixed-anchor";
    let weak = valid_summary(anchor)
        .replace(
            "- Validate the terminal summary before checkpoint persistence",
            "- TBD: run the next validation step",
        )
        .replace(
            "- Continue the main request after the accepted checkpoint",
            "- TODO: implement the pending parser",
        );
    assert_rejected_quality_summary(anchor, weak, "quality-prefixed", "NonActionableWork").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_source_anchor_summary_is_a_quality_failure_without_checkpoint() {
    let anchor = "wrong-anchor-request src/context_pipeline/compact.rs ENOENT";
    let wrong = valid_summary(anchor)
        .replace(
            "- src/context_pipeline/compact.rs: mechanical summary acceptance",
            "- src/other_module/unrelated.rs: unrelated implementation detail",
        )
        .replace(
            "- max_output_tokens and max_tokens map to incomplete summaries",
            "- EACCES permission denied while opening a different file",
        );
    assert_rejected_quality_summary(
        anchor,
        wrong,
        "quality-wrong-anchor",
        "MissingSectionAnchor",
    )
    .await;
}

fn valid_summary(anchor: &str) -> String {
    format!(
        "## Primary Request and Intent\n- Preserve the exact request: {anchor}\n\n\n## Key Technical Concepts\n- Responses, Chat, and Messages terminal status mapping\n\n## Files and Code\n- src/context_pipeline/compact.rs: mechanical summary acceptance\n\n## Errors and Fixes\n- max_output_tokens and max_tokens map to incomplete summaries\n\n## Pending Jobs\n- none\n\n## Current Work\n- Validate the terminal summary before checkpoint persistence\n\n## Next Step\n- Continue the main request after the accepted checkpoint\n\n## Critical Context\n- Keep the compaction prefix stable across retries"
    )
}

async fn serve_sse_once(sse: String) -> (String, tokio::task::JoinHandle<serde_json::Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut wire = Vec::new();
        let mut chunk = [0u8; 8192];
        let header_end = loop {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0, "connection closed before headers");
            wire.extend_from_slice(&chunk[..n]);
            if let Some(pos) = wire.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos;
            }
        };
        let headers = String::from_utf8_lossy(&wire[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .expect("content-length");
        let body_start = header_end + 4;
        while wire.len() < body_start + content_length {
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0, "connection closed before body");
            wire.extend_from_slice(&chunk[..n]);
        }
        let body = &wire[body_start..body_start + content_length];
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            sse.len(),
            sse
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        serde_json::from_slice(body).expect("request JSON body")
    });
    (format!("http://{address}/v1"), handle)
}

async fn serve_sse_count(
    sse: String,
    requests: usize,
) -> (String, tokio::task::JoinHandle<Vec<serde_json::Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut bodies = Vec::new();
        for _ in 0..requests {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut wire = Vec::new();
            let mut chunk = [0u8; 8192];
            let header_end = loop {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0, "connection closed before headers");
                wire.extend_from_slice(&chunk[..n]);
                if let Some(pos) = wire.windows(4).position(|w| w == b"\r\n\r\n") {
                    break pos;
                }
            };
            let headers = String::from_utf8_lossy(&wire[..header_end]).to_string();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("content-length");
            let body_start = header_end + 4;
            while wire.len() < body_start + content_length {
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0, "connection closed before body");
                wire.extend_from_slice(&chunk[..n]);
            }
            let body = &wire[body_start..body_start + content_length];
            bodies.push(serde_json::from_slice(body).expect("request JSON body"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sse.len(),
                sse
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        bodies
    });
    (format!("http://{address}/v1"), handle)
}

fn catalog_text(endpoint: &str, endpoint_type: &str) -> String {
    format!(
        r#"version = 1
[[providers]]
id = "wire"
name = "Wire"
endpoint = "{endpoint}"
endpoint_type = "{endpoint_type}"
[[models]]
id = "m"
provider_id = "wire"
context_window = 128000
context_window_max = 128000
max_output = 8192
modalities = ["text"]
"#
    )
}

fn wire_request(endpoint: &str, endpoint_type: &str, session_id: &str) -> CompactionRequestContext {
    let catalog = ProviderCatalog::parse(
        &catalog_text(endpoint, endpoint_type),
        Path::new("<real-codec-finish>"),
    )
    .unwrap();
    let target = Arc::clone(catalog.model("wire/m").unwrap());
    let provider: Arc<dyn LlmProvider> =
        Arc::from(provider_from_model(Arc::clone(&target)).unwrap());
    CompactionRequestContext {
        provider,
        api_key: "test-key".into(),
        model: target.id.clone(),
        context_window: target.context_window,
        max_output_tokens: target.max_output,
        target,
        thinking: ThinkingSpec::Tier(Default::default()),
        instructions: "main system / instructions".into(),
        tools: Vec::new(),
        session_id: session_id.to_string(),
    }
}

fn response_output(text: &str, status: &str) -> serde_json::Value {
    serde_json::json!([{
        "type": "message",
        "id": "msg_wire",
        "role": "assistant",
        "status": status,
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    }])
}

fn response_object(status: &str, output: serde_json::Value, with_usage: bool) -> serde_json::Value {
    let mut response = serde_json::json!({
        "id": "resp_wire",
        "object": "response",
        "created_at": 1,
        "model": "m",
        "status": status,
        "output": output
    });
    if status == "incomplete" {
        response["incomplete_details"] = serde_json::json!({"reason": "max_output_tokens"});
    }
    if with_usage {
        response["usage"] = serde_json::json!({
            "input_tokens": 11,
            "output_tokens": 7,
            "total_tokens": 18,
            "input_tokens_details": {"cached_tokens": 2},
            "output_tokens_details": {"reasoning_tokens": 0}
        });
    }
    response
}

fn responses_sse(event_type: &str, response: serde_json::Value) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "type": event_type,
            "sequence_number": 1,
            "response": response
        })
    )
}

fn chat_chunk(delta: serde_json::Value, finish_reason: Option<&str>) -> String {
    let mut choice = serde_json::json!({"index": 0, "delta": delta});
    if let Some(reason) = finish_reason {
        choice["finish_reason"] = serde_json::json!(reason);
    }
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "id": "chat_wire",
            "object": "chat.completion.chunk",
            "model": "m",
            "choices": [choice]
        })
    )
}

fn chat_done() -> &'static str {
    "data: [DONE]\n\n"
}

fn messages_sse(events: Vec<serde_json::Value>) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn message_start() -> serde_json::Value {
    serde_json::json!({
        "type": "message_start",
        "message": {
            "id": "msg_wire",
            "model": "m",
            "usage": {
                "input_tokens": 11,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 2,
                "output_tokens": 1
            }
        }
    })
}

fn message_text_events(text: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
    ]
}

struct CodecCase {
    name: &'static str,
    endpoint_type: &'static str,
    sse: String,
}

fn real_codec_failure_cases() -> Vec<CodecCase> {
    let responses_incomplete = responses_sse(
        "response.incomplete",
        response_object(
            "incomplete",
            response_output("partial summary", "incomplete"),
            true,
        ),
    );
    let responses_empty = responses_sse(
        "response.completed",
        response_object("completed", serde_json::json!([]), true),
    );
    let responses_tool_call = responses_sse(
        "response.completed",
        response_object(
            "completed",
            serde_json::json!([{
                "type": "function_call",
                "id": "fc_wire",
                "call_id": "call_wire",
                "name": "read",
                "arguments": "{}",
                "status": "completed"
            }]),
            true,
        ),
    );
    let chat_length = format!(
        "{}{}{}",
        chat_chunk(serde_json::json!({"content": "partial summary"}), None),
        chat_chunk(serde_json::json!({}), Some("length")),
        chat_done()
    );
    let chat_max_tokens = format!(
        "{}{}{}",
        chat_chunk(serde_json::json!({"content": "truncated summary"}), None),
        chat_chunk(serde_json::json!({}), Some("max_tokens")),
        chat_done()
    );
    let chat_missing_finish = format!(
        "{}{}",
        chat_chunk(serde_json::json!({"content": "missing finish"}), None),
        chat_done()
    );
    let chat_empty = format!(
        "{}{}",
        chat_chunk(serde_json::json!({}), Some("stop")),
        chat_done()
    );
    let messages_max_tokens = messages_sse({
        let mut events = vec![message_start()];
        events.extend(message_text_events("partial summary"));
        events.push(serde_json::json!({
            "type":"message_delta",
            "delta":{"stop_reason":"max_tokens"},
            "usage":{"output_tokens":7}
        }));
        events.push(serde_json::json!({"type":"message_stop"}));
        events
    });
    let messages_missing_stop = messages_sse({
        let mut events = vec![message_start()];
        events.extend(message_text_events("missing stop"));
        events.push(serde_json::json!({"type":"message_stop"}));
        events
    });
    vec![
        CodecCase {
            name: "responses response.incomplete",
            endpoint_type: "responses",
            sse: responses_incomplete,
        },
        CodecCase {
            name: "responses empty output",
            endpoint_type: "responses",
            sse: responses_empty,
        },
        CodecCase {
            name: "responses tool call",
            endpoint_type: "responses",
            sse: responses_tool_call,
        },
        CodecCase {
            name: "chat finish_reason=length",
            endpoint_type: "chat_completions",
            sse: chat_length,
        },
        CodecCase {
            name: "chat finish_reason=max_tokens",
            endpoint_type: "chat_completions",
            sse: chat_max_tokens,
        },
        CodecCase {
            name: "chat [DONE] without finish_reason",
            endpoint_type: "chat_completions",
            sse: chat_missing_finish,
        },
        CodecCase {
            name: "chat empty output",
            endpoint_type: "chat_completions",
            sse: chat_empty,
        },
        CodecCase {
            name: "messages stop_reason=max_tokens",
            endpoint_type: "messages",
            sse: messages_max_tokens,
        },
        CodecCase {
            name: "messages message_stop without stop_reason",
            endpoint_type: "messages",
            sse: messages_missing_stop,
        },
    ]
}

async fn assert_real_codec_case_rejected(case: CodecCase) {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<_> = (0..12)
        .map(|i| user_text(format!("codec-gate-seed-{i}")))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
    let before = row_snapshot(&rows);

    let (endpoint, handle) = serve_sse_count(case.sse, 2).await;
    let request = wire_request(&endpoint, case.endpoint_type, &sid);
    let error = CompactPolicy::compact_transcript(
        &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
        &sessions,
        &sid,
        &request,
        litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
        &ProviderPromptBaseline::default(),
        &mut rows,
        &CancellationToken::new(),
        CompactionTrigger::Auto,
        Some(case.name),
        &sessions.data_root_path(),
    )
    .await
    .expect_err(case.name);
    let bodies = handle.await.expect("captured request bodies");
    let body = bodies.first().expect("first request body");
    assert!(
        error.is_compaction_failure(),
        "{} must fail the finish gate before commit, got {error}",
        case.name
    );
    assert_eq!(
        error.compaction_failure_kind(),
        Some(litecode::types::CompactionFailureKind::SummaryFinish),
        "{} must classify as a finish/empty/tool-call rejection",
        case.name
    );
    assert_eq!(
        row_snapshot(&rows),
        before,
        "{} must restore the in-memory snapshot",
        case.name
    );
    assert!(
        !sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted)),
        "{} must not write a Compacted event",
        case.name
    );
    assert_eq!(body["model"], "m");
    let cap = body
        .get("max_output_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(serde_json::Value::as_u64);
    assert_eq!(cap, Some(8192), "{} wire summary cap", case.name);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_codec_missing_or_truncated_finish_never_writes_a_checkpoint() {
    for case in real_codec_failure_cases() {
        assert_real_codec_case_rejected(case).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_real_codec_preserves_artifact_finish_usage_and_model() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let mut seed: Vec<_> = (0..11)
        .map(|i| user_text(format!("codec-success-seed-{i}-{}", "word ".repeat(200))))
        .collect();
    let anchor = "codec-success-anchor";
    seed.push(user_text(anchor));
    seed.push(user_text("codec-kept-tail"));
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
    let before = row_snapshot(&rows);

    let summary = valid_summary(anchor);
    let (endpoint, handle) = serve_sse_once(responses_sse(
        "response.completed",
        response_object("completed", response_output(&summary, "completed"), true),
    ))
    .await;
    let request = wire_request(&endpoint, "responses", &sid);
    let artifact = CompactPolicy::compact_transcript(
        &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
        &sessions,
        &sid,
        &request,
        litecode::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
        &ProviderPromptBaseline::default(),
        &mut rows,
        &CancellationToken::new(),
        CompactionTrigger::Auto,
        Some("real-codec-finish-success"),
        &sessions.data_root_path(),
    )
    .await
    .expect("completed summary")
    .expect("compaction ran");
    let body = handle.await.expect("captured request body");
    assert_eq!(body["model"], "m");
    assert_eq!(artifact.text, summary);
    assert_eq!(artifact.finish, SummaryFinish::Completed);
    assert_eq!(artifact.model, "m");
    let usage = artifact
        .usage
        .expect("provider usage must survive the boundary");
    assert_eq!(usage.prompt_tokens, 11);
    assert_eq!(usage.completion_tokens, 7);
    assert_ne!(
        row_snapshot(&rows),
        before,
        "successful compaction rewrites rows"
    );
    assert!(
        sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|event| matches!(event.event_type, EventType::Compacted)),
        "successful compaction writes Compacted"
    );
}
