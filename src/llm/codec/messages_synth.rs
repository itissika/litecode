//! Anthropic Messages SSE → authority [`ResponseStreamEvent`]s.
//!
//! Blocks are addressed by the protocol's `index`. Item ids are
//! `{message_id}_{index}` so a turn never invents a second identity for a
//! block. The signature is stored on the reasoning item's `encrypted_content`;
//! a redacted block uses the `redacted_thinking:` prefix so the encoder can
//! tell the two apart.

use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use crate::authority::responses::{
    AssistantRole, FunctionToolCall, OutputItem, OutputMessage, OutputStatus, ReasoningItem,
    ReasoningItemContent, ReasoningTextContent, ResponseFunctionCallArgumentsDeltaEvent,
    ResponseFunctionCallArgumentsDoneEvent, ResponseOutputItemAddedEvent,
    ResponseOutputItemDoneEvent, ResponseReasoningTextDeltaEvent, ResponseStreamEvent,
    ResponseTextDeltaEvent,
};
use crate::types::{LitecodeError, Result};

use super::messages_usage::MessagesUsage;

/// Prefix that marks `encrypted_content` as a redacted thinking blob rather
/// than a thinking-block signature. Signatures are base64 and never start
/// with this ASCII label.
pub(super) const REDACTED_PREFIX: &str = "redacted_thinking:";

fn synth_id() -> String {
    format!("msg_{}", Uuid::new_v4().simple())
}

enum OpenKind {
    Thinking {
        text: String,
        signature: String,
    },
    Redacted {
        data: String,
    },
    Text {
        text: String,
    },
    Tool {
        call_id: String,
        name: String,
        arguments: String,
        closed: bool,
    },
}

struct OpenBlock {
    item_id: String,
    kind: OpenKind,
}

pub(super) struct MessagesSynth {
    seq: u64,
    message_id: Option<String>,
    model: String,
    stop_reason: Option<String>,
    usage: MessagesUsage,
    blocks: BTreeMap<u32, OpenBlock>,
    finished: bool,
    /// Final `thinking_dropped` counts. `message_start.message` sets them.
    /// A later `message_delta` list replaces them. Warned once at `message_stop`.
    thinking_dropped: BTreeMap<String, usize>,
}

impl MessagesSynth {
    pub(super) fn new(model: &str) -> Self {
        Self {
            seq: 0,
            message_id: None,
            model: model.to_string(),
            stop_reason: None,
            usage: MessagesUsage::default(),
            blocks: BTreeMap::new(),
            finished: false,
            thinking_dropped: BTreeMap::new(),
        }
    }

    pub(super) fn thinking_dropped(&self, reason: &str) -> usize {
        self.thinking_dropped.get(reason).copied().unwrap_or(0)
    }

