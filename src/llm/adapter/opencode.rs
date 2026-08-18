//! OpenCode Chat-Completions adapter (Zen default host; Go via endpoint override).
//!
//! Dialect conversion stays in this file. Kernel still sees authority Items /
//! `ResponseStreamEvent` only, forwarded through [`super::stream_contract`].

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::authority::responses::{
    AssistantRole, FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, Item,
    MessageItem, OutputItem, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
    ReasoningItem, ReasoningItemContent, ReasoningTextContent,
    ResponseFunctionCallArgumentsDeltaEvent, ResponseFunctionCallArgumentsDoneEvent,
    ResponseOutputItemAddedEvent, ResponseReasoningTextDeltaEvent, ResponseStreamEvent,
    ResponseTextDeltaEvent,
};
use crate::config::schema::ProviderAuth;
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::types::{LitecodeError, Result, StreamEvents};

use super::responses_sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};

/// Official Zen host. Empty Settings endpoint fills this; override for Go.
pub(crate) const DEFAULT_ENDPOINT: &str = "https://opencode.ai/zen/v1";

const ERROR_PREFIX: &str =
    "OpenCode adapter only speaks Chat Completions; this host's catalog may not include this id";

pub struct OpencodeProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl OpencodeProvider {
    pub fn new(endpoint: String, auth: ProviderAuth) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint);
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            client,
            endpoint_url: endpoint,
            auth,
        })
    }

    fn auth_header(&self, api_key: &str) -> (String, String) {
        match self.auth {
            ProviderAuth::Bearer => ("Authorization".to_string(), format!("Bearer {api_key}")),
            ProviderAuth::ApiKey => ("api-key".to_string(), api_key.to_string()),
        }
    }

    fn post_url(&self) -> String {
        chat_post_url(&self.endpoint_url)
    }
}

fn chat_post_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/{}/{}", "chat", "completions")
}

pub(crate) fn models_get_url(base: &str) -> String {
    format!("{}/models", base.trim_end_matches('/'))
}

fn reasoning_json_key() -> String {
    format!("{}_{}", "reasoning", "content")
}

fn wrap_upstream(status: reqwest::StatusCode, body: &str) -> LitecodeError {
    LitecodeError::Llm(format!("{ERROR_PREFIX}. HTTP {status}: {body}"))
}

pub(crate) fn normalize_endpoint(endpoint: String) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}

pub(crate) fn parse_model_catalog(body: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| LitecodeError::Llm(format!("OpenCode model catalog is not JSON: {e}")))?;
    let Some(data) = value.get("data").and_then(Value::as_array) else {
        return Err(LitecodeError::Llm(
            "OpenCode model catalog missing data array".into(),
        ));
    };
    Ok(data
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str))
        .map(|s| s.to_string())
        .collect())
}

fn oc_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

fn apply_opencode_headers(
    builder: reqwest::RequestBuilder,
    header_name: String,
    header_value: String,
) -> reqwest::RequestBuilder {
    builder
        .header(header_name, header_value)
        .header("content-type", "application/json")
        .header("user-agent", "litecode-opencode/1")
        .header("x-opencode-request", Uuid::new_v4().to_string())
        .header("x-opencode-session", Uuid::new_v4().to_string())
        .header("x-opencode-client", "litecode")
}

fn item_text(item: &Item) -> String {
    crate::types::item_text_preview(item)
}

