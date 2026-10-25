pub mod budget;
pub mod compact;
pub mod context_meter;
pub mod env;
pub mod estimate;
pub mod keep_recent;
pub mod limits;
pub mod media_budget;
pub mod summary;
pub mod system;
pub mod view;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;

use crate::llm::item_id_of;
use crate::session::manager::SessionManager;
use crate::session::store::Session;
use crate::session::working::{CompactedView, WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result, ToolResultData};

pub use budget::{BudgetPolicy, OccupancyBill, ProviderPromptBaseline, occupancy_bill_from};
pub use compact::{
    CompactPolicy, CompactionOutcome, CompactionPlan, MechanicalPruneConfig, MechanicalPrunePlan,
    PRUNE_MIN_SAVED_TOKENS_FLOOR, PruneReplacement, SummaryArtifact, SummaryFailure, SummaryFinish,
    SummaryReview, SummaryUsage, TOOL_RESULT_PRUNE_HEAD_CHARS, TOOL_RESULT_PRUNE_MARKER,
    TOOL_RESULT_PRUNE_TAIL_CHARS, TOOL_RESULT_PRUNE_THRESHOLD_CHARS, prune_min_saved_tokens,
};
pub use context_meter::{
    ContextEnvelope, ContextMeter, ContextMeterSource, latest_persisted_envelope,
    measure_persisted_context, measure_persisted_context_with_breakdown, measure_request_context,
    request_token_breakdown, request_token_breakdown_for_rows, tool_schema_tokens,
};
pub use env::{Context, build_context};
pub use limits::{COMPACT_MAX_OUTPUT_TOKENS, KEEP_RECENT_CAP, MAX_PROMPT_RESERVE_TOKENS};
pub use system::build_system_prompt;
pub use view::{HotView, PreparedView};

/// Result of persisting a transcript delta.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommitStepOutcome {
    pub committed: bool,
    pub discarded: bool,
    pub preview: Option<crate::session::data::command::SessionListPreview>,
    /// Existing log rows sealed by this commit and requiring live re-stamps.
    pub sealed_seqs: Vec<crate::session::event::Seq>,
}

struct PipelineState {
    hot: HotView,
    prepared: Option<PreparedView>,
    working: Vec<WorkingRow>,
    /// Fixed request shape frozen by the latest `compact_step` call.
    envelope: ContextEnvelope,
    /// Last observed log `MAX(seq)` (`-1` if empty). A later commit treats a
    /// smaller cursor as a revert and discards the turn's tail.
    log_max_seq: i64,
    turn_id: Option<String>,
}

/// L1 context pipeline: single prepare / commit entry for LLM-bound context.
pub struct ContextPipeline {
    budget: BudgetPolicy,
    compact: CompactPolicy,
    data_root: PathBuf,
    /// Mutex so a subagent turn can run as a `Send` tool future on the parent runtime.
    state: Mutex<PipelineState>,
}

/// Map a padded LLM view back to durable rows without treating a provider item ID
/// as unique across requests. The pad operation preserves the original item order;
/// synthetic outputs have no matching source item and receive `None`.
fn align_padded_item_seqs(
    source_items: &[Item],
    source_seqs: &[Option<crate::session::event::Seq>],
    view_items: &[Item],
) -> Vec<Option<crate::session::event::Seq>> {
    debug_assert_eq!(source_items.len(), source_seqs.len());
    let mut source_index = 0usize;
    let aligned: Vec<_> = view_items
        .iter()
        .map(|item| {
            if source_items.get(source_index) == Some(item) {
                let seq = source_seqs[source_index];
                source_index += 1;
                seq
            } else {
                None
            }
        })
        .collect();
    debug_assert_eq!(source_index, source_items.len());
    aligned
}

/// Projected request items plus the positional provenance needed by the
/// compaction call mode. `source_indices` points into the unpadded turn view;
/// a synthetic unanswered-call pad inherits the call's source index.
pub(crate) struct ModelProjection {
    pub items: Vec<Item>,
    /// Durable provenance for compaction/prefix identity. Synthetic pads stay
    /// `None` here.
    pub item_seqs: Vec<Option<crate::session::event::Seq>>,
    /// Provider-bill coverage provenance. Synthetic pads inherit the source
    /// call's seq; ordinary items keep their durable seq.
    pub coverage_seqs: Vec<Option<crate::session::event::Seq>>,
    pub source_indices: Vec<Option<usize>>,
    pub rebased: Vec<bool>,
    pub projected_parts: u32,
}

