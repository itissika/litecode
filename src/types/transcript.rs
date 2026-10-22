//! Agent transcript authority — OpenAI Responses `Item` model.
//!
//! **Hard rule:** do not define parallel `Message` / `ContentBlock` types.
//! All atoms are [`async_openai::types::responses`] types (via [`crate::authority`]).

use crate::authority::responses::{
    AssistantRole, InputContent, InputImageContent, InputMessage, InputRole, InputTextContent,
    OutputMessageContent, OutputStatus, OutputTextContent,
};

/// What the composer sends: plain text plus `litecode-media:` image refs.
///
/// Refs stay refs until the ephemeral model view. Auto-turns and plan
/// execution use [`UserInput::text`] and carry no images.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserInput {
    pub text: String,
    pub images: Vec<String>,
}

impl UserInput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.images.is_empty()
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

pub use crate::authority::responses::{
    FunctionCallOutputItemParam, FunctionToolCall, InputItem, Item, MessageItem, OutputItem,
    OutputMessage, ReasoningItem, Response, ResponseStreamEvent,
};

/// Ordered Responses Items — sole kernel transcript working set / persistence atom sequence.
pub type Transcript = Vec<Item>;

/// Stream events from the Responses API — sole kernel stream authority (Phase 2 wires providers).
pub type StreamEvents = ResponseStreamEvent;

/// Build a user text `Item` using authority types only (no homemade message envelope).
pub fn user_text(text: impl Into<String>) -> Item {
    user_message(text, &[])
}

/// User `Item` with text and `input_image` parts.
///
/// `images` are `litecode-media:` refs (or any image URL the caller already
/// resolved). An empty text part is omitted so an image-only message stays an
/// image. Both empty still yields one empty text part.
pub fn user_message(text: impl Into<String>, images: &[String]) -> Item {
    let text = text.into();
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(InputContent::InputText(InputTextContent { text }));
    }
    for image_url in images {
        content.push(InputContent::InputImage(InputImageContent {
            detail: Default::default(),
            file_id: None,
            image_url: Some(image_url.clone()),
        }));
    }
    if content.is_empty() {
        content.push(InputContent::InputText(InputTextContent {
            text: String::new(),
        }));
    }
    Item::Message(MessageItem::Input(InputMessage {
        content,
        role: InputRole::User,
        status: None,
    }))
}

/// Build an assistant text `Item` (AgentView synthesis, never a user message).
pub fn assistant_text(text: impl Into<String>) -> Item {
    Item::Message(MessageItem::Output(OutputMessage {
        id: String::new(),
        role: AssistantRole::Assistant,
        content: vec![OutputMessageContent::OutputText(OutputTextContent {
            text: text.into(),
            annotations: vec![],
            logprobs: None,
        })],
        status: OutputStatus::Completed,
        phase: None,
    }))
}

/// Extract plain text preview from an Item (best-effort).
///
/// **Allowed:** logs, UI labels, revert/compact previews.
/// **Forbidden:** budget truth ([`crate::session::estimate::compute_token_estimate`]),
/// request rehydration, or any path that must preserve multimodal / structured semantics.
pub fn item_text_preview(item: &Item) -> String {
    match item {
        Item::Message(MessageItem::Input(msg)) => msg
            .content
            .iter()
            .filter_map(|c| match c {
                InputContent::InputText(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Item::Message(MessageItem::Output(msg)) => msg
            .content
            .iter()
            .filter_map(|c| match c {
                crate::authority::responses::OutputMessageContent::OutputText(t) => {
                    Some(t.text.as_str())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        // OpenAI-family reasoning arrives as `summary` parts and carries no
        // `content`; dialects that stream raw reasoning text do the opposite.
        // Either one is this item's text, so fall back rather than report empty.
        Item::Reasoning(r) => {
            let from_content = r
                .content
                .as_ref()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| match p {
                            crate::authority::responses::ReasoningItemContent::ReasoningText(t) => {
                                Some(t.text.as_str())
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if !from_content.is_empty() {
                return from_content;
            }
            r.summary
                .iter()
                .map(|part| match part {
                    crate::authority::responses::SummaryPart::SummaryText(t) => t.text.as_str(),
                })
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        Item::FunctionCall(fc) => format!("{}({})", fc.name, fc.arguments),
        Item::FunctionCallOutput(out) => match &out.output {
            crate::authority::responses::FunctionCallOutput::Text(s) => s.clone(),
            crate::authority::responses::FunctionCallOutput::Content(_) => {
                "[function_call_output content]".into()
            }
        },
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_text_item_roundtrip() {
        let item = user_text("hello");
        let v = serde_json::to_value(&item).expect("serialize");
        assert_eq!(v["type"], "message");
        assert_eq!(v["role"], "user");
        let back: Item = serde_json::from_value(v).expect("deserialize");
        assert_eq!(item_text_preview(&back), "hello");
    }

    #[test]
    fn assistant_text_item_is_output_assistant_role() {
        use crate::authority::responses::{AssistantRole, MessageItem};
        let item = assistant_text("hello");
        match item {
            Item::Message(MessageItem::Output(out)) => {
                assert_eq!(out.role, AssistantRole::Assistant);
            }
            other => panic!("expected assistant output, got {other:?}"),
        }
    }
}
