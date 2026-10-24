//! PR-1 wire-level proof: the compaction HTTP body must carry the same
//! reasoning replay shape and thinking controls as the main request.
//!
//! Product-level `ModelRequest` equality is not enough: Responses `replay=text`
//! rewrites reasoning items inside the codec, and Messages serializes a signed
//! thinking block differently when thinking is disabled. These tests capture
//! the actual JSON POSTed to a loopback provider.

use std::path::Path;
use std::sync::Arc;

use litecode::authority::responses::{
    OutputStatus, ReasoningItem, ReasoningItemContent, ReasoningTextContent, SummaryPart,
    SummaryTextContent,
};
use litecode::llm::{
    COMPACTION_INSTRUCTION, CompactionRequestContext, LlmProvider, ModelRequest, RequestPurpose,
    ToolDef, provider_from_model,
};
use litecode::platform_knobs::{ThinkingSpec, ThinkingTier};
use litecode::provider_catalog::ProviderCatalog;
use litecode::types::{Item, item_text_preview, user_text};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

async fn serve_once(sse: String) -> (String, tokio::task::JoinHandle<serde_json::Value>) {
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

fn responses_sse() -> String {
    let completed = serde_json::json!({
        "id": "resp_1",
        "object": "response",
        "created_at": 1,
        "model": "m",
        "status": "completed",
        "output": [
            {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
             "content": [{"type": "output_text", "text": "ok", "annotations": []}]}
        ],
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
            "total_tokens": 2,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens_details": {"reasoning_tokens": 0}
        }
    });
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "type": "response.completed",
            "sequence_number": 1,
            "response": completed,
        })
    )
}

fn messages_sse() -> String {
    [
        serde_json::json!({"type":"message_start","message":{"id":"msg_1","model":"m","usage":{"input_tokens":1,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}),
        serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}),
        serde_json::json!({"type":"content_block_stop","index":0}),
        serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
        serde_json::json!({"type":"message_stop"}),
    ]
    .iter()
    .map(|event| format!("data: {event}\n\n"))
    .collect()
}

fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "read".into(),
        description: "read a file".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }]
}

fn reasoning_summary() -> Item {
    Item::Reasoning(ReasoningItem {
        id: Some("rs_1".into()),
        summary: vec![SummaryPart::SummaryText(SummaryTextContent {
            text: "the model's own prior reasoning".into(),
        })],
        content: None,
        encrypted_content: None,
        status: Some(OutputStatus::Completed),
    })
}

fn signed_reasoning() -> Item {
    Item::Reasoning(ReasoningItem {
        id: Some("rs_signed".into()),
        summary: vec![],
        content: Some(vec![ReasoningItemContent::ReasoningText(
            ReasoningTextContent {
                text: "the signed prior thought".into(),
            },
        )]),
        encrypted_content: Some("sig-1".into()),
        status: Some(OutputStatus::Completed),
    })
}

fn main_request(input: Vec<Item>) -> ModelRequest {
    ModelRequest {
        model: "m".into(),
        instructions: "same system".into(),
        input,
        tools: tools(),
        max_output_tokens: 8192,
        thinking: ThinkingSpec::Tier(ThinkingTier::Medium),
        json_output: false,
        session_id: Some("ses_wire".into()),
        purpose: RequestPurpose::Main,
    }
}

fn responses_catalog(endpoint: &str) -> String {
    format!(
        r#"version = 1

[[providers]]
id = "p"
name = "Wire"
endpoint = "{endpoint}"
endpoint_type = "responses"

[[models]]
id = "m"
provider_id = "p"
context_window = 128000
context_window_max = 128000
max_output = 8192
modalities = ["text"]
reasoning = {{ summary = "summarized", replay = "text", tiers = {{ low = "low", medium = "medium", high = "high" }} }}
"#
    )
}

fn messages_catalog(endpoint: &str) -> String {
    format!(
        r#"version = 1

[[providers]]
id = "p"
name = "Wire"
endpoint = "{endpoint}"
endpoint_type = "messages"

[[models]]
id = "m"
provider_id = "p"
context_window = 128000
context_window_max = 128000
max_output = 8192
modalities = ["text"]
reasoning = {{ summary = "summarized", tiers = {{ off = "disabled", low = "low", medium = "medium", high = "high" }} }}
"#
    )
}