/// Resolve a synthetic pad to the unanswered call it repairs.
///
/// `pad_unanswered_calls` emits all pads for a batch only after the whole
/// call batch, so "the previous source item" is wrong for two or more
/// consecutive unanswered calls. The provider call id is unique within a
/// request and is the stable join key between a pad and its source call.
fn pad_source_index(source_items: &[Item], before: usize, item: &Item) -> Option<usize> {
    let Item::FunctionCallOutput(output) = item else {
        return None;
    };
    source_items[..before]
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, source)| match source {
            Item::FunctionCall(call) if call.call_id == output.call_id => Some(index),
            _ => None,
        })
}

/// Map padded view items back to their source item index. A synthetic pad
/// inherits its source call's index while remaining `None` in `item_seqs`.
fn align_padded_item_sources(source_items: &[Item], view_items: &[Item]) -> Vec<Option<usize>> {
    let mut source_index = 0usize;
    let aligned: Vec<_> = view_items
        .iter()
        .map(|item| {
            if source_items.get(source_index) == Some(item) {
                let index = source_index;
                source_index += 1;
                Some(index)
            } else {
                pad_source_index(source_items, source_index, item)
            }
        })
        .collect();
    debug_assert_eq!(source_index, source_items.len());
    aligned
}

/// One model projection shared by the main view builder and compaction.
///
/// Order is deliberate: pad, capability projection, stale-media projection,
/// then media-ref expansion. Compaction takes a source-index slice of this
/// result, so its prefix is byte-for-byte the same prefix the main request
/// would carry (before the port applies the requested thinking mode).
pub(crate) fn project_model_view(
    turn_view: &[Item],
    source_seqs: &[Option<crate::session::event::Seq>],
    inherited_before_checkpoint: Option<&[bool]>,
    replacement_media: Option<&[bool]>,
    model: &crate::provider_catalog::ResolvedModel,
    data_root: &std::path::Path,
) -> ModelProjection {
    debug_assert_eq!(turn_view.len(), source_seqs.len());
    if let Some(inherited) = inherited_before_checkpoint {
        debug_assert_eq!(turn_view.len(), inherited.len());
    }
    let mut llm_items = turn_view.to_vec();
    Session::pad_unanswered_calls(&mut llm_items);
    let item_seqs = align_padded_item_seqs(turn_view, source_seqs, &llm_items);
    let source_indices = align_padded_item_sources(turn_view, &llm_items);
    let coverage_seqs = source_indices
        .iter()
        .map(|source_index| source_index.and_then(|index| source_seqs[index]))
        .collect();
    let projected_parts = crate::runtime::project_llm_input_for_model(&mut llm_items, model);
    // Runs before refs expand, so a dropped part never becomes a data URL.
    let mut rebased = media_budget::drop_stale_media(&mut llm_items);
    if let Some(replacement_media) = replacement_media {
        debug_assert_eq!(replacement_media.len(), turn_view.len());
        let mut replacement_flags = vec![false; llm_items.len()];
        for (index, source_index) in source_indices.iter().enumerate() {
            if source_index
                .and_then(|source| replacement_media.get(source))
                .copied()
                .unwrap_or(false)
            {
                replacement_flags[index] = true;
            }
        }
        let replacement_rebased =
            media_budget::rebased_after_media_replacements(&llm_items, &replacement_flags);
        for (current, replacement) in rebased.iter_mut().zip(replacement_rebased) {
            *current |= replacement;
        }
    }
    if let Some(inherited) = inherited_before_checkpoint {
        for (index, source_index) in source_indices.iter().enumerate() {
            if source_index
                .and_then(|source| inherited.get(source))
                .copied()
                .unwrap_or(false)
            {
                rebased[index] = true;
            }
        }
    }
    crate::session::media::resolve_user_media(&mut llm_items, data_root);
    ModelProjection {
        items: llm_items,
        item_seqs,
        coverage_seqs,
        source_indices,
        rebased,
        projected_parts,
    }
}

/// The unprojected turn view and its provenance inputs.
fn working_turn_view(
    rows: &[WorkingRow],
) -> (
    Vec<Item>,
    Vec<Option<crate::session::event::Seq>>,
    Option<Vec<bool>>,
    Vec<bool>,
) {
    if let Some(view) = CompactedView::from_working_rows(rows) {
        (
            view.items,
            view.item_seqs,
            Some(view.inherited_before_checkpoint),
            view.replacement_media,
        )
    } else {
        (
            project_items(rows),
            rows.iter().map(|row| row.log_seq).collect(),
            None,
            rows.iter().map(|row| row.replacement_media).collect(),
        )
    }
}

