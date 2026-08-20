//! Session log envelope: `seq` is identity and order. Payload remains Responses [`Item`].

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::types::{Item, LitecodeError, Result, item_text_preview};

use super::surface::{Surface, SurfaceOp, fold_surface};

/// Monotonic log position. Never rewritten, never reused.
pub type Seq = u64;

/// Discriminator stored as the mental-model `type` strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    ItemUser,
    ItemAssistant,
    ItemToolCall,
    ItemToolResult,
    TurnStart,
    TurnEnd,
    StepStart,
    StepEnd,
    AssistantChunk,
    RequestHeader,
    RequestContext,
    Unknown(String),
}

impl Serialize for EventType {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_str_name(&s))
    }
}

impl EventType {
    pub fn from_str_name(s: &str) -> Self {
        match s {
            "item/user" => Self::ItemUser,
            "item/assistant" => Self::ItemAssistant,
            "item/tool_call" => Self::ItemToolCall,
            "item/tool_result" => Self::ItemToolResult,
            "turn/start" => Self::TurnStart,
            "turn/end" => Self::TurnEnd,
            "step/start" => Self::StepStart,
            "step/end" => Self::StepEnd,
            "assistant/chunk" => Self::AssistantChunk,
            "request/header" => Self::RequestHeader,
            "request/context" => Self::RequestContext,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::ItemUser => "item/user",
            Self::ItemAssistant => "item/assistant",
            Self::ItemToolCall => "item/tool_call",
            Self::ItemToolResult => "item/tool_result",
            Self::TurnStart => "turn/start",
            Self::TurnEnd => "turn/end",
            Self::StepStart => "step/start",
            Self::StepEnd => "step/end",
            Self::AssistantChunk => "assistant/chunk",
            Self::RequestHeader => "request/header",
            Self::RequestContext => "request/context",
            Self::Unknown(s) => s.as_str(),
        }
    }

    pub fn is_surface_eligible(&self) -> bool {
        matches!(
            self,
            Self::ItemUser | Self::ItemAssistant | Self::ItemToolCall | Self::ItemToolResult
        )
    }
}

/// One append-only log row. `seq` is required; it is never optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: Seq,
    pub time: i64,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub data: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_op: Option<SurfaceOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_seqs: Option<Vec<Seq>>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignorable: bool,
}

/// Caller-supplied fields for [`EventLog::append`]. `seq` is assigned by the log.
#[derive(Debug, Clone)]
pub struct EventDraft {
    pub time: i64,
    pub event_type: EventType,
    pub data: Value,
    pub surface_op: Option<SurfaceOp>,
    pub source_seqs: Option<Vec<Seq>>,
    pub ignorable: bool,
}

impl EventDraft {
    pub fn surface_item(event_type: EventType, item: &Item, surface_op: SurfaceOp) -> Result<Self> {
        Ok(Self {
            time: 0,
            event_type,
            data: serde_json::to_value(item)?,
            surface_op: Some(surface_op),
            source_seqs: None,
            ignorable: false,
        })
    }
}

/// In-memory append-only log. Persistence is a later ticket.
#[derive(Debug, Clone, Default)]
pub struct EventLog {
    events: Vec<SessionEvent>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn next_seq(&self) -> Seq {
        self.events.last().map(|e| e.seq + 1).unwrap_or(0)
    }

    /// Assign `seq`, freeze `data` as JSON, validate surface transfer. Failed drafts do not enter.
    pub fn append(&mut self, draft: EventDraft) -> Result<&SessionEvent> {
        if matches!(&draft.event_type, EventType::Unknown(_)) && !draft.ignorable {
            return Err(LitecodeError::InvalidSessionEvent(format!(
                "unknown type `{}` is not ignorable",
                draft.event_type.as_str()
            )));
        }

        let frozen: Value = serde_json::from_str(&draft.data.to_string())?;

        if draft.event_type.is_surface_eligible() {
            if draft.surface_op.is_none() {
                return Err(LitecodeError::InvalidSessionEvent(
                    "surface-eligible event must carry surface_op".into(),
                ));
            }
            serde_json::from_value::<Item>(frozen.clone()).map_err(|e| {
                LitecodeError::InvalidSessionEvent(format!("surface data is not an Item: {e}"))
            })?;
        } else if draft.surface_op.is_some() {
            return Err(LitecodeError::InvalidSessionEvent(
                "log-only event must not carry surface_op".into(),
            ));
        }

        let seq = self.next_seq();
        let event = SessionEvent {
            seq,
            time: draft.time,
            event_type: draft.event_type,
            data: frozen,
            surface_op: draft.surface_op,
            source_seqs: draft.source_seqs,
            ignorable: draft.ignorable,
        };

        validate_surface_transfer(&self.events, &event)?;
        self.events.push(event);
        Ok(self.events.last().expect("just pushed"))
    }
}

fn validate_surface_transfer(existing: &[SessionEvent], incoming: &SessionEvent) -> Result<()> {
    let Some(op) = incoming.surface_op.as_ref() else {
        return Ok(());
    };
    let surface = fold_surface(existing)?;
    match op {
        SurfaceOp::Append => Ok(()),
        SurfaceOp::Replace { start, end } => {
            let shadowed = shadowed_nodes(&surface, *start, *end)?;
            let Some(sources) = incoming.source_seqs.as_ref() else {
                return Err(LitecodeError::InvalidSessionEvent(
                    "replace requires source_seqs covering shadowed surface nodes".into(),
                ));
            };
            for seq in &shadowed {
                if !sources.contains(seq) {
                    return Err(LitecodeError::InvalidSessionEvent(format!(
                        "source_seqs missing shadowed seq {seq}"
                    )));
                }
            }
            Ok(())
        }
    }
}

pub(crate) fn shadowed_nodes(surface: &Surface, start: Seq, end: Seq) -> Result<Vec<Seq>> {
    let start_i = surface
        .nodes
        .iter()
        .position(|s| *s == start)
        .ok_or_else(|| {
            LitecodeError::InvalidSessionEvent(format!("replace start {start} is not on surface"))
        })?;
    let end_i = surface
        .nodes
        .iter()
        .position(|s| *s == end)
        .ok_or_else(|| {
            LitecodeError::InvalidSessionEvent(format!("replace end {end} is not on surface"))
        })?;
    if start_i > end_i {
        return Err(LitecodeError::InvalidSessionEvent(
            "replace start is after end on surface".into(),
        ));
    }
    Ok(surface.nodes[start_i..=end_i].to_vec())
}

pub fn item_from_event(event: &SessionEvent) -> Result<Item> {
    serde_json::from_value(event.data.clone()).map_err(Into::into)
}

pub fn skip_empty_assistant(event: &SessionEvent, item: &Item) -> bool {
    matches!(event.event_type, EventType::ItemAssistant) && item_text_preview(item).is_empty()
}