pub(crate) fn encode_chat_body(params: &ModelRequest, stream: bool) -> Result<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if !params.instructions.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": params.instructions,
        }));
    }

    let reasoning_key = reasoning_json_key();
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
                turn.flush(&mut messages, &reasoning_key, replay_reasoning);
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
                turn.flush(&mut messages, &reasoning_key, replay_reasoning);
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
    turn.flush(&mut messages, &reasoning_key, replay_reasoning);

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
    if stream {
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

fn chat_reasoning_text(node: &Value) -> Option<&str> {
    let key = reasoning_json_key();
    if let Some(text) = node.get(&key).and_then(Value::as_str).filter(|s| !s.is_empty()) {
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

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
}

/// Chat Completions usage → Responses `usage` (what the ctx ring reads).
fn chat_usage_to_responses(usage: &Value) -> Option<Value> {
    let prompt = json_u64(usage.get("prompt_tokens")).or_else(|| json_u64(usage.get("input_tokens")))?;
    let completion = json_u64(usage.get("completion_tokens"))
        .or_else(|| json_u64(usage.get("output_tokens")))
        .unwrap_or(0);
    let cached = json_u64(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| json_u64(usage.pointer("/input_tokens_details/cached_tokens")))
        .or_else(|| json_u64(usage.get("cached_tokens")))
        .or_else(|| json_u64(usage.get("prompt_cache_hit_tokens")))
        .unwrap_or(0);
    let reasoning = json_u64(usage.pointer("/completion_tokens_details/reasoning_tokens"))
        .or_else(|| json_u64(usage.pointer("/output_tokens_details/reasoning_tokens")))
        .unwrap_or(0);
    Some(serde_json::json!({
        "input_tokens": prompt,
        "output_tokens": completion,
        "total_tokens": prompt.saturating_add(completion),
        "input_tokens_details": { "cached_tokens": cached },
        "output_tokens_details": { "reasoning_tokens": reasoning },
    }))
}

struct ToolAcc {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    output_index: u32,
    opened: bool,
}

struct ChatSynth {
    seq: u64,
    next_output: u32,
    msg_id: Option<String>,
    msg_index: u32,
    msg_text: String,
    rs_id: Option<String>,
    rs_index: u32,
    rs_text: String,
    tools: BTreeMap<u32, ToolAcc>,
    usage: Option<Value>,
}

impl ChatSynth {
    fn new() -> Self {
        Self {
            seq: 0,
            next_output: 0,
            msg_id: None,
            msg_index: 0,
            msg_text: String::new(),
            rs_id: None,
            rs_index: 0,
            rs_text: String::new(),
            tools: BTreeMap::new(),
            usage: None,
        }
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn alloc_out(&mut self) -> u32 {
        let i = self.next_output;
        self.next_output += 1;
        i
    }

    fn ensure_message(&mut self, events: &mut Vec<ResponseStreamEvent>) -> (String, u32) {
        if let Some(id) = &self.msg_id {
            return (id.clone(), self.msg_index);
        }
        let id = oc_id("oc_msg");
        let idx = self.alloc_out();
        let seq = self.bump();
        events.push(ResponseStreamEvent::ResponseOutputItemAdded(
            ResponseOutputItemAddedEvent {
                sequence_number: seq,
                output_index: idx,
                item: OutputItem::Message(OutputMessage {
                    id: id.clone(),
                    role: AssistantRole::Assistant,
                    status: OutputStatus::InProgress,
                    phase: None,
                    content: vec![],
                }),
            },
        ));
        self.msg_id = Some(id.clone());
        self.msg_index = idx;
        (id, idx)
    }

    fn ensure_reasoning(&mut self, events: &mut Vec<ResponseStreamEvent>) -> (String, u32) {
        if let Some(id) = &self.rs_id {
            return (id.clone(), self.rs_index);
        }
        let id = oc_id("oc_rs");
        let idx = self.alloc_out();
        let seq = self.bump();
        events.push(ResponseStreamEvent::ResponseOutputItemAdded(
            ResponseOutputItemAddedEvent {
                sequence_number: seq,
                output_index: idx,
                item: OutputItem::Reasoning(ReasoningItem {
                    id: Some(id.clone()),
                    summary: vec![],
                    content: Some(vec![]),
                    encrypted_content: None,
                    status: Some(OutputStatus::InProgress),
                }),
            },
        ));
        self.rs_id = Some(id.clone());
        self.rs_index = idx;
        (id, idx)
    }

    fn ingest_reasoning(&mut self, text: &str, events: &mut Vec<ResponseStreamEvent>) {
        if text.is_empty() {
            return;
        }
        let (item_id, output_index) = self.ensure_reasoning(events);
        self.rs_text.push_str(text);
        let seq = self.bump();
        events.push(ResponseStreamEvent::ResponseReasoningTextDelta(
            ResponseReasoningTextDeltaEvent {
                sequence_number: seq,
                item_id,
                output_index,
                content_index: 0,
                delta: text.to_string(),
            },
        ));
    }

    fn ingest_chunk(&mut self, chunk: &Value, events: &mut Vec<ResponseStreamEvent>) {
        if let Some(usage) = chunk.get("usage")
            && let Some(mapped) = chat_usage_to_responses(usage)
        {
            self.usage = Some(mapped);
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
        else {
            return;
        };
        let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else {
            return;
        };
        if let Some(text) = delta.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            let (item_id, output_index) = self.ensure_message(events);
            self.msg_text.push_str(text);
            let seq = self.bump();
            events.push(ResponseStreamEvent::ResponseOutputTextDelta(
                ResponseTextDeltaEvent {
                    sequence_number: seq,
                    item_id,
                    output_index,
                    content_index: 0,
                    delta: text.to_string(),
                    logprobs: None,
                },
            ));
        }
        if let Some(text) = chat_reasoning_text(delta) {
            self.ingest_reasoning(text, events);
        }
        if self.rs_text.is_empty()
            && let Some(msg) = choice.get("message")
            && let Some(text) = chat_reasoning_text(msg)
        {
            self.ingest_reasoning(text, events);
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0) as u32;
                self.tools.entry(idx).or_insert_with(|| ToolAcc {
                    item_id: oc_id("oc_fc"),
                    call_id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                    output_index: 0,
                    opened: false,
                });
                if let Some(entry) = self.tools.get_mut(&idx) {
                    if let Some(id) = call.get("id").and_then(Value::as_str)
                        && !id.is_empty()
                    {
                        entry.call_id = id.to_string();
                    }
                    if let Some(name) = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                    {
                        entry.name = name.to_string();
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str)
                    {
                        entry.arguments.push_str(args);
                    }
                }
                let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str) else {
                    continue;
                };
                let opened = self.tools.get(&idx).map(|e| e.opened).unwrap_or(true);
                if !opened {
                    let output_index = self.alloc_out();
                    let seq = self.bump();
                    let entry = self.tools.get_mut(&idx).expect("tool acc");
                    entry.output_index = output_index;
                    entry.opened = true;
                    let call_id = if entry.call_id.is_empty() {
                        entry.item_id.clone()
                    } else {
                        entry.call_id.clone()
                    };
                    let item_id = entry.item_id.clone();
                    let name = entry.name.clone();
                    events.push(ResponseStreamEvent::ResponseOutputItemAdded(
                        ResponseOutputItemAddedEvent {
                            sequence_number: seq,
                            output_index,
                            item: OutputItem::FunctionCall(FunctionToolCall {
                                id: Some(item_id),
                                call_id,
                                name,
                                arguments: String::new(),
                                status: Some(OutputStatus::InProgress),
                                namespace: None,
                            }),
                        },
                    ));
                }
                if !args.is_empty() {
                    let seq = self.bump();
                    let entry = self.tools.get(&idx).expect("tool acc");
                    events.push(ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
                        ResponseFunctionCallArgumentsDeltaEvent {
                            sequence_number: seq,
                            item_id: entry.item_id.clone(),
                            output_index: entry.output_index,
                            delta: args.to_string(),
                        },
                    ));
                }
            }
        }
    }

    fn finish_events(&mut self, model: &str) -> Result<Vec<ResponseStreamEvent>> {
        let mut events = Vec::new();
        let tools: Vec<&ToolAcc> = self.tools.values().collect();
        let opened: Vec<(String, String, u32, String)> = tools
            .iter()
            .filter(|t| t.opened)
            .map(|t| {
                (
                    t.name.clone(),
                    t.item_id.clone(),
                    t.output_index,
                    t.arguments.clone(),
                )
            })
            .collect();
        for (name, item_id, output_index, arguments) in opened {
            let seq = self.bump();
            events.push(ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                ResponseFunctionCallArgumentsDoneEvent {
                    name: if name.is_empty() { None } else { Some(name) },
                    sequence_number: seq,
                    item_id,
                    output_index,
                    arguments,
                },
            ));
        }
        let output = self.output_values();
        let seq = self.bump();
        let mut response = serde_json::json!({
            "id": oc_id("oc_resp"),
            "object": "response",
            "created_at": 0,
            "model": model,
            "status": "completed",
            "output": output,
        });
        if let Some(usage) = self.usage.take() {
            response["usage"] = usage;
        }
        let completed = serde_json::json!({
            "type": "response.completed",
            "sequence_number": seq,
            "response": response,
        });
        let event: ResponseStreamEvent = serde_json::from_value(completed)
            .map_err(|e| LitecodeError::Llm(format!("synthesize response.completed: {e}")))?;
        events.push(event);
        Ok(events)
    }

    fn output_values(&self) -> Vec<Value> {
        let mut out = Vec::new();
        if let Some(id) = &self.rs_id {
            out.push(serde_json::json!({
                "type": "reasoning",
                "id": id,
                "summary": [],
                "content": [{"type": "reasoning_text", "text": self.rs_text}],
                "status": "completed"
            }));
        }
        if let Some(id) = &self.msg_id {
            out.push(serde_json::json!({
                "type": "message",
                "id": id,
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": self.msg_text, "annotations": []}]
            }));
        }
        for tool in self.tools.values() {
            let call_id = if tool.call_id.is_empty() {
                tool.item_id.clone()
            } else {
                tool.call_id.clone()
            };
            out.push(serde_json::json!({
                "type": "function_call",
                "id": tool.item_id,
                "call_id": call_id,
                "name": tool.name,
                "arguments": tool.arguments,
                "status": "completed"
            }));
        }
        out
    }
}

