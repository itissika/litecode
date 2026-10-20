pub mod error;
pub mod media;
pub mod result;
pub mod transcript;
pub mod utf8;

#[cfg(test)]
mod death_list_gate;
#[cfg(test)]
mod golden_item_fixtures;

pub use error::{LitecodeError, Result};
pub use media::{MediaArtifact, MediaKind, MediaSource, ToolOutputPart};
pub use result::{ToolCallResult, ToolError, ToolErrorKind, ToolSignalLevel};
pub use transcript::{
    FunctionCallOutputItemParam, FunctionToolCall, InputItem, Item, OutputItem, OutputMessage,
    ReasoningItem, Response, StreamEvents, Transcript, UserInput, assistant_text,
    item_text_preview, user_message, user_text,
};

/// WP-098 L5: who stopped or deleted a turn/session. Persisted as
/// `turn/end.cancelled_by` and carried on `ParentNotice` / `TurnFinished`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopOrigin {
    User,
    Parent,
}

impl StopOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Parent => "parent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "parent" => Some(Self::Parent),
            _ => None,
        }
    }
}
