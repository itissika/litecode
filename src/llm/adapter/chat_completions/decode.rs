use serde_json::Value;
use uuid::Uuid;

use crate::authority::responses::{
    AssistantRole, FunctionToolCall, Item, MessageItem, OutputMessage, OutputMessageContent,
    OutputStatus, OutputTextContent, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
};

use super::encode::REASONING_CONTENT_KEY;

fn synth_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

pub(crate) fn chat_reasoning_text(node: &Value) -> Option<&str> {
    if let Some(text) = node
        .get(REASONING_CONTENT_KEY)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(text);
    }
    if let Some(text) = node
        .get("reasoning")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(text);
    }
    node.pointer("/reasoning/content")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

pub(crate) fn items_from_chat_message(message: &Value) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(text) = chat_reasoning_text(message) {
        items.push(Item::Reasoning(ReasoningItem {
            id: Some(synth_id("cc_rs")),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent {
                    text: text.to_string(),
                },
            )]),
            encrypted_content: None,
            status: Some(OutputStatus::Completed),
        }));
    }
    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !content.is_empty() {
        items.push(Item::Message(MessageItem::Output(OutputMessage {
            id: synth_id("cc_msg"),
            role: AssistantRole::Assistant,
            status: OutputStatus::Completed,
            phase: None,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: content,
                annotations: vec![],
                logprobs: None,
            })],
        })));
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let item_id = if call_id.is_empty() {
                synth_id("cc_fc")
            } else {
                call_id.clone()
            };
            items.push(Item::FunctionCall(FunctionToolCall {
                id: Some(item_id),
                call_id,
                name,
                arguments,
                status: Some(OutputStatus::Completed),
                namespace: None,
            }));
        }
    }
    items
}
