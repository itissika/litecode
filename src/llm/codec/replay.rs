//! Reasoning replay for Responses-dialect vendors that read `reasoning_text`.
//!
//! DeepSeek and MiMo ask for thinking the model already produced to be passed
//! back when the request carries tools. Their samples append that field as the
//! API returned it. They do not ask the client to write a thought the model
//! never emitted.
//!
//! - DeepSeek thinking mode: with `tools`, pass back each earlier turn's
//!   `reasoning_content`. The Responses guide accepts a `reasoning` item's
//!   plain-text `content` and merges it into the adjacent assistant message.
//!   `summary` and `encrypted_content` are not accepted on that input.
//! - MiMo Responses: while a thinking-mode tool loop is open, keep the
//!   reasoning content the model returned in the next `input`. The chat guide
//!   says the same thing as a 400 when `reasoning_content` is dropped.
//!
//! Readable text from another model therefore moves into `reasoning_text`:
//! a public summary when this item has no raw content. Ciphertext and a
//! Messages signature stay in `encrypted_content` and are never copied into
//! the text. A turn with no recorded text is sent without an invented item.
//! Vendors without this rule (OpenAI) never call this.

use crate::authority::responses::{
    Item, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
};

/// Move readable reasoning into `reasoning_text` for a vendor that only reads
/// that field.
///
/// No-op when thinking is off or the request carries no tools (both vendors
/// ignore prior reasoning then). Does not insert an item, and does not replace
/// empty text with a sentence.
pub(crate) fn ensure_reasoning_replay(
    input: &[Item],
    tools_present: bool,
    thinking_on: bool,
) -> Vec<Item> {
    if !tools_present || !thinking_on {
        return input.to_vec();
    }
    input
        .iter()
        .map(|item| {
            let Item::Reasoning(reasoning) = item else {
                return item.clone();
            };
            if !reasoning_content_text(reasoning).is_empty() {
                return item.clone();
            }
            let summary = reasoning_summary_text(reasoning);
            if summary.trim().is_empty() {
                return item.clone();
            }
            // One copy. The summary is the model's own text; this dialect
            // will not read it in `summary`, so it moves into `content`.
            // `encrypted_content` is left where it is.
            let mut patched = reasoning.clone();
            patched.summary = Vec::new();
            patched.content = Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent { text: summary },
            )]);
            Item::Reasoning(patched)
        })
        .collect()
}

/// The item's raw reasoning text, empty when the producer exposed none.
fn reasoning_content_text(r: &ReasoningItem) -> String {
    r.content
        .as_ref()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| match part {
                    ReasoningItemContent::ReasoningText(text) => Some(text.text.as_str()),
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// The item's public summary, empty when it carries none.
fn reasoning_summary_text(r: &ReasoningItem) -> String {
    use crate::authority::responses::SummaryPart;
    r.summary
        .iter()
        .filter_map(|part| match part {
            SummaryPart::SummaryText(text) => Some(text.text.as_str()),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, FunctionToolCall, MessageItem, OutputMessage, OutputMessageContent,
        OutputStatus, OutputTextContent,
    };
    use crate::types::user_text;

    fn assistant(text: &str) -> Item {
        Item::Message(MessageItem::Output(OutputMessage {
            id: "msg_1".into(),
            role: AssistantRole::Assistant,
            status: OutputStatus::Completed,
            phase: None,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: text.into(),
                annotations: vec![],
                logprobs: None,
            })],
        }))
    }

    fn call(name: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            id: Some("fc_1".into()),
            call_id: "call_1".into(),
            name: name.into(),
            arguments: "{}".into(),
            status: Some(OutputStatus::Completed),
            namespace: None,
        })
    }

    fn reasoning(text: &str) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some("rs_1".into()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent { text: text.into() },
            )]),
            encrypted_content: None,
            status: Some(OutputStatus::Completed),
        })
    }

    #[test]
    fn a_turn_without_reasoning_is_not_given_one() {
        let input = vec![
            user_text("hi"),
            assistant("hello"),
            call("read"),
            Item::FunctionCallOutput(crate::authority::responses::FunctionCallOutputItemParam {
                call_id: "call_1".into(),
                output: crate::authority::responses::FunctionCallOutput::Text("ok".into()),
                id: None,
                status: None,
            }),
        ];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(replayed, input);
    }

    #[test]
    fn recorded_reasoning_is_kept_and_never_duplicated() {
        let input = vec![user_text("hi"), reasoning("think"), assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(replayed, input);
    }

    #[test]
    fn empty_reasoning_text_stays_empty() {
        let input = vec![user_text("hi"), reasoning(""), assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(replayed, input);
    }

    /// Ciphertext and a Messages signature are not readable text. They stay in
    /// `encrypted_content` and are not copied into `reasoning_text`.
    #[test]
    fn ciphertext_is_not_copied_into_reasoning_text() {
        let mut cipher_only = reasoning("");
        let Item::Reasoning(item) = &mut cipher_only else {
            panic!("expected reasoning");
        };
        item.content = None;
        item.encrypted_content = Some("sig-or-cipher".into());
        let input = vec![user_text("hi"), cipher_only, assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        let Item::Reasoning(kept) = &replayed[1] else {
            panic!("expected reasoning");
        };
        assert!(kept.content.is_none());
        assert_eq!(kept.encrypted_content.as_deref(), Some("sig-or-cipher"));
    }

    /// A public summary is readable text from another model. This dialect reads
    /// `reasoning_text`, so the summary moves into that field. It is not duplicated,
    /// and the ciphertext beside it is not treated as text.
    #[test]
    fn summary_fills_missing_reasoning_text() {
        use crate::authority::responses::{SummaryPart, SummaryTextContent};
        let summary_only = Item::Reasoning(ReasoningItem {
            id: Some("rs_foreign".into()),
            summary: vec![SummaryPart::SummaryText(SummaryTextContent {
                text: "what the other model thought".into(),
            })],
            content: None,
            encrypted_content: Some("foreign-cipher".into()),
            status: None,
        });
        let input = vec![user_text("hi"), summary_only, assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(replayed.len(), input.len());
        let Item::Reasoning(filled) = &replayed[1] else {
            panic!("expected reasoning");
        };
        let text = filled
            .content
            .as_ref()
            .and_then(|parts| parts.first())
            .map(|part| {
                let ReasoningItemContent::ReasoningText(text) = part;
                text.text.clone()
            })
            .unwrap_or_default();
        assert_eq!(text, "what the other model thought");
        assert!(
            filled.summary.is_empty(),
            "the summary moved, it is not duplicated"
        );
        assert_eq!(
            filled.encrypted_content.as_deref(),
            Some("foreign-cipher"),
            "ciphertext stays beside the text, it is not the text"
        );
    }

    #[test]
    fn replay_is_skipped_without_tools_or_without_thinking() {
        let input = vec![user_text("hi"), assistant("hello")];
        assert_eq!(ensure_reasoning_replay(&input, false, true), input);
        assert_eq!(ensure_reasoning_replay(&input, true, false), input);
    }
}
