//! Surface projection over an append-only [`super::event::SessionEvent`] log.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::event::{
    EventType, Seq, SessionEvent, item_from_event, shadowed_nodes, skip_empty_assistant,
};
use crate::types::{Item, LitecodeError, Result};

/// Transfer applied by a surface-eligible event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceOp {
    Append,
    Replace { start: Seq, end: Seq },
}

impl Serialize for SurfaceOp {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Append => serializer.serialize_str("append"),
            Self::Replace { start, end } => {
                #[derive(Serialize)]
                struct ReplaceWire {
                    op: &'static str,
                    start: Seq,
                    end: Seq,
                }
                ReplaceWire {
                    op: "replace",
                    start: *start,
                    end: *end,
                }
                .serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for SurfaceOp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.as_str() == Some("append") {
            return Ok(Self::Append);
        }
        #[derive(Deserialize)]
        struct ReplaceWire {
            op: String,
            start: Seq,
            end: Seq,
        }
        let wire: ReplaceWire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        if wire.op != "replace" {
            return Err(serde::de::Error::custom(format!(
                "unknown surface_op {}",
                wire.op
            )));
        }
        Ok(Self::Replace {
            start: wire.start,
            end: wire.end,
        })
    }
}

/// Folded model-visible order. Not a fourth data layer: derived only.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Surface {
    pub nodes: Vec<Seq>,
    pub replace_generation: u64,
}

/// Replay surface transfers. Unknown non-ignorable types refuse the whole session.
pub fn fold_surface(events: &[SessionEvent]) -> Result<Surface> {
    let mut surface = Surface::default();
    for event in events {
        if matches!(event.event_type, EventType::Unknown(_)) && !event.ignorable {
            return Err(LitecodeError::InvalidSessionEvent(format!(
                "unknown type `{}` is not ignorable",
                event.event_type.as_str()
            )));
        }
        let Some(op) = event.surface_op.as_ref() else {
            continue;
        };
        if !event.event_type.is_surface_eligible() {
            return Err(LitecodeError::InvalidSessionEvent(format!(
                "non-surface type `{}` carries surface_op",
                event.event_type.as_str()
            )));
        }
        match op {
            SurfaceOp::Append => surface.nodes.push(event.seq),
            SurfaceOp::Replace { start, end } => {
                let shadowed = shadowed_nodes(&surface, *start, *end)?;
                let start_i = surface
                    .nodes
                    .iter()
                    .position(|s| *s == *start)
                    .expect("shadowed_nodes found start");
                surface
                    .nodes
                    .splice(start_i..start_i + shadowed.len(), [event.seq]);
                surface.replace_generation += 1;
            }
        }
    }
    Ok(surface)
}

