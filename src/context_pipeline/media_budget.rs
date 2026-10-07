//! Carried-media cap for the ephemeral model view, and a full strip for compact.
//!
//! The view keeps at most [`MAX_CARRIED_MEDIA_PARTS`] image/file parts. Older
//! parts become a short text note. The newest user message is preferred: other
//! media is removed first, and that message is trimmed only when its own parts
//! still exceed the cap. The persisted transcript is never mutated.
//!
//! Compaction serializes history as one text prompt. [`strip_media_for_summary`]
//! removes every media part before that JSON is built, so stored data URLs are
//! not sent to the summarizer.

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputRole, InputTextContent,
    MessageItem,
};
use crate::types::Item;

/// Image/file parts an ephemeral model view may still carry.
pub const MAX_CARRIED_MEDIA_PARTS: usize = 2;

/// Apply the carried-media cap to a transcript view (ephemeral LLM view only).
pub fn apply_carried_media_budget(items: &mut [Item]) {
    retain_newest_media(items, MAX_CARRIED_MEDIA_PARTS, true);
}

/// Remove every image/file part before a transcript is serialized for compact.
pub fn strip_media_for_summary(items: &mut [Item]) {
    retain_newest_media(items, 0, false);
}

fn retain_newest_media(items: &mut [Item], max_parts: usize, protect_latest_user: bool) {
    let protected = if protect_latest_user {
        last_user_index(items)
    } else {
        None
    };
    let mut to_strip = count_media_parts(items).saturating_sub(max_parts);
    if to_strip == 0 {
        return;
    }

    for (index, item) in items.iter_mut().enumerate() {
        if to_strip == 0 {
            break;
        }
        if Some(index) == protected {
            continue;
        }
        to_strip = strip_oldest_media(item, to_strip);
    }
    if to_strip > 0
        && let Some(index) = protected
    {
        strip_oldest_media(&mut items[index], to_strip);
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

fn count_media_parts(items: &[Item]) -> usize {
    items.iter().map(item_media_parts).sum()
}

fn item_media_parts(item: &Item) -> usize {
    match item {
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
    }
}

fn is_media(part: &InputContent) -> bool {
    matches!(
        part,
        InputContent::InputImage(_) | InputContent::InputFile(_)
    )
}

fn strip_oldest_media(item: &mut Item, to_strip: usize) -> usize {
    if to_strip == 0 {
        return 0;
    }
    match item {
        Item::FunctionCallOutput(output) => strip_tool_output(output, to_strip),
        Item::Message(MessageItem::Input(message)) => {
            strip_input_media(&mut message.content, to_strip)
        }
        _ => to_strip,
    }
}

fn strip_input_media(parts: &mut [InputContent], mut to_strip: usize) -> usize {
    for part in parts.iter_mut() {
        if to_strip == 0 {
            break;
        }
        let kind = match part {
            InputContent::InputImage(_) => "image",
            InputContent::InputFile(_) => "file",
            InputContent::InputText(_) => continue,
        };
        *part = InputContent::InputText(InputTextContent {
            text: format!("[media trimmed: {kind}]"),
        });
        to_strip -= 1;
    }
    to_strip
}

fn strip_tool_output(output: &mut FunctionCallOutputItemParam, mut to_strip: usize) -> usize {
    let FunctionCallOutput::Content(parts) = &mut output.output else {
        return to_strip;
    };
    let mut kept = Vec::with_capacity(parts.len());
    let mut stripped = 0usize;
    for part in parts.drain(..) {
        let drop_part = is_media(&part) && to_strip > 0;
        if drop_part {
            to_strip -= 1;
            stripped += 1;
            continue;
        }
        kept.push(part);
    }
    let collapse = stripped > 0
        && kept
            .iter()
            .all(|part| matches!(part, InputContent::InputText(_)));
    if collapse {
        let text = kept
            .iter()
            .filter_map(|part| match part {
                InputContent::InputText(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let note = if text.is_empty() {
            format!("[media trimmed: {stripped} part(s)]")
        } else {
            format!("{text}\n[media trimmed: {stripped} part(s)]")
        };
        output.output = FunctionCallOutput::Text(note);
    } else {
        *parts = kept;
    }
    to_strip
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::InputImageContent;

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

    fn image_urls(item: &Item) -> Vec<String> {
        let parts: Vec<&InputContent> = match item {
            Item::Message(MessageItem::Input(message)) => message.content.iter().collect(),
            Item::FunctionCallOutput(output) => match &output.output {
                FunctionCallOutput::Content(parts) => parts.iter().collect(),
                FunctionCallOutput::Text(_) => return Vec::new(),
            },
            _ => return Vec::new(),
        };
        parts
            .into_iter()
            .filter_map(|part| match part {
                InputContent::InputImage(image) => image.image_url.clone(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn under_cap_keeps_every_media_part() {
        let mut items = vec![
            tool_image("c1", "https://example.com/a.png"),
            crate::types::user_message("", &["https://example.com/b.png".into()]),
        ];
        apply_carried_media_budget(&mut items);
        assert_eq!(count_media_parts(&items), 2);
        assert_eq!(
            image_urls(&items[0]),
            vec!["https://example.com/a.png".to_string()]
        );
        assert_eq!(
            image_urls(&items[1]),
            vec!["https://example.com/b.png".to_string()]
        );
    }

    #[test]
    fn cap_drops_oldest_and_keeps_two() {
        let mut items = vec![
            tool_image("c1", "https://example.com/old.png"),
            tool_image("c2", "https://example.com/mid.png"),
            tool_image("c3", "https://example.com/new.png"),
        ];
        apply_carried_media_budget(&mut items);
        assert_eq!(count_media_parts(&items), 2);
        match &items[0] {
            Item::FunctionCallOutput(output) => match &output.output {
                FunctionCallOutput::Text(text) => assert!(text.contains("media trimmed")),
                other => panic!("oldest tool image should be trimmed, got {other:?}"),
            },
            other => panic!("expected function_call_output, got {other:?}"),
        }
        assert_eq!(
            image_urls(&items[1]),
            vec!["https://example.com/mid.png".to_string()]
        );
        assert_eq!(
            image_urls(&items[2]),
            vec!["https://example.com/new.png".to_string()]
        );
    }

    #[test]
    fn newest_user_message_is_preferred_over_older_media() {
        let mut items = vec![
            tool_image("c1", "https://example.com/old.png"),
            tool_image("c2", "https://example.com/mid.png"),
            crate::types::user_message("", &["https://example.com/user.png".into()]),
        ];
        apply_carried_media_budget(&mut items);
        assert_eq!(count_media_parts(&items), 2);
        assert!(image_urls(&items[0]).is_empty());
        assert_eq!(
            image_urls(&items[1]),
            vec!["https://example.com/mid.png".to_string()]
        );
        assert_eq!(
            image_urls(&items[2]),
            vec!["https://example.com/user.png".to_string()]
        );
    }

    #[test]
    fn newest_user_message_itself_is_capped_at_two() {
        let mut items = vec![crate::types::user_message(
            "",
            &[
                "https://example.com/a.png".into(),
                "https://example.com/b.png".into(),
                "https://example.com/c.png".into(),
            ],
        )];
        apply_carried_media_budget(&mut items);
        assert_eq!(
            image_urls(&items[0]),
            vec![
                "https://example.com/b.png".to_string(),
                "https://example.com/c.png".to_string(),
            ]
        );
        match &items[0] {
            Item::Message(MessageItem::Input(message)) => {
                assert!(message.content.iter().any(|part| matches!(
                    part,
                    InputContent::InputText(text) if text.text.contains("media trimmed")
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
        assert_eq!(count_media_parts(&items), 0);
        let rendered = format!("{items:?}");
        assert!(!rendered.contains("data:image"));
        assert!(!rendered.contains("litecode-media:"));
    }
}
