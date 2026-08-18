//! Product streaming contract for tool calls — single outbound choke point.
//!
//! # Contract
//!
//! Before any `response.function_call_arguments.delta` for an `item_id` is
//! forwarded to `on_event`, that `item_id` must already have had a
//! `response.output_item.added` (`function_call`) emitted.
//!
//! If the provider/dialect omits early `added`, this gate synthesizes one
//! (name may be empty; a later non-empty name triggers a patch `added`).
//! Empty name is **not** fail-closed — some providers deliver name late.
//!
//! All adapters that emit authority [`ResponseStreamEvent`]s must route through
//! [`forward_stream_event`] — never call `on_event` directly for live stream.
//!
//! Terminal outcomes: `response.completed` and `response.incomplete` both yield
//! Items (`Ok(Some(...))`). `response.failed` / `error` remain `Err`.

use std::collections::HashMap;

use crate::authority::responses::{
    AssistantRole, FunctionToolCall, Item, MessageItem, OutputItem, OutputMessage,
    OutputMessageContent, OutputStatus, OutputTextContent, ReasoningItem, ReasoningItemContent,
    ReasoningTextContent, Response, ResponseOutputItemAddedEvent, ResponseStreamEvent,
};
use crate::types::{LitecodeError, Result, StreamEvents};

#[derive(Debug, Default)]
struct ToolOpenState {
    /// `output_item.added` already forwarded (provider or synthesized).
    opened: bool,
    name: String,
    call_id: String,
    /// `output_index` of the tool's `added`; `None` until known. `0` is a real
    /// provider index, so it must not double as an "unknown" sentinel.
    output_index: Option<u32>,
}

/// Per-turn gate enforcing early tool-name ordering on the authority stream.
#[derive(Debug, Default)]
pub(super) struct StreamContractGate {
    tools: HashMap<String, ToolOpenState>,
    /// Synthetic sequence numbers for gate-injected events.
    synth_seq: u64,
    /// Highest provider `sequence_number` seen so far — synthesized seqs stay
    /// strictly above it so they can never collide with a provider seq.
    max_provider_seq: u64,
}

impl StreamContractGate {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn bump_seq(&mut self, at_least: u64) -> u64 {
        self.synth_seq = self
            .synth_seq
            .max(self.max_provider_seq)
            .max(at_least)
            .saturating_add(1);
        self.synth_seq
    }

    fn ensure_opened(
        &mut self,
        item_id: &str,
        output_index: u32,
        on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
        hint_seq: u64,
    ) {
        {
            let entry = self.tools.entry(item_id.to_string()).or_default();
            if entry.opened {
                if entry.output_index.is_none() {
                    entry.output_index = Some(output_index);
                }
                return;
            }
            entry.opened = true;
            entry.output_index = Some(output_index);
            if entry.call_id.is_empty() {
                entry.call_id = item_id.to_string();
            }
        }
        let (name, call_id) = {
            let entry = self.tools.get(item_id).expect("just inserted");
            (entry.name.clone(), entry.call_id.clone())
        };
        let seq = self.bump_seq(hint_seq);
        let added = ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
            sequence_number: seq,
            output_index,
            item: OutputItem::FunctionCall(FunctionToolCall {
                id: Some(item_id.to_string()),
                call_id,
                name,
                arguments: String::new(),
                status: Some(OutputStatus::InProgress),
                namespace: None,
            }),
        });
        if let Some(cb) = on_event.as_mut() {
            cb(added);
        }
    }

    fn record_added_function_call(&mut self, fc: &FunctionToolCall, output_index: u32) {
        let item_id = fc
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fc.call_id.clone());
        if item_id.is_empty() {
            return;
        }
        let entry = self.tools.entry(item_id).or_default();
        entry.opened = true;
        entry.output_index = Some(output_index);
        if !fc.call_id.is_empty() {
            entry.call_id = fc.call_id.clone();
        }
        if !fc.name.is_empty() {
            entry.name = fc.name.clone();
        }
    }

    /// If name newly becomes non-empty after we already opened, emit a patch `added`.
    fn maybe_patch_name(
        &mut self,
        item_id: &str,
        new_name: &str,
        output_index: u32,
        on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
        hint_seq: u64,
    ) {
        if new_name.is_empty() {
            return;
        }
        let (should_patch, call_id, idx) = {
            let Some(entry) = self.tools.get_mut(item_id) else {
                return;
            };
            if !entry.opened {
                entry.name = new_name.to_string();
                return;
            }
            if !entry.name.is_empty() {
                return;
            }
            entry.name = new_name.to_string();
            let call_id = if entry.call_id.is_empty() {
                item_id.to_string()
            } else {
                entry.call_id.clone()
            };
            let idx = entry.output_index.unwrap_or(output_index);
            (true, call_id, idx)
        };
        if !should_patch {
            return;
        }
        let seq = self.bump_seq(hint_seq);
        let added = ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
            sequence_number: seq,
            output_index: idx,
            item: OutputItem::FunctionCall(FunctionToolCall {
                id: Some(item_id.to_string()),
                call_id,
                name: new_name.to_string(),
                arguments: String::new(),
                status: Some(OutputStatus::InProgress),
                namespace: None,
            }),
        });
        if let Some(cb) = on_event.as_mut() {
            cb(added);
        }
    }
}

