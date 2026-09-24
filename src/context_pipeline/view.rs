use crate::types::{Item, Transcript};

/// Derived model-visible Items for this turn. Not a second persisted working set.
#[derive(Debug, Clone, Default)]
pub struct HotView {
    model_items: Transcript,
}

impl HotView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(&mut self, items: Transcript) {
        self.model_items = items;
    }

    /// Derived `Item[]` for the agent loop (`derive_messages`), not persist/FE truth.
    pub fn model_items(&self) -> &[Item] {
        &self.model_items
    }

    pub fn len(&self) -> usize {
        self.model_items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.model_items.is_empty()
    }
}

/// Prepared transcript view for a single agent step.
///
/// `items` is the ephemeral LLM view (capability-projected, unanswered calls
/// padded). The persisted working set stays in the turn transcript.
#[derive(Debug, Clone)]
pub struct PreparedView {
    pub items: Transcript,
    /// Source Session seq per item, parallel to `items`.
    ///
    /// A `None` entry is a host-synthesized view-only item (e.g. an unanswered
    /// tool-call pad). The mapping is carried positionally; provider IDs are not
    /// used to recover it because providers may reuse IDs across requests.
    pub item_seqs: Vec<Option<crate::session::event::Seq>>,
    pub token_count: usize,
    /// Optional system / instructions string for this step (set by prepare or caller).
    pub instructions: Option<String>,
}
