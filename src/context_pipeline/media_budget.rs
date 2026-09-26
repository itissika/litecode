//! Media token budget over tool results and older user messages.
//!
//! **Downgrade strategy (ephemeral LLM view only):** when estimated media tokens exceed
//! `budget_limit`, strip the oldest `InputImage` / `InputFile` parts until under budget.
//! The newest user message is never stripped. Persisted transcript is never mutated.
//! Per-part costs come from [`crate::session::media_tokens`] — same helpers as
//! [`crate::session::estimate`].

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputRole, InputTextContent, MessageItem,
};
use crate::session::media_tokens::input_content_media_tokens;
use crate::types::Item;

/// One fifth of the context window. A zero window disables the trim.
pub fn media_budget_limit(context_window: usize) -> usize {
    context_window / 5
}

/// Apply media token budget to a transcript view (ephemeral LLM view only).
///
/// When estimated media tokens exceed `budget_limit`, remove the oldest
/// image/file parts so the view stays under budget. The newest user message
/// keeps its images even when that alone exceeds the limit.
pub fn apply_media_token_budget(items: &mut [Item], budget_limit: usize) {
    if budget_limit == 0 {
        return;
    }
    let protected = last_user_index(items);
    let mut media_tokens = estimate_view_media_tokens(items);
    if media_tokens <= budget_limit {
        return;
    }

    for (index, item) in items.iter_mut().enumerate() {
        if media_tokens <= budget_limit {
            break;
        }
        if Some(index) == protected {
            continue;
        }
        match item {
            Item::FunctionCallOutput(output) => {
                trim_tool_output(output, &mut media_tokens, budget_limit);
            }
            Item::Message(MessageItem::Input(message)) => {
                trim_input_media(&mut message.content, &mut media_tokens, budget_limit);
            }
            _ => {}
        }
    }
}

fn last_user_index(items: &[Item]) -> Option<usize> {
    items.iter().rposition(|item| {
        matches!(
            item,
            Item::Message(MessageItem::Input(message)) if message.role == InputRole::User
        )
    })
}

/// Estimate media tokens in tool results and user input messages.
pub fn estimate_view_media_tokens(items: &[Item]) -> usize {
    let mut total = 0usize;
    for item in items {
        match item {
            Item::FunctionCallOutput(output) => {
                let FunctionCallOutput::Content(parts) = &output.output else {
                    continue;
                };
                for part in parts {
                    total += input_content_media_tokens(part);
                }
            }
            Item::Message(MessageItem::Input(message)) => {
                for part in &message.content {
                    total += input_content_media_tokens(part);
                }
            }
            _ => {}
        }
    }
    total
}

fn trim_input_media(parts: &mut [InputContent], media_tokens: &mut usize, budget_limit: usize) {
    for part in parts.iter_mut() {
        if *media_tokens <= budget_limit {
            break;
        }
        let kind = match part {
            InputContent::InputImage(_) => "image",
            InputContent::InputFile(_) => "file",
            InputContent::InputText(_) => continue,
        };
        let cost = input_content_media_tokens(part);
        *part = InputContent::InputText(InputTextContent {
            text: format!("[media trimmed: {kind} over budget]"),
        });
        *media_tokens = media_tokens.saturating_sub(cost);
    }
}

fn trim_tool_output(
    output: &mut crate::authority::responses::FunctionCallOutputItemParam,
    media_tokens: &mut usize,
    budget_limit: usize,
) {
    let FunctionCallOutput::Content(parts) = &mut output.output else {
        return;
    };
    let before = parts.len();
    let mut kept = Vec::with_capacity(parts.len());
    let mut stripped = 0usize;
    for part in parts.drain(..) {
        match &part {
            InputContent::InputImage(_) | InputContent::InputFile(_) => {
                if *media_tokens > budget_limit {
                    let cost = input_content_media_tokens(&part);
                    *media_tokens = media_tokens.saturating_sub(cost);
                    stripped += 1;
                    continue;
                }
            }
            InputContent::InputText(_) => {}
        }
        kept.push(part);
    }
    *parts = kept;
    if stripped > 0
        && parts
            .iter()
            .all(|part| matches!(part, InputContent::InputText(_)))
    {
        let text = parts
            .iter()
            .filter_map(|part| match part {
                InputContent::InputText(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let note = if text.is_empty() {
            format!("[media trimmed: {stripped} part(s) over budget]")
        } else {
            format!("{text}\n[media trimmed: {stripped} part(s) over budget]")
        };
        output.output = FunctionCallOutput::Text(note);
    } else if stripped > 0 && before > 0 {
        let _ = before;
    }
}

/// Estimate media tokens in FunctionCallOutput Content (image/file parts).
pub fn estimate_tool_media_tokens(items: &[Item]) -> usize {
    let mut n = 0usize;
    for item in items {
        let Item::FunctionCallOutput(out) = item else {
            continue;
        };
        let FunctionCallOutput::Content(parts) = &out.output else {
            continue;
        };
        for part in parts {
            n += input_content_media_tokens(part);
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        FunctionCallOutputItemParam, InputImageContent, InputTextContent, MessageItem,
    };
    use crate::session::media_tokens::IMAGE_FALLBACK_TOKENS;
    use crate::types::user_text;

    fn fc_content_with_image() -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
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
        })
    }

    #[test]
    fn media_budget_preserves_non_media_items() {
        let mut items = vec![user_text("hi")];
        apply_media_token_budget(&mut items, 100);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn media_budget_trims_when_over_limit() {
        let mut items = vec![fc_content_with_image()];
        assert_eq!(estimate_tool_media_tokens(&items), IMAGE_FALLBACK_TOKENS);
        apply_media_token_budget(&mut items, 1);
        assert_eq!(estimate_tool_media_tokens(&items), 0);
        match &items[0] {
            Item::FunctionCallOutput(out) => match &out.output {
                FunctionCallOutput::Text(t) => assert!(t.contains("media trimmed")),
                FunctionCallOutput::Content(parts) => {
                    assert!(
                        parts
                            .iter()
                            .all(|p| !matches!(p, InputContent::InputImage(_)))
                    );
                }
            },
            _ => panic!("expected function_call_output"),
        }
    }

    #[test]
    fn trim_cost_matches_shared_helper() {
        let items = vec![fc_content_with_image()];
        assert_eq!(
            estimate_tool_media_tokens(&items),
            input_content_media_tokens(&InputContent::InputImage(InputImageContent {
                detail: Default::default(),
                file_id: None,
                image_url: Some("https://example.com/a.png".into()),
            }))
        );
    }

    #[test]
    fn budget_keeps_the_latest_user_image() {
        let older = crate::types::user_message("", &["https://example.com/old.png".into()]);
        let latest = crate::types::user_message("", &["https://example.com/new.png".into()]);
        let mut items = vec![older, latest];
        apply_media_token_budget(&mut items, 1);
        match &items[0] {
            Item::Message(MessageItem::Input(message)) => {
                assert!(message.content.iter().any(|part| matches!(
                    part,
                    InputContent::InputText(text) if text.text.contains("media trimmed")
                )));
            }
            _ => panic!("older user message"),
        }
        match &items[1] {
            Item::Message(MessageItem::Input(message)) => {
                assert!(
                    message
                        .content
                        .iter()
                        .any(|part| matches!(part, InputContent::InputImage(_)))
                );
            }
            _ => panic!("latest user message"),
        }
    }
}
