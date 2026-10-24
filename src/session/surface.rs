//! Surface projection over an append-only [`super::event::SessionEvent`] log.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::event::{
    EventType, Seq, SessionEvent, item_from_event, skip_empty_assistant_item,
    skip_unmatched_tool_output, spine_agent_item,
};
use super::model::CompactedBody;
use super::working::WorkingPayload;
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

/// Durable working-set projection in surface order. Checkpoint payloads keep
/// their role-free identity; only Item payloads participate in item skip rules.
pub fn project_working_payloads(
    surface: &Surface,
    lookup: impl Fn(Seq) -> Result<WorkingPayload>,
) -> Result<Vec<(Seq, WorkingPayload)>> {
    let mut loaded = Vec::with_capacity(surface.nodes.len());
    for seq in &surface.nodes {
        loaded.push((*seq, lookup(*seq)?));
    }
    let valid_call_ids: std::collections::HashSet<String> = loaded
        .iter()
        .filter_map(|(_, payload)| match payload.as_item() {
            Some(Item::FunctionCall(fc)) => Some(fc.call_id.clone()),
            _ => None,
        })
        .collect();
    let mut out = Vec::with_capacity(loaded.len());
    for (seq, payload) in loaded {
        if let Some(item) = payload.as_item() {
            if skip_empty_assistant_item(item) {
                continue;
            }
            if skip_unmatched_tool_output(item, &valid_call_ids) {
                continue;
            }
        }
        out.push((seq, payload));
    }
    Ok(out)
}

/// Model-visible `(seq, Item)` pairs in surface order, with the same skip rules
/// as [`derive_messages`]. Compatibility projection: checkpoint payloads use the
/// same unique user projector as the working-set path.
pub fn project_working_pairs(
    surface: &Surface,
    lookup: impl Fn(Seq) -> Result<Item>,
) -> Result<Vec<(Seq, Item)>> {
    Ok(
        project_working_payloads(surface, |seq| Ok(WorkingPayload::item(lookup(seq)?)))?
            .into_iter()
            .map(|(seq, payload)| (seq, payload.model_item()))
            .collect(),
    )
}
/// Diff/Restore baseline folded from the current spine.
pub fn spine_reminder_view(events: &[SessionEvent]) -> Result<crate::reminder::SpineReminderView> {
    let surface = fold_surface(events)?;
    let by_seq: std::collections::HashMap<Seq, &SessionEvent> =
        events.iter().map(|event| (event.seq, event)).collect();
    let mut nodes = Vec::with_capacity(surface.nodes.len());
    for seq in surface.nodes {
        let event = by_seq.get(&seq).ok_or_else(|| {
            LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing from log"))
        })?;
        let node = match &event.event_type {
            EventType::Compacted => crate::reminder::SpineNode::Compacted,
            EventType::Reminder(_) => {
                let reminder = serde_json::from_value(event.data.clone()).map_err(|e| {
                    LitecodeError::InvalidSessionEvent(format!("reminder body at {seq}: {e}"))
                })?;
                crate::reminder::SpineNode::Reminder(reminder)
            }
            _ => crate::reminder::SpineNode::Other,
        };
        nodes.push((seq, node));
    }
    Ok(crate::reminder::view_from_spine(&nodes))
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
        if matches!(
            event.event_type,
            EventType::Reminder(_) | EventType::PlanExecute
        ) {
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
mod tests {}
