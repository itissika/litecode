//! Surface projection over an append-only [`super::event::SessionEvent`] log.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::event::{
    EventType, Seq, SessionEvent, item_from_event, skip_empty_assistant_item,
    skip_unmatched_tool_output, spine_agent_item,
};
use super::model::CompactedBody;
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

/// Validated surface transition that has not mutated fold state yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfacePlan {
    Append {
        seq: Seq,
    },
    Replace {
        seq: Seq,
        start_idx: usize,
        len: usize,
    },
}

/// Locate an inclusive replace range on the current surface without mutating it.
pub fn shadowed_nodes(surface: &Surface, start: Seq, end: Seq) -> Result<Vec<Seq>> {
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

/// Locate a half-open spine interval `[from, to)` on the current surface.
///
/// `to` may be absent from the surface (empty keep: exclusive end is the
/// upcoming compact seq). Then the range runs through the surface tail.
pub fn shadowed_half_open(surface: &Surface, from: Seq, to: Seq) -> Result<Vec<Seq>> {
    if from == to {
        return Ok(Vec::new());
    }
    let start_i = surface
        .nodes
        .iter()
        .position(|s| *s == from)
        .ok_or_else(|| {
            LitecodeError::InvalidSessionEvent(format!("compact from {from} is not on surface"))
        })?;
    let end_i = match surface.nodes.iter().position(|s| *s == to) {
        Some(i) if i >= start_i => i,
        Some(_) => {
            return Err(LitecodeError::InvalidSessionEvent(
                "compact from is after to on surface".into(),
            ));
        }
        None => surface.nodes.len(),
    };
    Ok(surface.nodes[start_i..end_i].to_vec())
}

/// Validate one event against the current surface and return its fold transition.
pub fn plan_surface(surface: &Surface, event: &SessionEvent) -> Result<Option<SurfacePlan>> {
    if matches!(event.event_type, EventType::Unknown(_)) && !event.ignorable {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "unknown type `{}` is not ignorable",
            event.event_type.as_str()
        )));
    }
    if event.event_type.is_injection() {
        if event.surface_op.is_some() {
            return Err(LitecodeError::InvalidSessionEvent(format!(
                "injection type `{}` must not carry surface_op",
                event.event_type.as_str()
            )));
        }
        return Ok(Some(SurfacePlan::Append { seq: event.seq }));
    }
    if matches!(event.event_type, EventType::Compacted) {
        let body: CompactedBody = serde_json::from_value(event.data.clone()).map_err(|e| {
            LitecodeError::InvalidSessionEvent(format!("invalid compacted body: {e}"))
        })?;
        if surface.nodes.is_empty() || body.from == body.to {
            // A summary-only compact starts a new spine and therefore has no
            // predecessor range to replace.
            return Ok(Some(SurfacePlan::Append { seq: event.seq }));
        }
        let shadowed = shadowed_half_open(surface, body.from, body.to)?;
        if shadowed.is_empty() {
            return Ok(Some(SurfacePlan::Append { seq: event.seq }));
        }
        let start_idx = surface
            .nodes
            .iter()
            .position(|seq| *seq == body.from)
            .expect("shadowed_half_open found compact start");
        return Ok(Some(SurfacePlan::Replace {
            seq: event.seq,
            start_idx,
            len: shadowed.len(),
        }));
    }
    let Some(op) = event.surface_op.as_ref() else {
        return Ok(None);
    };
    if !event.event_type.is_surface_eligible() {
        return Err(LitecodeError::InvalidSessionEvent(format!(
            "non-surface type `{}` carries surface_op",
            event.event_type.as_str()
        )));
    }
    match op {
        SurfaceOp::Append => Ok(Some(SurfacePlan::Append { seq: event.seq })),
        SurfaceOp::Replace { start, end } => {
            let shadowed = shadowed_nodes(surface, *start, *end)?;
            let Some(sources) = event.source_seqs.as_ref() else {
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
            let start_idx = surface
                .nodes
                .iter()
                .position(|s| *s == *start)
                .expect("shadowed_nodes found start");
            Ok(Some(SurfacePlan::Replace {
                seq: event.seq,
                start_idx,
                len: shadowed.len(),
            }))
        }
    }
}

/// Commit one previously validated surface transition.
pub fn apply_plan(surface: &mut Surface, plan: SurfacePlan) {
    match plan {
        SurfacePlan::Append { seq } => surface.nodes.push(seq),
        SurfacePlan::Replace {
            seq,
            start_idx,
            len,
        } => {
            surface.nodes.splice(start_idx..start_idx + len, [seq]);
            surface.replace_generation += 1;
        }
    }
}

