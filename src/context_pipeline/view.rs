use crate::types::{Item, Transcript};

/// Turn-local hot ring view — working Items for the agent loop.
#[derive(Debug, Clone, Default)]
pub struct HotView {
    items: Transcript,
}

impl HotView {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(&mut self, items: Transcript) {
        self.items = items;
    }

    /// Read-only view of turn-local working items.
    pub fn as_slice(&self) -> &[Item] {
        &self.items
    }

    pub fn items(&self) -> &[Item] {
        self.as_slice()
    }

    pub fn as_mut_vec(&mut self) -> &mut Transcript {
        &mut self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Prepared transcript view for a single agent step.
///
/// `items` is the ephemeral LLM view (capability-projected, unanswered calls
/// padded). The persisted working set stays in the turn transcript.
#[derive(Debug, Clone)]
pub struct PreparedView {
    pub items: Transcript,
    pub token_count: usize,
    /// Optional system / instructions string for this step (set by prepare or caller).
    pub instructions: Option<String>,
}
