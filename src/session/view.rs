//! The single derivation of the model-visible working set from the append-only log.
//!
//! Full load and incremental extend are the same fold: [`WorkingView::from_events`]
//! is the empty-view case of [`WorkingView::apply`]. A caller that keeps the view
//! across steps only has to feed the rows it has not folded yet; a caller starting
//! cold replays everything. There is one set of rules for order, payload identity,
//! and `replacement_media` — not one per caller.
//!
//! A cold load may start at a checkpoint instead of seq 0
//! ([`WorkingView::from_events_anchored`]). That is still the same rule set: the
//! anchor only removes rows the compaction already replaced, so the folded result
//! is row-for-row identical to the full fold. A view refuses to build when it
//! cannot prove that, and the caller falls back to the full log.

use std::collections::{HashMap, HashSet};

use crate::session::SurfaceOp;
use crate::session::event::{EventType, SessionEvent, Seq, spine_working_payload};
use crate::session::media::item_has_media;
use crate::session::model::LogState;
use crate::session::surface::{
    FoldAnchor, Surface, SurfacePlan, apply_plan, plan_surface_anchored, project_working_payloads,
};
use crate::session::working::{WorkingPayload, WorkingRow};
use crate::types::{LitecodeError, Result};

/// The folded log state the working set is derived from.
#[derive(Debug, Clone, Default)]
pub struct WorkingView {
    surface: Surface,
    payloads: HashMap<Seq, WorkingPayload>,
    /// Rows whose payload carried media. Kept after they leave the surface: a
    /// shadowed source still decides whether its replacement rewrote media.
    media: HashSet<Seq>,
    replacement_media: HashSet<Seq>,
    in_progress: HashSet<Seq>,
    /// Rows whose body did not parse, kept out of `payloads` but remembered so a
    /// row that is later shadowed stays silent while a live one still fails.
    unparsed: HashMap<Seq, String>,
    max_seq: Option<Seq>,
    /// Truncate epoch this fold was built against. A revert deletes rows and
    /// appends nothing, so seq continuity cannot see it; the epoch can.
    epoch: u64,
    /// Where this fold started instead of seq 0; `None` means the whole log.
    /// Rows below `anchor.start` were replaced by the checkpoint.
    anchor: Option<FoldAnchor>,
}

impl WorkingView {
    /// The full fold: an empty view with every event applied in order.
    pub fn from_events(events: &[SessionEvent]) -> Result<Self> {
        Self::from_events_anchored(events, None)
    }

    /// Fold the log from `anchor` (`compacted.body.to` of the newest
    /// checkpoint). Rows below it were summarised away and are never read.
    ///
    /// Refuses any log it cannot fold exactly — a row below the anchor, a missing
    /// checkpoint, a checkpoint newer than the named one, or a fold that does not
    /// begin with the checkpoint — so the caller falls back to
    /// [`Self::from_events`] instead of guessing.
    pub fn from_events_anchored(events: &[SessionEvent], anchor: Option<FoldAnchor>) -> Result<Self> {
        let mut view = Self {
            anchor,
            ..Self::default()
        };
        // A full fold legitimately replays every compaction the session ever
        // made, so it checks nothing here; only an anchored fold has an expected
        // first node.
        let mut saw_checkpoint = false;
        for event in events {
            if let Some(anchor) = anchor {
                if event.seq < anchor.start {
                    // The caller handed in rows the anchor says are gone. Folding
                    // them would add rows no full fold has, so refuse loudly.
                    return Err(LitecodeError::InvalidSessionEvent(format!(
                        "anchored fold at {} was given row {} below the anchor",
                        anchor.start, event.seq
                    )));
                }
                if matches!(event.event_type, EventType::Compacted)
                    && event.seq == anchor.checkpoint
                {
                    saw_checkpoint = true;
                }
            }
            view.apply(event)?;
        }
        if let Some(anchor) = anchor {
            if !saw_checkpoint {
                return Err(LitecodeError::InvalidSessionEvent(format!(
                    "no row at the anchored checkpoint {} (fold starts at {})",
                    anchor.checkpoint, anchor.start
                )));
            }
            if view.surface.nodes.first() != Some(&anchor.checkpoint) {
                return Err(LitecodeError::InvalidSessionEvent(format!(
                    "anchored fold at {} did not begin with checkpoint {}",
                    anchor.start, anchor.checkpoint
                )));
            }
        }
        Ok(view)
    }

    /// The truncate epoch this view reflects.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Record the truncate epoch a freshly built view reflects.
    pub fn set_epoch(&mut self, epoch: u64) {
        self.epoch = epoch;
    }