/// Model-visible Items in `surface.nodes` order.
/// Usage-only assistant rows (empty content) are omitted from the Item[] projection;
/// the seq remains on `surface.nodes`.
pub fn derive_messages(events: &[SessionEvent]) -> Result<Vec<Item>> {
    let surface = fold_surface(events)?;
    let mut out = Vec::with_capacity(surface.nodes.len());
    for seq in surface.nodes {
        let event = events
            .iter()
            .find(|e| e.seq == seq)
            .ok_or_else(|| LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing")))?;
        let item = item_from_event(event)?;
        if skip_empty_assistant(event, &item) {
            continue;
        }
        out.push(item);
    }
    Ok(out)
}

/// Human transcript: append-origin surface events only, seq ascending. Replace copies omitted.
pub fn derive_transcript_items(events: &[SessionEvent]) -> Result<Vec<Item>> {
    let mut origin: Vec<&SessionEvent> = events
        .iter()
        .filter(|event| event.surface_op.as_ref() == Some(&SurfaceOp::Append))
        .collect();
    origin.sort_by_key(|event| event.seq);
    let mut out = Vec::with_capacity(origin.len());
    for event in origin {
        let item = item_from_event(event)?;
        if skip_empty_assistant(event, &item) {
            continue;
        }
        out.push(item);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::event::{EventDraft, EventLog, EventType};
    use crate::types::{item_text_preview, user_text};

    fn append_user(log: &mut EventLog, text: &str) {
        log.append(
            EventDraft::surface_item(EventType::ItemUser, &user_text(text), SurfaceOp::Append)
                .expect("draft"),
        )
        .expect("append");
    }

    #[test]
    fn replace_puts_summary_where_shadowed_range_was_not_at_log_tail() {
        let mut log = EventLog::new();
        append_user(&mut log, "d0");
        append_user(&mut log, "d1");
        append_user(&mut log, "d2");
        append_user(&mut log, "d3");
        append_user(&mut log, "d4");
        assert_eq!(log.next_seq(), 5);

        let mut summary = EventDraft::surface_item(
            EventType::ItemUser,
            &user_text("summary"),
            SurfaceOp::Replace { start: 0, end: 1 },
        )
        .expect("draft");
        summary.source_seqs = Some(vec![0, 1]);
        log.append(summary).expect("replace");

        let pre = fold_surface(&log.events()[..5]).expect("pre");
        let shadowed = crate::session::event::shadowed_nodes(&pre, 0, 1).expect("shadowed");
        assert_eq!(shadowed, vec![0, 1]);
        assert_eq!(
            &pre.nodes[shadowed.len()..],
            &[2, 3, 4],
            "cut is the shadowed/unshadowed surface boundary, not the log tail"
        );

        let messages = derive_messages(log.events()).expect("derive_messages");
        let texts: Vec<_> = messages.iter().map(item_text_preview).collect();
        assert_eq!(texts, vec!["summary", "d2", "d3", "d4"]);
        let after = fold_surface(log.events()).expect("after");
        assert_eq!(after.nodes, vec![5, 2, 3, 4]);
        assert_ne!(
            after.nodes[0],
            *shadowed.last().expect("non-empty"),
            "replace seq is not the cut"
        );

        let transcript = derive_transcript_items(log.events()).expect("transcript");
        let t: Vec<_> = transcript.iter().map(item_text_preview).collect();
        assert_eq!(t, vec!["d0", "d1", "d2", "d3", "d4"]);
    }

    #[test]
    fn transcript_is_append_origin_sorted_by_seq_not_slice_order() {
        let mut log = EventLog::new();
        append_user(&mut log, "d0");
        append_user(&mut log, "d1");
        let mut shuffled = log.events().to_vec();
        shuffled.reverse();
        let transcript = derive_transcript_items(&shuffled).expect("transcript");
        let t: Vec<_> = transcript.iter().map(item_text_preview).collect();
        assert_eq!(t, vec!["d0", "d1"]);
    }

    #[test]
    fn failed_append_does_not_enter_the_log() {
        let mut log = EventLog::new();
        append_user(&mut log, "only");
        let before = log.events().len();

        let mut bad = EventDraft::surface_item(
            EventType::ItemUser,
            &user_text("nope"),
            SurfaceOp::Replace { start: 9, end: 9 },
        )
        .expect("draft");
        bad.source_seqs = Some(vec![9]);
        assert!(log.append(bad).is_err());
        assert_eq!(log.events().len(), before);
        assert_eq!(log.next_seq(), 1);
    }

    #[test]
    fn unknown_non_ignorable_type_is_rejected() {
        let mut log = EventLog::new();
        let err = log
            .append(EventDraft {
                time: 0,
                event_type: EventType::Unknown("plugin/mystery".into()),
                data: serde_json::json!({}),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
            })
            .unwrap_err();
        assert!(matches!(err, LitecodeError::InvalidSessionEvent(_)));
    }

    #[test]
    fn session_event_seq_is_required_not_optional() {
        let mut log = EventLog::new();
        append_user(&mut log, "x");
        let event = &log.events()[0];
        let seq: Seq = event.seq;
        assert_eq!(seq, 0);
        assert!(event.surface_op.is_some());
        let _ = event.source_seqs.as_ref();
    }
}
