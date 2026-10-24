//! Show-once media for the ephemeral model view, and a full strip for compact.
//!
//! A model request carries only fresh media:
//! - tool media from the latest tool batch (after the last model-produced item);
//! - user media from the current turn (after the last final assistant answer).
//!
//! Everything older becomes a short text note, so a screenshot or video is
//! billed once and stale frames cannot steer later reasoning. The note sits at
//! the tail of the previous request, so the cached prefix stays intact. The
//! persisted transcript is never mutated.
//!
//! Compaction reuses the same model projection; it never serializes a separate
//! text prompt or strips media outside this one projection.

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputRole, InputTextContent,
    MessageItem,
};
use crate::types::Item;

/// Drop media the model has already seen (ephemeral LLM view and budget only).
///
/// Returns, per item, whether it was produced while a part removed here was
/// still in its prefix: the model output that answered a stripped tool batch,
/// and the model output in the turn a stripped user message opened, through
/// that turn's final answer.
pub fn drop_stale_media(items: &mut [Item]) -> Vec<bool> {
    let tool_fresh_from = items
        .iter()
        .rposition(is_model_produced)
        .map_or(0, |index| index + 1);
    let user_fresh_from = current_turn_start(items);
    let mut rebased = vec![false; items.len()];
    for index in 0..items.len() {
        let keep = match &items[index] {
            Item::FunctionCallOutput(_) => index >= tool_fresh_from,
            Item::Message(MessageItem::Input(_)) => index >= user_fresh_from,
            _ => true,
        };
        if keep || !strip_item_media(&mut items[index]) {
            continue;
        }
        let mut next = index + 1;
        if matches!(items[index], Item::FunctionCallOutput(_)) {
            while next < items.len() && !is_model_produced(&items[next]) {
                next += 1;
            }
            while next < items.len() && is_model_produced(&items[next]) {
                rebased[next] = true;
                next += 1;
            }
        } else {
            // The image's own turn, through its final answer. A later turn was
            // signed against the note, so it is not part of this rewrite.
            let turn_end = end_of_turn_opened_at(items, index);
            for later in next..turn_end {
                if is_model_produced(&items[later]) {
                    rebased[later] = true;
                }
            }
        }
    }
    rebased
}

fn is_model_produced(item: &Item) -> bool {
    matches!(
        item,
        Item::Message(MessageItem::Output(_)) | Item::Reasoning(_) | Item::FunctionCall(_)
    )
}

/// Exclusive end of the turn opened by the user message at `opener`: the
/// index after its final answer. A final answer is an assistant message with
/// no tool call before the next user message, the same boundary
/// [`current_turn_start`] uses.
fn end_of_turn_opened_at(items: &[Item], opener: usize) -> usize {
    let mut pending_answer: Option<usize> = None;
    for index in opener + 1..items.len() {
        match &items[index] {
            Item::Message(MessageItem::Output(_)) => pending_answer = Some(index),
            Item::FunctionCall(_) => pending_answer = None,
            Item::Message(MessageItem::Input(message)) if message.role == InputRole::User => {
                if let Some(answer) = pending_answer {
                    return answer + 1;
                }
            }
            _ => {}
        }
    }
    items.len()
}

/// Reconstruct the media-rebase boundary for durable media replacements.
///
/// The original media is gone from the surface, so [`drop_stale_media`] can no
/// longer see it. A replacement row still knows its position and source through
/// `WorkingRow::replacement_media`; this marks the same dependent model outputs
/// that dropping that media would have marked.
pub fn rebased_after_media_replacements(items: &[Item], media_replacement: &[bool]) -> Vec<bool> {
    let mut rebased = vec![false; items.len()];
    for (index, is_replacement) in media_replacement.iter().enumerate() {
        if !is_replacement {
            continue;
        }
        match &items[index] {
            Item::FunctionCallOutput(_) => {
                let mut next = index + 1;
                while next < items.len() && !is_model_produced(&items[next]) {
                    next += 1;
                }
                while next < items.len() && is_model_produced(&items[next]) {
                    rebased[next] = true;
                    next += 1;
                }
            }
            Item::Message(MessageItem::Input(message)) if message.role == InputRole::User => {
                let turn_end = end_of_turn_opened_at(items, index);
                for later in index + 1..turn_end {
                    if is_model_produced(&items[later]) {
                        rebased[later] = true;
                    }
                }
            }
            _ => {}
        }
    }
    rebased
}