/// Build the exact model-visible request projection from persisted/staged rows.
///
/// Auto, manual, UI, and `build_view` all call this helper, so pads, capability
/// projection, stale-media rewriting, and media-ref expansion are measured as
/// the request would send them.
pub(crate) fn project_working_rows(
    rows: &[WorkingRow],
    model: &crate::provider_catalog::ResolvedModel,
    data_root: &std::path::Path,
) -> (Vec<Item>, ModelProjection) {
    let (turn_view, source_seqs, inherited_before_checkpoint, replacement_media) =
        working_turn_view(rows);
    let projection = project_model_view(
        &turn_view,
        &source_seqs,
        inherited_before_checkpoint.as_deref(),
        Some(&replacement_media),
        model,
        data_root,
    );
    (turn_view, projection)
}

/// Bind this step's items onto stream rows opened after `cursor`. A provider
/// id is only unique inside one call, so older rows are not candidates.
fn attach_step_items(rows: &mut Vec<WorkingRow>, items: &[Item], cursor: i64) {
    let mut used = HashSet::new();
    for item in items {
        let bound = item_id_of(item).and_then(|id| {
            rows.iter().position(|row| {
                let Some(seq) = row.log_seq else {
                    return false;
                };
                (seq as i64) > cursor
                    && !used.contains(&seq)
                    && row.item().and_then(item_id_of).as_deref() == Some(id.as_str())
            })
        });
        if let Some(index) = bound {
            let seq = rows[index].log_seq.expect("bound row has a seq");
            used.insert(seq);
            rows[index].replace_item(item.clone());
        } else {
            rows.push(WorkingRow::pending(item.clone()));
        }
    }
}

impl ContextPipeline {
    pub fn new(context_window: usize, _ctx: Context, data_root: PathBuf) -> Self {
        Self {
            budget: BudgetPolicy::new(context_window),
            compact: CompactPolicy,
            data_root,
            state: Mutex::new(PipelineState {
                hot: HotView::new(),
                prepared: None,
                working: Vec::new(),
                envelope: ContextEnvelope::default(),
                log_max_seq: -1,
                turn_id: None,
            }),
        }
    }

    /// Override keep-recent token window (integration tests).
    pub fn with_keep_recent_tokens(mut self, tokens: usize) -> Self {
        self.budget = self.budget.with_keep_recent_tokens(tokens);
        self
    }

    /// Raw binding window used by this pipeline. `BudgetPolicy` keeps its 128k
    /// fallback only for keep-recent sizing; pressure meters must see the raw
    /// 0/1 window so they stay non-pressure.
    pub fn context_window(&self) -> usize {
        self.budget.context_window
    }

    pub fn data_root(&self) -> &PathBuf {
        &self.data_root
    }

    pub fn sync_context(&self, _ctx: &Context) {}

