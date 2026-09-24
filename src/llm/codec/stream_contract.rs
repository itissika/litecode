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
//! Every codec that emits authority [`ResponseStreamEvent`]s must route through
//! [`forward_stream_event`] — never call `on_event` directly for live stream.
//!
//! Terminal outcomes: `response.completed` and `response.incomplete` both yield
//! Items (`Ok(Some(...))`). `response.failed` / `error` remain `Err`.

use std::collections::HashMap;

use crate::authority::responses::{
    AssistantRole, FunctionToolCall, Item, MessageItem, OutputItem, OutputMessage,
    OutputMessageContent, OutputStatus, OutputTextContent, ReasoningItem, ReasoningItemContent,
    ReasoningTextContent, Response, ResponseOutputItemAddedEvent, ResponseStreamEvent, SummaryPart,
    SummaryTextContent,
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
        if item_id.is_empty() {
            return;
        }
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
///
/// The accumulator mirrors the provider's own item lifecycle so that a stream
/// that ends without `response.completed` still yields the same items the
/// terminal payload would have carried:
///
/// * `output_item.added` opens a shell (or a later `added` re-opens one).
/// * deltas mutate that shell — `output_text`, `reasoning_text`, function-call
///   arguments, and reasoning **summary parts** keyed by `summary_index`.
/// * `*.done` / `output_item.done` are that item's terminal payload and replace
///   the shell outright.
///
/// Nothing here assigns Session identity: the same item id always denotes one
/// item, in `order`, for the whole step.
#[derive(Debug, Default)]
pub(crate) struct StreamItemAccumulator {
    order: Vec<String>,
    items: HashMap<String, Item>,
}

impl StreamItemAccumulator {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The payload accumulated for `item_id` so far, if the stream opened it.
    pub(crate) fn get(&self, item_id: &str) -> Option<&Item> {
        self.items.get(item_id)
    }

    fn upsert(&mut self, id: String, item: Item) {
        if !self.items.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.items.insert(id, item);
    }

    pub(crate) fn observe(&mut self, event: &ResponseStreamEvent) {
        match event {
            ResponseStreamEvent::ResponseOutputItemAdded(ev) => {
                let item = Item::from(ev.item.clone());
                if let Some(id) = item_id_of(&item) {
                    self.upsert(id, item);
                }
            }
            ResponseStreamEvent::ResponseOutputItemDone(ev) => {
                let item = Item::from(ev.item.clone());
                if let Some(id) = item_id_of(&item) {
                    self.upsert(id, item);
                }
            }
            ResponseStreamEvent::ResponseOutputTextDelta(ev) if !ev.item_id.is_empty() => {
                self.append_message_text(&ev.item_id, &ev.delta);
            }
            ResponseStreamEvent::ResponseOutputTextDone(ev) if !ev.item_id.is_empty() => {
                self.set_message_text(&ev.item_id, &ev.text);
            }
            ResponseStreamEvent::ResponseReasoningTextDelta(ev) if !ev.item_id.is_empty() => {
                self.append_reasoning_text(&ev.item_id, &ev.delta);
            }
            ResponseStreamEvent::ResponseReasoningTextDone(ev) if !ev.item_id.is_empty() => {
                self.set_reasoning_text(&ev.item_id, &ev.text);
            }
            ResponseStreamEvent::ResponseReasoningSummaryTextDelta(ev)
                if !ev.item_id.is_empty() =>
            {
                self.append_reasoning_summary(&ev.item_id, ev.summary_index, &ev.delta);
            }
            ResponseStreamEvent::ResponseReasoningSummaryTextDone(ev) if !ev.item_id.is_empty() => {
                self.set_reasoning_summary(&ev.item_id, ev.summary_index, &ev.text);
            }
            ResponseStreamEvent::ResponseReasoningSummaryPartAdded(ev)
                if !ev.item_id.is_empty() =>
            {
                let SummaryPart::SummaryText(part) = &ev.part;
                if !part.text.is_empty() {
                    self.set_reasoning_summary(&ev.item_id, ev.summary_index, &part.text);
                }
            }
            ResponseStreamEvent::ResponseReasoningSummaryPartDone(ev) if !ev.item_id.is_empty() => {
                let SummaryPart::SummaryText(part) = &ev.part;
                self.set_reasoning_summary(&ev.item_id, ev.summary_index, &part.text);
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

    /// One `summary_text` slot per `summary_index`, padded so a part that
    /// arrives out of order still lands in its own place.
    fn reasoning_summary_slot<'a>(r: &'a mut ReasoningItem, index: u32) -> &'a mut String {
        while r.summary.len() <= index as usize {
            r.summary.push(SummaryPart::SummaryText(SummaryTextContent {
                text: String::new(),
            }));
        }
        match &mut r.summary[index as usize] {
            SummaryPart::SummaryText(part) => &mut part.text,
        }
    }

    fn append_reasoning_summary(&mut self, item_id: &str, index: u32, delta: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Reasoning(r)) => {
                Self::reasoning_summary_slot(r, index).push_str(delta);
            }
            Some(_) => {}
            None => {
                let mut r = ReasoningItem {
                    id: Some(item_id.to_string()),
                    summary: vec![],
                    content: None,
                    encrypted_content: None,
                    status: Some(OutputStatus::InProgress),
                };
                Self::reasoning_summary_slot(&mut r, index).push_str(delta);
                self.upsert(item_id.to_string(), Item::Reasoning(r));
            }
        }
    }

    fn set_reasoning_summary(&mut self, item_id: &str, index: u32, text: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Reasoning(r)) => {
                *Self::reasoning_summary_slot(r, index) = text.to_string();
            }
            Some(_) => {}
            None => {
                let mut r = ReasoningItem {
                    id: Some(item_id.to_string()),
                    summary: vec![],
                    content: None,
                    encrypted_content: None,
                    status: Some(OutputStatus::InProgress),
                };
                *Self::reasoning_summary_slot(&mut r, index) = text.to_string();
                self.upsert(item_id.to_string(), Item::Reasoning(r));
            }
        }
    }

    fn set_message_text(&mut self, item_id: &str, text: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Message(MessageItem::Output(msg))) => {
                set_output_text(msg, text);
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
                            text: text.to_string(),
                            annotations: vec![],
                            logprobs: None,
                        })],
                    })),
                );
            }
        }
    }

    fn set_reasoning_text(&mut self, item_id: &str, text: &str) {
        match self.items.get_mut(item_id) {
            Some(Item::Reasoning(r)) => set_reasoning_text(r, text),
            Some(_) => {}
            None => {
                self.upsert(
                    item_id.to_string(),
                    Item::Reasoning(ReasoningItem {
                        id: Some(item_id.to_string()),
                        summary: vec![],
                        content: Some(vec![ReasoningItemContent::ReasoningText(
                            ReasoningTextContent {
                                text: text.to_string(),
                            },
                        )]),
                        encrypted_content: None,
                        status: Some(OutputStatus::InProgress),
                    }),
                );
            }
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

    pub(crate) fn seal_incomplete(&self) -> Vec<Item> {
        let mut out: Vec<Item> = self
            .order
            .iter()
            .filter_map(|id| self.items.get(id).cloned())
            .collect();
        mark_items_incomplete(&mut out);
        out
    }
}

/// The provider's own id for an item, when it has one.
///
/// This is the key the stream and the terminal payload agree on; it is not log
/// identity (the log's identity is `seq`).
pub(crate) fn item_id_of(item: &Item) -> Option<String> {
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

fn set_output_text(msg: &mut OutputMessage, text: &str) {
    for part in &mut msg.content {
        if let OutputMessageContent::OutputText(t) = part {
            t.text = text.to_string();
            return;
        }
    }
    msg.content
        .push(OutputMessageContent::OutputText(OutputTextContent {
            text: text.to_string(),
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

fn set_reasoning_text(r: &mut ReasoningItem, text: &str) {
    match r.content.as_mut().and_then(|parts| parts.first_mut()) {
        Some(ReasoningItemContent::ReasoningText(t)) => t.text = text.to_string(),
        _ => {
            r.content = Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent {
                    text: text.to_string(),
                },
            )]);
        }
    }
}

pub(crate) fn mark_items_incomplete(items: &mut [Item]) {
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
        ResponseStreamEvent::ResponseOutputItemDone(e) => e.sequence_number,
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(e) => e.sequence_number,
        ResponseStreamEvent::ResponseFunctionCallArgumentsDone(e) => e.sequence_number,
        _ => 0,
    }
}

/// Enforce that one provider item id denotes one item in the terminal payload.
///
/// The whole session log is built on that: an item that arrives twice under the
/// same id would either shadow or duplicate a row downstream. The provider is
/// the only place that can get this wrong, so it fails loudly here rather than
/// being de-duplicated silently and surfacing as two rows later.
fn assert_unique_item_ids(items: &[Item]) -> Result<()> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for item in items {
        let Some(id) = item_id_of(item) else {
            continue;
        };
        *seen.entry(id).or_insert(0) += 1;
    }
    if let Some((id, count)) = seen.into_iter().find(|(_, n)| *n > 1) {
        return Err(LitecodeError::Llm(format!(
            "stream terminal carries item id `{id}` {count} times; one provider item must appear once"
        )));
    }
    Ok(())
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
            let items = output_items_to_items(ev.response.output);
            assert_unique_item_ids(&items)?;
            Ok(Some(items))
        }
        ResponseStreamEvent::ResponseFailed(ev) => Err(LitecodeError::Llm(terminal_error_message(
            "response.failed",
            &ev.response,
        ))),
        ResponseStreamEvent::ResponseIncomplete(ev) => {
            let mut items = output_items_to_items(ev.response.output);
            assert_unique_item_ids(&items)?;
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
    fn empty_item_id_delta_does_not_synthesize_added() {
        let mut gate = StreamContractGate::new();
        let out = collect_forward(&mut gate, vec![delta("", 1, "{}")]);
        assert!(
            !out.iter()
                .any(|e| matches!(e, ResponseStreamEvent::ResponseOutputItemAdded(_))),
            "empty item_id must not synthesize a function_call added, got {out:?}"
        );
        assert_eq!(out.len(), 1);
        assert!(is_delta(&out[0]));
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

    /// Upstream streams one reasoning summary as several parts. Each part owns a
    /// `summary_index` slot, and a part finishing must not disturb the others —
    /// the interrupted stream has to yield exactly what `response.completed`
    /// would have carried.
    #[test]
    fn reasoning_summary_parts_accumulate_by_index_without_dropping_earlier_parts() {
        let mut acc = StreamItemAccumulator::new();
        for ev in [
            json_event(
                "response.reasoning_summary_part.added",
                serde_json::json!({"sequence_number": 1, "item_id": "rs_1", "output_index": 0, "summary_index": 0, "part": {"type": "summary_text", "text": ""}}),
            ),
            json_event(
                "response.reasoning_summary_text.delta",
                serde_json::json!({"sequence_number": 2, "item_id": "rs_1", "output_index": 0, "summary_index": 0, "delta": "first "}),
            ),
            json_event(
                "response.reasoning_summary_text.delta",
                serde_json::json!({"sequence_number": 3, "item_id": "rs_1", "output_index": 0, "summary_index": 0, "delta": "part"}),
            ),
            json_event(
                "response.reasoning_summary_text.done",
                serde_json::json!({"sequence_number": 4, "item_id": "rs_1", "output_index": 0, "summary_index": 0, "text": "first part"}),
            ),
            json_event(
                "response.reasoning_summary_part.added",
                serde_json::json!({"sequence_number": 5, "item_id": "rs_1", "output_index": 0, "summary_index": 1, "part": {"type": "summary_text", "text": ""}}),
            ),
            json_event(
                "response.reasoning_summary_text.delta",
                serde_json::json!({"sequence_number": 6, "item_id": "rs_1", "output_index": 0, "summary_index": 1, "delta": "second part"}),
            ),
            json_event(
                "response.reasoning_summary_part.done",
                serde_json::json!({"sequence_number": 7, "item_id": "rs_1", "output_index": 0, "summary_index": 1, "part": {"type": "summary_text", "text": "second part"}}),
            ),
        ] {
            acc.observe(&ev);
        }
        let sealed = acc.seal_incomplete();
        assert_eq!(sealed.len(), 1, "one reasoning item, not one per part");
        match &sealed[0] {
            Item::Reasoning(r) => {
                assert_eq!(summary_texts(r), vec!["first part", "second part"]);
            }
            other => panic!("expected reasoning, got {other:?}"),
        }
    }

    #[test]
    fn summary_text_done_lands_on_its_own_index() {
        let mut acc = StreamItemAccumulator::new();
        for ev in [
            json_event(
                "response.reasoning_summary_text.delta",
                serde_json::json!({"sequence_number": 1, "item_id": "rs_2", "output_index": 0, "summary_index": 0, "delta": "zero"}),
            ),
            json_event(
                "response.reasoning_summary_text.done",
                serde_json::json!({"sequence_number": 2, "item_id": "rs_2", "output_index": 0, "summary_index": 1, "text": "one"}),
            ),
        ] {
            acc.observe(&ev);
        }
        let sealed = acc.seal_incomplete();
        match &sealed[0] {
            Item::Reasoning(r) => assert_eq!(summary_texts(r), vec!["zero", "one"]),
            other => panic!("expected reasoning, got {other:?}"),
        }
    }

    #[test]
    fn output_item_done_replaces_the_streamed_shell() {
        let mut acc = StreamItemAccumulator::new();
        acc.observe(&json_event(
            "response.reasoning_summary_text.delta",
            serde_json::json!({"sequence_number": 1, "item_id": "rs_3", "output_index": 0, "summary_index": 0, "delta": "partial"}),
        ));
        acc.observe(&json_event(
            "response.output_item.done",
            serde_json::json!({
                "sequence_number": 2,
                "output_index": 0,
                "item": {
                    "type": "reasoning",
                    "id": "rs_3",
                    "summary": [{"type": "summary_text", "text": "the whole thing"}],
                    "encrypted_content": "enc",
                }
            }),
        ));
        let sealed = acc.seal_incomplete();
        assert_eq!(sealed.len(), 1);
        match &sealed[0] {
            Item::Reasoning(r) => {
                assert_eq!(summary_texts(r), vec!["the whole thing"]);
                assert_eq!(r.encrypted_content.as_deref(), Some("enc"));
            }
            other => panic!("expected reasoning, got {other:?}"),
        }
    }

    #[test]
    fn output_text_done_replaces_streamed_text() {
        let mut acc = StreamItemAccumulator::new();
        acc.observe(&json_event(
            "response.output_text.delta",
            serde_json::json!({"sequence_number": 1, "item_id": "msg_9", "output_index": 0, "content_index": 0, "delta": "half"}),
        ));
        acc.observe(&json_event(
            "response.output_text.done",
            serde_json::json!({"sequence_number": 2, "item_id": "msg_9", "output_index": 0, "content_index": 0, "text": "full text"}),
        ));
        let sealed = acc.seal_incomplete();
        assert_eq!(crate::types::item_text_preview(&sealed[0]), "full text");
    }

    /// One provider item id denotes one item: a terminal that carries the same id
    /// twice is an upstream contract violation and must not be silently merged
    /// downstream (that is what produced two transcript rows).
    #[test]
    fn duplicate_item_id_in_completed_output_is_refused() {
        let mut gate = StreamContractGate::new();
        let mut acc = StreamItemAccumulator::new();
        let completed: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.completed",
            "sequence_number": 9,
            "response": {
                "id": "resp_dup",
                "object": "response",
                "created_at": 1,
                "model": "gpt-4o",
                "status": "completed",
                "output": [
                    {"type": "reasoning", "id": "rs_dup", "summary": []},
                    {"type": "reasoning", "id": "rs_dup", "summary": [{"type": "summary_text", "text": "x"}]}
                ]
            }
        }))
        .expect("completed event");
        let err = forward_stream_event(&mut gate, &mut acc, completed, &mut None)
            .expect_err("duplicate ids are refused");
        assert!(
            err.to_string().contains("rs_dup"),
            "error should name the offending id, got {err}"
        );
    }

    fn json_event(event_type: &str, mut fields: serde_json::Value) -> ResponseStreamEvent {
        let map = fields.as_object_mut().expect("object");
        map.insert("type".into(), serde_json::Value::String(event_type.into()));
        serde_json::from_value(fields).expect("stream event")
    }

    fn fixture_events(name: &str) -> Vec<ResponseStreamEvent> {
        let raw = match name {
            "reasoning_summary_parts" => {
                include_str!("../../../tests/fixtures/sse/responses/reasoning_summary_parts.txt")
            }
            other => panic!("unknown fixture {other}"),
        };
        raw.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|json| serde_json::from_str(json).expect("fixture event"))
            .collect()
    }

    /// The real shape of a GPT-family response: every item announces itself, streams
    /// its content, and then arrives a second time in the terminal payload with a
    /// **different** `encrypted_content`. Normalizing it must yield each item once,
    /// in the order the model produced them, with the terminal copy as the value.
    #[test]
    fn real_stream_shape_normalizes_to_one_item_per_id_in_order() {
        let mut gate = StreamContractGate::new();
        let mut acc = StreamItemAccumulator::new();
        let mut terminal: Option<Vec<Item>> = None;
        for ev in fixture_events("reasoning_summary_parts") {
            if let Some(items) = forward_stream_event(&mut gate, &mut acc, ev, &mut None).unwrap() {
                terminal = Some(items);
            }
        }
        let items = terminal.expect("the fixture ends with response.completed");
        assert_eq!(items.len(), 13, "12 reasoning items and one message");

        // Order is the model's, not the payload's: reasoning first, text last.
        let kinds: Vec<&str> = items
            .iter()
            .map(|item| match item {
                Item::Reasoning(_) => "reasoning",
                Item::Message(_) => "message",
                _ => "other",
            })
            .collect();
        assert_eq!(&kinds[..12], &["reasoning"; 12]);
        assert_eq!(kinds[12], "message");

        // Every id once: the streamed copies were replaced, not appended to.
        let ids: Vec<String> = items.iter().filter_map(item_id_of).collect();
        assert_eq!(ids.len(), 13);
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 13, "ids: {ids:?}");

        // Multi-part summaries survive whole, and the terminal copy is the one kept.
        let parts = |idx: usize| -> Vec<String> {
            match &items[idx] {
                Item::Reasoning(r) => r
                    .summary
                    .iter()
                    .map(|p| match p {
                        SummaryPart::SummaryText(t) => t.text.clone(),
                    })
                    .collect(),
                other => panic!("expected reasoning at {idx}, got {other:?}"),
            }
        };
        assert_eq!(
            parts(0).len(),
            3,
            "first reasoning item streams three parts"
        );
        assert_eq!(parts(1).len(), 2);
        assert_eq!(parts(7).len(), 2);
        for i in [2, 3, 4, 5, 6, 8, 9, 10, 11] {
            assert!(parts(i).is_empty(), "item {i} carries no summary text");
        }
        let enc = |idx: usize| -> Option<String> {
            match &items[idx] {
                Item::Reasoning(r) => r.encrypted_content.clone(),
                other => panic!("expected reasoning at {idx}, got {other:?}"),
            }
        };
        assert_eq!(
            enc(0).as_deref(),
            Some("TERMINAL0"),
            "the terminal copy is authoritative, not the streamed one"
        );
    }

    fn summary_texts(r: &ReasoningItem) -> Vec<&str> {
        r.summary
            .iter()
            .map(|part| match part {
                SummaryPart::SummaryText(t) => t.text.as_str(),
            })
            .collect()
    }
}
