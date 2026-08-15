//! Transcript token estimates for budget / row `token_estimate`.
//!
//! Text is counted with **tiktoken-rs cl100k_base**. Media costs come from
//! [`crate::session::media_tokens`] (same helpers as media_budget trim).
//!
//! **Forbidden:** `item_text_preview` or character-length heuristics as budget truth.

use crate::authority::responses::{
    FunctionCallOutput, InputContent, Item, MessageItem, OutputMessageContent,
    ReasoningItemContent, SummaryPart,
};
use crate::session::media_tokens::input_content_media_tokens;

/// Count tokens for a UTF-8 string with cl100k_base (OpenAI gpt-4 / 3.5 family encoding).
pub fn count_text_tokens(text: &str) -> usize {
    let bpe = tiktoken_rs::cl100k_base_singleton();
    bpe.encode_with_special_tokens(text).len()
}

fn add_input_content_tokens(content: &InputContent, total: &mut usize) {
    match content {
        InputContent::InputText(t) => *total += count_text_tokens(&t.text),
        InputContent::InputImage(_) | InputContent::InputFile(_) => {
            *total += input_content_media_tokens(content);
        }
    }
}

pub(crate) fn item_token_estimate(item: &Item) -> usize {
    match item {
        Item::Message(MessageItem::Input(msg)) => {
            let mut n = 0usize;
            for c in &msg.content {
                add_input_content_tokens(c, &mut n);
            }
            n
        }
        Item::Message(MessageItem::Output(msg)) => {
            let mut n = 0usize;
            for c in &msg.content {
                match c {
                    OutputMessageContent::OutputText(t) => n += count_text_tokens(&t.text),
                    OutputMessageContent::Refusal(r) => n += count_text_tokens(&r.refusal),
                }
            }
            n
        }
        Item::Reasoning(r) => {
            let mut n = 0usize;
            for part in &r.summary {
                let SummaryPart::SummaryText(t) = part;
                n += count_text_tokens(&t.text);
            }
            if let Some(content) = &r.content {
                for part in content {
                    let ReasoningItemContent::ReasoningText(t) = part;
                    n += count_text_tokens(&t.text);
                }
            }
            n
        }
        Item::FunctionCall(fc) => count_text_tokens(&fc.name) + count_text_tokens(&fc.arguments),
        Item::FunctionCallOutput(out) => match &out.output {
            FunctionCallOutput::Text(s) => count_text_tokens(s),
            FunctionCallOutput::Content(parts) => {
                let mut n = 0usize;
                for c in parts {
                    add_input_content_tokens(c, &mut n);
                }
                n
            }
        },
        // Other Item variants (tool search, computer, …) are rare in the agent transcript;
        // serialize body and tokenize as a last-resort lower bound so budget still moves.
        other => serde_json::to_string(other)
            .map(|s| count_text_tokens(&s))
            .unwrap_or(1),
    }
}

/// Structured Item token estimate (cl100k_base text + shared media helpers).
pub fn compute_token_estimate(items: &[Item]) -> usize {
    if items.is_empty() {
        return 0;
    }
    items.iter().map(item_token_estimate).sum::<usize>().max(1)
}

/// Autocompact fires when estimated tokens exceed this fraction of the model context window.
///
/// `0.8` leaves headroom for the model reply / tool schemas while still compacting before
/// hard context overflow. Threshold and [`compute_token_estimate`] share the same token units.
pub fn autocompact_threshold(context_window: usize) -> usize {
    (context_window as f64 * 0.8) as usize
}

/// Compact prompt text (not tokenized into the threshold itself; estimate stays Item-based).
pub fn compact_prompt() -> &'static str {
    r#"Produce a faithful, concise summary of the conversation so far so that a successor
assistant can continue the work seamlessly after the earlier turns are discarded.
Capture what is needed to continue — the user's explicit requests, your most recent
actions, key technical details, file paths, commands, configuration, and architectural
decisions — but be economical: prefer tight prose and short references over long
verbatim dumps. A focused summary that fits is more useful than an exhaustive one
that gets cut off.

Output the summary as plain text with the following numbered sections, in order
(write "None" when a section is empty):

1. Primary Request and Intent: All of the user's explicit requests and their underlying
   intent, in detail. Preserve nuance, constraints, scope boundaries, and stated
   preferences.
2. Key Technical Concepts: Important technologies, languages, frameworks, libraries,
   tools, and patterns relied upon.
3. Files and Code Sections: Every file examined, created, or modified — full path, why
   it matters, and relevant code, with the most recent edits in full.
4. Errors and Fixes: Every error, failed command, or test/build failure, the root cause,
   and exactly how it was fixed. Note any fix that came from user feedback verbatim.
5. Problem Solving: Problems solved and any in-progress diagnosis, including hypotheses
   still being evaluated.
6. All User Messages: List ALL user messages that are not tool results, in order,
   verbatim or high-fidelity. These are critical for understanding intent and how it
   evolved.
7. Pending Tasks: Tasks the user explicitly asked for that are not yet complete. Do not
   invent tasks the user never requested.
8. Current Work: Precisely what you were doing immediately before this summary, with the
   most recent file names, code, commands, and state, specific enough to resume mid-stream.
9. Optional Next Step: The single next step that directly continues the most recent work,
   strictly in line with the user's latest request. If the prior task was finished, only
   propose a next step that is clearly part of the user's stated goal; otherwise state
   that you should confirm with the user. Include a verbatim quote from the most recent
   messages showing where you left off.

Do not call any tools. Respond with only the summary text."#
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutputItemParam, InputImageContent, InputTextContent,
    };
    use crate::session::media_tokens::IMAGE_FALLBACK_TOKENS;
    use crate::types::user_text;

    #[test]
    fn cl100k_gold_sample_hello_world() {
        // Documented gold: cl100k_base encodes "hello world" as exactly 2 tokens.
        assert_eq!(count_text_tokens("hello world"), 2);
    }

    #[test]
    fn estimate_grows_with_text() {
        let short = vec![user_text("hi")];
        let long = vec![user_text("x".repeat(400))];
        assert!(compute_token_estimate(&long) > compute_token_estimate(&short));
    }

    #[test]
    fn media_adds_tokens_beyond_text_only() {
        let text_only = vec![Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Content(vec![InputContent::InputText(InputTextContent {
                text: "caption".into(),
            })]),
            id: None,
            status: None,
        })];
        let with_image = vec![Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Content(vec![
                InputContent::InputText(InputTextContent {
                    text: "caption".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some("https://example.com/a.png".into()),
                }),
            ]),
            id: None,
            status: None,
        })];
        let text_n = compute_token_estimate(&text_only);
        let media_n = compute_token_estimate(&with_image);
        assert!(media_n >= text_n + IMAGE_FALLBACK_TOKENS);
    }

    #[test]
    fn empty_transcript_is_zero() {
        assert_eq!(compute_token_estimate(&[]), 0);
    }
}