    fn bump(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn message_id(&mut self) -> String {
        if self.message_id.as_deref().unwrap_or("").is_empty() {
            self.message_id = Some(synth_id());
        }
        self.message_id.clone().unwrap_or_default()
    }

    pub(super) fn ingest(&mut self, value: &Value) -> Result<Vec<ResponseStreamEvent>> {
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "ping" | "" => Ok(Vec::new()),
            "message_start" => {
                if let Some(message) = value.get("message") {
                    if let Some(id) = message.get("id").and_then(Value::as_str) {
                        if !id.is_empty() {
                            self.message_id = Some(id.to_string());
                        }
                    }
                    if let Some(model) = message.get("model").and_then(Value::as_str) {
                        if !model.is_empty() {
                            self.model = model.to_string();
                        }
                    }
                    if let Some(usage) = message.get("usage") {
                        self.usage.observe(usage);
                    }
                }
                self.observe_input_transformations(value, "/message/input_transformations");
                Ok(Vec::new())
            }
            "content_block_start" => self.on_start(value),
            "content_block_delta" => self.on_delta(value),
            "content_block_stop" => self.on_stop(value),
            "message_delta" => {
                if let Some(usage) = value.get("usage") {
                    self.usage.observe(usage);
                }
                self.observe_input_transformations(value, "/input_transformations");
                if let Some(reason) = value
                    .pointer("/delta/stop_reason")
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.is_empty())
                {
                    self.stop_reason = Some(reason.to_string());
                }
                Ok(Vec::new())
            }
            "message_stop" => self.finish(),
            _ => Ok(Vec::new()),
        }
    }

    /// `pointer` is `/message/input_transformations` on `message_start`, or
    /// `/input_transformations` on `message_delta`. A present list replaces
    /// the previous one. Only `thinking_dropped` counts; other types are ignored.
    fn observe_input_transformations(&mut self, value: &Value, pointer: &str) {
        let Some(entries) = value.pointer(pointer).and_then(Value::as_array) else {
            return;
        };
        let mut counts = BTreeMap::new();
        for entry in entries {
            if entry.get("type").and_then(Value::as_str) != Some("thinking_dropped") {
                continue;
            }
            let reason = entry
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            *counts.entry(reason.to_string()).or_default() += 1;
        }
        self.thinking_dropped = counts;
    }

    fn warn_dropped_thinking(&self) {
        if self.thinking_dropped.is_empty() {
            return;
        }
        let dropped: usize = self.thinking_dropped.values().sum();
        tracing::warn!(
            target: "litecode.debug.llm_request",
            dropped,
            reasons = ?self.thinking_dropped,
            "anthropic dropped thinking blocks"
        );
    }

    fn on_start(&mut self, value: &Value) -> Result<Vec<ResponseStreamEvent>> {
        let Some(index) = value.get("index").and_then(Value::as_u64) else {
            return Ok(Vec::new());
        };
        let index = index as u32;
        let block = value.get("content_block").cloned().unwrap_or(Value::Null);
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        let message_id = self.message_id();
        let item_id = format!("{message_id}_{index}");
        let mut events = Vec::new();
        let kind = match block_type {
            "thinking" => OpenKind::Thinking {
                text: String::new(),
                signature: String::new(),
            },
            "redacted_thinking" => {
                let data = block
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let seq = self.bump();
                events.push(added_reasoning(
                    seq,
                    index,
                    &item_id,
                    "",
                    Some(format!("{REDACTED_PREFIX}{data}")),
                    OutputStatus::Completed,
                ));
                self.blocks.insert(
                    index,
                    OpenBlock {
                        item_id,
                        kind: OpenKind::Redacted { data },
                    },
                );
                return Ok(events);
            }
            "text" => OpenKind::Text {
                text: String::new(),
            },
            "tool_use" => OpenKind::Tool {
                call_id: block
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .unwrap_or(&item_id)
                    .to_string(),
                name: block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                arguments: String::new(),
                closed: false,
            },
            _ => return Ok(Vec::new()),
        };
        let seq = self.bump();
        match &kind {
            OpenKind::Thinking { .. } => events.push(added_reasoning(
                seq,
                index,
                &item_id,
                "",
                None,
                OutputStatus::InProgress,
            )),
            OpenKind::Text { .. } => events.push(ResponseStreamEvent::ResponseOutputItemAdded(
                ResponseOutputItemAddedEvent {
                    sequence_number: seq,
                    output_index: index,
                    item: OutputItem::Message(OutputMessage {
                        id: item_id.clone(),
                        role: AssistantRole::Assistant,
                        status: OutputStatus::InProgress,
                        phase: None,
                        content: vec![],
                    }),
                },
            )),
            OpenKind::Tool { call_id, name, .. } => {
                events.push(ResponseStreamEvent::ResponseOutputItemAdded(
                    ResponseOutputItemAddedEvent {
                        sequence_number: seq,
                        output_index: index,
                        item: OutputItem::FunctionCall(FunctionToolCall {
                            id: Some(item_id.clone()),
                            call_id: call_id.clone(),
                            name: name.clone(),
                            arguments: String::new(),
                            status: Some(OutputStatus::InProgress),
                            namespace: None,
                        }),
                    },
                ));
            }
            OpenKind::Redacted { .. } => {}
        }
        self.blocks.insert(index, OpenBlock { item_id, kind });
        Ok(events)
    }

    fn on_delta(&mut self, value: &Value) -> Result<Vec<ResponseStreamEvent>> {
        let Some(index) = value.get("index").and_then(Value::as_u64) else {
            return Ok(Vec::new());
        };
        let index = index as u32;
        let Some(delta) = value.get("delta") else {
            return Ok(Vec::new());
        };
        let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
        // Citations are not requested, so that delta carries nothing we keep.
        let pending = {
            let Some(block) = self.blocks.get_mut(&index) else {
                return Ok(Vec::new());
            };
            match delta_type {
                "thinking_delta" => {
                    let text = delta
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let OpenKind::Thinking { text: acc, .. } = &mut block.kind {
                        acc.push_str(&text);
                    }
                    (!text.is_empty()).then(|| DeltaOut::Reasoning {
                        item_id: block.item_id.clone(),
                        text,
                    })
                }
                "signature_delta" => {
                    let signature = delta
                        .get("signature")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let OpenKind::Thinking {
                        text,
                        signature: acc,
                    } = &mut block.kind
                    else {
                        return Ok(Vec::new());
                    };
                    acc.push_str(&signature);
                    Some(DeltaOut::Signature {
                        item_id: block.item_id.clone(),
                        text: text.clone(),
                        signature: acc.clone(),
                    })
                }
                "text_delta" => {
                    let text = delta
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let OpenKind::Text { text: acc } = &mut block.kind {
                        acc.push_str(&text);
                    }
                    (!text.is_empty()).then(|| DeltaOut::Text {
                        item_id: block.item_id.clone(),
                        text,
                    })
                }
                "input_json_delta" => {
                    let partial = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let OpenKind::Tool { arguments, .. } = &mut block.kind {
                        arguments.push_str(&partial);
                    }
                    (!partial.is_empty()).then(|| DeltaOut::Arguments {
                        item_id: block.item_id.clone(),
                        partial,
                    })
                }
                _ => None,
            }
        };
        let Some(pending) = pending else {
            return Ok(Vec::new());
        };
        let seq = self.bump();
        let event = match pending {
            DeltaOut::Reasoning { item_id, text } => {
                ResponseStreamEvent::ResponseReasoningTextDelta(ResponseReasoningTextDeltaEvent {
                    sequence_number: seq,
                    item_id,
                    output_index: index,
                    content_index: 0,
                    delta: text,
                })
            }
            DeltaOut::Signature {
                item_id,
                text,
                signature,
            } => done_reasoning(seq, index, &item_id, &text, Some(signature)),
            DeltaOut::Text { item_id, text } => {
                ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
                    sequence_number: seq,
                    item_id,
                    output_index: index,
                    content_index: 0,
                    delta: text,
                    logprobs: None,
                })
            }
            DeltaOut::Arguments { item_id, partial } => {
                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
                    ResponseFunctionCallArgumentsDeltaEvent {
                        sequence_number: seq,
                        item_id,
                        output_index: index,
                        delta: partial,
                    },
                )
            }
        };
        Ok(vec![event])
    }

    fn on_stop(&mut self, value: &Value) -> Result<Vec<ResponseStreamEvent>> {
        let Some(index) = value.get("index").and_then(Value::as_u64) else {
            return Ok(Vec::new());
        };
        let index = index as u32;
        let closed = {
            let Some(block) = self.blocks.get_mut(&index) else {
                return Ok(Vec::new());
            };
            let OpenKind::Tool {
                name,
                arguments,
                closed,
                ..
            } = &mut block.kind
            else {
                return Ok(Vec::new());
            };
            if *closed {
                return Ok(Vec::new());
            }
            *closed = true;
            if arguments.is_empty() {
                arguments.push_str("{}");
            }
            Some((name.clone(), block.item_id.clone(), arguments.clone()))
        };
        let Some((name, item_id, arguments)) = closed else {
            return Ok(Vec::new());
        };
        let seq = self.bump();
        Ok(vec![
            ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                ResponseFunctionCallArgumentsDoneEvent {
                    name: if name.is_empty() { None } else { Some(name) },
                    sequence_number: seq,
                    item_id,
                    output_index: index,
                    arguments,
                },
            ),
        ])
    }

    fn finish(&mut self) -> Result<Vec<ResponseStreamEvent>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        self.warn_dropped_thinking();
        let mut events = Vec::new();
        let tools: Vec<(u32, String, String, String)> = self
            .blocks
            .iter_mut()
            .filter_map(|(index, block)| {
                let OpenKind::Tool {
                    name,
                    arguments,
                    closed,
                    ..
                } = &mut block.kind
                else {
                    return None;
                };
                if *closed {
                    return None;
                }
                *closed = true;
                if arguments.is_empty() {
                    arguments.push_str("{}");
                }
                Some((
                    *index,
                    name.clone(),
                    block.item_id.clone(),
                    arguments.clone(),
                ))
            })
            .collect();
        for (index, name, item_id, arguments) in tools {
            let seq = self.bump();
            events.push(ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                ResponseFunctionCallArgumentsDoneEvent {
                    name: if name.is_empty() { None } else { Some(name) },
                    sequence_number: seq,
                    item_id,
                    output_index: index,
                    arguments,
                },
            ));
        }

        let (status, incomplete) = terminal_status(self.stop_reason.as_deref());
        let output = self.output_values();
        let seq = self.bump();
        let id = self.message_id.clone().unwrap_or_else(synth_id);
        let model = self.model.clone();
        let mut response = serde_json::json!({
            "id": id,
            "object": "response",
            "created_at": 0,
            "model": model,
            "status": status,
            "output": output,
        });
        if let Some(reason) = incomplete {
            response["incomplete_details"] = serde_json::json!({ "reason": reason });
        }
        if let Some(usage) = self.usage.to_responses() {
            response["usage"] = usage;
        }
        let completed = serde_json::json!({
            "type": "response.completed",
            "sequence_number": seq,
            "response": response,
        });
        let event: ResponseStreamEvent = serde_json::from_value(completed).map_err(|error| {
            LitecodeError::Llm(format!("synthesize response.completed: {error}"))
        })?;
        events.push(event);
        Ok(events)
    }

    fn output_values(&self) -> Vec<Value> {
        self.blocks
            .values()
            .map(|block| match &block.kind {
                OpenKind::Thinking { text, signature } => {
                    let mut item = serde_json::json!({
                        "type": "reasoning",
                        "id": block.item_id,
                        "summary": [],
                        "content": [{"type": "reasoning_text", "text": text}],
                        "status": "completed"
                    });
                    if !signature.is_empty() {
                        item["encrypted_content"] = Value::String(signature.clone());
                    }
                    item
                }
                OpenKind::Redacted { data } => serde_json::json!({
                    "type": "reasoning",
                    "id": block.item_id,
                    "summary": [],
                    "status": "completed",
                    "encrypted_content": format!("{REDACTED_PREFIX}{data}"),
                }),
                OpenKind::Text { text } => serde_json::json!({
                    "type": "message",
                    "id": block.item_id,
                    "role": "assistant",
                    "status": "completed",
                    "content": [{"type": "output_text", "text": text, "annotations": []}]
                }),
                OpenKind::Tool {
                    call_id,
                    name,
                    arguments,
                    ..
                } => serde_json::json!({
                    "type": "function_call",
                    "id": block.item_id,
                    "call_id": call_id,
                    "name": name,
                    "arguments": if arguments.is_empty() { "{}".to_string() } else { arguments.clone() },
                    "status": "completed"
                }),
            })
            .collect()
    }
}

