//! The single derivation of the model-visible working set from the append-only log.
//!
//! Full load and incremental extend are the same fold: [`WorkingView::from_events`]
//! is the empty-view case of [`WorkingView::apply`]. A caller that keeps the view
//! across steps only has to feed the rows it has not folded yet; a caller starting
//! cold replays everything. There is one set of rules for order, payload identity,
//! and `replacement_media` — not one per caller.

use std::collections::{HashMap, HashSet};

use crate::session::SurfaceOp;
use crate::session::event::{SessionEvent, Seq, spine_working_payload};
use crate::session::media::item_has_media;
use crate::session::model::LogState;
use crate::session::surface::{
    Surface, SurfacePlan, apply_plan, plan_surface, project_working_payloads,
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
}

impl WorkingView {
    /// The full fold: an empty view with every event applied in order.
    pub fn from_events(events: &[SessionEvent]) -> Result<Self> {
        let mut view = Self::default();
        for event in events {
            view.apply(event)?;
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
        if let Some(plan) = plan_surface(&self.surface, event)? {
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
