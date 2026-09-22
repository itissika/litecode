//! Codec golden tests extracted from the OpenCode adapter.

use super::catalog::{chat_post_url, models_get_url, parse_model_catalog};
use super::encode::{ChatEncodeOpts, REASONING_CONTENT_KEY, encode_chat_body};
use super::usage::chat_usage_to_responses;
use crate::authority::responses::{
    AssistantRole, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, Item,
    MessageItem, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
    ReasoningItem, ReasoningItemContent, ReasoningTextContent,
};
use crate::llm::request::{ModelRequest, ToolDef};
use crate::platform_knobs::{ThinkingSpec, ThinkingTier};
use crate::types::user_text;

fn sample_request(tools: Vec<ToolDef>) -> ModelRequest {
    ModelRequest {
        model: "deepseek-v4-flash-free".into(),
        instructions: "sys".into(),
        input: vec![user_text("hello")],
        tools,
        max_output_tokens: 64,
        temperature: 0.0,
        thinking: ModelRequest::sample_thinking(),
        json_output: false,
        session_id: Some("ses_test".into()),
    }
}

#[test]
fn chat_url_is_standard_path() {
    let url = chat_post_url("https://opencode.ai/zen/v1");
    assert_eq!(url, "https://opencode.ai/zen/v1/chat/completions");
}

#[test]
fn parse_catalog_ids() {
    let body = r#"{"object":"list","data":[{"id":"a"},{"id":"b"}]}"#;
    assert_eq!(parse_model_catalog(body).unwrap(), vec!["a", "b"]);
}

#[test]
fn encode_user_and_system() {
    let body = encode_chat_body(&sample_request(vec![]), true, &ChatEncodeOpts::OPENCODE).unwrap();
    assert_eq!(body["model"], "deepseek-v4-flash-free");
    assert_eq!(body["stream"], true);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "hello");
    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[test]
fn commandcode_encodes_reasoning_effort_for_every_model() {
    let mut req = sample_request(vec![]);
    req.thinking = ThinkingSpec::Tier(ThinkingTier::High);
    req.model = "glm-5.2".into();

    let commandcode = encode_chat_body(&req, false, &ChatEncodeOpts::COMMANDCODE).unwrap();
    assert_eq!(commandcode["reasoning_effort"], "max");
}

#[test]
fn opencode_encodes_reasoning_effort_for_declared_families() {
    let mut req = sample_request(vec![]);
    assert_eq!(req.model, "deepseek-v4-flash-free");

    // Zen/Go declare `low | high | max` for the DeepSeek v4 flash line and for
    // GLM 5.3, so the three platform tiers land on three distinct wire values.
    for model in ["deepseek-v4-flash-free", "glm-5.3", "glm-5.3-flash"] {
        req.model = model.into();
        for (tier, expected) in [
            (ThinkingTier::Low, "low"),
            (ThinkingTier::Medium, "high"),
            (ThinkingTier::High, "max"),
        ] {
            req.thinking = ThinkingSpec::Tier(tier);
            let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
            assert_eq!(body["reasoning_effort"], expected, "{model} tier {tier:?}");
        }
    }

    // Generation marker, not a bare `deepseek` prefix: a later 4.x keeps working.
    req.model = "deepseek-v4.1-flash".into();
    req.thinking = ThinkingSpec::Tier(ThinkingTier::High);
    let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
    assert_eq!(body["reasoning_effort"], "max");
}

#[test]
fn opencode_clamps_low_up_to_a_models_declared_floor() {
    // `deepseek-v4-pro` declares `high | max` only, so a raw `low` is rejected.
    let mut req = sample_request(vec![]);
    req.model = "deepseek-v4-pro".into();

    for (tier, expected) in [
        (ThinkingTier::Low, "high"),
        (ThinkingTier::Medium, "high"),
        (ThinkingTier::High, "max"),
    ] {
        req.thinking = ThinkingSpec::Tier(tier);
        let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
        assert_eq!(body["reasoning_effort"], expected, "tier {tier:?}");
    }
}

#[test]
fn opencode_leaves_other_families_on_the_vendor_default() {
    // Every model whose declared vocabulary is not exactly `low | high | max`
    // keeps the vendor default: `glm-5.2` / `kimi-k3` / `hy3` drift per model,
    // `mimo-v2.5` declares no effort at all, and the undocumented
    // `deepseek-flash` / legacy `deepseek-reasoner`-style ids are unverified.
    let mut req = sample_request(vec![]);
    req.thinking = ThinkingSpec::Tier(ThinkingTier::High);
    for model in [
        "glm-5.2",
        "glm-5.1",
        "kimi-k3",
        "kimi-k2.6",
        "mimo-v2.5",
        "hy3",
        "minimax-m3",
        "deepseek-flash",
    ] {
        req.model = model.into();
        let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
        assert!(body.get("reasoning_effort").is_none(), "{model}: {body}");
    }
}

#[test]
fn thinking_off_omits_reasoning_effort_on_both_chat_adapters() {
    let mut req = sample_request(vec![]);
    req.thinking = ThinkingSpec::Off;

    // `none` is not in every host's effort literal (Zen rejects it for models
    // whose upstream omits it), so Off omits the field instead of cancelling.
    for opts in [ChatEncodeOpts::COMMANDCODE, ChatEncodeOpts::OPENCODE] {
        let body = encode_chat_body(&req, true, &opts).unwrap();
        assert!(body.get("reasoning_effort").is_none(), "got {body}");
    }
}

