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
//! Compaction serializes history as one text prompt. [`strip_media_for_summary`]
//! removes every media part before that JSON is built, so stored data URLs are
//! not sent to the summarizer.

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputRole, InputTextContent,
    MessageItem,
};
use crate::types::Item;

/// Drop media the model has already seen (ephemeral LLM view and budget only).
pub fn drop_stale_media(items: &mut [Item]) {
    let tool_fresh_from = items
        .iter()
        .rposition(is_model_produced)
        .map_or(0, |index| index + 1);
    let user_fresh_from = current_turn_start(items);
    for (index, item) in items.iter_mut().enumerate() {
        let keep = match item {
            Item::FunctionCallOutput(_) => index >= tool_fresh_from,
            Item::Message(MessageItem::Input(_)) => index >= user_fresh_from,
            _ => true,
        };
        if !keep {
            strip_item_media(item);
        }
    }
}

/// Remove every image/file part before a transcript is serialized for compact.
pub fn strip_media_for_summary(items: &mut [Item]) {
    for item in items.iter_mut() {
        strip_item_media(item);
    }
}

fn is_model_produced(item: &Item) -> bool {
    matches!(
        item,
        Item::Message(MessageItem::Output(_)) | Item::Reasoning(_) | Item::FunctionCall(_)
    )
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

fn strip_item_media(item: &mut Item) {
    match item {
        Item::FunctionCallOutput(output) => strip_tool_output(output),
        Item::Message(MessageItem::Input(message)) => strip_input_media(&mut message.content),
        _ => {}
    }
}

fn strip_input_media(parts: &mut [InputContent]) {
    for part in parts.iter_mut() {
        let kind = match part {
            InputContent::InputImage(_) => "image",
            InputContent::InputFile(_) => "file",
            InputContent::InputText(_) => continue,
        };
        *part = InputContent::InputText(InputTextContent {
            text: format!("[{kind} already viewed; removed from context]"),
        });
    }
}

fn strip_tool_output(output: &mut FunctionCallOutputItemParam) {
    let FunctionCallOutput::Content(parts) = &mut output.output else {
        return;
    };
    let before = parts.len();
    parts.retain(|part| !is_media(part));
    let stripped = before - parts.len();
    if stripped == 0 {
        return;
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

    #[test]
    fn user_media_stays_for_the_whole_turn() {
        let mut items = vec![
            user_image("litecode-media:a.png"),
            assistant_text("checking"),
            call("c1"),
            tool_image("c1", "https://example.com/shot.png"),
            call("c2"),
            tool_image("c2", "https://example.com/shot2.png"),
            user_text("<system-reminder>\nnote\n</system-reminder>"),
        ];
        drop_stale_media(&mut items);
        assert!(has_media(&items[0]));
        assert!(!has_media(&items[3]));
        assert!(has_media(&items[5]));
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
    fn summary_strip_removes_every_media_part() {
        let blob = format!("data:image/png;base64,{}", "A".repeat(80));
        let mut items = vec![
            tool_image("c1", &blob),
            crate::types::user_message("", &["litecode-media:abc.png".into()]),
        ];
        strip_media_for_summary(&mut items);
        assert_eq!(media_count(&items), 0);
        let rendered = format!("{items:?}");
        assert!(!rendered.contains("data:image"));
        assert!(!rendered.contains("litecode-media:"));
    }
}
