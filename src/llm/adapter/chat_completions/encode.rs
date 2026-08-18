use serde_json::{Map, Value};

use crate::authority::responses::{FunctionCallOutput, Item, MessageItem};
use crate::llm::request::ModelRequest;
use crate::types::Result;

pub(crate) const REASONING_CONTENT_KEY: &str = "reasoning_content";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningWriteKey {
    ReasoningContent,
}

impl ReasoningWriteKey {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReasoningContent => REASONING_CONTENT_KEY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChatEncodeOpts {
    pub include_stream_usage: bool,
    pub reasoning_write_key: ReasoningWriteKey,
}

impl ChatEncodeOpts {
    pub(crate) const OPENCODE: Self = Self {
        include_stream_usage: true,
        reasoning_write_key: ReasoningWriteKey::ReasoningContent,
    };

    pub(crate) const ARK: Self = Self {
        include_stream_usage: true,
        reasoning_write_key: ReasoningWriteKey::ReasoningContent,
    };
}

fn item_text(item: &Item) -> String {
    crate::types::item_text_preview(item)
}

pub(crate) fn encode_chat_body(
    params: &ModelRequest,
    stream: bool,
    opts: &ChatEncodeOpts,
) -> Result<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if !params.instructions.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": params.instructions,
        }));
    }

    let reasoning_key = opts.reasoning_write_key.as_str();
    let replay_reasoning = !params.tools.is_empty()
        || params
            .input
            .iter()
            .any(|item| matches!(item, Item::Reasoning(_)));
    let mut turn = AssistantTurn::default();

    for item in &params.input {
        match item {
            Item::Reasoning(_) => {
                let text = item_text(item);
                if !text.is_empty() {
                    turn.push_reasoning(&text);
                }
            }
            Item::FunctionCall(fc) => {
                turn.tool_calls.push(serde_json::json!({
                    "id": fc.call_id,
                    "type": "function",
                    "function": {
                        "name": fc.name,
                        "arguments": fc.arguments,
                    }
                }));
            }
            Item::FunctionCallOutput(out) => {
                turn.flush(&mut messages, reasoning_key, replay_reasoning);
                let content = match &out.output {
                    FunctionCallOutput::Text(s) => s.clone(),
                    FunctionCallOutput::Content(_) => item_text(item),
                };
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": out.call_id,
                    "content": content,
                }));
            }
            Item::Message(MessageItem::Input(_)) => {
                turn.flush(&mut messages, reasoning_key, replay_reasoning);
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": item_text(item),
                }));
            }
            Item::Message(MessageItem::Output(_)) => {
                let text = item_text(item);
                match &mut turn.content {
                    Some(existing) => {
                        if !existing.is_empty() && !text.is_empty() {
                            existing.push('\n');
                        }
                        existing.push_str(&text);
                    }
                    None => turn.content = Some(text),
                }
            }
            _ => {}
        }
    }
    turn.flush(&mut messages, reasoning_key, replay_reasoning);

    let tools: Vec<Value> = params
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": params.model,
        "messages": messages,
        "stream": stream,
        "temperature": params.temperature,
    });
    if params.max_output_tokens > 0 {
        body["max_tokens"] = Value::from(params.max_output_tokens);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if stream && opts.include_stream_usage {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    Ok(body)
}

#[derive(Default)]
struct AssistantTurn {
    reasoning: String,
    content: Option<String>,
    tool_calls: Vec<Value>,
}

impl AssistantTurn {
    fn push_reasoning(&mut self, text: &str) {
        if !self.reasoning.is_empty() {
            self.reasoning.push('\n');
        }
        self.reasoning.push_str(text);
    }

    fn is_empty(&self) -> bool {
        self.reasoning.is_empty() && self.content.is_none() && self.tool_calls.is_empty()
    }

    fn flush(&mut self, messages: &mut Vec<Value>, reasoning_key: &str, replay_reasoning: bool) {
        if self.is_empty() {
            return;
        }
        let mut obj = Map::new();
        obj.insert("role".into(), Value::String("assistant".into()));
        if !self.tool_calls.is_empty() {
            let content = match &self.content {
                Some(s) if !s.is_empty() => Value::String(s.clone()),
                _ => Value::Null,
            };
            obj.insert("content".into(), content);
            obj.insert(
                "tool_calls".into(),
                Value::Array(std::mem::take(&mut self.tool_calls)),
            );
        } else {
            obj.insert(
                "content".into(),
                Value::String(self.content.take().unwrap_or_default()),
            );
        }
        if replay_reasoning || !self.reasoning.is_empty() {
            obj.insert(
                reasoning_key.to_string(),
                Value::String(std::mem::take(&mut self.reasoning)),
            );
        }
        self.content = None;
        messages.push(Value::Object(obj));
    }
}
