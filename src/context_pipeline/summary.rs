use crate::types::{Item, assistant_text, item_text_preview};

pub const CONVERSATION_SUMMARY_PREFIX: &str = "[Conversation summary]";
pub const AGGRESSIVE_SUMMARY_PREFIX: &str = "[Aggressive summary]";

const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";

pub fn format_compact_summary(text: &str, aggressive: bool) -> String {
    format_compact_summary_with_reminder(text, aggressive, None)
}

/// Compact checkpoint body: label first (detector / FE user-anchor), then optional
/// anti-compact `<system-reminder>`, then summary prose. Never put the reminder
/// before the label.
pub fn format_compact_summary_with_reminder(
    text: &str,
    aggressive: bool,
    reminder: Option<&str>,
) -> String {
    let label = if aggressive {
        AGGRESSIVE_SUMMARY_PREFIX
    } else {
        CONVERSATION_SUMMARY_PREFIX
    };
    match reminder.map(str::trim).filter(|s| !s.is_empty()) {
        Some(tail) => {
            format!("{label}\n{SYSTEM_REMINDER_OPEN}\n{tail}\n{SYSTEM_REMINDER_CLOSE}\n{text}")
        }
        None => format!("{label}\n{text}"),
    }
}

pub fn compact_summary_message(text: &str, aggressive: bool) -> Item {
    compact_summary_message_with_reminder(text, aggressive, None)
}

pub fn compact_summary_message_with_reminder(
    text: &str,
    aggressive: bool,
    reminder: Option<&str>,
) -> Item {
    assistant_text(format_compact_summary_with_reminder(
        text, aggressive, reminder,
    ))
}

/// True when this item is a prior compaction summary (labeled assistant message).
pub fn is_compact_summary_item(item: &Item) -> bool {
    let text = item_text_preview(item);
    text.starts_with(CONVERSATION_SUMMARY_PREFIX) || text.starts_with(AGGRESSIVE_SUMMARY_PREFIX)
}

/// Strip the label and any embedded `<system-reminder>` for UPDATE prompts.
pub fn summary_body_text(item: &Item) -> String {
    let text = item_text_preview(item);
    for prefix in [CONVERSATION_SUMMARY_PREFIX, AGGRESSIVE_SUMMARY_PREFIX] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return strip_leading_system_reminder(rest.trim_start_matches('\n'));
        }
    }
    text
}

fn strip_leading_system_reminder(rest: &str) -> String {
    let rest = rest.trim_start_matches('\n');
    let Some(after_open) = rest.strip_prefix(SYSTEM_REMINDER_OPEN) else {
        return rest.to_string();
    };
    let Some(close_at) = after_open.find(SYSTEM_REMINDER_CLOSE) else {
        return rest.to_string();
    };
    after_open[close_at + SYSTEM_REMINDER_CLOSE.len()..]
        .trim_start_matches('\n')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detector_requires_label_prefix_even_with_reminder() {
        let item = compact_summary_message_with_reminder("decisions", false, Some("Todos:\n[~] a"));
        assert!(is_compact_summary_item(&item));
        let preview = item_text_preview(&item);
        assert!(preview.starts_with(CONVERSATION_SUMMARY_PREFIX));
        assert!(preview.contains("<system-reminder>"));
        assert!(preview.contains("Todos:"));
        let label_end = CONVERSATION_SUMMARY_PREFIX.len();
        assert!(
            preview[label_end..].contains("<system-reminder>"),
            "reminder must sit after the label"
        );
    }

    #[test]
    fn reminder_must_not_precede_label() {
        let item = compact_summary_message_with_reminder("body", false, Some("keep me"));
        assert!(!item_text_preview(&item).starts_with("<system-reminder>"));
    }

    #[test]
    fn summary_body_strips_reminder_for_update_prompt() {
        let item = compact_summary_message_with_reminder(
            "old decisions",
            false,
            Some("[Active plan] .litecode/plan/x.md"),
        );
        let body = summary_body_text(&item);
        assert_eq!(body, "old decisions");
        assert!(!body.contains("system-reminder"));
        assert!(!body.contains("Active plan"));
    }

    #[test]
    fn summary_body_without_reminder_is_prose() {
        let item = compact_summary_message("just prose", false);
        assert_eq!(summary_body_text(&item), "just prose");
    }

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
