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

use super::chat_usage::chat_usage_to_responses;

/// Reasoning text carried by one Chat Completions message/delta.
///
/// Vendors disagree on the key (`reasoning_content`, `reasoning`, or a nested
/// `reasoning.content`); all three are read, in that order.
pub(super) fn chat_reasoning_text(node: &Value) -> Option<&str> {
    if let Some(text) = node
        .get("reasoning_content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text);
    }
    if let Some(text) = node
        .get("reasoning")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text);
    }
    node.pointer("/reasoning/content")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// Item ids are always synthesized: Chat Completions has no item identity.
fn synth_id(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::new_v4().simple())
}

const SEAM_MARK: &str = "[SEAM]";
const SAMPLE_LIMIT: usize = 3;
const EXCERPT_CHARS: usize = 24;

/// Forensics for the vendor's `reasoning_content` / `content` split.
///
/// Some Chat Completions vendors end a thinking block mid-token and keep
/// streaming the rest of the reasoning as `content`. The field is the only
/// signal this protocol carries, so the text is never rewritten; this report
/// only makes the seam visible in the logs.
#[derive(Debug, Default)]
pub(super) struct SeamReport {
    /// Content opened mid-token right after reasoning (a code span or quoted
    /// token cut in half between the two fields).
    pub(super) glued: usize,
    /// Reasoning resumed after the message item had already opened.
    pub(super) resumed: usize,
    /// Up to [`SAMPLE_LIMIT`] excerpts around a seam, `<tail>[SEAM]<head>`.
    pub(super) samples: Vec<String>,
}

impl SeamReport {
    fn is_empty(&self) -> bool {
        self.glued == 0 && self.resumed == 0
    }
}

/// Chars that hold a token together: a seam inside one of these spans cannot
/// be where a real answer starts.
fn is_seam_delimiter(c: char) -> bool {
    "`\"'“”‘’()（）[]{}<>「」『』".contains(c)
}

fn tail_excerpt(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().rev().take(EXCERPT_CHARS).collect();
    chars.reverse();
    sanitize_excerpt(chars)
}

fn head_excerpt(text: &str) -> String {
    sanitize_excerpt(text.chars().take(EXCERPT_CHARS).collect())
}

/// Keep a sample on one log line.
fn sanitize_excerpt(chars: Vec<char>) -> String {
    chars
        .into_iter()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
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
    /// Whitespace seen before the first visible token; dropped if nothing
    /// visible ever follows.
    pending_content: String,
    content_started: bool,
    rs_id: Option<String>,
    rs_index: u32,
    rs_text: String,
    tools: BTreeMap<u32, ToolAcc>,
    usage: Option<Value>,
    seams: SeamReport,
    finish_reason: Option<String>,
}

