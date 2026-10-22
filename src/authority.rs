//! Agent data-shape authority (Phase 0 freeze).
//!
//! **Hard rule:** conversation atoms and stream events used as kernel truth MUST be
//! types from [`async_openai::types::responses`]. Do not define parallel
//! `Message` / `ContentBlock` / homemade `StreamEvent` dialects here or elsewhere.
//!
//! This module only re-exports. It does not wrap, subset, or rename serde shapes.

/// OpenAI Responses API types — sole Rust authority for agent transcript & stream.
pub mod responses {
    pub use async_openai::types::responses::*;
}

#[cfg(test)]
mod tests {
    use super::responses::{FunctionToolCall, Item, MessageItem, OutputMessage, ReasoningItem};

    #[test]
    fn authority_item_roundtrip_function_call() {
        let json = r#"{
            "type": "function_call",
            "id": "fc_1",
            "call_id": "call_1",
            "name": "bash",
            "arguments": "{\"command\":\"ls\"}"
        }"#;
        let item: Item = serde_json::from_str(json).expect("deserialize Item::FunctionCall");
        match &item {
            Item::FunctionCall(FunctionToolCall { name, call_id, .. }) => {
                assert_eq!(name, "bash");
                assert_eq!(call_id, "call_1");
            }
            other => panic!("expected FunctionCall, got {other:?}"),
        }
        let out = serde_json::to_value(&item).expect("serialize");
        assert_eq!(out["type"], "function_call");
        assert_eq!(out["name"], "bash");
    }

    #[test]
    fn authority_item_roundtrip_reasoning_and_message() {
        let reasoning = r#"{
            "type": "reasoning",
            "id": "rs_1",
            "summary": []
        }"#;
        let item: Item = serde_json::from_str(reasoning).expect("deserialize Reasoning");
        assert!(matches!(item, Item::Reasoning(ReasoningItem { .. })));

        let message = r#"{
            "type": "message",
            "id": "msg_1",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "hi", "annotations": []}]
        }"#;
        let item: Item = serde_json::from_str(message).expect("deserialize Message");
        match item {
            Item::Message(MessageItem::Output(OutputMessage { .. })) => {}
            other => panic!("expected OutputMessage, got {other:?}"),
        }
    }
}
