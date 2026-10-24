//! Records each model request so a test can read the tool result the next
//! turn actually sent.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use litecode::authority::responses::{
    MessageItem, OutputItem, OutputStatus, ResponseOutputItemAddedEvent, ResponseStreamEvent,
};
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::types::{Item, Result, StreamEvents};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct CapturedLlmRequest {
    pub model: String,
    pub instructions: String,
    pub tools: Vec<serde_json::Value>,
    pub input: Vec<serde_json::Value>,
    pub max_output_tokens: u32,
    pub thinking: litecode::platform_knobs::ThinkingSpec,
    pub purpose: litecode::llm::RequestPurpose,
    /// Full product request, including typed Items and ToolDefs.
    pub request: ModelRequest,
}

fn item_json(item: &Item) -> serde_json::Value {
    serde_json::to_value(item).expect("serialize Item")
}

fn items_json(items: &[Item]) -> Vec<serde_json::Value> {
    items.iter().map(item_json).collect()
}

pub fn capture_request(request: &ModelRequest) -> CapturedLlmRequest {
    CapturedLlmRequest {
        model: request.model.clone(),
        instructions: request.instructions.clone(),
        tools: request
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect(),
        input: items_json(&request.input),
        max_output_tokens: request.max_output_tokens,
        thinking: request.thinking,
        purpose: request.purpose,
        request: request.clone(),
    }
}

/// Queued Items, plus `output_item.added` (InProgress) so `call_model` hits
/// `sessions.persist_item` before the completed Items are returned.
#[derive(Clone)]
pub struct RecordingProvider {
    responses: Arc<Mutex<Vec<Vec<Item>>>>,
    index: Arc<AtomicUsize>,
    captured: Arc<Mutex<Vec<CapturedLlmRequest>>>,
    accepting_compaction_summary: bool,
}

impl RecordingProvider {
    pub fn new(responses: Vec<Vec<Item>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            index: Arc::new(AtomicUsize::new(0)),
            captured: Arc::new(Mutex::new(Vec::new())),
            accepting_compaction_summary: false,
        }
    }

    /// Legacy request-shape fixtures only. Do not use for PR-5 acceptance or
    /// finish assertions; queue explicit summary items instead.
    pub fn new_legacy_accepting_compaction_summary(responses: Vec<Vec<Item>>) -> Self {
        let mut provider = Self::new(responses);
        provider.accepting_compaction_summary = true;
        provider
    }

    pub fn captured(&self) -> Vec<CapturedLlmRequest> {
        self.captured.lock().expect("captured").clone()
    }

    fn next_items(&self) -> Result<Vec<Item>> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        let guard = self.responses.lock().expect("responses");
        guard.get(idx).cloned().ok_or_else(|| {
            litecode::types::LitecodeError::Llm(format!(
                "RecordingProvider: no response queued at index {idx}"
            ))
        })
    }
}

fn emit_in_progress_added(
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + '_>>,
    item: &Item,
    output_index: u32,
) {
    let Some(cb) = on_event else {
        return;
    };
    let output = match item {
        Item::FunctionCall(fc) => {
            let mut fc = fc.clone();
            fc.arguments.clear();
            fc.status = Some(OutputStatus::InProgress);
            OutputItem::FunctionCall(fc)
        }
        Item::Message(MessageItem::Output(msg)) => {
            let mut msg = msg.clone();
            msg.status = OutputStatus::InProgress;
            OutputItem::Message(msg)
        }
        Item::Reasoning(reasoning) => {
            let mut reasoning = reasoning.clone();
            reasoning.status = Some(OutputStatus::InProgress);
            OutputItem::Reasoning(reasoning)
        }
        _ => return,
    };
    cb(ResponseStreamEvent::ResponseOutputItemAdded(
        ResponseOutputItemAddedEvent {
            sequence_number: u64::from(output_index) + 1,
            output_index,
            item: output,
        },
    ));
}

impl LlmProvider for RecordingProvider {
    fn endpoint(&self) -> &str {
        "recording://turn-prefix"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.captured
            .lock()
            .expect("captured")
            .push(capture_request(request));
        let items = self.next_items();
        let accepting = self.accepting_compaction_summary;
        Box::pin(async move {
            let items = items?;
            let items = if accepting {
                super::legacy_accepting_compaction_items(request, items)
            } else {
                items
            };
            for (i, item) in items.iter().enumerate() {
                emit_in_progress_added(&mut on_event, item, i as u32);
            }
            super::emit_completed_terminal(&mut on_event);
            Ok(items)
        })
    }
}