enum DeltaOut {
    Reasoning {
        item_id: String,
        text: String,
    },
    Signature {
        item_id: String,
        text: String,
        signature: String,
    },
    Text {
        item_id: String,
        text: String,
    },
    Arguments {
        item_id: String,
        partial: String,
    },
}

fn terminal_status(stop_reason: Option<&str>) -> (&'static str, Option<String>) {
    match stop_reason.unwrap_or("end_turn") {
        "end_turn" | "tool_use" | "stop_sequence" => ("completed", None),
        "max_tokens" | "model_context_window_exceeded" => {
            ("incomplete", Some("max_output_tokens".into()))
        }
        "refusal" => ("incomplete", Some("content_filter".into())),
        other => {
            tracing::info!(
                stop_reason = other,
                "messages stop_reason mapped to incomplete"
            );
            ("incomplete", Some(other.to_string()))
        }
    }
}

fn added_reasoning(
    seq: u64,
    index: u32,
    item_id: &str,
    text: &str,
    encrypted: Option<String>,
    status: OutputStatus,
) -> ResponseStreamEvent {
    let content = if text.is_empty() && encrypted.is_none() {
        Some(vec![])
    } else if text.is_empty() {
        None
    } else {
        Some(vec![ReasoningItemContent::ReasoningText(
            ReasoningTextContent {
                text: text.to_string(),
            },
        )])
    };
    ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
        sequence_number: seq,
        output_index: index,
        item: OutputItem::Reasoning(ReasoningItem {
            id: Some(item_id.to_string()),
            summary: vec![],
            content,
            encrypted_content: encrypted,
            status: Some(status),
        }),
    })
}