/// First index after the last final assistant answer: an output message with
/// no tool call before the next user message.
fn current_turn_start(items: &[Item]) -> usize {
    let mut pending_answer: Option<usize> = None;
    let mut start = 0;
    for (index, item) in items.iter().enumerate() {
        match item {
            Item::Message(MessageItem::Output(_)) => pending_answer = Some(index),
            Item::FunctionCall(_) => pending_answer = None,
            Item::Message(MessageItem::Input(message)) if message.role == InputRole::User => {
                if let Some(answer) = pending_answer.take() {
                    start = answer + 1;
                }
            }
            _ => {}
        }
    }
    start
}

fn is_media(part: &InputContent) -> bool {
    matches!(
        part,
        InputContent::InputImage(_) | InputContent::InputFile(_)
    )
}

/// Returns whether any part was removed.
fn strip_item_media(item: &mut Item) -> bool {
    match item {
        Item::FunctionCallOutput(output) => strip_tool_output(output),
        Item::Message(MessageItem::Input(message)) => strip_input_media(&mut message.content),
        _ => false,
    }
}

fn strip_input_media(parts: &mut [InputContent]) -> bool {
    let mut stripped = false;
    for part in parts.iter_mut() {
        let kind = match part {
            InputContent::InputImage(_) => "image",
            InputContent::InputFile(_) => "file",
            InputContent::InputText(_) => continue,
        };
        *part = InputContent::InputText(InputTextContent {
            text: format!("[{kind} already viewed; removed from context]"),
        });
        stripped = true;
    }
    stripped
}

