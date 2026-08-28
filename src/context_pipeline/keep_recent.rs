//! Keep-recent cut selection and summarizer input shaping for compaction.
//!
//! Split: output reserve (next completion) comes off the window first; the
//! remainder is history_budget. Keep-recent takes verbatim items up to
//! `min(20k, window/4)` inside that budget; compact `max_output_tokens` gets
//! the leftover. One LLM pass — no second-pass / aggressive rewrite.

use crate::authority::responses::{FunctionCallOutput, InputContent, Item, MessageItem};
use crate::context_pipeline::summary::{is_compact_summary_item, summary_body_text};
use crate::session::estimate::{autocompact_threshold, item_token_estimate};

/// Max characters of each tool-result text part fed to the summarizer LLM.
pub const SUMMARIZE_TOOL_RESULT_CHARS: usize = 2000;
/// Verbatim recent-window cap (Pi / Zed-style ~20k).
pub const KEEP_RECENT_CAP: usize = 20_000;
/// Hard ceiling for compaction LLM `max_output_tokens` (independent of model config).
pub const COMPACT_MAX_OUTPUT_TOKENS: u32 = 20_480;
const OUTPUT_RESERVE_MIN: usize = 2_048;
const OUTPUT_RESERVE_MAX: usize = 16_384;
const MIN_SUMMARY_TOKENS: usize = 512;
const FALLBACK_WINDOW: usize = 128_000;

fn window_or_fallback(context_window: usize) -> usize {
    if context_window > 0 {
        context_window
    } else {
        FALLBACK_WINDOW
    }
}

/// Tokens reserved for the next main-model completion (not compacted).
pub fn output_reserve_tokens(context_window: usize) -> usize {
    let w = window_or_fallback(context_window);
    (w / 10)
        .clamp(OUTPUT_RESERVE_MIN, OUTPUT_RESERVE_MAX)
        .min(w / 2)
}

/// Post-compact history ceiling: under the 80% trigger and after output reserve.
/// Keep-recent + compact `max_output_tokens` must fit in this.
pub fn history_budget_tokens(context_window: usize) -> usize {
    let w = window_or_fallback(context_window);
    let threshold = autocompact_threshold(w);
    let reserved = output_reserve_tokens(w);
    threshold.min(w.saturating_sub(reserved)).max(1)
}

/// Default recent window: `min(20_000, window/4, history_budget - min_summary)`.
pub fn default_keep_recent_tokens(context_window: usize) -> usize {
    let w = window_or_fallback(context_window);
    let quarter = (w / 4).max(1);
    let hist = history_budget_tokens(w);
    KEEP_RECENT_CAP
        .min(quarter)
        .min(hist.saturating_sub(MIN_SUMMARY_TOKENS))
        .max(1)
}

/// Compact-model output cap: model `max_tokens`, capped at [`COMPACT_MAX_OUTPUT_TOKENS`],
/// then clipped to leftover history budget.
pub fn compact_output_tokens(context_window: usize, keep_recent: usize, configured: u32) -> u32 {
    let configured = configured.min(COMPACT_MAX_OUTPUT_TOKENS) as usize;
    let hist = history_budget_tokens(context_window);
    let room = hist.saturating_sub(keep_recent).max(1);
    let lo = MIN_SUMMARY_TOKENS.min(room);
    let hi = room.max(lo);
    configured.clamp(lo, hi) as u32
}

/// Find the cut index so `items[cut..]` is the keep-recent window.
///
/// Returns `None` when the entire transcript fits in `keep_tokens` (nothing to
/// discard) or `items` is empty.
pub fn find_keep_recent_cut(items: &[Item], keep_tokens: usize) -> Option<usize> {
    if items.is_empty() {
        return None;
    }

    let mut acc = 0usize;
    let mut cut = items.len();
    while cut > 0 && acc < keep_tokens {
        cut -= 1;
        acc = acc.saturating_add(item_token_estimate(&items[cut]));
    }

    cut = adjust_cut_for_tool_pairs(items, cut);
    cut = adjust_cut_for_assistant_segment(items, cut);
    cut = ensure_after_leading_summary(items, cut);

    if cut == 0 { None } else { Some(cut) }
}