/// Replay surface transfers. Unknown non-ignorable types refuse the whole session.
pub fn fold_surface(events: &[SessionEvent]) -> Result<Surface> {
    let mut surface = Surface::default();
    for event in events {
        if let Some(plan) = plan_surface(&surface, event)? {
            apply_plan(&mut surface, plan);
        }
    }
    Ok(surface)
}

/// Model-visible Items in `surface.nodes` order.
/// Usage-only assistant rows (empty content) and unmatched tool outputs are
/// omitted from the Item[] projection; the seq remains on `surface.nodes`.
pub fn derive_messages(events: &[SessionEvent]) -> Result<Vec<Item>> {
    let surface = fold_surface(events)?;
    let mut loaded = Vec::with_capacity(surface.nodes.len());
    for seq in surface.nodes {
        let event = events.iter().find(|e| e.seq == seq).ok_or_else(|| {
            LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing"))
        })?;
        let item = spine_agent_item(event)?;
        loaded.push((event, item));
    }
    let valid_call_ids: std::collections::HashSet<String> = loaded
        .iter()
        .filter_map(|(_, item)| match item {
            Item::FunctionCall(fc) => Some(fc.call_id.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::with_capacity(loaded.len());
    for (_event, item) in loaded {
        if skip_empty_assistant_item(&item) {
            continue;
        }
        if skip_unmatched_tool_output(&item, &valid_call_ids) {
            continue;
        }
        out.push(item);
    }
    Ok(out)
}

/// Model-visible `(seq, Item)` pairs in surface order, with the same skip rules
/// as [`derive_messages`].
pub fn project_working_pairs(
    surface: &Surface,
    lookup: impl Fn(Seq) -> Result<Item>,
) -> Result<Vec<(Seq, Item)>> {
    let mut loaded = Vec::with_capacity(surface.nodes.len());
    for seq in &surface.nodes {
        loaded.push((*seq, lookup(*seq)?));
    }
    let valid_call_ids: std::collections::HashSet<String> = loaded
        .iter()
        .filter_map(|(_, item)| match item {
            Item::FunctionCall(fc) => Some(fc.call_id.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::with_capacity(loaded.len());
    for (seq, item) in loaded {
        if skip_empty_assistant_item(&item) {
            continue;
        }
        if skip_unmatched_tool_output(&item, &valid_call_ids) {
            continue;
        }
        out.push((seq, item));
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
    let mut loaded = Vec::with_capacity(origin.len());
    for event in origin {
        if event.event_type.is_injection() {
            continue;
        }
        loaded.push((event, item_from_event(event)?));
    }
    let valid_call_ids: std::collections::HashSet<String> = loaded
        .iter()
        .filter_map(|(_, item)| match item {
            Item::FunctionCall(fc) => Some(fc.call_id.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::with_capacity(loaded.len());
    for (_event, item) in loaded {
        if skip_empty_assistant_item(&item) {
            continue;
        }
        if skip_unmatched_tool_output(&item, &valid_call_ids) {
            continue;
        }
        out.push(item);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{AssistantRole, MessageItem};
    use crate::session::event::{EventDraft, EventLog, EventType};
    use crate::session::model::CompactedBody;
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
        let shadowed = shadowed_nodes(&pre, 0, 1).expect("shadowed");
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
                state: crate::session::model::LogState::Final,
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

    #[test]
    fn incremental_surface_matches_full_fold() {
        let mut log = EventLog::new();
        append_user(&mut log, "d0");
        append_user(&mut log, "d1");
        append_user(&mut log, "d2");
        let mut summary = EventDraft::surface_item(
            EventType::ItemUser,
            &user_text("summary"),
            SurfaceOp::Replace { start: 0, end: 1 },
        )
        .expect("draft");
        summary.source_seqs = Some(vec![0, 1]);
        log.append(summary).expect("replace");
        append_user(&mut log, "d3");

        assert_eq!(
            log.surface().nodes,
            fold_surface(log.events()).expect("fold").nodes
        );
        assert_eq!(log.surface().nodes, vec![3, 2, 4]);
    }

    fn append_compacted(log: &mut EventLog, summary: &str, from: Seq, to: Seq) {
        log.append(EventDraft {
            time: 0,
            event_type: EventType::Compacted,
            data: serde_json::to_value(&CompactedBody {
                summary: summary.into(),
                from,
                to,
            })
            .expect("json"),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("append compacted");
    }

    #[test]
    fn compacted_half_open_replaces_prefix_and_synthesizes_assistant() {
        let mut log = EventLog::new();
        append_user(&mut log, "d0");
        append_user(&mut log, "d1");
        append_user(&mut log, "d2");
        append_compacted(&mut log, "rolled", 0, 2);

        let after = fold_surface(log.events()).expect("fold");
        assert_eq!(after.nodes, vec![3, 2]);
        let messages = derive_messages(log.events()).expect("agent view");
        match &messages[0] {
            Item::Message(MessageItem::Output(out)) => {
                assert_eq!(out.role, AssistantRole::Assistant);
                assert_eq!(item_text_preview(&messages[0]), "rolled");
            }
            other => panic!("compact summary must be assistant Item, got {other:?}"),
        }
        assert_eq!(item_text_preview(&messages[1]), "d2");
        let transcript = derive_transcript_items(log.events()).expect("human");
        let t: Vec<_> = transcript.iter().map(item_text_preview).collect();
        assert_eq!(t, vec!["d0", "d1", "d2"]);
    }

    #[test]
    fn compacted_empty_keep_uses_exclusive_end_past_surface() {
        let mut log = EventLog::new();
        append_user(&mut log, "a");
        append_user(&mut log, "b");
        append_compacted(&mut log, "all", 0, 2);
        assert_eq!(fold_surface(log.events()).expect("fold").nodes, vec![2]);
    }

    #[test]
    fn control_plane_kinds_are_absent_from_both_views() {
        let mut log = EventLog::new();
        append_user(&mut log, "hi");
        log.append(EventDraft {
            time: 0,
            event_type: EventType::TurnStart,
            data: serde_json::json!({"turn": "t1"}),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("turn/start");
        log.append(EventDraft {
            time: 0,
            event_type: EventType::TurnEnd,
            data: serde_json::json!({"turn": "t1", "reason": "completed"}),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("turn/end");
        assert!(EventType::TurnEnd.is_control_plane());
        assert_eq!(log.events().len(), 3);
        let agent = derive_messages(log.events()).expect("agent");
        assert_eq!(agent.len(), 1);
        assert_eq!(item_text_preview(&agent[0]), "hi");
        let human = derive_transcript_items(log.events()).expect("human");
        assert_eq!(human.len(), 1);
    }

    #[test]
    fn injection_kinds_append_to_spine_and_assemble_tagged_agent_items() {
        use crate::authority::responses::{InputRole, MessageItem};
        use crate::session::model::{
            HookPromptBody, ReminderJobExitBody, ReminderJobExitReason, ReminderTurnAbortedBody,
        };

        let mut log = EventLog::new();
        append_user(&mut log, "hi");
        log.append(EventDraft {
            time: 0,
            event_type: EventType::HookPrompt,
            data: serde_json::to_value(HookPromptBody {
                text: "from hook".into(),
                hook_run_id: "run-1".into(),
                placement: Some("pre_turn".into()),
            })
            .unwrap(),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("hook/prompt");
        log.append(EventDraft {
            time: 0,
            event_type: EventType::ReminderJobExit,
            data: serde_json::to_value(ReminderJobExitBody {
                job_id: Some("job-9".into()),
                reason: ReminderJobExitReason::Timeout,
                text: "job timed out".into(),
            })
            .unwrap(),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("job_exit");
        log.append(EventDraft {
            time: 0,
            event_type: EventType::ReminderTurnAborted,
            data: serde_json::to_value(ReminderTurnAbortedBody {
                text: "turn was cancelled".into(),
            })
            .unwrap(),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        })
        .expect("turn_aborted");
        assert!(EventType::HookPrompt.enters_spine());
        assert_eq!(
            fold_surface(log.events()).expect("fold").nodes,
            vec![0, 1, 2, 3]
        );
        let agent = derive_messages(log.events()).expect("agent");
        assert_eq!(agent.len(), 4);
        assert_eq!(item_text_preview(&agent[0]), "hi");
        for item in &agent[1..] {
            match item {
                crate::types::Item::Message(MessageItem::Input(input)) => {
                    assert_eq!(input.role, InputRole::User);
                }
                other => panic!("injection AgentView must be tagged user Item, got {other:?}"),
            }
        }
        assert!(item_text_preview(&agent[1]).contains("[hook/prompt run-1]"));
        assert!(item_text_preview(&agent[1]).contains("from hook"));
        assert!(item_text_preview(&agent[2]).contains("[reminder/job_exit timeout job-9]"));
        assert!(item_text_preview(&agent[3]).contains("[reminder/turn_aborted]"));
        let human = derive_transcript_items(log.events()).expect("human");
        assert_eq!(human.len(), 1);
        assert_eq!(item_text_preview(&human[0]), "hi");
    }
}