    /// Fold one event. Applying events in log order is the only supported order;
    /// a caller that feeds a suffix must have folded the prefix already.
    pub fn apply(&mut self, event: &SessionEvent) -> Result<()> {
        if let Some(plan) = plan_surface_anchored(&self.surface, event, self.anchor)? {
            let shadowed: Vec<Seq> = match &plan {
                SurfacePlan::Replace { start_idx, len, .. } => {
                    self.surface.nodes[*start_idx..*start_idx + *len].to_vec()
                }
                _ => Vec::new(),
            };
            let rewrote_media = matches!(event.surface_op, Some(SurfaceOp::Replace { .. }))
                && event
                    .source_seqs
                    .as_ref()
                    .is_some_and(|sources| sources.iter().any(|seq| self.media.contains(seq)));
            if rewrote_media {
                self.replacement_media.insert(event.seq);
            } else {
                self.replacement_media.remove(&event.seq);
            }
            for seq in shadowed {
                self.payloads.remove(&seq);
                self.replacement_media.remove(&seq);
            }
            apply_plan(&mut self.surface, plan);
        }

        if event.state == LogState::InProgress {
            self.in_progress.insert(event.seq);
        } else {
            self.in_progress.remove(&event.seq);
        }
        self.max_seq = Some(self.max_seq.map_or(event.seq, |max| max.max(event.seq)));

        if self.surface.nodes.contains(&event.seq) {
            match spine_working_payload(event) {
                Ok(payload) => {
                    if payload.as_item().is_some_and(item_has_media) {
                        self.media.insert(event.seq);
                    }
                    self.unparsed.remove(&event.seq);
                    self.payloads.insert(event.seq, payload);
                }
                Err(error) => {
                    self.payloads.remove(&event.seq);
                    self.unparsed.insert(event.seq, error.to_string());
                }
            }
        }
        Ok(())
    }

    /// Model-visible rows in surface order, with the skip rules applied.
    pub fn rows(&self) -> Result<Vec<WorkingRow>> {
        let pairs = project_working_payloads(&self.surface, |seq| {
            self.payloads.get(&seq).cloned().ok_or_else(|| match self.unparsed.get(&seq) {
                Some(message) => LitecodeError::InvalidSessionEvent(message.clone()),
                None => {
                    LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing from view"))
                }
            })
        })?;
        Ok(pairs
            .into_iter()
            .map(|(seq, payload)| WorkingRow {
                log_seq: Some(seq),
                payload,
                replacement_media: self.replacement_media.contains(&seq),
            })
            .collect())
    }

    /// Refresh one row that is still in flight. A seal rewrites content and
    /// state on the same seq without appending a row, so an incremental caller
    /// that re-reads such a seq must fold it here rather than through [`Self::apply`]
    /// (`apply` would append the seq a second time).
    ///
    /// Returns `false` when the event cannot be folded in place; the caller must
    /// then rebuild from the full log.
    pub fn refresh(&mut self, event: &SessionEvent) -> Result<bool> {
        if !self.surface.nodes.contains(&event.seq) {
            return Ok(false);
        }
        if !matches!(event.surface_op, Some(SurfaceOp::Append) | None) {
            return Ok(false);
        }
        let payload = spine_working_payload(event)?;
        if payload.as_item().is_some_and(item_has_media) {
            self.media.insert(event.seq);
        }
        self.payloads.insert(event.seq, payload);
        if event.state == LogState::InProgress {
            self.in_progress.insert(event.seq);
        } else {
            self.in_progress.remove(&event.seq);
        }
        Ok(true)
    }

