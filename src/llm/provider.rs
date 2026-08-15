use std::future::Future;
use std::pin::Pin;

use tokio_util::sync::CancellationToken;

use crate::types::{Item, Result, StreamEvents};

use super::request::ModelRequest;

/// Product-facing LLM provider — dialect stays inside `llm::adapter`.
pub trait LlmProvider: Send + Sync {
    fn endpoint(&self) -> &str;
    fn box_clone(&self) -> Box<dyn LlmProvider>;
    fn clone_for_isolated_runtime(&self) -> Box<dyn LlmProvider> {
        self.box_clone()
    }

    /// Explicit non-stream convenience / degradation path (`stream: false` where applicable).
    /// Runtime does **not** prefer this; use [`Self::complete_with_stream_events`].
    fn complete<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>>;

    /// Preferred runtime path: emit authority [`StreamEvents`] while completing.
    ///
    /// Every implementor must provide this explicitly. Silently discarding `on_event`
    /// (or defaulting to [`Self::complete`] while ignoring the callback) is forbidden.
    ///
    /// Final Items come from a **terminal** Responses payload:
    /// `response.completed` or `response.incomplete` (`response.output`).
    /// User abort is a terminal: stop reading SSE and seal opened Items as
    /// `incomplete`. Do not rebuild a parallel dialect from raw deltas; seal the
    /// same `item_id`s already opened via `output_item.added` / live deltas.
    /// `response.failed` / `error` remain errors.
    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>>;
}
