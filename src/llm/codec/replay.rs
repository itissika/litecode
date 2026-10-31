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
mod tests {}