fn items_from_chat_message(message: &Value) -> Vec<Item> {
    let mut items = Vec::new();
    let rkey = reasoning_json_key();
    if let Some(text) = message.get(&rkey).and_then(Value::as_str)
        && !text.is_empty()
    {
        items.push(Item::Reasoning(ReasoningItem {
            id: Some(oc_id("oc_rs")),
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
            id: oc_id("oc_msg"),
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
                oc_id("oc_fc")
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

impl LlmProvider for OpencodeProvider {
    fn endpoint(&self) -> &str {
        &self.endpoint_url
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(Self {
            client: self.client.clone(),
            endpoint_url: self.endpoint_url.clone(),
            auth: self.auth,
        })
    }

    fn clone_for_isolated_runtime(&self) -> Box<dyn LlmProvider> {
        match Self::new(self.endpoint_url.clone(), self.auth) {
            Ok(p) => Box::new(p),
            Err(_) => self.box_clone(),
        }
    }

    fn complete<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = encode_chat_body(request, false)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = apply_opencode_headers(
                self.client.post(self.post_url()),
                header_name,
                header_value,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| LitecodeError::Llm(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(wrap_upstream(status, &text));
            }
            let text = resp
                .text()
                .await
                .map_err(|e| LitecodeError::Llm(e.to_string()))?;
            let value: Value = serde_json::from_str(&text).map_err(|e| {
                LitecodeError::Llm(format!("{ERROR_PREFIX}. not Chat JSON: {e}; body={text}"))
            })?;
            let message = value
                .pointer("/choices/0/message")
                .cloned()
                .ok_or_else(|| {
                    LitecodeError::Llm(format!("{ERROR_PREFIX}. missing choices[0].message"))
                })?;
            Ok(items_from_chat_message(&message))
        })
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = encode_chat_body(request, true)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = apply_opencode_headers(
                self.client.post(self.post_url()),
                header_name,
                header_value,
            )
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| LitecodeError::Llm(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(wrap_upstream(status, &text));
            }
            let resp = check_event_stream_content_type(resp).await?;

            let mut terminal_items: Option<Vec<Item>> = None;
            let mut reader = SseLineReader::new();
            let mut stream = resp.bytes_stream();
            let mut gate = StreamContractGate::new();
            let mut acc = StreamItemAccumulator::new();
            let mut synth = ChatSynth::new();
            let mut cancelled = cancel.is_cancelled();

            let mut forward_all = |events: Vec<ResponseStreamEvent>| -> Result<Option<Vec<Item>>> {
                let mut last = None;
                for event in events {
                    if let Some(items) =
                        forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                    {
                        last = Some(items);
                    }
                }
                Ok(last)
            };

            while !cancelled {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        break;
                    }
                    chunk = stream.next() => {
                        let Some(chunk) = chunk else { break; };
                        let chunk = chunk.map_err(|e| LitecodeError::Llm(e.to_string()))?;
                        for line in reader.feed(&chunk)? {
                            let Some(data) = sse_data_payload(&line) else {
                                continue;
                            };
                            if data.trim() == "[DONE]" {
                                continue;
                            }
                            let value: Value = serde_json::from_str(data).map_err(|e| {
                                LitecodeError::Llm(format!(
                                    "{ERROR_PREFIX}. Chat SSE JSON: {e}; payload={data}"
                                ))
                            })?;
                            let mut events = Vec::new();
                            synth.ingest_chunk(&value, &mut events);
                            if let Some(items) = forward_all(events)? {
                                terminal_items = Some(items);
                            }
                            if cancel.is_cancelled() {
                                cancelled = true;
                                break;
                            }
                        }
                    }
                }
            }

            if !cancelled {
                if let Some(line) = reader.finish()?
                    && let Some(data) = sse_data_payload(&line)
                    && data.trim() != "[DONE]"
                {
                    let value: Value = serde_json::from_str(data).map_err(|e| {
                        LitecodeError::Llm(format!(
                            "{ERROR_PREFIX}. Chat SSE JSON: {e}; payload={data}"
                        ))
                    })?;
                    let mut events = Vec::new();
                    synth.ingest_chunk(&value, &mut events);
                    if let Some(items) = forward_all(events)? {
                        terminal_items = Some(items);
                    }
                }
                if terminal_items.is_none()
                    && let Some(items) = forward_all(synth.finish_events(&request.model)?)? {
                        terminal_items = Some(items);
                    }
            }

            resolve_stream_outcome(terminal_items, &acc, cancelled)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{Item, MessageItem, OutputMessage};
    use crate::llm::request::ToolDef;
    use crate::types::user_text;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_request(tools: Vec<ToolDef>) -> ModelRequest {
        ModelRequest {
            model: "deepseek-v4-flash-free".into(),
            instructions: "sys".into(),
            input: vec![user_text("hello")],
            tools,
            max_output_tokens: 64,
            temperature: 0.0,
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
        }
    }

    async fn serve_once(body: String, status: &str, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let content_type = content_type.to_string();
        let status = status.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}/v1")
    }

    #[test]
    fn normalize_keeps_zen_and_go_v1() {
        assert_eq!(
            normalize_endpoint("https://opencode.ai/zen/v1/".into()),
            "https://opencode.ai/zen/v1"
        );
        assert_eq!(
            normalize_endpoint("https://opencode.ai/zen/go/v1".into()),
            "https://opencode.ai/zen/go/v1"
        );
    }

    #[test]
    fn chat_url_splits_path_segments() {
        let url = chat_post_url("https://opencode.ai/zen/v1");
        let expected = format!("https://opencode.ai/zen/v1/{}/{}", "chat", "completions");
        assert_eq!(url, expected);
    }

    #[test]
    fn parse_catalog_ids() {
        let body = r#"{"object":"list","data":[{"id":"a"},{"id":"b"}]}"#;
        assert_eq!(parse_model_catalog(body).unwrap(), vec!["a", "b"]);
    }

    #[test]
    fn encode_user_and_system() {
        let body = encode_chat_body(&sample_request(vec![]), true).unwrap();
        assert_eq!(body["model"], "deepseek-v4-flash-free");
        assert_eq!(body["stream"], true);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "hello");
        assert_eq!(body["stream_options"]["include_usage"], true);
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
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
        };
        let body = encode_chat_body(&req, false).unwrap();
        let key = reasoning_json_key();
        let msgs = body["messages"].as_array().unwrap();
        let assistant = msgs
            .iter()
            .find(|m| m["role"] == "assistant" && m.get("tool_calls").is_some())
            .unwrap();
        assert_eq!(assistant[&key], "think");
        assert_eq!(assistant["tool_calls"][0]["id"], "call_1");
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
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
        };
        let body = encode_chat_body(&req, false).unwrap();
        let key = reasoning_json_key();
        let msgs = body["messages"].as_array().unwrap();
        let assistants: Vec<_> = msgs.iter().filter(|m| m["role"] == "assistant").collect();
        assert_eq!(assistants.len(), 1, "{msgs:?}");
        assert_eq!(assistants[0]["content"], "hello");
        assert_eq!(assistants[0][&key], "think");
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
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
        };
        let body = encode_chat_body(&req, false).unwrap();
        let key = reasoning_json_key();
        let assistant = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant[&key], "");
    }

    #[tokio::test]
    async fn stream_text_only() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete_with_stream_events(
                &sample_request(vec![]),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        let text: String = items
            .iter()
            .filter_map(|i| match i {
                Item::Message(MessageItem::Output(OutputMessage { content, .. })) => {
                    content.iter().find_map(|c| match c {
                        OutputMessageContent::OutputText(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert!(text.contains("hi"), "got {items:?}");
    }

    #[tokio::test]
    async fn stream_tool_call() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"p\\\":1}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));
        let items = provider
            .complete_with_stream_events(
                &sample_request(vec![]),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        assert!(
            items
                .iter()
                .any(|i| matches!(i, Item::FunctionCall(fc) if fc.name == "read")),
            "got {items:?}"
        );
        let events = seen.lock().unwrap().clone();
        let added = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseOutputItemAdded(ev)
                    if matches!(&ev.item, OutputItem::FunctionCall(fc) if fc.name == "read")
            )
        });
        let delta = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(_)
            )
        });
        assert!(added.is_some() && delta.is_some());
        assert!(added.unwrap() < delta.unwrap());
    }

    #[tokio::test]
    async fn http_400_mentions_chat_only() {
        let endpoint = serve_once("nope".into(), "400 Bad Request", "application/json").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete(&sample_request(vec![]), "sk-test")
            .await
            .expect_err("fail");
        let msg = err.to_string();
        assert!(msg.contains("Chat Completions"), "{msg}");
        assert!(msg.contains("HTTP 400"), "{msg}");
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

    #[tokio::test]
    async fn stream_usage_lands_on_completed() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));
        provider
            .complete_with_stream_events(
                &sample_request(vec![]),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        let events = seen.lock().unwrap().clone();
        let usage = events.iter().find_map(|e| match e {
            ResponseStreamEvent::ResponseCompleted(ev) => ev.response.usage.clone(),
            _ => None,
        });
        let usage = usage.expect("completed usage");
        assert_eq!(usage.input_tokens, 50);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.input_tokens_details.cached_tokens, 40);
    }
}