    /// Freeze the fixed request envelope without running compaction. Production
    /// calls this through `compact_step`; tests use it to measure a prepared view.
    pub fn freeze_request_envelope(&self, envelope: ContextEnvelope) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .envelope = envelope;
    }

    pub fn prepared_view(&self) -> Option<PreparedView> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared
            .clone()
    }

    pub fn take_prepared_view(&self) -> Option<PreparedView> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prepared
            .take()
    }

    /// Persist working set last synced from the session gate (and pending tail).
    pub fn working_set(&self) -> Vec<WorkingRow> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .working
            .clone()
    }

    /// The turn this pipeline is currently running, if any.
    ///
    /// Rows written while a turn is running belong to it; nothing may be written
    /// without one.
    pub fn current_turn_id(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .turn_id
            .clone()
    }

    /// Load turn working set from Session DB (§5.1 turn load — sole path).
    pub fn begin_turn(
        &self,
        sessions: &SessionManager,
        session_id: &str,
    ) -> Result<Vec<WorkingRow>> {
        self.begin_turn_with_id(sessions, session_id, None)
    }

    pub fn begin_turn_with_id(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        turn_id: Option<String>,
    ) -> Result<Vec<WorkingRow>> {
        let rows = sessions.data().working_set_blocking(session_id)?;
        let max_seq = sessions.entry_wire_seq_cursor(session_id).0;

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.turn_id = turn_id;
        state.log_max_seq = max_seq;
        state.working = rows.clone();
        state.hot.replace(project_items(&rows));
        state.prepared = None;
        Ok(rows)
    }

    /// Install rows the caller already holds (a `begin_turn` load plus pending
    /// tails) without writing them.
    pub fn stage_working(&self, rows: Vec<WorkingRow>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows;
    }

    /// Reload when the log cursor moved. Pending rows (`log_seq == None`) are
    /// appended after the fold. A shorter log is a revert: drop the pending
    /// tail and leave `log_max_seq` so the next commit still sees the shrink.
    fn sync_turn_working(&self, sessions: &SessionManager, session_id: &str) {
        let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
        let (must_reload, shrunk, pending) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let shrunk = !state.working.is_empty() && max_seq < state.log_max_seq;
            let must_reload = state.working.is_empty() || max_seq != state.log_max_seq;
            let pending: Vec<WorkingRow> = state
                .working
                .iter()
                .filter(|row| row.log_seq.is_none())
                .cloned()
                .collect();
            (must_reload, shrunk, pending)
        };
        if !must_reload {
            return;
        }
        let Ok(mut rows) = sessions.data().working_set_blocking(session_id) else {
            return;
        };
        if !shrunk {
            rows.extend(pending);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows;
        if !shrunk {
            state.log_max_seq = max_seq;
        }
    }

    pub fn end_turn(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.turn_id = None;
        state.log_max_seq = -1;
        state.working.clear();
        state.hot.replace(Vec::new());
        state.prepared = None;
    }

    /// Compact when the budget says so. Does not build the model view.
    ///
    /// Returns whether the compaction stage changed the durable surface (a
    /// checkpoint or a mechanical prune). Callers that compact sync the request
    /// seam again before [`Self::build_view`], so restored reminders land after
    /// the rewritten prefix.
    pub async fn compact_step(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        compaction_request: &crate::llm::CompactionRequestContext,
        compact_max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        step: u64,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        // Returns whether a full compaction ran — single source of truth for
        // the caller's phase/compaction events (no duplicate budget math).
        if cancel.is_cancelled() {
            return Ok(false);
        }

        self.sync_turn_working(sessions, session_id);
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.envelope = ContextEnvelope::from_compaction_request(compaction_request);
        }
        let mut rows = self.working_set();
        let compacted = self
            .compact
            .compact_if_needed(
                &self.budget,
                sessions,
                session_id,
                compaction_request,
                compact_max_tokens,
                prompt_baseline,
                &mut rows,
                step,
                cancel,
                &self.data_root,
            )
            .await?;

        if cancel.is_cancelled() {
            return Ok(false);
        }

        if compacted {
            // `compact_if_needed` already reloaded the fold once and put the
            // pending tail back. Do not load the log again.
            let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.log_max_seq = max_seq;
            state.working = rows;
        }
        Ok(compacted)
    }

    /// Provider-confirmed context-overflow recovery: force one compaction
    /// attempt, bypassing the normal pressure threshold. The caller retries the
    /// original main request only when the durable surface actually advanced.
    pub async fn force_compact_overflow(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        compaction_request: &crate::llm::CompactionRequestContext,
        compact_max_tokens: u32,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        self.sync_turn_working(sessions, session_id);
        let mut rows = self.working_set();
        let before_revision = sessions.data().revision_blocking(session_id)?;
        let baseline = ProviderPromptBaseline::default();
        let result = CompactPolicy::compact_transcript(
            &self.budget,
            sessions,
            session_id,
            compaction_request,
            compact_max_tokens,
            &baseline,
            &mut rows,
            cancel,
            crate::runtime::observer::CompactionTrigger::Overflow,
            None,
            &self.data_root,
        )
        .await;
        if matches!(result, Err(LitecodeError::Canceled)) {
            return Err(LitecodeError::Canceled);
        }
        let after_revision = sessions.data().revision_blocking(session_id)?;
        let advanced = after_revision > before_revision;
        if advanced {
            let loaded = sessions.data().working_set_blocking(session_id)?;
            let max_seq = sessions.entry_wire_seq_cursor(session_id).0;
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.working = loaded;
            state.log_max_seq = max_seq;
            state.prepared = None;
        }
        match result {
            Ok(_) => Ok(advanced),
            // A committed mechanical prune is sufficient retry proof even
            // when the optional summary phase failed afterwards.
            Err(_) if advanced => Ok(true),
            Err(error) => Err(error),
        }
    }

    /// Build the ephemeral model view from the current working set.
    ///
    /// Reloads the log first so reminders appended after compaction are in the
    /// view. Synthetic unanswered-call pads exist only on this view.
    pub fn build_view(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        prompt_baseline: &ProviderPromptBaseline,
        model: &crate::provider_catalog::ResolvedModel,
    ) -> Result<()> {
        self.sync_turn_working(sessions, session_id);
        let (rows, envelope) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.working.clone(), state.envelope.clone())
        };
        let (turn_view, projection) = project_working_rows(&rows, model, &self.data_root);
        let bill = resolve_occupancy_bill(sessions, session_id, prompt_baseline);
        let meter = ContextMeter::measure(
            model.context_window_max,
            model.max_output as usize,
            &envelope,
            &projection.items,
            &projection.coverage_seqs,
            bill,
        );
        let token_count = meter.used_tokens;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.hot.replace(turn_view);
        state.prepared = Some(PreparedView {
            items: projection.items,
            item_seqs: projection.item_seqs,
            coverage_seqs: projection.coverage_seqs,
            rebased: projection.rebased,
            token_count,
            context_meter: meter,
            projected_parts: projection.projected_parts,
            instructions: None,
        });
        Ok(())
    }

    /// Persist item delta since the last commit.
    ///
    /// On success, orphan `FunctionCallOutput`s are removed from `rows` (same
    /// set the store dropped from the in-memory working set). On commit failure,
    /// `rows` is unchanged.
    pub fn commit_step(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        rows: &mut Vec<WorkingRow>,
    ) -> Result<CommitStepOutcome> {
        self.commit_step_with_turn(sessions, session_id, rows, "", &[])
    }

    /// Commit `items` produced by this step.
    ///
    /// Rows the stream already opened (`seq` above the cursor at view-build
    /// time) are claimed by provider item id. Everything else is a pending
    /// tail. A failed commit leaves those new rows out of `state.working`.
    pub fn persist_new(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        items: &[Item],
        result_data: &[ToolResultData],
    ) -> Result<CommitStepOutcome> {
        let cursor = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .log_max_seq;
        self.sync_turn_working(sessions, session_id);
        let mut rows = self.working_set();
        attach_step_items(&mut rows, items, cursor);
        self.commit_step_with_turn(sessions, session_id, &mut rows, "", result_data)
    }

    pub fn commit_step_with_turn(
        &self,
        sessions: &SessionManager,
        session_id: &str,
        rows: &mut Vec<WorkingRow>,
        turn_id: &str,
        result_data: &[ToolResultData],
    ) -> Result<CommitStepOutcome> {
        let expected_max_seq = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .log_max_seq;
        let tid = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if turn_id.is_empty() {
                state.turn_id.clone().unwrap_or_default()
            } else {
                turn_id.to_string()
            }
        };
        let (kind, working, preview) = sessions
            .commit_turn_delta(
                session_id,
                rows.clone(),
                expected_max_seq,
                &tid,
                result_data,
            )
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        *rows = working;
        let (committed, discarded, sealed_seqs, clear_prepared) = match kind {
            crate::session::data::command::CommitKind::Idempotent => {
                (false, true, Vec::new(), true)
            }
            crate::session::data::command::CommitKind::Sealed { seqs } => {
                (true, false, seqs, false)
            }
            crate::session::data::command::CommitKind::MetaUpdated
            | crate::session::data::command::CommitKind::Appended { .. } => {
                (true, false, Vec::new(), false)
            }
            _ => (false, false, Vec::new(), false),
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.working = rows.clone();
        state.log_max_seq = sessions.entry_wire_seq_cursor(session_id).0;
        state.hot.replace(project_items(rows));
        if clear_prepared {
            state.prepared = None;
        }
        Ok(CommitStepOutcome {
            committed,
            discarded,
            preview,
            sealed_seqs,
        })
    }

    /// Unified prompt occupancy for the last prepared view, or a local-envelope
    /// estimate when no request has been prepared yet.
    pub fn current_token_estimate(&self, turn_items: &[Item]) -> usize {
        let (prepared, envelope) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (
                state.prepared.as_ref().map(|view| view.token_count),
                state.envelope.clone(),
            )
        };
        if let Some(count) = prepared {
            return count;
        }
        let context_window = self.budget.context_window;
        ContextMeter::measure_local(
            context_window,
            envelope.output_reserve,
            &envelope,
            turn_items,
        )
        .used_tokens
    }

    /// Test/telemetry convenience only. Production auto pressure is decided in
    /// `CompactPolicy::compact_if_needed` from the same [`ContextMeter`].
    pub fn will_compact(&self, items: &[Item], last_prompt_tokens: u64) -> bool {
        let envelope = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .envelope
            .clone();
        let context_window = self.budget.context_window;
        let bill = (last_prompt_tokens > 0).then_some(OccupancyBill {
            prompt_tokens: last_prompt_tokens,
            // With `None`, persisted rows with a seq are covered. Fake seqs are
            // enough here because this helper has no durable row list.
            covered_seq: None,
        });
        let seqs = vec![Some(0); items.len()];
        ContextMeter::measure(
            context_window,
            envelope.output_reserve,
            &envelope,
            items,
            &seqs,
            bill,
        )
        .should_compact()
    }
}

