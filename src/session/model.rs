//! Durable session-domain types.
//!
//! This module deliberately keeps product semantics outside Responses `Item`.
//! `Item` remains the payload of `item/*` rows and an AgentView output only.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::event::Seq;

/// Bumped whenever a reader cannot faithfully reconstruct the durable session
/// model from a prior database shape.
pub const SESSION_LOG_SCHEMA_VERSION: u32 = 3;

/// Stable discriminator for a durable session-log row.
///
/// Unknown values are only safe to retain when `ignorable` is true on the row.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    #[serde(rename = "item/user")]
    ItemUser,
    #[serde(rename = "item/assistant")]
    ItemAssistant,
    #[serde(rename = "item/tool_call")]
    ItemToolCall,
    #[serde(rename = "item/tool_result")]
    ItemToolResult,
    #[serde(rename = "compacted")]
    Compacted,
    #[serde(rename = "reminder/job_exit")]
    ReminderJobExit,
    #[serde(rename = "turn/start")]
    TurnStart,
    #[serde(rename = "turn/end")]
    TurnEnd,
    #[serde(rename = "request/header")]
    RequestHeader,
    #[serde(rename = "request/context")]
    RequestContext,
    #[serde(untagged)]
    Unknown(String),
}

impl SessionKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ItemUser => "item/user",
            Self::ItemAssistant => "item/assistant",
            Self::ItemToolCall => "item/tool_call",
            Self::ItemToolResult => "item/tool_result",
            Self::Compacted => "compacted",
            Self::ReminderJobExit => "reminder/job_exit",
            Self::TurnStart => "turn/start",
            Self::TurnEnd => "turn/end",
            Self::RequestHeader => "request/header",
            Self::RequestContext => "request/context",
            Self::Unknown(value) => value,
        }
    }

    pub fn enters_spine(&self) -> bool {
        matches!(
            self,
            Self::ItemUser
                | Self::ItemAssistant
                | Self::ItemToolCall
                | Self::ItemToolResult
                | Self::Compacted
                | Self::ReminderJobExit
        )
    }

    pub fn is_item(&self) -> bool {
        matches!(
            self,
            Self::ItemUser | Self::ItemAssistant | Self::ItemToolCall | Self::ItemToolResult
        )
    }
}

/// The durable status of a row whose producer can stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogState {
    #[default]
    Final,
    InProgress,
    Aborted,
}

impl LogState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Final => "final",
            Self::InProgress => "in_progress",
            Self::Aborted => "aborted",
        }
    }

    pub fn from_str_name(s: &str) -> Self {
        match s {
            "in_progress" => Self::InProgress,
            "aborted" => Self::Aborted,
            _ => Self::Final,
        }
    }
}

/// The durable, schema-versioned session-log envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionLogRow {
    pub seq: Seq,
    pub time: i64,
    pub kind: SessionKind,
    pub body: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cites: Vec<Seq>,
    #[serde(default)]
    pub state: LogState,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignorable: bool,
}

/// Body for `compacted`. `[from, to)` is a half-open spine interval:
/// `from` is the first replaced surface seq; `to` is the first kept seq, or
/// the compact row's own seq when the keep is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactedBody {
    pub summary: String,
    pub from: Seq,
    pub to: Seq,
}

impl CompactedBody {
    /// AgentView assembly: synthetic assistant `Item`. Never a user message.
    pub fn agent_item(&self) -> crate::types::Item {
        crate::types::assistant_text(&self.summary)
    }
}

fn tagged_user_item(kind: &str, text: &str) -> crate::types::Item {
    crate::types::user_text(format!("[{kind}]\n{text}"))
}

/// Body for `reminder/job_exit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderJobExitBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub reason: ReminderJobExitReason,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderJobExitReason {
    Exit,
    Kill,
    Timeout,
}

impl ReminderJobExitBody {
    pub fn agent_item(&self) -> crate::types::Item {
        let label = match self.job_id.as_deref() {
            Some(id) => format!("reminder/job_exit {} {id}", self.reason.as_str()),
            None => format!("reminder/job_exit {}", self.reason.as_str()),
        };
        tagged_user_item(&label, &self.text)
    }
}

impl ReminderJobExitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "exit",
            Self::Kill => "kill",
            Self::Timeout => "timeout",
        }
    }
}

/// The only persisted SessionMeta fields. Live and catalog projections do not
/// belong in this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub project: String,
    pub created_at: i64,
    pub parent_session_id: Option<String>,
    pub parent_call_id: Option<String>,
    pub subagent_depth: u32,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub thinking_tier: String,
    pub context_mode: String,
    pub updated_at: i64,
    pub compacted_seq: Option<Seq>,
    pub spine_from: Seq,
    pub todos: Vec<Value>,
    pub plan_slug: Option<String>,
    pub preview: String,
}