#[test]
fn encode_replays_tools_and_reasoning_key() {
    let fc = Item::FunctionCall(FunctionToolCall {
        id: Some("fc_1".into()),
        call_id: "call_1".into(),
        name: "read".into(),
        arguments: "{\"path\":\"a\"}".into(),
        status: Some(OutputStatus::Completed),
        namespace: None,
    });
    let out = Item::FunctionCallOutput(FunctionCallOutputItemParam {
        call_id: "call_1".into(),
        output: FunctionCallOutput::Text("ok".into()),
        id: None,
        status: None,
    });
    let reasoning = Item::Reasoning(ReasoningItem {
        id: Some("rs_1".into()),
        summary: vec![],
        content: Some(vec![ReasoningItemContent::ReasoningText(
            ReasoningTextContent {
                text: "think".into(),
            },
        )]),
        encrypted_content: None,
        status: Some(OutputStatus::Completed),
    });
    let req = ModelRequest {
        model: "big-pickle".into(),
        instructions: String::new(),
        input: vec![user_text("q"), reasoning, fc, out],
        tools: vec![ToolDef {
            name: "read".into(),
            description: "r".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
        max_output_tokens: 16,
        temperature: 0.0,
        thinking: ModelRequest::sample_thinking(),
        json_output: false,
        session_id: Some("ses_test".into()),
    };
    let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
    let msgs = body["messages"].as_array().unwrap();
    let assistant = msgs
        .iter()
        .find(|m| m["role"] == "assistant" && m.get("tool_calls").is_some())
        .unwrap();
    assert_eq!(assistant[REASONING_CONTENT_KEY], "think");
    assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
    assert!(assistant["content"].is_null());
    let tool = msgs.iter().find(|m| m["role"] == "tool").unwrap();
    assert_eq!(tool["content"], "ok");
}

#[test]
fn encode_keeps_reasoning_on_same_assistant_as_text() {
    let reasoning = Item::Reasoning(ReasoningItem {
        id: Some("rs_1".into()),
        summary: vec![],
        content: Some(vec![ReasoningItemContent::ReasoningText(
            ReasoningTextContent {
                text: "think".into(),
            },
        )]),
        encrypted_content: None,
        status: Some(OutputStatus::Completed),
    });
    let reply = Item::Message(MessageItem::Output(OutputMessage {
        id: "msg_1".into(),
        role: AssistantRole::Assistant,
        status: OutputStatus::Completed,
        phase: None,
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: "hello".into(),
            annotations: vec![],
            logprobs: None,
        })],
    }));
    let req = ModelRequest {
        model: "deepseek-v4-flash-free".into(),
        instructions: String::new(),
        input: vec![user_text("q"), reasoning, reply, user_text("again")],
        tools: vec![],
        max_output_tokens: 16,
        temperature: 0.0,
        thinking: ModelRequest::sample_thinking(),
        json_output: false,
        session_id: Some("ses_test".into()),
    };
    let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
    let msgs = body["messages"].as_array().unwrap();
    let assistants: Vec<_> = msgs.iter().filter(|m| m["role"] == "assistant").collect();
    assert_eq!(assistants.len(), 1, "{msgs:?}");
    assert_eq!(assistants[0]["content"], "hello");
    assert_eq!(assistants[0][REASONING_CONTENT_KEY], "think");
}

#[test]
fn encode_tools_request_puts_reasoning_key_on_every_assistant() {
    let reply = Item::Message(MessageItem::Output(OutputMessage {
        id: "msg_1".into(),
        role: AssistantRole::Assistant,
        status: OutputStatus::Completed,
        phase: None,
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: "ok".into(),
            annotations: vec![],
            logprobs: None,
        })],
    }));
    let req = ModelRequest {
        model: "deepseek-v4-flash-free".into(),
        instructions: String::new(),
        input: vec![user_text("q"), reply],
        tools: vec![ToolDef {
            name: "read".into(),
            description: "r".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
        max_output_tokens: 16,
        temperature: 0.0,
        thinking: ModelRequest::sample_thinking(),
        json_output: false,
        session_id: Some("ses_test".into()),
    };
    let body = encode_chat_body(&req, false, &ChatEncodeOpts::OPENCODE).unwrap();
    let assistant = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "assistant")
        .unwrap();
    assert_eq!(assistant[REASONING_CONTENT_KEY], "");
}

#[test]
fn catalog_url_strips_responses_suffix() {
    assert_eq!(
        models_get_url("https://opencode.ai/zen/v1"),
        "https://opencode.ai/zen/v1/models"
    );
    assert_eq!(
        models_get_url("https://ark.cn-beijing.volces.com/api/coding/v3"),
        "https://ark.cn-beijing.volces.com/api/coding/v3/models"
    );
    assert_eq!(
        models_get_url("https://ark.cn-beijing.volces.com/api/coding/v3/responses"),
        "https://ark.cn-beijing.volces.com/api/coding/v3/models"
    );
    assert_eq!(
        models_get_url("https://api.deepseek.com"),
        "https://api.deepseek.com/models"
    );
    assert_eq!(
        models_get_url("https://api.deepseek.com/responses"),
        "https://api.deepseek.com/models"
    );
}

#[test]
fn maps_chat_usage_cache_hit() {
    let usage = serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "total_tokens": 120,
        "prompt_tokens_details": { "cached_tokens": 80 }
    });
    let mapped = chat_usage_to_responses(&usage).unwrap();
    assert_eq!(mapped["input_tokens"], 100);
    assert_eq!(mapped["output_tokens"], 20);
    assert_eq!(mapped["input_tokens_details"]["cached_tokens"], 80);
}

#[test]
fn maps_prompt_cache_hit_tokens_alias() {
    let usage = serde_json::json!({
        "prompt_tokens": 50,
        "completion_tokens": 2,
        "prompt_cache_hit_tokens": 40
    });
    let mapped = chat_usage_to_responses(&usage).unwrap();
    assert_eq!(mapped["input_tokens_details"]["cached_tokens"], 40);
}