fn output_items_to_items(output: Vec<OutputItem>) -> Vec<Item> {
    output.into_iter().map(Item::from).collect()
}

/// Live Item shells opened by this step's stream (`output_item.added` and deltas).
/// Used to seal `incomplete` Items when the stream stops without `response.completed`.
#[derive(Debug, Default)]
pub(super) struct StreamItemAccumulator {
    order: Vec<String>,
    items: HashMap<String, Item>,
}

impl StreamItemAccumulator {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn upsert(&mut self, id: String, item: Item) {
        if !self.items.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.items.insert(id, item);
    }

    fn observe(&mut self, event: &ResponseStreamEvent) {
        match event {
            ResponseStreamEvent::ResponseOutputItemAdded(ev) => {
                let item = Item::from(ev.item.clone());
                if let Some(id) = item_id_of(&item) {
                    self.upsert(id, item);
                }
            }
            ResponseStreamEvent::ResponseOutputTextDelta(ev) if !ev.item_id.is_empty() => {
                self.append_message_text(&ev.item_id, &ev.delta);
            }
            ResponseStreamEvent::ResponseReasoningTextDelta(ev) if !ev.item_id.is_empty() => {
                self.append_reasoning_text(&ev.item_id, &ev.delta);
            }
            ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(ev)
                if !ev.item_id.is_empty() =>
            {
                self.ensure_function_call(&ev.item_id);
                self.append_fc_args(&ev.item_id, &ev.delta);
            }
            ResponseStreamEvent::ResponseFunctionCallArgumentsDone(ev)
                if !ev.item_id.is_empty() =>
            {
                self.ensure_function_call(&ev.item_id);
                if !ev.arguments.is_empty() {
                    self.set_fc_args(&ev.item_id, &ev.arguments);
                }
                if let Some(name) = ev.name.as_deref().filter(|s| !s.is_empty()) {
                    self.set_fc_name(&ev.item_id, name);
                }
            }
            _ => {}
        }
    }

