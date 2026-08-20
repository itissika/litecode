//! Session log envelope: `seq` is identity and order. Payload remains Responses [`Item`].

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::types::{Item, LitecodeError, Result, item_text_preview};

use crate::authority::responses::MessageItem;
use super::surface::{Surface, SurfaceOp, apply_plan, plan_surface};

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

/// In-memory append-only log with an incremental surface.
#[derive(Debug, Clone, Default)]
pub struct EventLog {
    events: Vec<SessionEvent>,
    surface: Surface,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_events(events: Vec<SessionEvent>) -> Result<Self> {
        let surface = super::surface::fold_surface(&events)?;
        Ok(Self { events, surface })
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn next_seq(&self) -> Seq {
        self.events.last().map(|e| e.seq + 1).unwrap_or(0)
    }

    /// Assign `seq`, freeze `data` as JSON, validate surface transfer. Failed drafts do not enter.
    pub fn append(&mut self, draft: EventDraft) -> Result<&SessionEvent> {
        let seq = self.next_seq();
        let event = finalize_draft(seq, draft)?;
        if let Some(plan) = plan_surface(&self.surface, &event)? {
            apply_plan(&mut self.surface, plan);
        }
        self.events.push(event);
        Ok(self.events.last().expect("just pushed"))
    }
}

/// Freeze a draft at `seq` without folding. Does not mutate any log.
pub fn finalize_draft(seq: Seq, draft: EventDraft) -> Result<SessionEvent> {
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

    Ok(SessionEvent {
        seq,
        time: draft.time,
        event_type: draft.event_type,
        data: frozen,
        surface_op: draft.surface_op,
        source_seqs: draft.source_seqs,
        ignorable: draft.ignorable,
    })
}

pub fn item_from_event(event: &SessionEvent) -> Result<Item> {
    serde_json::from_value(event.data.clone()).map_err(Into::into)
}

pub fn skip_empty_assistant_item(item: &Item) -> bool {
    matches!(item, Item::Message(MessageItem::Output(_))) && item_text_preview(item).is_empty()
}

pub fn skip_empty_assistant(event: &SessionEvent, item: &Item) -> bool {
    matches!(event.event_type, EventType::ItemAssistant) && skip_empty_assistant_item(item)
}

/// Unmatched tool output: omit from Surface Item[] like empty assistants. Do not DELETE the row.
pub fn skip_unmatched_tool_output(
    item: &Item,
    valid_call_ids: &std::collections::HashSet<String>,
) -> bool {
    matches!(
        item,
        Item::FunctionCallOutput(out) if !valid_call_ids.contains(&out.call_id)
    )
}
