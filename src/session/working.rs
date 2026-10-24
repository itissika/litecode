//! Transitional GateRow implementation.
//!
//! Identity for the write gate is `log_seq`; the payload carries either a
//! durable Item or a role-free compaction checkpoint. The checkpoint's model
//! role is created only by the projector in this module.

use crate::authority::responses::{InputRole, MessageItem};
use crate::config::global_db::CHECKPOINT_PREAMBLE;
use crate::types::{Item, user_text};

use super::event::Seq;
use super::model::{CompactedBody, SessionKind};

pub const SUMMARY_OPEN_TAG: &str = "<compacted-summary>";
pub const SUMMARY_CLOSE_TAG: &str = "</compacted-summary>";
const CONVERSATION_SUMMARY_PREFIX: &str = "[Conversation summary]";
const AGGRESSIVE_SUMMARY_PREFIX: &str = "[Aggressive summary]";

/// Role-free durable checkpoint body used by the working set and projector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionCheckpoint {
    pub summary: String,
    pub from: Seq,
    pub to: Seq,
}

impl CompactionCheckpoint {
    /// The one model projection of a checkpoint. The durable summary stays raw;
    /// this framed user message is view-only and must never be persisted back.
    pub fn model_item(&self) -> Item {
        let body = strip_legacy_summary_prefix(&self.summary);
        user_text(format!(
            "{CHECKPOINT_PREAMBLE}\n\n{SUMMARY_OPEN_TAG}{body}{SUMMARY_CLOSE_TAG}"
        ))
    }

    /// The raw summary shown to UI/reader consumers that need the body rather
    /// than the model-facing framing.
    pub fn summary_text(&self) -> &str {
        &self.summary
    }
}

impl From<CompactedBody> for CompactionCheckpoint {
    fn from(body: CompactedBody) -> Self {
        Self {
            summary: body.summary,
            from: body.from,
            to: body.to,
        }
    }
}

impl From<CompactionCheckpoint> for CompactedBody {
    fn from(checkpoint: CompactionCheckpoint) -> Self {
        Self {
            summary: checkpoint.summary,
            from: checkpoint.from,
            to: checkpoint.to,
        }
    }
}

/// Legacy labeled summaries are stripped only at the projection boundary.
/// Identity is always the checkpoint payload, never this text prefix.
pub fn strip_legacy_summary_prefix(text: &str) -> &str {
    for prefix in [CONVERSATION_SUMMARY_PREFIX, AGGRESSIVE_SUMMARY_PREFIX] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.trim_start_matches(|c| c == '\n' || c == '\r');
        }
    }
    text
}

/// One durable working-set payload. Checkpoints are explicit and are never
/// inferred from an Item's appearance.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkingPayload {
    Item { kind: SessionKind, item: Item },
    Checkpoint(CompactionCheckpoint),
}

impl WorkingPayload {
    pub fn item(item: Item) -> Self {
        Self::item_with_kind(kind_for_item(&item), item)
    }

    pub fn item_with_kind(kind: SessionKind, item: Item) -> Self {
        debug_assert!(!matches!(kind, SessionKind::Compacted));
        Self::Item { kind, item }
    }

    pub fn as_item(&self) -> Option<&Item> {
        match self {
            Self::Item { item, .. } => Some(item),
            Self::Checkpoint(_) => None,
        }
    }

    pub fn checkpoint(&self) -> Option<&CompactionCheckpoint> {
        match self {
            Self::Checkpoint(checkpoint) => Some(checkpoint),
            Self::Item { .. } => None,
        }
    }

    pub fn kind(&self) -> SessionKind {
        match self {
            Self::Item { kind, .. } => kind.clone(),
            Self::Checkpoint(_) => SessionKind::Compacted,
        }
    }

    /// The model-visible projection. Item payloads clone verbatim; checkpoint
    /// payloads flow through the unique checkpoint projector.
    pub fn model_item(&self) -> Item {
        match self {
            Self::Item { item, .. } => item.clone(),
            Self::Checkpoint(checkpoint) => checkpoint.model_item(),
        }
    }
}

