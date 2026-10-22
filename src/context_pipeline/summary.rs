use crate::session::event::Seq;
use crate::types::{Item, assistant_text, item_text_preview};

pub const CONVERSATION_SUMMARY_PREFIX: &str = "[Conversation summary]";
pub const AGGRESSIVE_SUMMARY_PREFIX: &str = "[Aggressive summary]";

pub fn format_compact_summary(text: &str, aggressive: bool) -> String {
    let label = if aggressive {
        AGGRESSIVE_SUMMARY_PREFIX
    } else {
        CONVERSATION_SUMMARY_PREFIX
    };
    format!("{label}\n{text}")
}

pub fn compact_summary_message(text: &str, aggressive: bool) -> Item {
    assistant_text(format_compact_summary(text, aggressive))
}

/// True when this item is a prior compaction summary (labeled assistant message).
pub fn is_compact_summary_item(item: &Item) -> bool {
    let text = item_text_preview(item);
    text.starts_with(CONVERSATION_SUMMARY_PREFIX) || text.starts_with(AGGRESSIVE_SUMMARY_PREFIX)
}

/// Per item: persisted before the leading compaction summary, so it was
/// produced on the history that summary replaced. The summary row's seq is the
/// compaction itself; kept rows are older and later rows are newer. Without a
/// seq on the summary, every item counts as rebased.
pub fn minted_before_summary(items: &[Item], seqs: &[Option<Seq>]) -> Vec<bool> {
    if !items.first().is_some_and(is_compact_summary_item) {
        return vec![false; items.len()];
    }
    let Some(compacted) = seqs.first().copied().flatten() else {
        return vec![true; items.len()];
    };
    (0..items.len())
        .map(|index| {
            seqs.get(index)
                .copied()
                .flatten()
                .is_some_and(|seq| seq < compacted)
        })
        .collect()
}

/// Strip the summary label for UPDATE prompts.
pub fn summary_body_text(item: &Item) -> String {
    let text = item_text_preview(item);
    for prefix in [CONVERSATION_SUMMARY_PREFIX, AGGRESSIVE_SUMMARY_PREFIX] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.trim_start_matches('\n').to_string();
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_summary_is_assistant_message_not_user() {
        use crate::authority::responses::{AssistantRole, MessageItem};
        let item = compact_summary_message("prose", false);
        match &item {
            Item::Message(MessageItem::Output(out)) => {
                assert_eq!(out.role, AssistantRole::Assistant);
            }
            other => panic!("expected assistant Output message, got {other:?}"),
        }
        assert!(!matches!(item, Item::Message(MessageItem::Input(_))));
    }
}
