//! Agent e2e against a local Responses SSE server (product default protocol).

mod common;

use std::sync::Arc;

use common::{
    TEST_PROVIDER_ID, build_runtime_with_provider, fixture_responses_sse, ready_test_provider,
    serve_responses_queue, test_agent,
};
use litecode::llm::provider_from_definition;
use litecode::types::LitecodeError;

fn responses_provider(endpoint: &str) -> Arc<dyn litecode::llm::LlmProvider> {
    let def = ready_test_provider(TEST_PROVIDER_ID, endpoint, "test-key");
    assert_eq!(
        def.adapter_id, "openai_responses",
        "e2e must wire openai_responses — Chat fixtures are not the product path"
    );
    Arc::from(provider_from_definition(&def).expect("Responses provider"))
}

#[tokio::test(flavor = "current_thread")]
async fn agent_e2e_responses_text_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = fixture_responses_sse("text_only");
    assert!(
        body.contains("Hello from Responses replay"),
        "Responses fixture must carry product text"
    );
    assert!(
        !body.contains("chat.completion"),
        "must not be Chat Completions dialect"
    );
    let endpoint = serve_responses_queue(vec![body]).await;
    let provider = responses_provider(&endpoint);
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider,
    );

    let result = runtime.run("say hello").await.expect("turn completes");
    assert!(
        result.contains("Hello from Responses replay"),
        "final text must contain Responses fixture output (got {result:?}) — \
         empty-or-any-text would be a false green"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn agent_e2e_responses_tool_loop_hits_max_steps() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("test.txt"), "content").unwrap();

    // Two identical tool-call Responses fixtures; max_steps=1 ⇒ MaxStepsReached
    // after the first tool round continues into step 2.
    let tool = fixture_responses_sse("tool_call");
    assert!(
        tool.contains("function_call") && tool.contains("\"name\":\"read\""),
        "tool fixture must be Responses function_call for read"
    );
    assert!(
        tool.contains("response.output_item.added"),
        "tool fixture must emit early name via output_item.added"
    );
    let endpoint = serve_responses_queue(vec![tool.clone(), tool]).await;
    let provider = responses_provider(&endpoint);
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 1),
        provider,
    );

    let err = runtime.run("read file").await.expect_err("max steps");
    assert!(
        matches!(err, LitecodeError::MaxStepsReached),
        "expected MaxStepsReached after Responses FunctionCall Items enter the loop, got {err:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn agent_e2e_incomplete_seals_text_to_disk() {
    let dir = tempfile::tempdir().expect("tempdir");
    let text_delta = serde_json::json!({
        "type": "response.output_text.delta",
        "sequence_number": 1,
        "item_id": "msg_inc_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "stopped mid thought"
    });
    let incomplete = serde_json::json!({
        "type": "response.incomplete",
        "sequence_number": 2,
        "response": {
            "id": "resp_inc_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "incomplete",
            "output": [{
                "type": "message",
                "id": "msg_inc_1",
                "role": "assistant",
                "status": "incomplete",
                "content": [{
                    "type": "output_text",
                    "text": "stopped mid thought",
                    "annotations": []
                }]
            }]
        }
    });
    let body = format!("data: {text_delta}\n\ndata: {incomplete}\n\n");
    let endpoint = serve_responses_queue(vec![body]).await;
    let provider = responses_provider(&endpoint);
    let mut runtime = build_runtime_with_provider(
        dir.path(),
        test_agent(vec!["read".into()], "readonly", 5),
        provider,
    );

    let result = runtime.run("talk").await.expect("incomplete still commits");
    assert!(
        result.contains("stopped mid thought"),
        "final text must include incomplete seal (got {result:?})"
    );

    let sid = runtime.session_id.clone();
    let items = runtime
        .sessions()
        .with_entry_store(&sid, |s| Ok(s.load_transcript()?))
        .expect("load transcript");
    assert!(
        items.iter().any(|item| {
            litecode::types::item_text_preview(item) == "stopped mid thought"
                && matches!(
                    item,
                    litecode::types::Item::Message(
                        litecode::authority::responses::MessageItem::Output(msg)
                    ) if msg.status == litecode::authority::responses::OutputStatus::Incomplete
                )
        }),
        "disk must contain sealed incomplete text: {items:?}"
    );
}

#[test]
fn responses_fixtures_are_not_chat_dialect() {
    let text = fixture_responses_sse("text_only");
    let tool = fixture_responses_sse("tool_call");
    for body in [&text, &tool] {
        assert!(
            body.contains("response.completed")
                || body.contains("\"type\":\"response.completed\"")
                || body.contains("response.completed".trim())
                || body.contains("response"),
            "fixture must look like Responses SSE"
        );
        assert!(!body.contains("chat.completion.chunk"));
        assert!(!body.contains("\"object\":\"chat.completion"));
    }
}
