use std::collections::BTreeMap;

use serde_json::Value;
use uuid::Uuid;

use crate::authority::responses::{
    AssistantRole, FunctionToolCall, OutputItem, OutputMessage, OutputStatus, ReasoningItem,
    ResponseFunctionCallArgumentsDeltaEvent, ResponseFunctionCallArgumentsDoneEvent,
    ResponseOutputItemAddedEvent, ResponseReasoningTextDeltaEvent, ResponseStreamEvent,
    ResponseTextDeltaEvent,
};
use crate::types::{LitecodeError, Result};

use super::decode::chat_reasoning_text;
use super::usage::chat_usage_to_responses;

fn synth_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

struct ToolAcc {
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    output_index: u32,
    opened: bool,
}

pub(super) struct ChatSynth {
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
    pub(super) fn new() -> Self {
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
        let id = synth_id("cc_msg");
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
        let id = synth_id("cc_rs");
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

    pub(super) fn ingest_chunk(&mut self, chunk: &Value, events: &mut Vec<ResponseStreamEvent>) {
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
                    item_id: synth_id("cc_fc"),
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

    pub(super) fn finish_events(&mut self, model: &str) -> Result<Vec<ResponseStreamEvent>> {
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
            "id": synth_id("cc_resp"),
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