fn strip_tool_output(output: &mut FunctionCallOutputItemParam) -> bool {
    let FunctionCallOutput::Content(parts) = &mut output.output else {
        return false;
    };
    let before = parts.len();
    parts.retain(|part| !is_media(part));
    let stripped = before - parts.len();
    if stripped == 0 {
        return false;
    }
    let text = parts
        .iter()
        .filter_map(|part| match part {
            InputContent::InputText(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let note = format!(
        "[{stripped} media part(s) already viewed; removed from context. Call the tool again to view]"
    );
    output.output = FunctionCallOutput::Text(if text.is_empty() {
        note
    } else {
        format!("{text}\n{note}")
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{FunctionToolCall, InputImageContent};
    use crate::types::{assistant_text, user_text};

    fn call(call_id: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            call_id: call_id.into(),
            name: "shot".into(),
            arguments: "{}".into(),
            id: None,
            status: None,
            namespace: None,
        })
    }

    fn tool_image(call_id: &str, url: &str) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Content(vec![
                InputContent::InputText(InputTextContent {
                    text: "caption".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some(url.into()),
                }),
            ]),
            id: None,
            status: None,
        })
    }

    fn user_image(url: &str) -> Item {
        crate::types::user_message("look", &[url.into()])
    }

    fn media_count(items: &[Item]) -> usize {
        items
            .iter()
            .map(|item| match item {
                Item::FunctionCallOutput(output) => match &output.output {
                    FunctionCallOutput::Content(parts) => {
                        parts.iter().filter(|part| is_media(part)).count()
                    }
                    FunctionCallOutput::Text(_) => 0,
                },
                Item::Message(MessageItem::Input(message)) => {
                    message.content.iter().filter(|part| is_media(part)).count()
                }
                _ => 0,
            })
            .sum()
    }

    fn has_media(item: &Item) -> bool {
        media_count(std::slice::from_ref(item)) > 0
    }

    #[test]
    fn latest_tool_batch_keeps_every_media_part() {
        let mut items = vec![
            user_text("go"),
            call("c1"),
            call("c2"),
            tool_image("c1", "https://example.com/a.png"),
            tool_image("c2", "https://example.com/b.png"),
        ];
        drop_stale_media(&mut items);
        assert_eq!(media_count(&items), 2);
    }

    #[test]
    fn tool_media_is_dropped_once_the_model_has_seen_it() {
        let mut items = vec![
            user_text("go"),
            call("c1"),
            tool_image("c1", "https://example.com/old.png"),
            call("c2"),
            tool_image("c2", "https://example.com/new.png"),
        ];
        drop_stale_media(&mut items);
        assert!(!has_media(&items[2]));
        assert!(has_media(&items[4]));
        match &items[2] {
            Item::FunctionCallOutput(output) => match &output.output {
                FunctionCallOutput::Text(text) => {
                    assert!(text.starts_with("caption\n"));
                    assert!(text.contains("already viewed"));
                }
                other => panic!("stale tool output should collapse to text, got {other:?}"),
            },
            other => panic!("expected function_call_output, got {other:?}"),
        }
    }

    fn reasoning() -> Item {
        Item::Reasoning(crate::authority::responses::ReasoningItem {
            id: None,
            summary: vec![],
            content: None,
            encrypted_content: Some("sig".into()),
            status: None,
        })
    }

    #[test]
    fn only_the_answer_to_a_stripped_batch_is_rebased() {
        let mut items = vec![
            user_text("go"),
            reasoning(),
            call("c1"),
            tool_image("c1", "https://example.com/old.png"),
            user_text("<system-reminder>\nnote\n</system-reminder>"),
            reasoning(),
            call("c2"),
            tool_image("c2", "https://example.com/new.png"),
        ];
        let rebased = drop_stale_media(&mut items);
        assert!(!has_media(&items[3]));
        assert_eq!(
            rebased,
            vec![false, false, false, false, false, true, true, false]
        );
    }

    #[test]
    fn user_media_from_an_earlier_turn_is_dropped() {
        let mut items = vec![
            user_image("litecode-media:old.png"),
            call("c1"),
            tool_image("c1", "https://example.com/shot.png"),
            assistant_text("done"),
            user_image("litecode-media:new.png"),
        ];
        drop_stale_media(&mut items);
        assert!(!has_media(&items[0]));
        assert!(!has_media(&items[2]));
        assert!(has_media(&items[4]));
        match &items[0] {
            Item::Message(MessageItem::Input(message)) => {
                assert!(message.content.iter().any(|part| matches!(
                    part,
                    InputContent::InputText(text) if text.text.contains("already viewed")
                )));
            }
            other => panic!("expected user message, got {other:?}"),
        }
    }

    #[test]
    fn durable_media_replacement_rebuilds_messages_signed_reasoning_rebase() {
        use crate::authority::responses::ReasoningItem;

        let items = vec![
            user_text("[image already viewed; removed from context]"),
            Item::Reasoning(ReasoningItem {
                id: Some("rs_media".into()),
                summary: vec![],
                content: None,
                encrypted_content: Some("signature-bound-to-old-media".into()),
                status: None,
            }),
            assistant_text("done"),
            user_text("next turn"),
        ];
        let rebased = rebased_after_media_replacements(&items, &[true, false, false, false]);
        assert_eq!(rebased, vec![false, true, true, false]);

        let mut request_items = items.clone();
        let stripped =
            crate::llm::strip_rebased_signatures(&mut request_items, &rebased, "messages");
        assert_eq!(stripped, 1, "Messages must strip the rebased signature");
        let Item::Reasoning(reasoning) = &request_items[1] else {
            panic!("expected reasoning");
        };
        assert!(
            reasoning.encrypted_content.is_none(),
            "signed reasoning after a durable media replacement must be rebased"
        );
    }
}