#[cfg(test)]
mod turn_window_tests {
    use super::*;
    use crate::types::user_text;

    #[test]
    fn synthetic_pad_inherits_the_unanswered_calls_source_index() {
        use crate::authority::responses::FunctionToolCall;

        let source = vec![
            user_text("go"),
            Item::FunctionCall(FunctionToolCall {
                call_id: "call-1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                id: Some("fc-1".into()),
                status: None,
                namespace: None,
            }),
        ];
        let mut view = source.clone();
        Session::pad_unanswered_calls(&mut view);
        assert_eq!(view.len(), 3, "the unanswered call needs one synthetic pad");
        assert_eq!(
            align_padded_item_sources(&source, &view),
            vec![Some(0), Some(1), Some(1)],
            "the pad must share the call's source index so it follows it across compaction"
        );
    }

    #[test]
    fn batched_unanswered_calls_keep_distinct_coverage_provenance() {
        use crate::authority::responses::FunctionToolCall;

        let source = vec![
            user_text("go"),
            Item::FunctionCall(FunctionToolCall {
                call_id: "call-1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                id: Some("fc-1".into()),
                status: None,
                namespace: None,
            }),
            Item::FunctionCall(FunctionToolCall {
                call_id: "call-2".into(),
                name: "read".into(),
                arguments: "{}".into(),
                id: Some("fc-2".into()),
                status: None,
                namespace: None,
            }),
        ];
        let mut view = source.clone();
        Session::pad_unanswered_calls(&mut view);
        assert_eq!(
            align_padded_item_sources(&source, &view),
            vec![Some(0), Some(1), Some(2), Some(1), Some(2)],
            "each batched pad must inherit its own call, not the last pending call"
        );
    }