fn done_reasoning(
    seq: u64,
    index: u32,
    item_id: &str,
    text: &str,
    signature: Option<String>,
) -> ResponseStreamEvent {
    ResponseStreamEvent::ResponseOutputItemDone(ResponseOutputItemDoneEvent {
        sequence_number: seq,
        output_index: index,
        item: OutputItem::Reasoning(ReasoningItem {
            id: Some(item_id.to_string()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent {
                    text: text.to_string(),
                },
            )]),
            encrypted_content: signature,
            status: Some(OutputStatus::Completed),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{Item, Response};

    fn run(events: &[Value]) -> (Vec<ResponseStreamEvent>, Response) {
        let mut synth = MessagesSynth::new("claude-sonnet-5-5");
        let mut out = Vec::new();
        for event in events {
            out.extend(synth.ingest(event).unwrap());
        }
        let response = out.iter().rev().find_map(|event| match event {
            ResponseStreamEvent::ResponseCompleted(completed) => Some(completed.response.clone()),
            _ => None,
        });
        (out, response.expect("message_stop synthesizes a terminal"))
    }

    #[test]
    fn message_start_records_dropped_thinking_without_changing_the_stream() {
        let mut synth = MessagesSynth::new("claude-sonnet-5-5");
        let events = synth
            .ingest(&serde_json::json!({
                "type": "message_start",
                "message": {
                    "id": "msg_01",
                    "model": "claude-sonnet-5-5",
                    "input_transformations": [
                        {
                            "path": "messages.1.content.0",
                            "type": "thinking_dropped",
                            "reason": "prefix_binding_mismatch"
                        },
                        {
                            "path": "messages.1.content.1",
                            "type": "other_change",
                            "reason": "not_thinking"
                        },
                        {
                            "path": "messages.1.content.2",
                            "type": "thinking_dropped",
                            "reason": "model_binding_mismatch"
                        }
                    ]
                },
                "input_transformations": [
                    {
                        "path": "messages.0.content.0",
                        "type": "thinking_dropped",
                        "reason": "organization_binding_mismatch"
                    }
                ]
            }))
            .unwrap();
        assert!(events.is_empty());
        assert_eq!(synth.message_id(), "msg_01");
        assert_eq!(synth.thinking_dropped("prefix_binding_mismatch"), 1);
        assert_eq!(synth.thinking_dropped("model_binding_mismatch"), 1);
        assert_eq!(synth.thinking_dropped("organization_binding_mismatch"), 0);
        assert_eq!(synth.thinking_dropped("not_thinking"), 0);
    }

    fn start_message() -> Value {
        serde_json::json!({
            "type": "message_start",
            "message": {
                "id": "msg_01",
                "model": "claude-sonnet-5-5",
                "usage": {
                    "input_tokens": 100,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": 1
                }
            }
        })
    }

    #[test]
    fn thinking_text_and_tool_round_trip_with_the_signature() {
        let (events, response) = run(&[
            start_message(),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Need the file."}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCg=="}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Reading."}}),
            serde_json::json!({"type":"content_block_stop","index":1}),
            serde_json::json!({"type":"ping"}),
            serde_json::json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01ABC","name":"read","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}),
            serde_json::json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":" \"a.rs\"}"}}),
            serde_json::json!({"type":"content_block_stop","index":2}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":30,"cache_read_input_tokens":40,"output_tokens_details":{"thinking_tokens":8}}}),
            serde_json::json!({"type":"message_stop"}),
        ]);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ResponseStreamEvent::ResponseReasoningTextDelta(_)))
        );
        assert_eq!(
            response.status,
            crate::authority::responses::Status::Completed
        );
        assert!(response.incomplete_details.is_none());
        let usage = response.usage.unwrap();
        assert_eq!(usage.input_tokens, 140);
        assert_eq!(usage.input_tokens_details.cached_tokens, 40);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.output_tokens_details.reasoning_tokens, 8);
        match &response.output[0] {
            OutputItem::Reasoning(reasoning) => {
                assert_eq!(reasoning.id.as_deref(), Some("msg_01_0"));
                assert_eq!(reasoning.encrypted_content.as_deref(), Some("EqQBCg=="));
                assert_eq!(
                    crate::types::item_text_preview(&Item::Reasoning(reasoning.clone())),
                    "Need the file."
                );
            }
            other => panic!("expected reasoning, got {other:?}"),
        }
        match &response.output[2] {
            OutputItem::FunctionCall(call) => {
                assert_eq!(call.call_id, "toolu_01ABC");
                assert_eq!(call.name, "read");
                assert_eq!(call.arguments, "{\"path\": \"a.rs\"}");
                assert_eq!(call.id.as_deref(), Some("msg_01_2"));
            }
            other => panic!("expected tool, got {other:?}"),
        }
    }

    #[test]
    fn parallel_tool_calls_keep_their_own_ids() {
        let (_, response) = run(&[
            start_message(),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_b","name":"grep","input":{}}}),
            serde_json::json!({"type":"content_block_stop","index":1}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":4}}),
            serde_json::json!({"type":"message_stop"}),
        ]);
        let calls: Vec<_> = response
            .output
            .iter()
            .filter_map(|item| match item {
                OutputItem::FunctionCall(call) => Some((
                    call.call_id.as_str(),
                    call.arguments.as_str(),
                    call.name.as_str(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            calls,
            vec![("toolu_a", "{}", "read"), ("toolu_b", "{}", "grep")]
        );
    }

}