impl ChatSynth {
    pub(super) fn new() -> Self {
        Self {
            seq: 0,
            next_output: 0,
            msg_id: None,
            msg_index: 0,
            msg_text: String::new(),
            pending_content: String::new(),
            content_started: false,
            rs_id: None,
            rs_index: 0,
            rs_text: String::new(),
            tools: BTreeMap::new(),
            usage: None,
            seams: SeamReport::default(),
            finish_reason: None,
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
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else {
            return;
        };
        let content_text = delta
            .get("content")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty());
        if let Some(text) = content_text {
            if !self.content_started {
                self.content_started = true;
                self.note_content_start(text);
            }
            if self.msg_id.is_none() && text.trim().is_empty() {
                // Vendors close a thinking block with whitespace ("\n\n") even
                // when no visible answer follows. A message is born on its
                // first visible token; until then the separator is held.
                self.pending_content.push_str(text);
            } else {
                let mut visible = std::mem::take(&mut self.pending_content);
                visible.push_str(text);
                let (item_id, output_index) = self.ensure_message(events);
                self.msg_text.push_str(&visible);
                let seq = self.bump();
                events.push(ResponseStreamEvent::ResponseOutputTextDelta(
                    ResponseTextDeltaEvent {
                        sequence_number: seq,
                        item_id,
                        output_index,
                        content_index: 0,
                        delta: visible,
                        logprobs: None,
                    },
                ));
            }
        }
        if let Some(text) = chat_reasoning_text(delta) {
            if self.msg_id.is_some() && content_text.is_none() {
                self.note_reasoning_resume(text);
            }
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

    /// The first content token of a request: a character-level seam inside a
    /// code span or quoted token means the vendor ended its thinking block
    /// mid-token. Detect and report; never rewrite the text.
    fn note_content_start(&mut self, text: &str) {
        let (Some(tail), Some(head)) = (self.rs_text.chars().last(), text.chars().next()) else {
            return;
        };
        if tail.is_whitespace() || !is_seam_delimiter(tail) || !is_seam_delimiter(head) {
            return;
        }
        self.seams.glued += 1;
        let sample = format!(
            "content-after-reasoning: ...{}{SEAM_MARK}{}...",
            tail_excerpt(&self.rs_text),
            head_excerpt(text),
        );
        self.push_sample(sample);
    }

    /// Reasoning after the message item opened never happens in a well-formed
    /// stream; record it and keep the text in arrival order.
    fn note_reasoning_resume(&mut self, text: &str) {
        self.seams.resumed += 1;
        let sample = format!(
            "reasoning-after-content: ...{}{SEAM_MARK}{}...",
            tail_excerpt(&self.msg_text),
            head_excerpt(text),
        );
        self.push_sample(sample);
    }

    fn push_sample(&mut self, sample: String) {
        if self.seams.samples.len() < SAMPLE_LIMIT {
            self.seams.samples.push(sample);
        }
    }

    /// Forensics-only view of this request's vendor seams, `None` when clean.
    pub(super) fn seam_report(&self) -> Option<&SeamReport> {
        (!self.seams.is_empty()).then_some(&self.seams)
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
        let (status, incomplete) = Self::chat_terminal_status(self.finish_reason.as_deref());
        let output_status = if status == "completed" {
            OutputStatus::Completed
        } else {
            OutputStatus::Incomplete
        };
        let output = self.output_values(output_status);
        let seq = self.bump();
        let mut response = serde_json::json!({
            "id": synth_id("cc_resp"),
            "object": "response",
            "created_at": 0,
            "model": model,
            "status": status,
            "output": output,
        });
        if let Some(reason) = incomplete {
            response["incomplete_details"] = serde_json::json!({ "reason": reason });
        }
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

    fn chat_terminal_status(finish_reason: Option<&str>) -> (&'static str, Option<String>) {
        let Some(finish_reason) = finish_reason else {
            // `[DONE]` is a transport sentinel, not provider finish evidence.
            // A stream that never named its finish reason is not accepted as a
            // completed request (and therefore cannot commit a summary).
            tracing::warn!("chat stream ended without finish_reason");
            return ("incomplete", Some("missing_finish_reason".into()));
        };
        match finish_reason {
            "stop" | "tool_calls" | "function_call" => ("completed", None),
            "length" => ("incomplete", Some("max_output_tokens".into())),
            "content_filter" => ("incomplete", Some("content_filter".into())),
            other => {
                tracing::info!(
                    finish_reason = other,
                    "chat finish_reason mapped to incomplete"
                );
                ("incomplete", Some(other.to_string()))
            }
        }
    }

    fn output_values(&self, status: OutputStatus) -> Vec<Value> {
        let mut out: Vec<(u32, Value)> = Vec::new();
        if let Some(id) = &self.rs_id {
            out.push((
                self.rs_index,
                serde_json::json!({
                    "type": "reasoning",
                    "id": id,
                    "summary": [],
                    "content": [{"type": "reasoning_text", "text": self.rs_text}],
                    "status": status
                }),
            ));
        }
        if let Some(id) = &self.msg_id {
            out.push((
                self.msg_index,
                serde_json::json!({
                    "type": "message",
                    "id": id,
                    "role": "assistant",
                    "status": status,
                    "content": [{"type": "output_text", "text": self.msg_text, "annotations": []}]
                }),
            ));
        }
        for tool in self.tools.values().filter(|tool| tool.opened) {
            let call_id = if tool.call_id.is_empty() {
                tool.item_id.clone()
            } else {
                tool.call_id.clone()
            };
            out.push((
                tool.output_index,
                serde_json::json!({
                    "type": "function_call",
                    "id": tool.item_id,
                    "call_id": call_id,
                    "name": tool.name,
                    "arguments": tool.arguments,
                    "status": status
                }),
            ));
        }
        out.sort_by_key(|(index, _)| *index);
        out.into_iter().map(|(_, item)| item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{OutputMessageContent, Status};

    fn chunk(delta: Value) -> Value {
        serde_json::json!({ "choices": [{ "index": 0, "delta": delta }] })
    }

    fn completed_output(events: &[ResponseStreamEvent]) -> Vec<OutputItem> {
        events
            .iter()
            .find_map(|event| match event {
                ResponseStreamEvent::ResponseCompleted(completed) => {
                    Some(completed.response.output.clone())
                }
                _ => None,
            })
            .expect("response.completed")
    }

    fn message_text(items: &[OutputItem]) -> Option<String> {
        items.iter().find_map(|item| match item {
            OutputItem::Message(message) => message.content.iter().find_map(|part| match part {
                OutputMessageContent::OutputText(text) => Some(text.text.clone()),
                _ => None,
            }),
            _ => None,
        })
    }

    fn messages_opened(events: &[ResponseStreamEvent]) -> usize {
        events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    ResponseStreamEvent::ResponseOutputItemAdded(added)
                        if matches!(&added.item, OutputItem::Message(_))
                )
            })
            .count()
    }

    #[test]
    fn missing_finish_reason_marks_summary_output_incomplete() {
        let mut synth = ChatSynth::new();
        let mut events = Vec::new();
        synth.ingest_chunk(
            &chunk(serde_json::json!({ "content": "malformed summary" })),
            &mut events,
        );
        let done = synth.finish_events("m").unwrap();
        let completed = done
            .iter()
            .find_map(|event| match event {
                ResponseStreamEvent::ResponseCompleted(completed) => Some(completed),
                _ => None,
            })
            .expect("response.completed");
        assert_eq!(completed.response.status, Status::Incomplete);
        let output = completed_output(&done);
        let message = output
            .iter()
            .find_map(|item| match item {
                OutputItem::Message(message) => Some(message),
                _ => None,
            })
            .expect("assistant message");
        assert_eq!(message.status, OutputStatus::Incomplete);
    }

    #[test]
    fn finish_length_marks_summary_output_incomplete() {
        let mut synth = ChatSynth::new();
        let mut events = Vec::new();
        synth.ingest_chunk(
            &chunk(serde_json::json!({ "content": "truncated summary" })),
            &mut events,
        );
        synth.ingest_chunk(
            &serde_json::json!({
                "choices": [{
                    "index": 0,
                    "finish_reason": "length",
                    "delta": {}
                }]
            }),
            &mut events,
        );
        let done = synth.finish_events("m").unwrap();
        let completed = done
            .iter()
            .find_map(|event| match event {
                ResponseStreamEvent::ResponseCompleted(completed) => Some(completed),
                _ => None,
            })
            .expect("response.completed");
        assert_eq!(completed.response.status, Status::Incomplete);
        let output = completed_output(&done);
        let message = output
            .iter()
            .find_map(|item| match item {
                OutputItem::Message(message) => Some(message),
                _ => None,
            })
            .expect("assistant message");
        assert_eq!(message.status, OutputStatus::Incomplete);
    }

    #[test]
    fn whitespace_only_content_never_births_a_message() {
        let mut synth = ChatSynth::new();
        let mut events = Vec::new();
        synth.ingest_chunk(
            &chunk(serde_json::json!({ "content": "\n\n" })),
            &mut events,
        );
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(messages_opened(&events), 0);
        let done = synth.finish_events("m").unwrap();
        assert_eq!(message_text(&completed_output(&done)), None);
        assert!(synth.seam_report().is_none());
    }
}