/// One model-visible row in the persist working set.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkingRow {
    /// `None` means this row has not crossed the write gate.
    pub log_seq: Option<Seq>,
    /// Explicit payload identity; never inferred from serialized Item JSON.
    pub payload: WorkingPayload,
    /// This surface node is a persisted mechanical-prune replacement whose
    /// shadowed source carried media. Rebuilt from `source_seqs` on every cold
    /// load; the projection uses it to preserve media-rebase signatures.
    pub replacement_media: bool,
}

impl WorkingRow {
    pub fn pending(item: Item) -> Self {
        Self {
            log_seq: None,
            payload: WorkingPayload::item(item),
            replacement_media: false,
        }
    }

    pub fn pending_with_kind(kind: SessionKind, item: Item) -> Self {
        Self {
            log_seq: None,
            payload: WorkingPayload::item_with_kind(kind, item),
            replacement_media: false,
        }
    }

    pub fn persisted(seq: Seq, item: Item) -> Self {
        Self {
            log_seq: Some(seq),
            payload: WorkingPayload::item(item),
            replacement_media: false,
        }
    }

    pub fn persisted_with_kind(seq: Seq, kind: SessionKind, item: Item) -> Self {
        Self {
            log_seq: Some(seq),
            payload: WorkingPayload::item_with_kind(kind, item),
            replacement_media: false,
        }
    }

    /// A persisted replacement row loaded from the append-only log. The
    /// `replacement_media` flag is derived from its `source_seqs` at load time,
    /// never stored as a new Item/role attribute.
    pub fn persisted_replacement(seq: Seq, item: Item, replacement_media: bool) -> Self {
        Self {
            log_seq: Some(seq),
            payload: WorkingPayload::item(item),
            replacement_media,
        }
    }

    pub fn pending_checkpoint(checkpoint: CompactionCheckpoint) -> Self {
        Self {
            log_seq: None,
            payload: WorkingPayload::Checkpoint(checkpoint),
            replacement_media: false,
        }
    }

    pub fn persisted_checkpoint(seq: Seq, checkpoint: CompactionCheckpoint) -> Self {
        Self {
            log_seq: Some(seq),
            payload: WorkingPayload::Checkpoint(checkpoint),
            replacement_media: false,
        }
    }

    pub fn item(&self) -> Option<&Item> {
        self.payload.as_item()
    }

    pub fn checkpoint(&self) -> Option<&CompactionCheckpoint> {
        self.payload.checkpoint()
    }

    pub fn kind(&self) -> SessionKind {
        self.payload.kind()
    }

    /// Replace the payload and keep `kind` in step with it. `log_seq` stays.
    /// Checkpoints are never replaced by this item-oriented setter.
    pub fn replace_item(&mut self, item: Item) {
        self.payload = WorkingPayload::item(item);
    }
}

pub type GateRow = WorkingRow;

/// Item-only kind inference for freshly produced item payloads. It can never
/// produce `Compacted`; checkpoint identity only enters through the explicit
/// `Checkpoint` payload.
fn kind_for_item(item: &Item) -> SessionKind {
    match item {
        Item::FunctionCall(_) => SessionKind::ItemToolCall,
        Item::FunctionCallOutput(_) => SessionKind::ItemToolResult,
        Item::Message(MessageItem::Input(input)) if matches!(input.role, InputRole::User) => {
            SessionKind::ItemUser
        }
        _ => SessionKind::ItemAssistant,
    }
}

/// Model-visible view of a working set whose first row is a checkpoint.
///
/// `items` includes the projected user checkpoint followed by the kept tail.
/// `inherited_before_checkpoint` is parallel to the pre-padding items and is
/// true only for persisted rows older than the checkpoint seq.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactedView {
    pub checkpoint_seq: Seq,
    pub items: Vec<Item>,
    pub item_seqs: Vec<Option<Seq>>,
    pub inherited_before_checkpoint: Vec<bool>,
    /// Parallel to `items`: this row is a persisted media replacement, so the
    /// model projection must keep the media-rebase boundary even though the
    /// current row no longer contains media.
    pub replacement_media: Vec<bool>,
}

