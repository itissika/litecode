//! Media token budget over FunctionCallOutput Content parts.
//!
//! **Downgrade strategy (ephemeral LLM view only):** when estimated media tokens exceed
//! `budget_limit`, strip the oldest `InputImage` / `InputFile` parts until under budget.
//! Persisted transcript is never mutated. Per-part costs come from
//! [`crate::session::media_tokens`] — same helpers as [`crate::session::estimate`].

use crate::authority::responses::{FunctionCallOutput, InputContent};
use crate::session::media_tokens::input_content_media_tokens;
use crate::types::Item;

/// Apply media token budget to a transcript view (ephemeral LLM view only).
///
/// When estimated media tokens exceed `budget_limit`, remove the oldest
/// `FunctionCallOutput::Content` image/file parts so the view stays under budget.
pub fn apply_media_token_budget(items: &mut [Item], budget_limit: usize) {
    if budget_limit == 0 {
        return;
    }
    let mut media_tokens = estimate_tool_media_tokens(items);
    if media_tokens <= budget_limit {
        return;
    }

    for item in items.iter_mut() {
        if media_tokens <= budget_limit {
            break;
        }
        let Item::FunctionCallOutput(out) = item else {
            continue;
        };
        let FunctionCallOutput::Content(parts) = &mut out.output else {
            continue;
        };
        let before = parts.len();
        let mut kept = Vec::with_capacity(parts.len());
        let mut stripped = 0usize;
        for part in parts.drain(..) {
            match &part {
                InputContent::InputImage(_) | InputContent::InputFile(_) => {
                    if media_tokens > budget_limit {
                        let cost = input_content_media_tokens(&part);
                        media_tokens = media_tokens.saturating_sub(cost);
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
                .all(|p| matches!(p, InputContent::InputText(_)))
        {
            // Collapse pure-text Content back to Text for a smaller payload.
            let text = parts
                .iter()
                .filter_map(|p| match p {
                    InputContent::InputText(t) => Some(t.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let note = if text.is_empty() {
                format!("[media trimmed: {stripped} part(s) over budget]")
            } else {
                format!("{text}\n[media trimmed: {stripped} part(s) over budget]")
            };
            out.output = FunctionCallOutput::Text(note);
        } else if stripped > 0 && before > 0 {
            let _ = before;
        }
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
        FunctionCallOutputItemParam, InputImageContent, InputTextContent,
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
}
