//! Scripted [`LlmProvider`] for integration tests — returns queued authority Items.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use litecode::llm::LlmProvider;
use litecode::llm::ModelRequest;
use litecode::types::{Item, LitecodeError, Result, StreamEvents};

use litecode::authority::responses::{
    AssistantRole, MessageItem, OutputMessage, OutputMessageContent, OutputStatus,
    OutputTextContent,
};

/// Pops one `Vec<Item>` per `complete_with_stream_events` call.
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
        Self::with_texts(&[text])
    }

    pub fn with_texts(texts: &[&str]) -> Self {
        Self::with_responses(
            texts
                .iter()
                .enumerate()
                .map(|(i, text)| vec![assistant_text(text, i)])
                .collect(),
        )
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

fn assistant_text(text: &str, idx: usize) -> Item {
    Item::Message(MessageItem::Output(OutputMessage {
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: text.into(),
            annotations: vec![],
            logprobs: None,
        })],
        id: format!("msg_scripted_{idx}"),
        role: AssistantRole::Assistant,
        phase: None,
        status: OutputStatus::Completed,
    }))
}

/// Never completes until the future is dropped (or `cancel` fires when `ignore_cancel` is false).
#[derive(Clone)]
pub struct HangProvider {
    pub started: Arc<AtomicBool>,
    pub dropped: Arc<AtomicBool>,
    ignore_cancel: bool,
}

impl HangProvider {
    pub fn ignore_cancel() -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicBool::new(false)),
            ignore_cancel: true,
        }
    }

    pub fn until_cancel() -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            dropped: Arc::new(AtomicBool::new(false)),
            ignore_cancel: false,
        }
    }
}

struct DropNotify(Arc<AtomicBool>);

impl Drop for DropNotify {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl LlmProvider for HangProvider {
    fn endpoint(&self) -> &str {
        "scripted://hang"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        _request: &'a ModelRequest,
        _api_key: &'a str,
        _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a tokio_util::sync::CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.started.store(true, Ordering::SeqCst);
        let dropped = Arc::clone(&self.dropped);
        let ignore_cancel = self.ignore_cancel;
        let cancel = cancel.clone();
        Box::pin(async move {
            let _notify = DropNotify(dropped);
            if ignore_cancel {
                std::future::pending::<()>().await;
                unreachable!("HangProvider pending resolved");
            }
            cancel.cancelled().await;
            Err(LitecodeError::Canceled)
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