fn without_cache_control(parts: &[serde_json::Value]) -> Vec<serde_json::Value> {
    parts
        .iter()
        .map(|part| {
            let mut part = part.clone();
            if let Some(object) = part.as_object_mut() {
                object.remove("cache_control");
            }
            part
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn responses_text_replay_wire_prefix_matches_the_main_request() {
    let input = vec![user_text("before"), reasoning_summary(), user_text("after")];
    let main = main_request(input.clone());

    let (endpoint, main_handle) = serve_once(responses_sse()).await;
    let catalog = ProviderCatalog::parse(
        &responses_catalog(&endpoint),
        Path::new("<responses-wire-main>"),
    )
    .unwrap();
    let main_model = Arc::clone(catalog.model("p/m").unwrap());
    let main_provider = provider_from_model(Arc::clone(&main_model)).unwrap();
    let cancel = CancellationToken::new();
    main_provider
        .complete_with_stream_events(&main, "test-key", None, &cancel)
        .await
        .expect("main request");
    let main_body = main_handle.await.expect("captured main body");

    let (endpoint, compact_handle) = serve_once(responses_sse()).await;
    let catalog = ProviderCatalog::parse(
        &responses_catalog(&endpoint),
        Path::new("<responses-wire-compact>"),
    )
    .unwrap();
    let compact_model = Arc::clone(catalog.model("p/m").unwrap());
    let compact_provider: Arc<dyn LlmProvider> =
        Arc::from(provider_from_model(Arc::clone(&compact_model)).unwrap());
    let context = CompactionRequestContext {
        provider: compact_provider,
        api_key: "test-key".into(),
        model: compact_model.id.clone(),
        context_window: compact_model.context_window,
        max_output_tokens: compact_model.max_output,
        target: Arc::clone(&compact_model),
        thinking: main.thinking,
        instructions: main.instructions.clone(),
        tools: main.tools.clone(),
        session_id: main.session_id.clone().expect("session"),
    };
    let compact = context.build_model_request(input, 8192);
    context
        .provider
        .as_ref()
        .complete_with_stream_events(&compact, "test-key", None, &cancel)
        .await
        .expect("compaction request");
    let compact_body = compact_handle.await.expect("captured compaction body");

    let main_input = main_body["input"].as_array().expect("main input");
    let compact_input = compact_body["input"].as_array().expect("compact input");
    assert_eq!(
        compact_input.len(),
        main_input.len() + 1,
        "compaction must append exactly one trailing instruction"
    );
    assert_eq!(
        &compact_input[..main_input.len()],
        main_input,
        "Responses reasoning replay must produce the same wire input prefix"
    );
    assert_eq!(
        compact_body["reasoning"], main_body["reasoning"],
        "thinking controls must match the main request exactly"
    );
    let reasoning = main_input
        .iter()
        .find(|item| item["type"] == "reasoning")
        .expect("reasoning item in wire body");
    assert_eq!(
        reasoning["content"][0]["type"], "reasoning_text",
        "thinking main request must replay readable reasoning as reasoning_text"
    );
    assert_eq!(
        reasoning["content"][0]["text"], "the model's own prior reasoning",
        "the summary must be the text the main request carried"
    );
    assert_eq!(compact_body["model"], main_body["model"]);
    assert_eq!(compact_body["instructions"], main_body["instructions"]);
    assert_eq!(compact_body["tools"], main_body["tools"]);
    assert_eq!(
        item_text_preview(compact.input.last().expect("instruction")),
        COMPACTION_INSTRUCTION,
        "last compaction item must be the instruction"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_signed_thinking_wire_prefix_keeps_main_thinking_enabled() {
    let input = vec![user_text("before"), signed_reasoning(), user_text("after")];
    let main = main_request(input.clone());

    let (endpoint, main_handle) = serve_once(messages_sse()).await;
    let catalog = ProviderCatalog::parse(
        &messages_catalog(&endpoint),
        Path::new("<messages-wire-main>"),
    )
    .unwrap();
    let main_model = Arc::clone(catalog.model("p/m").unwrap());
    let main_provider = provider_from_model(Arc::clone(&main_model)).unwrap();
    let cancel = CancellationToken::new();
    main_provider
        .complete_with_stream_events(&main, "test-key", None, &cancel)
        .await
        .expect("main request");
    let main_body = main_handle.await.expect("captured main body");

    let (endpoint, compact_handle) = serve_once(messages_sse()).await;
    let catalog = ProviderCatalog::parse(
        &messages_catalog(&endpoint),
        Path::new("<messages-wire-compact>"),
    )
    .unwrap();
    let compact_model = Arc::clone(catalog.model("p/m").unwrap());
    let compact_provider: Arc<dyn LlmProvider> =
        Arc::from(provider_from_model(Arc::clone(&compact_model)).unwrap());
    let context = CompactionRequestContext {
        provider: compact_provider,
        api_key: "test-key".into(),
        model: compact_model.id.clone(),
        context_window: compact_model.context_window,
        max_output_tokens: compact_model.max_output,
        target: Arc::clone(&compact_model),
        thinking: main.thinking,
        instructions: main.instructions.clone(),
        tools: main.tools.clone(),
        session_id: main.session_id.clone().expect("session"),
    };
    let compact = context.build_model_request(input, 8192);
    context
        .provider
        .as_ref()
        .complete_with_stream_events(&compact, "test-key", None, &cancel)
        .await
        .expect("compaction request");
    let compact_body = compact_handle.await.expect("captured compaction body");

    let main_messages = main_body["messages"].as_array().expect("main messages");
    let compact_messages = compact_body["messages"]
        .as_array()
        .expect("compact messages");
    assert_eq!(
        main_messages.len(),
        3,
        "fixture must encode user / signed assistant / user"
    );
    assert_eq!(
        compact_messages.len(),
        main_messages.len(),
        "the trailing user instruction is merged into the final user turn"
    );
    assert_eq!(
        &compact_messages[..main_messages.len() - 1],
        &main_messages[..main_messages.len() - 1],
        "Messages signed thinking prefix must be byte-identical"
    );
    let main_last_content = main_messages.last().expect("main last message")["content"]
        .as_array()
        .expect("main last content");
    let compact_last_content = compact_messages.last().expect("compact last message")["content"]
        .as_array()
        .expect("compact last content");
    assert_eq!(
        without_cache_control(&compact_last_content[..main_last_content.len()]),
        without_cache_control(main_last_content),
        "the final user content prefix must differ only by cache breakpoint placement"
    );
    assert!(
        compact_last_content[main_last_content.len()..]
            .iter()
            .any(|part| part["text"]
                .as_str()
                .is_some_and(|text| text.contains(COMPACTION_INSTRUCTION))),
        "the only trailing content change is the compaction instruction"
    );
    assert_eq!(
        compact_body["thinking"], main_body["thinking"],
        "Messages thinking controls must match the main request exactly"
    );
    assert_ne!(
        compact_body["thinking"],
        serde_json::json!({"type": "disabled"}),
        "signed thinking blocks must not coexist with disabled thinking"
    );
    let signed_blocks: Vec<_> = compact_messages
        .iter()
        .flat_map(|message| {
            message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|part| part["type"] == "thinking")
        })
        .collect();
    assert!(
        !signed_blocks.is_empty(),
        "compaction must replay the signed thinking block"
    );
    assert_eq!(
        signed_blocks[0]["signature"], "sig-1",
        "the signed thinking block must retain its provider signature"
    );
    assert_eq!(compact_body["model"], main_body["model"]);
    assert_eq!(compact_body["system"], main_body["system"]);
    assert_eq!(compact_body["tools"], main_body["tools"]);
    assert_eq!(
        item_text_preview(compact.input.last().expect("instruction")),
        COMPACTION_INSTRUCTION,
        "last compaction input item must be the instruction"
    );
}
