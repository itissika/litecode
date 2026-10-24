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
    accepting_compaction_summary: bool,
}

impl ScriptedProvider {
    pub fn with_responses(responses: Vec<Vec<Item>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            index: Arc::new(AtomicUsize::new(0)),
            accepting_compaction_summary: false,
        }
    }

    /// Legacy request-shape fixtures only. Do not use for PR-5 acceptance or
    /// finish assertions; queue explicit summary text/finish fixtures instead.
    pub fn with_texts_legacy_accepting_compaction_summary(texts: &[&str]) -> Self {
        let mut provider = Self::with_texts(texts);
        provider.accepting_compaction_summary = true;
        provider
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

/// A provider that returns an accepted DSH-shaped summary for the exact
/// compaction prefix it receives. Useful for tests that need a successful
/// checkpoint without restating the recent-user anchor in every fixture.
#[derive(Clone)]
pub struct DshSummaryProvider {
    pub calls: Arc<AtomicUsize>,
}

impl DshSummaryProvider {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Default for DshSummaryProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for DshSummaryProvider {
    fn endpoint(&self) -> &str {
        "scripted://dsh-summary"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        _api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a tokio_util::sync::CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        super::emit_completed_terminal(&mut on_event);
        let text = super::valid_summary_for_request(request);
        Box::pin(async move { Ok(vec![assistant_text(&text, 0)]) })
    }
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
        request: &'a ModelRequest,
        _api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a tokio_util::sync::CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        super::emit_completed_terminal(&mut on_event);
        let items = self.next_items().map(|items| {
            if self.accepting_compaction_summary {
                super::legacy_accepting_compaction_items(request, items)
            } else {
                items
            }
        });
        Box::pin(async move { items })
    }
}