    fn append_message_text(&mut self, item_id: &str, delta: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Message(MessageItem::Output(msg))) => {
                append_output_text(msg, delta);
            }
            Some(_) => {}
            None => {
                self.upsert(
                    item_id.to_string(),
                    Item::Message(MessageItem::Output(OutputMessage {
                        id: item_id.to_string(),
                        role: AssistantRole::Assistant,
                        status: OutputStatus::InProgress,
                        phase: None,
                        content: vec![OutputMessageContent::OutputText(OutputTextContent {
                            text: delta.to_string(),
                            annotations: vec![],
                            logprobs: None,
                        })],
                    })),
                );
            }
        }
    }

    fn append_reasoning_text(&mut self, item_id: &str, delta: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Reasoning(r)) => append_reasoning_text(r, delta),
            Some(_) => {}
            None => {
                self.upsert(
                    item_id.to_string(),
                    Item::Reasoning(ReasoningItem {
                        id: Some(item_id.to_string()),
                        summary: vec![],
                        content: Some(vec![ReasoningItemContent::ReasoningText(
                            ReasoningTextContent {
                                text: delta.to_string(),
                            },
                        )]),
                        encrypted_content: None,
                        status: Some(OutputStatus::InProgress),
                    }),
                );
            }
        }
    }

    fn ensure_function_call(&mut self, item_id: &str) {
        if self.items.contains_key(item_id) {
            return;
        }
        self.upsert(
            item_id.to_string(),
            Item::FunctionCall(FunctionToolCall {
                id: Some(item_id.to_string()),
                call_id: item_id.to_string(),
                name: String::new(),
                arguments: String::new(),
                status: Some(OutputStatus::InProgress),
                namespace: None,
            }),
        );
    }

    fn append_fc_args(&mut self, item_id: &str, delta: &str) {
        if let Some(Item::FunctionCall(fc)) = self.items.get_mut(item_id) {
            fc.arguments.push_str(delta);
        }
    }

    fn set_fc_args(&mut self, item_id: &str, arguments: &str) {
        if let Some(Item::FunctionCall(fc)) = self.items.get_mut(item_id) {
            fc.arguments = arguments.to_string();
        }
    }

    fn set_fc_name(&mut self, item_id: &str, name: &str) {
        if let Some(Item::FunctionCall(fc)) = self.items.get_mut(item_id)
            && fc.name.is_empty()
        {
            fc.name = name.to_string();
        }
    }

    pub(super) fn seal_incomplete(&self) -> Vec<Item> {
        let mut out: Vec<Item> = self
            .order
            .iter()
            .filter_map(|id| self.items.get(id).cloned())
            .collect();
        mark_items_incomplete(&mut out);
        out
    }
}

fn item_id_of(item: &Item) -> Option<String> {
    match item {
        Item::Message(MessageItem::Output(m)) if !m.id.is_empty() => Some(m.id.clone()),
        Item::Reasoning(r) => r.id.clone().filter(|s| !s.is_empty()),
        Item::FunctionCall(fc) => fc
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| (!fc.call_id.is_empty()).then(|| fc.call_id.clone())),
        _ => None,
    }
}

fn append_output_text(msg: &mut OutputMessage, delta: &str) {
    for part in &mut msg.content {
        if let OutputMessageContent::OutputText(t) = part {
            t.text.push_str(delta);
            return;
        }
    }
    msg.content
        .push(OutputMessageContent::OutputText(OutputTextContent {
            text: delta.to_string(),
            annotations: vec![],
            logprobs: None,
        }));
}

fn append_reasoning_text(r: &mut ReasoningItem, delta: &str) {
    let parts = r.content.get_or_insert_with(Vec::new);
    if let Some(p) = parts.iter_mut().next() {
        let ReasoningItemContent::ReasoningText(t) = p;
        t.text.push_str(delta);
        return;
    }
    parts.push(ReasoningItemContent::ReasoningText(ReasoningTextContent {
        text: delta.to_string(),
    }));
}

pub(super) fn mark_items_incomplete(items: &mut [Item]) {
    for item in items {
        match item {
            Item::Message(MessageItem::Output(m)) => m.status = OutputStatus::Incomplete,
            Item::FunctionCall(fc) => fc.status = Some(OutputStatus::Incomplete),
            Item::Reasoning(r) => r.status = Some(OutputStatus::Incomplete),
            _ => {}
        }
    }
}

/// After SSE ends or cancel: use a terminal payload if present, else seal opened Items.
pub(super) fn resolve_stream_outcome(
    terminal_items: Option<Vec<Item>>,
    acc: &StreamItemAccumulator,
    cancelled: bool,
) -> Result<Vec<Item>> {
    if let Some(items) = terminal_items {
        return Ok(items);
    }
    if !acc.is_empty() {
        return Ok(acc.seal_incomplete());
    }
    if cancelled {
        return Err(LitecodeError::Canceled);
    }
    Err(LitecodeError::Llm(
        "stream ended without a terminal response".into(),
    ))
}

