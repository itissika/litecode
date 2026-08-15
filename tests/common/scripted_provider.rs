//! Scripted [`LlmProvider`] for integration tests — returns queued authority Items.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use litecode::llm::LlmProvider;
use litecode::llm::ModelRequest;
use litecode::types::{Item, Result, StreamEvents};

/// Pops one `Vec<Item>` per `complete` / `complete_with_stream_events` call.
#[derive(Clone)]
pub struct ScriptedProvider {
    responses: Arc<Mutex<Vec<Vec<Item>>>>,
    index: Arc<AtomicUsize>,
}

impl ScriptedProvider {
    pub fn with_responses(responses: Vec<Vec<Item>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_text(text: &str) -> Self {
        use litecode::authority::responses::{
            AssistantRole, MessageItem, OutputMessage, OutputMessageContent, OutputStatus,
            OutputTextContent,
        };
        let item = Item::Message(MessageItem::Output(OutputMessage {
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: text.into(),
                annotations: vec![],
                logprobs: None,
            })],
            id: "msg_scripted_1".into(),
            role: AssistantRole::Assistant,
            phase: None,
            status: OutputStatus::Completed,
        }));
        Self::with_responses(vec![vec![item]])
    }

    fn next_items(&self) -> Result<Vec<Item>> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        let guard = self.responses.lock().unwrap();
        guard.get(idx).cloned().ok_or_else(|| {
            litecode::types::LitecodeError::Llm(format!(
                "ScriptedProvider: no response queued at index {idx}"
            ))
        })
    }
}

impl LlmProvider for ScriptedProvider {
    fn endpoint(&self) -> &str {
        "scripted://test"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete<'a>(
        &'a self,
        _request: &'a ModelRequest,
        _api_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        let items = self.next_items();
        Box::pin(async move { items })
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        _request: &'a ModelRequest,
        _api_key: &'a str,
        _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a tokio_util::sync::CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        let items = self.next_items();
        Box::pin(async move { items })
    }
}