/// Never split a FunctionCall from its FunctionCallOutput — prefer moving the
/// whole pair into the recent window (cut before the call).
fn adjust_cut_for_tool_pairs(items: &[Item], mut cut: usize) -> usize {
    loop {
        let mut changed = false;
        for item in items.iter().skip(cut) {
            let Item::FunctionCallOutput(out) = item else {
                continue;
            };
            if let Some(call_idx) = find_function_call_index(items, &out.call_id)
                && call_idx < cut
            {
                cut = call_idx;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    cut
}

fn find_function_call_index(items: &[Item], call_id: &str) -> Option<usize> {
    items.iter().position(|item| match item {
        Item::FunctionCall(fc) => fc.call_id == call_id,
        _ => false,
    })
}

/// Never split an assistant wire-turn: `Reasoning*` + output `Message` +
/// `FunctionCall*`. If `cut` lands inside that segment, move it to the segment start.
fn adjust_cut_for_assistant_segment(items: &[Item], cut: usize) -> usize {
    if cut == 0 || cut >= items.len() {
        return cut;
    }
    if !is_assistant_segment_atom(&items[cut]) {
        return cut;
    }
    assistant_segment_start(items, cut)
}

fn is_assistant_segment_atom(item: &Item) -> bool {
    matches!(
        item,
        Item::Reasoning(_) | Item::FunctionCall(_) | Item::Message(MessageItem::Output(_))
    )
}

fn assistant_segment_start(items: &[Item], mut i: usize) -> usize {
    while i > 0 && matches!(items[i - 1], Item::FunctionCall(_)) {
        i -= 1;
    }
    if i > 0 && matches!(items[i - 1], Item::Message(MessageItem::Output(_))) {
        i -= 1;
    }
    while i > 0 && matches!(items[i - 1], Item::Reasoning(_)) {
        i -= 1;
    }
    i
}

/// Leading compaction summary must go into the discarded region when we compact.
fn ensure_after_leading_summary(items: &[Item], cut: usize) -> usize {
    if cut > 0 && is_compact_summary_item(&items[0]) {
        cut.max(1)
    } else {
        cut
    }
}

/// Deep-copy items and truncate tool-result text for the summarizer prompt.
pub fn truncate_items_for_summary(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .cloned()
        .map(truncate_item_tool_results)
        .collect()
}

fn truncate_item_tool_results(mut item: Item) -> Item {
    let Item::FunctionCallOutput(out) = &mut item else {
        return item;
    };
    match &mut out.output {
        FunctionCallOutput::Text(s) => {
            *s = truncate_chars(s, SUMMARIZE_TOOL_RESULT_CHARS);
        }
        FunctionCallOutput::Content(parts) => {
            for part in parts.iter_mut() {
                if let InputContent::InputText(t) = part {
                    t.text = truncate_chars(&t.text, SUMMARIZE_TOOL_RESULT_CHARS);
                }
            }
        }
    }
    item
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}\n…[truncated for summarizer]")
}

/// Serialize discarded items for the summarizer (tool outputs already truncated).
pub fn serialize_for_summary(items: &[Item]) -> String {
    let truncated = truncate_items_for_summary(items);
    serde_json::to_string(&truncated).unwrap_or_else(|_| "[]".into())
}

/// Build the LLM compaction user message: data only (history / prior summary).
/// Structure, budget, and merge rules live in the compaction system prompt.
pub fn build_compaction_prompt(discarded: &[Item]) -> String {
    if discarded.first().is_some_and(is_compact_summary_item) {
        let prev = summary_body_text(&discarded[0]);
        let rest = if discarded.len() > 1 {
            serialize_for_summary(&discarded[1..])
        } else {
            "[]".into()
        };
        format!(
            "Previous summary:\n{}\n\nNew transcript JSON:\n{}",
            prev, rest
        )
    } else {
        format!("Transcript JSON:\n{}", serialize_for_summary(discarded))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall,
    };
    use crate::context_pipeline::summary::compact_summary_message;
    use crate::types::{item_text_preview, user_text};

    fn fc(call_id: &str, name: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            call_id: call_id.into(),
            name: name.into(),
            arguments: "{}".into(),
            id: Some(call_id.into()),
            status: None,
            namespace: None,
        })
    }

    fn fco(call_id: &str, text: &str) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Text(text.into()),
            id: None,
            status: None,
        })
    }

    #[test]
    fn entire_transcript_within_keep_returns_none() {
        let items = vec![user_text("a"), user_text("b")];
        assert!(find_keep_recent_cut(&items, 100_000).is_none());
    }

    #[test]
    fn empty_returns_none() {
        assert!(find_keep_recent_cut(&[], 100).is_none());
    }

    #[test]
    fn cut_discards_prefix() {
        // Tiny keep budget forces only the last message into recent.
        let items: Vec<Item> = (0..10).map(|i| user_text(format!("msg-{i}"))).collect();
        let cut = find_keep_recent_cut(&items, 1).expect("cut");
        assert!(cut > 0);
        assert!(cut < items.len());
        assert_eq!(items[cut..].len(), 1);
    }

    #[test]
    fn tool_pair_not_split() {
        let items = vec![
            user_text("old"),
            fc("c1", "bash"),
            fco("c1", "ok"),
            user_text("recent"),
        ];
        // keep tokens sized so raw cut would land between call and output.
        let call_tokens = item_token_estimate(&items[1]);
        let out_tokens = item_token_estimate(&items[2]);
        let recent_tokens = item_token_estimate(&items[3]);
        // Want accumulation: recent + output >= keep, but recent alone < keep,
        // so raw cut lands on the output index (splitting the pair).
        let keep = recent_tokens + 1;
        assert!(keep <= recent_tokens + out_tokens);
        assert!(keep > recent_tokens);

        let cut = find_keep_recent_cut(&items, keep).expect("cut");
        // Must include the FunctionCall in kept (not start at output).
        assert!(
            cut <= 1,
            "cut={cut} must be at or before the FunctionCall (index 1)"
        );
        assert!(
            matches!(&items[cut], Item::FunctionCall(_))
                || matches!(&items[cut], Item::Message(_)) && cut < 1,
            "kept must not start at orphan output"
        );
        // Specifically: if we kept from output only, cut would be 2 — forbidden.
        assert_ne!(cut, 2);
        let _ = (call_tokens, out_tokens);
    }

    #[test]
    fn leading_summary_goes_to_discarded() {
        let items = vec![
            compact_summary_message("prior work", false),
            user_text("x".repeat(200)),
            user_text("y".repeat(200)),
            user_text("tail"),
        ];
        let cut = find_keep_recent_cut(&items, 5).expect("cut");
        assert!(cut >= 1, "leading summary must be discarded, cut={cut}");
        assert!(is_compact_summary_item(&items[0]));
        assert!(
            !items[..cut]
                .iter()
                .any(|i| matches!(i, Item::Message(_)) && item_text_preview(i) == "tail")
        );
    }

    #[test]
    fn serialize_truncates_long_tool_output() {
        let long = "z".repeat(5000);
        let items = vec![fco("c1", &long)];
        let json = serialize_for_summary(&items);
        assert!(json.len() < long.len());
        assert!(json.contains("truncated for summarizer"));
        // Raw long run should not appear at full length.
        assert!(!json.contains(&long));
    }

    #[test]
    fn update_prompt_when_leading_summary() {
        let discarded = vec![
            compact_summary_message("old decisions", false),
            user_text("new turn"),
        ];
        let prompt = build_compaction_prompt(&discarded);
        assert!(prompt.contains("Previous summary:"));
        assert!(prompt.contains("old decisions"));
        assert!(prompt.contains("New transcript JSON:"));
        assert!(!prompt.contains("20,000 tokens"));
        assert!(!prompt.contains("User messages and intent"));
        assert!(!prompt.contains("[Conversation summary]\nold decisions"));
    }

    #[test]
    fn fresh_prompt_without_leading_summary() {
        let discarded = vec![user_text("a"), user_text("b")];
        let prompt = build_compaction_prompt(&discarded);
        assert!(prompt.contains("Transcript JSON:"));
        assert!(!prompt.contains("20,000 tokens"));
        assert!(!prompt.contains("User messages and intent"));
        assert!(!prompt.contains("Previous summary:"));
    }

    #[test]
    fn default_keep_recent_caps_at_20k() {
        assert_eq!(default_keep_recent_tokens(200_000), 20_000);
        assert_eq!(default_keep_recent_tokens(10_000), 2_500);
    }

    #[test]
    fn compact_output_tokens_caps_above_product_limit() {
        assert_eq!(
            compact_output_tokens(200_000, default_keep_recent_tokens(200_000), 65_536),
            COMPACT_MAX_OUTPUT_TOKENS
        );
    }

    #[test]
    fn compact_split_fits_under_history_budget() {
        for window in [8_000usize, 16_000, 32_000, 128_000, 200_000] {
            let keep = default_keep_recent_tokens(window);
            let out = compact_output_tokens(window, keep, 4096) as usize;
            let hist = history_budget_tokens(window);
            assert!(
                keep + out <= hist,
                "window={window}: keep={keep} + out={out} > hist={hist}"
            );
            let threshold = crate::session::estimate::autocompact_threshold(window);
            assert!(
                keep + out <= threshold,
                "window={window}: post-compact cap must stay under 80% trigger"
            );
        }
    }

    fn reasoning(text: &str) -> Item {
        use crate::authority::responses::{
            OutputStatus, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
        };
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

    fn assistant_text(text: &str) -> Item {
        use crate::authority::responses::{
            AssistantRole, OutputMessage, OutputMessageContent, OutputStatus, OutputTextContent,
        };
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

    #[test]
    fn assistant_segment_not_split_between_reasoning_and_message() {
        let items = vec![
            user_text("old"),
            reasoning("think"),
            assistant_text("reply"),
            user_text("recent"),
        ];
        let recent = item_token_estimate(&items[3]);
        let msg = item_token_estimate(&items[2]);
        // Raw walk would land on the output Message (between Reasoning and Message).
        let keep = recent + 1;
        assert!(keep <= recent + msg);
        assert!(keep > recent);

        let cut = find_keep_recent_cut(&items, keep).expect("cut");
        assert!(
            cut <= 1,
            "cut={cut} must not start kept at the output Message (index 2)"
        );
        assert_ne!(cut, 2);
    }

    #[test]
    fn assistant_segment_includes_function_calls_with_reasoning() {
        let items = vec![
            user_text("old"),
            reasoning("need tool"),
            fc("c1", "bash"),
            fco("c1", "ok"),
            user_text("recent"),
        ];
        let keep = item_token_estimate(&items[4]) + 1;
        let cut = find_keep_recent_cut(&items, keep).expect("cut");
        assert!(
            cut <= 1,
            "cut={cut} must include Reasoning+FunctionCall, not start mid-segment"
        );
    }
}