fn terminal_error_message(kind: &str, response: &Response) -> String {
    if let Some(err) = &response.error {
        format!("{kind}: {} ({})", err.message, err.code)
    } else {
        format!("{kind}: status={:?}", response.status)
    }
}

fn emit(
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
    event: ResponseStreamEvent,
) {
    if let Some(cb) = on_event.as_mut() {
        cb(event);
    }
}

/// Normalize tool-call ordering, forward to `on_event`, then apply terminal outcomes.
///
/// Returns `Ok(Some(items))` on `response.completed` / `response.incomplete`,
/// `Ok(None)` for non-terminal events, and `Err` for failed / error events.
fn provider_seq_of(ev: &ResponseStreamEvent) -> u64 {
    match ev {
        ResponseStreamEvent::ResponseOutputItemAdded(e) => e.sequence_number,
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(e) => e.sequence_number,
        ResponseStreamEvent::ResponseFunctionCallArgumentsDone(e) => e.sequence_number,
        _ => 0,
    }
}

pub(super) fn forward_stream_event(
    gate: &mut StreamContractGate,
    acc: &mut StreamItemAccumulator,
    event: ResponseStreamEvent,
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
) -> Result<Option<Vec<Item>>> {
    // Keep synthesized seqs strictly above every provider seq seen so far, so a
    // gate-injected event can never collide with a provider sequence_number.
    gate.max_provider_seq = gate.max_provider_seq.max(provider_seq_of(&event));
    acc.observe(&event);
    match &event {
        ResponseStreamEvent::ResponseOutputItemAdded(ev) => {
            if let OutputItem::FunctionCall(fc) = &ev.item {
                gate.record_added_function_call(fc, ev.output_index);
            }
            emit(on_event, event.clone());
        }
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(ev) => {
            gate.ensure_opened(&ev.item_id, ev.output_index, on_event, ev.sequence_number);
            emit(on_event, event.clone());
        }
        ResponseStreamEvent::ResponseFunctionCallArgumentsDone(ev) => {
            gate.ensure_opened(&ev.item_id, ev.output_index, on_event, ev.sequence_number);
            if let Some(name) = ev.name.as_deref().filter(|s| !s.is_empty()) {
                gate.maybe_patch_name(
                    &ev.item_id,
                    name,
                    ev.output_index,
                    on_event,
                    ev.sequence_number,
                );
            }
            emit(on_event, event.clone());
        }
        _ => {
            emit(on_event, event.clone());
        }
    }

    match event {
        ResponseStreamEvent::ResponseCompleted(ev) => {
            Ok(Some(output_items_to_items(ev.response.output)))
        }
        ResponseStreamEvent::ResponseFailed(ev) => Err(LitecodeError::Llm(terminal_error_message(
            "response.failed",
            &ev.response,
        ))),
        ResponseStreamEvent::ResponseIncomplete(ev) => {
            let mut items = output_items_to_items(ev.response.output);
            if items.is_empty() {
                items = acc.seal_incomplete();
            } else {
                mark_items_incomplete(&mut items);
            }
            Ok(Some(items))
        }
        ResponseStreamEvent::ResponseError(ev) => Err(LitecodeError::Llm(format!(
            "response error: {} (code={:?}, param={:?})",
            ev.message, ev.code, ev.param
        ))),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        ResponseFunctionCallArgumentsDeltaEvent, ResponseFunctionCallArgumentsDoneEvent,
    };

    fn collect_forward(
        gate: &mut StreamContractGate,
        events: Vec<ResponseStreamEvent>,
    ) -> Vec<ResponseStreamEvent> {
        let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let out_cb = std::sync::Arc::clone(&out);
        let mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                out_cb.lock().unwrap().push(ev);
            }));
        let mut acc = StreamItemAccumulator::new();
        for ev in events {
            forward_stream_event(gate, &mut acc, ev, &mut on_event).unwrap();
        }
        drop(on_event);
        std::sync::Arc::try_unwrap(out)
            .expect("callback dropped")
            .into_inner()
            .unwrap()
    }

    fn delta(item_id: &str, seq: u64, text: &str) -> ResponseStreamEvent {
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
            ResponseFunctionCallArgumentsDeltaEvent {
                sequence_number: seq,
                item_id: item_id.into(),
                output_index: 0,
                delta: text.into(),
            },
        )
    }

    fn added(item_id: &str, name: &str, seq: u64) -> ResponseStreamEvent {
        ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
            sequence_number: seq,
            output_index: 0,
            item: OutputItem::FunctionCall(FunctionToolCall {
                id: Some(item_id.into()),
                call_id: item_id.into(),
                name: name.into(),
                arguments: String::new(),
                status: Some(OutputStatus::InProgress),
                namespace: None,
            }),
        })
    }

    fn done(item_id: &str, name: Option<&str>, seq: u64) -> ResponseStreamEvent {
        ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
            ResponseFunctionCallArgumentsDoneEvent {
                name: name.map(str::to_string),
                sequence_number: seq,
                item_id: item_id.into(),
                output_index: 0,
                arguments: "{}".into(),
            },
        )
    }

    fn is_added_named(ev: &ResponseStreamEvent, name: &str) -> bool {
        matches!(
            ev,
            ResponseStreamEvent::ResponseOutputItemAdded(e)
                if matches!(&e.item, OutputItem::FunctionCall(fc) if fc.name == name)
        )
    }

    fn is_delta(ev: &ResponseStreamEvent) -> bool {
        matches!(
            ev,
            ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(_)
        )
    }

    #[test]
    fn delta_before_added_synthesizes_added_first() {
        let mut gate = StreamContractGate::new();
        let out = collect_forward(&mut gate, vec![delta("fc_1", 1, "{\"a\":1}")]);
        assert_eq!(out.len(), 2);
        assert!(is_added_named(&out[0], ""));
        assert!(is_delta(&out[1]));
    }

    #[test]
    fn in_order_added_then_delta_does_not_duplicate_added() {
        let mut gate = StreamContractGate::new();
        let out = collect_forward(
            &mut gate,
            vec![added("fc_1", "read", 1), delta("fc_1", 2, "{}")],
        );
        assert_eq!(out.len(), 2);
        assert!(is_added_named(&out[0], "read"));
        assert!(is_delta(&out[1]));
    }

    #[test]
    fn empty_name_then_done_with_name_emits_patch_added() {
        let mut gate = StreamContractGate::new();
        let out = collect_forward(
            &mut gate,
            vec![delta("fc_1", 1, "{}"), done("fc_1", Some("bash"), 2)],
        );
        // synth added (empty) → delta → patch added (bash) → done
        assert!(out.len() >= 3);
        assert!(is_added_named(&out[0], ""));
        assert!(is_delta(&out[1]));
        assert!(
            out.iter().any(|e| is_added_named(e, "bash")),
            "expected patch added with name bash, got {out:?}"
        );
    }

    #[test]
    fn real_output_index_zero_is_preserved_not_treated_as_unknown() {
        let mut gate = StreamContractGate::new();
        // The provider's `added` carries a real output_index of 0; a later `done`
        // for the same tool carries index 7. The gate must keep the real 0 for
        // the patch `added` — `0` is a valid index, not an "unknown" sentinel.
        let evs = vec![
            ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
                sequence_number: 1,
                output_index: 0,
                item: OutputItem::FunctionCall(FunctionToolCall {
                    id: Some("fc_1".into()),
                    call_id: "fc_1".into(),
                    name: String::new(),
                    arguments: String::new(),
                    status: Some(OutputStatus::InProgress),
                    namespace: None,
                }),
            }),
            ResponseStreamEvent::ResponseFunctionCallArgumentsDone(
                ResponseFunctionCallArgumentsDoneEvent {
                    name: Some("bash".into()),
                    sequence_number: 2,
                    item_id: "fc_1".into(),
                    output_index: 7,
                    arguments: "{}".into(),
                },
            ),
        ];
        let out = collect_forward(&mut gate, evs);
        let patch = out
            .iter()
            .find(|e| is_added_named(e, "bash"))
            .expect("patch added with name");
        match patch {
            ResponseStreamEvent::ResponseOutputItemAdded(e) => assert_eq!(e.output_index, 0),
            _ => panic!("expected ResponseOutputItemAdded, got {patch:?}"),
        }
    }

    #[test]
    fn synthesized_seqs_stay_above_all_provider_seqs() {
        let mut gate = StreamContractGate::new();
        // A high provider seq (50) arrives first; a later delta with a low seq
        // still synthesizes an `added` above every provider seq seen so far, so
        // the synthesized seq can never collide with a future provider seq (2).
        let out = collect_forward(
            &mut gate,
            vec![
                added("x", "read", 50),
                delta("a", 1, "{}"),
                added("y", "grep", 2),
            ],
        );
        let synth = out
            .iter()
            .find(|e| is_added_named(e, ""))
            .expect("synthesized added");
        match synth {
            ResponseStreamEvent::ResponseOutputItemAdded(e) => {
                assert!(
                    e.sequence_number > 50,
                    "synth seq {} collides",
                    e.sequence_number
                );
            }
            _ => panic!("expected synthesized added, got {synth:?}"),
        }
    }

    fn incomplete_event(text: &str) -> ResponseStreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "response.incomplete",
            "sequence_number": 9,
            "response": {
                "id": "resp_inc",
                "object": "response",
                "created_at": 1,
                "model": "gpt-4o",
                "status": "incomplete",
                "output": [{
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "incomplete",
                    "content": [{"type": "output_text", "text": text, "annotations": []}]
                }]
            }
        }))
        .expect("incomplete event")
    }

    fn failed_event() -> ResponseStreamEvent {
        serde_json::from_value(serde_json::json!({
            "type": "response.failed",
            "sequence_number": 9,
            "response": {
                "id": "resp_fail",
                "object": "response",
                "created_at": 1,
                "model": "gpt-4o",
                "status": "failed",
                "output": [],
                "error": { "code": "server_error", "message": "boom" }
            }
        }))
        .expect("failed event")
    }

    #[test]
    fn incomplete_terminal_yields_items() {
        let mut gate = StreamContractGate::new();
        let mut acc = StreamItemAccumulator::new();
        let items = forward_stream_event(&mut gate, &mut acc, incomplete_event("hi"), &mut None)
            .expect("incomplete is not Err")
            .expect("terminal items");
        assert_eq!(items.len(), 1);
        match &items[0] {
            Item::Message(MessageItem::Output(msg)) => {
                assert_eq!(msg.status, OutputStatus::Incomplete);
                assert_eq!(crate::types::item_text_preview(&items[0]), "hi");
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn failed_terminal_still_errors() {
        let mut gate = StreamContractGate::new();
        let mut acc = StreamItemAccumulator::new();
        let err = forward_stream_event(&mut gate, &mut acc, failed_event(), &mut None)
            .expect_err("failed stays Err");
        assert!(err.to_string().contains("response.failed"));
    }

    #[test]
    fn text_delta_seal_incomplete_without_terminal() {
        let mut acc = StreamItemAccumulator::new();
        let ev: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "partial"
        }))
        .unwrap();
        acc.observe(&ev);
        let sealed = acc.seal_incomplete();
        assert_eq!(sealed.len(), 1);
        assert_eq!(crate::types::item_text_preview(&sealed[0]), "partial");
        match &sealed[0] {
            Item::Message(MessageItem::Output(msg)) => {
                assert_eq!(msg.status, OutputStatus::Incomplete);
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[test]
    fn resolve_empty_cancel_is_canceled() {
        let acc = StreamItemAccumulator::new();
        let err = resolve_stream_outcome(None, &acc, true).unwrap_err();
        assert!(matches!(err, LitecodeError::Canceled));
    }
}