    /// Extend with the rows the log gained since this view's high-water.
    ///
    /// `read_range(from, to)` must return the session's events in `[from, to)`.
    /// Returns `false` when the caller must rebuild from the full log instead:
    /// a shrunken tail (revert/truncate), a hole in the seq range, a retroactive
    /// row, or a row that cannot be folded in place. Accepted rows are a plain
    /// append plus in-place seals of rows this view still believes are streaming.
    pub fn extend(
        &mut self,
        epoch: u64,
        max_seq: i64,
        mut read_range: impl FnMut(u64, u64) -> Result<Vec<SessionEvent>>,
    ) -> bool {
        // A revert below the fold leaves a continuity the seq range cannot
        // expose: the tail is deleted, and the next append resumes above the
        // high-water mark, so it looks like a plain append. The epoch is the
        // only exact signal, and it costs one integer comparison.
        if epoch != self.epoch {
            return false;
        }
        let held = self.max_seq;
        if let Some(held_max) = held
            && max_seq < held_max as i64
        {
            return false;
        }
        let from = held.map_or(0, |seq| seq + 1);
        let to = (max_seq.max(-1) + 1).max(0) as u64;
        if to < from {
            return false;
        }
        let events = if to > from {
            let Ok(events) = read_range(from, to) else {
                return false;
            };
            if events.is_empty() {
                return false;
            }
            let mut expected = from;
            for event in &events {
                // A gap means rows above the fold were deleted (revert); what we
                // read is then not a suffix of the log this view was built from.
                if event.seq != expected
                    || crate::session::surface::is_retroactive(event, held)
                {
                    return false;
                }
                expected += 1;
            }
            events
        } else {
            Vec::new()
        };

        // A seal rewrites a row in place and never appears in the append range.
        let in_progress: Vec<u64> = self.in_progress.iter().copied().collect();
        for seq in in_progress {
            let Ok(row) = read_range(seq, seq + 1) else {
                return false;
            };
            match row.first() {
                Some(event) => {
                    if !matches!(self.refresh(event), Ok(true)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        for event in &events {
            if self.apply(event).is_err() {
                return false;
            }
        }
        true
    }

    /// Highest seq folded so far; `None` for an empty view.
    pub fn max_seq(&self) -> Option<Seq> {
        self.max_seq
    }

    /// Rows the log says are still streaming. A seal rewrites one of these in
    /// place without appending a row, so an incremental caller must re-read
    /// exactly these seqs before trusting the fold.
    pub fn in_progress_seqs(&self) -> &HashSet<Seq> {
        &self.in_progress
    }
}

#[cfg(test)]
mod anchored_fold_tests {
    use super::*;
    use crate::session::model::CompactedBody;
    use crate::session::{EventDraft, EventLog};
    use crate::types::user_text;

    fn item_log(texts: &[&str]) -> EventLog {
        let mut log = EventLog::new();
        for text in texts {
            log.append(
                EventDraft::surface_item(EventType::ItemUser, &user_text(*text), SurfaceOp::Append)
                    .expect("draft"),
            )
            .expect("append");
        }
        log
    }

    /// The checkpoint the writer appends for a keep-recent compact.
    fn append_compact(log: &mut EventLog, summary: &str, shadowed: Vec<Seq>, to: Seq) {
        let from = shadowed.first().copied().unwrap_or(to);
        log.append(EventDraft {
            time: 0,
            event_type: EventType::Compacted,
            data: serde_json::to_value(CompactedBody {
                summary: summary.to_string(),
                from,
                to,
                at: None,
            })
            .expect("body"),
            surface_op: None,
            source_seqs: Some(shadowed),
            ignorable: false,
            state: LogState::Final,
        })
        .expect("compact");
    }

    /// What the loader feeds in: the whole log, or only rows at or above the
    /// anchor. EventLog seqs are dense, so the anchor is also the slice index.
    fn rows(log: &EventLog, anchor: Option<FoldAnchor>) -> Vec<WorkingRow> {
        let events = log.events();
        let fed = match anchor {
            Some(anchor) => &events[anchor.start as usize..],
            None => events,
        };
        WorkingView::from_events_anchored(fed, anchor)
            .expect("fold")
            .rows()
            .expect("rows")
    }

    fn assert_anchor_matches_full(log: &EventLog, start: Seq, checkpoint: Seq) {
        let anchor = FoldAnchor { start, checkpoint };
        assert_eq!(
            rows(log, Some(anchor)),
            rows(log, None),
            "anchored fold at {anchor:?} must be row-for-row identical to the full fold"
        );
    }

    /// a,b,c then a compact keeping b onward. One compaction.
    #[test]
    fn one_compaction_anchored_matches_full() {
        let mut log = item_log(&["a", "b", "c"]);
        append_compact(&mut log, "sum", vec![0], 1);
        assert_anchor_matches_full(&log, 1, 3);
    }

    /// Two compactions: the second keeps a row that predates the first
    /// checkpoint, so its kept boundary (`to`) sits *below* that checkpoint's
    /// seq. The old checkpoint is inside the read range and must be dropped, not
    /// folded — a surface never holds two checkpoints.
    #[test]
    fn two_compactions_anchored_matches_full() {
        let mut log = item_log(&["a", "b", "c"]);
        append_compact(&mut log, "first", vec![0], 1);
        append_compact(&mut log, "second", vec![3, 1], 2);
        assert_anchor_matches_full(&log, 2, 4);
    }

    /// Three compactions keep the same shape: only the newest checkpoint is a
    /// surface node, every older one in range is noise.
    #[test]
    fn three_compactions_anchored_matches_full() {
        let mut log = item_log(&["a", "b", "c", "d"]);
        append_compact(&mut log, "first", vec![0], 1);
        append_compact(&mut log, "second", vec![4, 1], 2);
        append_compact(&mut log, "third", vec![5, 2], 3);
        assert_anchor_matches_full(&log, 3, 6);
    }

    /// A mechanical prune below the current anchor is a no-op for an anchored
    /// fold: the checkpoint that set the anchor already shadowed it.
    #[test]
    fn replace_below_the_anchor_is_a_no_op() {
        let mut log = item_log(&["a", "b", "c"]);
        log.append(EventDraft {
            time: 0,
            event_type: EventType::ItemUser,
            data: serde_json::to_value(user_text("pruned")).expect("item"),
            surface_op: Some(SurfaceOp::Replace { start: 1, end: 1 }),
            source_seqs: Some(vec![1]),
            ignorable: false,
            state: LogState::Final,
        })
        .expect("prune");
        append_compact(&mut log, "sum", vec![0, 3], 2);
        assert_anchor_matches_full(&log, 2, 4);
    }

    /// A compact always replaces a prefix of the spine (`nodes[..keep_i]`), so
    /// `from` is the head. A row claiming to start mid-spine would punch a hole
    /// through the tail: no writer produces it, and the fold refuses it instead
    /// of building a surface that never existed.
    #[test]
    fn full_fold_refuses_a_compact_that_is_not_the_spine_head() {
        let log = item_log(&["a", "b", "c"]);
        // The log itself refuses to admit this draft, so build the row by hand:
        // this is the shape a hand-edited store would produce.
        let mut events = log.events().to_vec();
        events.push(crate::session::SessionEvent {
            seq: 3,
            time: 0,
            event_type: EventType::Compacted,
            data: serde_json::to_value(CompactedBody {
                summary: "bad".into(),
                from: 1,
                to: 3,
                at: None,
            })
            .expect("body"),
            result_data: None,
            surface_op: None,
            source_seqs: Some(vec![1, 2]),
            state: LogState::Final,
            ignorable: false,
        });
        let err = WorkingView::from_events(&events).unwrap_err();
        assert!(
            err.to_string().contains("not the spine head"),
            "unexpected error: {err}"
        );
    }

    /// The write side refuses the same shape before it can reach the log.
    #[test]
    fn the_log_refuses_a_compact_that_is_not_the_spine_head() {
        let mut log = item_log(&["a", "b", "c"]);
        let err = log
            .append(EventDraft {
                time: 0,
                event_type: EventType::Compacted,
                data: serde_json::to_value(CompactedBody {
                    summary: "bad".into(),
                    from: 1,
                    to: 3,
                    at: None,
                })
                .expect("body"),
                surface_op: None,
                source_seqs: Some(vec![1, 2]),
                ignorable: false,
                state: LogState::Final,
            })
            .unwrap_err();
        assert!(
            err.to_string().contains("not the spine head"),
            "unexpected error: {err}"
        );
    }

    /// A full fold legitimately replays every compaction.
    #[test]
    fn full_fold_accepts_many_compactions() {
        let mut log = item_log(&["a", "b", "c"]);
        append_compact(&mut log, "first", vec![0], 1);
        append_compact(&mut log, "second", vec![3, 1], 2);
        assert!(WorkingView::from_events(log.events()).is_ok());
    }

    #[test]
    fn anchored_refuses_a_missing_checkpoint() {
        let log = item_log(&["a", "b"]);
        let anchor = FoldAnchor {
            start: 1,
            checkpoint: 99,
        };
        // EventLog seqs are dense, so `start` is also the slice index.
        let err =
            WorkingView::from_events_anchored(&log.events()[1..], Some(anchor)).unwrap_err();
        assert!(
            err.to_string().contains("anchored checkpoint"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn anchored_refuses_an_anchor_the_checkpoint_does_not_end_at() {
        let mut log = item_log(&["a", "b", "c"]);
        append_compact(&mut log, "sum", vec![0], 1);
        // The checkpoint ends at 1; claiming the fold starts at 2 is a caller
        // bug and must not silently produce a shorter surface.
        let anchor = FoldAnchor {
            start: 2,
            checkpoint: 3,
        };
        let err = WorkingView::from_events_anchored(&log.events()[2..], Some(anchor)).unwrap_err();
        assert!(
            err.to_string().contains("fold anchor"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn anchored_refuses_rows_below_the_anchor() {
        let mut log = item_log(&["a", "b", "c"]);
        append_compact(&mut log, "sum", vec![0], 1);
        let anchor = FoldAnchor {
            start: 1,
            checkpoint: 3,
        };
        // Handing in the whole log instead of the suffix would add a row no full
        // fold has, so it must fail rather than quietly grow the surface.
        let err = WorkingView::from_events_anchored(log.events(), Some(anchor)).unwrap_err();
        assert!(
            err.to_string().contains("below the anchor"),
            "unexpected error: {err}"
        );
    }
}
