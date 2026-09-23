//! Reasoning replay guard for Responses-dialect vendors that require it.
//!
//! Both DeepSeek and Xiaomi MiMo hard-require, in thinking mode with `tools`,
//! that every historical assistant turn's reasoning is passed back — omitting
//! it is a 400:
//!
//! - DeepSeek: thinking mode docs — "The `reasoning_text` in the thinking mode
//!   must be passed back to the API." (empty text is rejected too).
//! - MiMo: Deep Thinking docs — "must completely pass back the
//!   `reasoning_content` field, otherwise the API will return a 400 error"
//!   (<https://mimo.mi.com/docs/en-US/quick-start/usage-guide/text-generation/deep-thinking>);
//!   the Responses dialect asks for prior reasoning in the `input` array
//!   (<https://mimo.mi.com/docs/en-US/api/chat/responses>).
//!
//! Turns whose reasoning was never recorded — the compaction summary, history
//! produced with thinking off — get a placeholder item so replay never 400s.
//! Vendors without this rule (OpenAI) never call this.

use crate::authority::responses::{
    Item, MessageItem, OutputStatus, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
};

/// Placeholder reasoning text for assistant turns that have no recorded
/// reasoning. Must be non-empty: DeepSeek rejects an empty `reasoning_text`
/// the same as a missing one.
pub(crate) const REPLAY_REASONING_PLACEHOLDER: &str = "[reasoning not recorded]";

/// Ensure every assistant segment (`message`/`function_call`) in the replayed
/// input is preceded by a reasoning item with non-empty `reasoning_text`.
///
/// No-op when thinking is off or the request carries no tools (both vendors
/// ignore reasoning then).
pub(crate) fn ensure_reasoning_replay(
    input: &[Item],
    tools_present: bool,
    thinking_on: bool,
) -> Vec<Item> {
    if !tools_present || !thinking_on {
        return input.to_vec();
    }
    let mut out: Vec<Item> = Vec::with_capacity(input.len());
    let mut synth = 0usize;
    // `true` while the next assistant item starts a segment with no reasoning.
    let mut needs_reasoning = true;
    for item in input {
        match item {
            Item::Reasoning(r) => {
                if reasoning_text_is_empty(r) {
                    let mut patched = r.clone();
                    patched.content = Some(vec![ReasoningItemContent::ReasoningText(
                        ReasoningTextContent {
                            text: REPLAY_REASONING_PLACEHOLDER.into(),
                        },
                    )]);
                    out.push(Item::Reasoning(patched));
                } else {
                    out.push(item.clone());
                }
                needs_reasoning = false;
            }
            Item::Message(MessageItem::Output(_)) | Item::FunctionCall(_) => {
                if needs_reasoning {
                    out.push(Item::Reasoning(ReasoningItem {
                        id: Some(format!("rs_replay_{synth}")),
                        summary: vec![],
                        content: Some(vec![ReasoningItemContent::ReasoningText(
                            ReasoningTextContent {
                                text: REPLAY_REASONING_PLACEHOLDER.into(),
                            },
                        )]),
                        encrypted_content: None,
                        status: Some(OutputStatus::Completed),
                    }));
                    synth += 1;
                }
                needs_reasoning = false;
                out.push(item.clone());
            }
            _ => {
                needs_reasoning = true;
                out.push(item.clone());
            }
        }
    }
    out
}

fn reasoning_text_is_empty(r: &ReasoningItem) -> bool {
    r.content.as_ref().is_none_or(|parts| {
        parts
            .iter()
            .all(|p| matches!(p, ReasoningItemContent::ReasoningText(t) if t.text.is_empty()))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, FunctionToolCall, OutputMessage, OutputMessageContent, OutputTextContent,
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

    fn synthesized(items: &[Item]) -> usize {
        items
            .iter()
            .filter(|item| {
                matches!(item, Item::Reasoning(r)
                    if r.id.as_deref().is_some_and(|id| id.starts_with("rs_replay_")))
            })
            .count()
    }

    #[test]
    fn one_placeholder_covers_a_whole_assistant_segment() {
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
        assert_eq!(synthesized(&replayed), 1, "{replayed:?}");
        assert_eq!(replayed.len(), input.len() + 1);
        // The placeholder precedes the assistant message it protects.
        assert!(matches!(replayed[1], Item::Reasoning(_)));
        assert!(matches!(replayed[2], Item::Message(MessageItem::Output(_))));
    }

    #[test]
    fn recorded_reasoning_is_kept_and_never_duplicated() {
        let input = vec![user_text("hi"), reasoning("think"), assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(synthesized(&replayed), 0);
        assert_eq!(replayed.len(), input.len());
    }

    #[test]
    fn empty_reasoning_text_is_patched_in_place() {
        let input = vec![user_text("hi"), reasoning(""), assistant("hello")];
        let replayed = ensure_reasoning_replay(&input, true, true);
        assert_eq!(replayed.len(), input.len());
        let Item::Reasoning(patched) = &replayed[1] else {
            panic!("expected reasoning");
        };
        let text = patched
            .content
            .as_ref()
            .and_then(|parts| parts.first())
            .map(|part| {
                let ReasoningItemContent::ReasoningText(text) = part;
                text.text.clone()
            })
            .unwrap_or_default();
        assert_eq!(text, REPLAY_REASONING_PLACEHOLDER);
    }

    #[test]
    fn replay_is_skipped_without_tools_or_without_thinking() {
        let input = vec![user_text("hi"), assistant("hello")];
        assert_eq!(ensure_reasoning_replay(&input, false, true), input);
        assert_eq!(ensure_reasoning_replay(&input, true, false), input);
    }
}