    #[test]
    fn padding_alignment_uses_order_not_provider_ids() {
        let source = vec![user_text("same"), user_text("same")];
        let padded = vec![
            user_text("same"),
            crate::types::assistant_text("host pad"),
            user_text("same"),
        ];
        assert_eq!(
            align_padded_item_seqs(&source, &[Some(10), Some(20)], &padded),
            vec![Some(10), None, Some(20)]
        );
    }
}

/// Live in-memory bill, else the persisted occupancy the ring shows.
pub(crate) fn resolve_occupancy_bill(
    sessions: &SessionManager,
    session_id: &str,
    baseline: &ProviderPromptBaseline,
) -> Option<OccupancyBill> {
    let meta = sessions.data().meta_blocking(session_id).ok();
    let compacted_seq = meta.as_ref().and_then(|meta| meta.compacted_seq);
    let surface_rewrite_seq = meta.as_ref().map_or(0, |meta| meta.surface_rewrite_seq);
    let rollup = sessions
        .provider_usage_rollup(session_id)
        .unwrap_or_default();
    let meter = sessions
        .data()
        .meter_blocking(session_id)
        .unwrap_or_default();
    occupancy_bill_from(
        baseline,
        compacted_seq,
        surface_rewrite_seq,
        &rollup,
        &meter,
    )
}

/// A streamed item already owns a row; the commit that follows must find it.
///
/// The projector writes rows while the model is still talking, so the pipeline's
/// cached working set is stale by the time the agent hands the same items back.
/// Treating them as new is what appends a second copy of every streamed item —
/// the duplicate the reader sees as repeated reasoning and repeated tool calls.
#[cfg(test)]
mod stream_commit_tests {}