impl CompactedView {
    pub fn from_working_rows(rows: &[WorkingRow]) -> Option<Self> {
        let first = rows.first()?;
        if first.checkpoint().is_none() {
            return None;
        }
        let checkpoint_seq = first.log_seq?;
        debug_assert!(
            rows.iter().skip(1).all(|row| row.checkpoint().is_none()),
            "a working-set surface carries at most one checkpoint"
        );
        let mut items = Vec::with_capacity(rows.len());
        let mut item_seqs = Vec::with_capacity(rows.len());
        let mut replacement_media = Vec::with_capacity(rows.len());
        for row in rows {
            items.push(row.payload.model_item());
            item_seqs.push(row.log_seq);
            replacement_media.push(row.replacement_media);
        }
        let inherited_before_checkpoint = crate::context_pipeline::summary::minted_before_summary(
            &item_seqs,
            Some(checkpoint_seq),
        );
        Some(Self {
            checkpoint_seq,
            items,
            item_seqs,
            inherited_before_checkpoint,
            replacement_media,
        })
    }

    pub fn model_items(&self) -> &[Item] {
        &self.items
    }
}

/// Project payload items for the agent loop and LLM view.
pub fn project_items(rows: &[WorkingRow]) -> Vec<Item> {
    rows.iter().map(|row| row.payload.model_item()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkpoint(summary: &str) -> CompactionCheckpoint {
        CompactionCheckpoint {
            summary: summary.into(),
            from: 2,
            to: 8,
        }
    }

    #[test]
    fn checkpoint_projects_as_the_unique_user_message() {
        let item = checkpoint("## Primary Request and Intent\n- ship PR-2").model_item();
        let Item::Message(MessageItem::Input(message)) = &item else {
            panic!("checkpoint must project as an input message: {item:?}");
        };
        assert_eq!(message.role, InputRole::User);
        let text = crate::types::item_text_preview(&item);
        assert!(text.starts_with(CHECKPOINT_PREAMBLE));
        assert_eq!(text.matches(SUMMARY_OPEN_TAG).count(), 1, "{text}");
        assert_eq!(text.matches(SUMMARY_CLOSE_TAG).count(), 1, "{text}");
        assert!(text.contains("## Primary Request and Intent"));
    }

    #[test]
    fn compacted_payload_is_explicit_and_never_item_inferred() {
        let row = WorkingRow::persisted_checkpoint(9, checkpoint("raw summary"));
        assert_eq!(row.kind(), SessionKind::Compacted);
        assert!(row.checkpoint().is_some());
        assert!(row.item().is_none());

        let legacy_prefix = WorkingRow::persisted(
            1,
            crate::types::assistant_text("[Conversation summary]\nnot mine"),
        );
        assert_eq!(legacy_prefix.kind(), SessionKind::ItemAssistant);
        assert!(legacy_prefix.checkpoint().is_none());
        assert!(CompactedView::from_working_rows(&[legacy_prefix]).is_none());
    }

    #[test]
    fn compacted_view_marks_only_rows_older_than_the_checkpoint() {
        let rows = vec![
            WorkingRow::persisted_checkpoint(10, checkpoint("prior")),
            WorkingRow::persisted(5, crate::types::user_text("old kept")),
            WorkingRow::persisted(11, crate::types::user_text("new")),
            WorkingRow::pending(crate::types::user_text("pending")),
        ];
        let view = CompactedView::from_working_rows(&rows).expect("checkpoint view");
        assert_eq!(view.checkpoint_seq, 10);
        assert_eq!(view.items.len(), 4);
        assert_eq!(view.item_seqs, vec![Some(10), Some(5), Some(11), None]);
        assert_eq!(
            view.inherited_before_checkpoint,
            vec![false, true, false, false]
        );
    }

    #[test]
    fn legacy_label_is_stripped_only_inside_a_real_checkpoint() {
        let projected = checkpoint("[Aggressive summary]\n\nprior facts").model_item();
        let text = crate::types::item_text_preview(&projected);
        assert!(!text.contains(CONVERSATION_SUMMARY_PREFIX));
        assert!(!text.contains(AGGRESSIVE_SUMMARY_PREFIX));
        assert!(text.ends_with("prior facts</compacted-summary>"));
    }
}
