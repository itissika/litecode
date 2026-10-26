mod session_lifecycle;
pub(crate) use session_lifecycle::build_session_list;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::client_protocol::connection::SessionRequest;
use crate::client_protocol::observer::InternalEvent;
use crate::client_protocol::output;
use crate::client_protocol::permission_bridge::{self, PendingPermission};
use crate::client_protocol::project::{self, session_snapshot, wire_phase};
use crate::client_protocol::protocol::{
    ErrorCode, ModelInfo, OperationKind, SessionBindingProjection, SessionSnapshot,
    StructuredError, TurnSnapshot,
};
use crate::permission::PermissionSink;
use crate::runtime::RuntimeHandle;
use crate::runtime::observer::InternalEnvelope;
use crate::runtime::observer::TurnPhase;
use crate::session::manager::SessionManager;
use crate::session::snapshot;
use crate::types::LitecodeError;

#[derive(Debug)]
pub enum StartTurnError {
    AgentAlreadyRunning,
    Runtime(anyhow::Error),
}

impl std::fmt::Display for StartTurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartTurnError::AgentAlreadyRunning => write!(f, "agent already running"),
            StartTurnError::Runtime(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for StartTurnError {}

impl From<anyhow::Error> for StartTurnError {
    fn from(e: anyhow::Error) -> Self {
        StartTurnError::Runtime(e)
    }
}

impl StartTurnError {
    pub fn error_code(&self) -> ErrorCode {
        match self {
            StartTurnError::AgentAlreadyRunning => ErrorCode::AgentAlreadyRunning,
            StartTurnError::Runtime(_) => ErrorCode::Internal,
        }
    }
}

/// Loaded log window `[from_seq, to_seq)`.
pub struct MaterializedRange {
    pub events: Vec<crate::client_protocol::protocol::WireBufferEvent>,
}

/// Per-session projection state. Each subscribed session on a connection gets
/// its own `Projection` with independent turn view, buffer tracking, and
/// outgoing queue.
///
/// Does NOT hold its own SQLite connection — persistence is owned by
/// `SessionManager.entries` and accessed via delegation.
pub struct Projection {
    pub session_id: String,
    pub sessions: Arc<SessionManager>,
    pub buffer_revision: u64,
    /// Wire cursor high-water: the last persisted `next_seq` this projection has
    /// delivered up to. New rows are fetched from `[next_seq, <fresh next_seq>)`.
    pub next_seq: u64,
    pub turn_committed_next_seq: u64,
    pub turn_id: Option<String>,
    pub phase: TurnPhase,
    pub step: u64,
    pub step_max: u32,
    pub started_at_ms: i64,
    pub outgoing: Vec<serde_json::Value>,
    pub turn_completed_emitted: bool,
    pub context_window: usize,
    /// Unified prompt occupancy from the server-owned ContextMeter. Kept in
    /// sync with `context_meter.used_tokens`; legacy field for wire consumers.
    pub context_tokens_estimate: usize,
    /// Authoritative occupancy/pressure snapshot inputs.
    pub context_meter: crate::context_pipeline::ContextMeter,
    /// Server-owned fixed request envelope (instructions/tools/overhead/output)
    /// used for local fallback. Seeded from the latest request origin row and
    /// refreshed from live `LlmRequestBuilt` / manual compaction setup.
    pub context_envelope: crate::context_pipeline::ContextEnvelope,
    /// Resolved main-turn model used to build the exact request projection.
    /// `None` only in projection-only tests that do not exercise the meter.
    pub request_model: Option<Arc<crate::provider_catalog::ResolvedModel>>,
    /// Item-text mix for the occupancy bar. Request-aligned after `LlmRequestBuilt`.
    pub context_token_breakdown: crate::session::estimate::ItemTokenBreakdown,
    /// No persisted/cold request shape is available, so context fields must be
    /// omitted from every outgoing snapshot instead of publishing a fake meter.
    pub(crate) context_unknown: bool,
    /// Fold of the log behind `context_meter`. `None` means the next refresh
    /// rebuilds it; a `false` from `WorkingView::extend` does the same.
    pub(crate) view: Option<crate::session::view::WorkingView>,
    /// Whether the last meter refresh extended the fold instead of re-reading
    /// the log. Observability only; never read for control flow.
    pub(crate) last_estimate_incremental: bool,
    /// Last-known provider usage (persisted session meter); feeds snapshot ring hydrate.
    pub last_turn_token_stats: Option<crate::client_protocol::protocol::TurnTokenStats>,
    /// Session-total provider usage (Σ per-request); feeds whole-session hit rate.
    pub cumulative_token_stats: Option<crate::client_protocol::protocol::TurnTokenStats>,
    /// Sum of recorded round-trip bills. `None` until the first stored request.
    pub provider_billed_tokens: Option<u64>,
    pub provider_billed_requests: u64,
    /// Cached from snapshot patch files; see [`snapshot::max_file_revert_seq`].
    pub max_file_revert_seq: Option<i64>,
    pub deferred: VecDeque<SessionRequest>,
    /// Handle for the forwarding task spawned by `subscribe()`.
    /// Aborted on `unsubscribe()` to prevent resource leaks.
    pub(super) forward_task: Option<tokio::task::JoinHandle<()>>,
}

impl Projection {
    fn seq_cursor(&self) -> (i64, u64) {
        self.sessions.entry_wire_seq_cursor(&self.session_id)
    }

    fn child_session_id_for_encoded(&self, encoded: Option<&crate::types::Item>) -> Option<String> {
        match encoded {
            Some(crate::types::Item::FunctionCall(fc)) if fc.name == "subagent_launch" => self
                .sessions
                .child_session_id_for_call(&self.session_id, &fc.call_id),
            _ => None,
        }
    }

    fn push_buffer_log_row(
        &mut self,
        event: &crate::session::event::SessionEvent,
        child_session_id: Option<String>,
    ) {
        self.push_outgoing(project::buffer_log_row(
            &self.session_id,
            event,
            child_session_id,
        ));
    }

    /// Current-envelope restamp for already-allocated seqs. Order is caller-defined.
    /// Idempotent: the durable row is re-projected as-is.
    fn restamp_changed_seqs(&mut self, seqs: &[crate::session::event::Seq]) {
        let data_root = self.sessions.data_root_path();
        for seq in seqs {
            let events = match self.sessions.entry_load_events_range(
                &self.session_id,
                *seq,
                seq.saturating_add(1),
            ) {
                Ok(events) => events,
                Err(e) => {
                    tracing::error!(
                        session_id = %self.session_id,
                        seq,
                        error = %e,
                        "restamp_changed_seqs: failed to load sealed log row"
                    );
                    continue;
                }
            };
            for event in events {
                let encoded = crate::session::event::item_from_event(&event)
                    .ok()
                    .map(|item| output::encode_client_item(item, &data_root))
                    .transpose();
                let encoded = match encoded {
                    Ok(item) => item,
                    Err(_) => {
                        tracing::error!(
                            session_id = %self.session_id,
                            seq = event.seq,
                            "restamp_changed_seqs: failed to encode item body"
                        );
                        None
                    }
                };
                let child_session_id = self.child_session_id_for_encoded(encoded.as_ref());
                self.push_buffer_log_row(&event, child_session_id);
            }
        }
    }

    fn measure_context(
        &self,
        max_output_tokens: u32,
    ) -> Option<crate::context_pipeline::ContextMeter> {
        let model = self.request_model.as_ref()?;
        let output_reserve = if max_output_tokens > 0 {
            max_output_tokens as usize
        } else {
            self.context_envelope.output_reserve
        };
        let envelope = self
            .context_envelope
            .clone()
            .with_output_reserve(output_reserve);
        crate::context_pipeline::measure_persisted_context(
            &self.sessions,
            &self.session_id,
            &envelope,
            model,
            &self.sessions.data_root_path(),
        )
        .ok()
    }

    /// Whether the last `refresh_context_estimate` extended the held fold
    /// instead of re-reading the whole session log.
    pub fn last_estimate_incremental(&self) -> bool {
        self.last_estimate_incremental
    }

    fn refresh_context_estimate(&mut self) {
        let Some(model) = self.request_model.clone() else {
            return;
        };
        let envelope = self
            .context_envelope
            .clone()
            .with_output_reserve(self.context_envelope.output_reserve);
        let sessions = Arc::clone(&self.sessions);
        let data_root = sessions.data_root_path();
        let session_id = self.session_id.clone();

        // Rows for the meter come from the fold this projection already holds,
        // extended with whatever the log gained. Any doubt rebuilds it from the
        // log — the same read this replaced, so the worst case is the old cost.
        let max_seq = sessions.entry_wire_seq_cursor(&session_id).0;
        let epoch = sessions.entry_truncate_epoch(&session_id);
        let mut extended = None;
        if let Some(view) = self.view.as_mut()
            && view.extend(epoch, max_seq, |from, to| {
                sessions.entry_load_events_range(&session_id, from, to)
            })
        {
            extended = view.rows().ok();
        }
        self.last_estimate_incremental = extended.is_some();
        let rows = match extended {
            Some(rows) => rows,
            None => {
                // Drop the stale fold before any fallible step: a failed rebuild
                // must leave nothing behind that a later extend could accept.
                self.view = None;
                let Ok(events) = sessions.data().events_blocking(&session_id) else {
                    return;
                };
                let Ok(mut view) = crate::session::view::WorkingView::from_events(&events) else {
                    return;
                };
                view.set_epoch(epoch);
                let Ok(rows) = view.rows() else {
                    return;
                };
                self.view = Some(view);
                rows
            }
        };

        let Ok((meter, breakdown)) =
            crate::context_pipeline::measure_working_rows_with_breakdown(
                &sessions,
                &session_id,
                &envelope,
                &model,
                &data_root,
                &rows,
            )
        else {
            return;
        };
        self.context_tokens_estimate = meter.used_tokens;
        self.context_meter = meter;
        // Meter and bucket mix came from the same projection above. Provider
        // usage scales this mix in the ring, so retaining a request-time mix
        // after a suffix commit would misattribute the new occupancy.
        self.context_token_breakdown = breakdown;
    }

    fn snapshot_meter(
        &self,
        context_window: usize,
        max_output_tokens: u32,
    ) -> crate::context_pipeline::ContextMeter {
        let raw_output = if max_output_tokens > 0 {
            max_output_tokens as usize
        } else {
            self.context_envelope.output_reserve
        };
        if self.context_meter.context_window == context_window
            && self.context_meter.output_reserve == raw_output
        {
            return self.context_meter.clone();
        }
        self.measure_context(max_output_tokens).unwrap_or_else(|| {
            // No resolved model means there is no request projection to
            // measure. Publish a disabled local meter rather than a
            // transcript-only estimate.
            let model_context_window = self
                .request_model
                .as_ref()
                .map(|model| model.context_window_max)
                .unwrap_or(context_window);
            crate::context_pipeline::ContextMeter::new(
                context_window,
                model_context_window,
                raw_output,
                self.context_envelope.fixed_prompt_tokens(),
                crate::context_pipeline::ContextMeterSource::LocalEnvelope,
            )
        })
    }
}

impl Projection {
    pub fn new(session_id: String, sessions: Arc<SessionManager>, context_window: usize) -> Self {
        Self::new_with_model(session_id, sessions, context_window, None)
    }

    pub fn new_with_model(
        session_id: String,
        sessions: Arc<SessionManager>,
        context_window: usize,
        request_model: Option<Arc<crate::provider_catalog::ResolvedModel>>,
    ) -> Self {
        let (_, next_seq) = sessions.entry_wire_seq_cursor(&session_id);
        let context_envelope =
            crate::context_pipeline::latest_persisted_envelope(&sessions, &session_id)
                .unwrap_or_else(|| crate::context_pipeline::ContextEnvelope::cold(0));
        let (context_meter, context_token_breakdown) = match request_model.as_ref() {
            Some(model) => crate::context_pipeline::measure_persisted_context_with_breakdown(
                &sessions,
                &session_id,
                &context_envelope,
                model,
                &sessions.data_root_path(),
            )
            .unwrap_or_default(),
            None => (
                crate::context_pipeline::ContextMeter::default(),
                crate::session::estimate::ItemTokenBreakdown::default(),
            ),
        };
        let context_tokens_estimate = context_meter.used_tokens;
        let meter = sessions
            .data()
            .meter_blocking(&session_id)
            .unwrap_or_default();
        let rollup = sessions
            .provider_usage_rollup(&session_id)
            .unwrap_or_default();
        let meta = sessions.data().meta_blocking(&session_id).ok();
        let compacted_seq = meta.as_ref().and_then(|meta| meta.compacted_seq);
        let surface_rewrite_seq = meta.as_ref().map_or(0, |meta| meta.surface_rewrite_seq);
        // A bill newer than the last compact/surface rewrite is the occupancy
        // prompt. A later checkpoint or mechanical prune retires it. No bill yet
        // keeps the meter row.
        let last_turn_token_stats = crate::session::store::occupancy_usage(
            &rollup,
            compacted_seq,
            surface_rewrite_seq,
            &meter,
        )
        .map(|usage| crate::client_protocol::protocol::TurnTokenStats {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cache_hit_tokens: usage.cache_hit_tokens,
            cache_miss_tokens: usage.cache_miss_tokens,
        });
        let provider_billed_requests = rollup.request_count;
        let provider_billed_tokens = if provider_billed_requests == 0 {
            None
        } else {
            Some(rollup.billed_tokens)
        };
        // Session-total accumulators ride the same meter row; absent on legacy rows
        // until the next usage-bearing turn writes them (cum starts at 0).
        let cumulative_token_stats = Some(crate::client_protocol::protocol::TurnTokenStats {
            prompt_tokens: meter.cum_prompt_tokens,
            completion_tokens: meter.cum_completion_tokens,
            cache_hit_tokens: meter.cum_cache_hit_tokens,
            cache_miss_tokens: meter.cum_cache_miss_tokens,
        });
        Self {
            session_id,
            sessions,
            buffer_revision: 0,
            next_seq,
            turn_committed_next_seq: 0,
            turn_id: None,
            phase: TurnPhase::Idle,
            step: 0,
            step_max: 0,
            started_at_ms: 0,
            outgoing: Vec::new(),
            turn_completed_emitted: false,
            context_window,
            context_tokens_estimate,
            context_meter,
            context_envelope,
            request_model,
            context_token_breakdown,
            context_unknown: false,
            view: None,
            last_estimate_incremental: false,
            last_turn_token_stats,
            cumulative_token_stats,
            provider_billed_tokens,
            provider_billed_requests,
            max_file_revert_seq: None,
            deferred: VecDeque::new(),
            forward_task: None,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    // ── outgoing ──

    pub fn take_outgoing(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.outgoing)
    }

    pub fn push_outgoing(&mut self, mut msg: serde_json::Value) {
        if self.context_unknown {
            project::omit_snapshot_context_fields(&mut msg);
        }
        self.outgoing.push(msg);
    }

    // ── snapshot ──

    /// Replace the server-owned request envelope and immediately refresh the
    /// projected meter/breakdown. Used by cold seeding and tests.
    pub fn set_context_envelope(&mut self, envelope: crate::context_pipeline::ContextEnvelope) {
        self.context_envelope = envelope;
        self.context_unknown = false;
        self.refresh_context_estimate();
    }

    /// Replace the resolved request model and refresh from the real projection.
    pub fn set_request_model(
        &mut self,
        model: Option<Arc<crate::provider_catalog::ResolvedModel>>,
    ) {
        self.request_model = model;
        self.refresh_context_estimate();
    }

    pub fn snapshot(&self, project: &str, binding: &SessionBindingProjection) -> SessionSnapshot {
        let (last_seq, next_seq) = self.seq_cursor();
        let context_window = if binding.context_window > 0 {
            binding.context_window
        } else {
            self.context_window
        };
        let meter = self.snapshot_meter(context_window, binding.max_output_tokens);
        let envelope = self
            .context_envelope
            .clone()
            .with_output_reserve(meter.output_reserve);
        let mut snap = project::buffer_snapshot(
            &self.session_id,
            project,
            binding,
            last_seq,
            next_seq,
            self.buffer_revision,
            self.turn_snapshot(),
            self.last_turn_token_stats.clone(),
            self.cumulative_token_stats.clone(),
            &meter,
            &envelope,
            self.sessions.is_compacting_blocking(&self.session_id),
        );
        snap.context_token_breakdown = self.context_token_breakdown.clone();
        if self.provider_billed_requests == 0 {
            snap.provider_billed_tokens = None;
            snap.provider_billed_requests = None;
        } else {
            snap.provider_billed_tokens = Some(self.provider_billed_tokens.unwrap_or(0));
            snap.provider_billed_requests = Some(self.provider_billed_requests);
        }
        snap.max_file_revert_seq = self.max_file_revert_seq;
        let task_state = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| Ok(state.clone()))
            .unwrap_or_default();
        snap.todos = task_state.todos;
        snap.active_plan_path = task_state.active_plan.map(|plan| plan.relative_path);
        // Memory-only queue, but still part of the snapshot: a rebuilt panel
        // (subscribed while idle after a cancel) must see it.
        snap.pending_messages = self.sessions.pending_messages_snapshot(&self.session_id);
        // WP-186: always present list (may be empty). Authority for FE cards.
        let pending = self.sessions.pending_permissions_snapshot(&self.session_id);
        snap.pending_permissions = project::pending_records_to_wire(&pending);
        if let Some(turn) = snap.turn.as_mut() {
            turn.awaiting_permission = !snap.pending_permissions.is_empty();
        }
        if let Some(live) = snap.live.as_mut() {
            live.awaiting_permission = !snap.pending_permissions.is_empty();
        }
        snap.llm_reconnect = self.sessions.llm_reconnect(&self.session_id);
        if let Ok(meta) = self.sessions.data().meta_blocking(&self.session_id) {
            snap.meta = crate::client_protocol::protocol::SessionMetaWire {
                id: meta.id,
                project: meta.project,
                created_at: meta.created_at,
                updated_at: meta.updated_at,
                parent_session_id: meta.parent_session_id,
                parent_call_id: meta.parent_call_id,
                responsibility: meta.responsibility,
                subagent_depth: meta.subagent_depth,
                agent_id: meta.agent_id,
                model_id: meta.model_id,
                thinking_tier: meta.thinking_tier,
                context_mode: meta.context_mode,
                compacted_seq: meta.compacted_seq,
                spine_from: meta.spine_from,
                todos: meta.todos,
                plan_slug: meta.plan_slug,
                preview: meta.preview,
            };
        }
        snap
    }

    fn turn_snapshot(&self) -> Option<TurnSnapshot> {
        // Whether a turn is present is owned by SessionManager, not projection cache.
        if !self.sessions.is_turn_running_blocking(&self.session_id) {
            return None;
        }
        if let Some(progress) = self.sessions.get_cached_progress(&self.session_id) {
            return Some(project::turn_progress_to_snapshot(&progress));
        }
        self.turn_id.as_ref().map(|turn_id| TurnSnapshot {
            turn_id: turn_id.clone(),
            phase: wire_phase(&self.phase),
            step: self.step,
            step_max: self.step_max,
            started_at_ms: self.started_at_ms,
            awaiting_permission: false,
        })
    }

    // ── event dispatch ──

    pub fn on_internal(
        &mut self,
        envelope: InternalEnvelope,
        project: &str,
        binding: &SessionBindingProjection,
    ) {
        let InternalEnvelope {
            event: ev,
            parent_session_id: _,
        } = envelope;
        let ev = match ev {
            InternalEvent::TurnCompleted {
                turn_id,
                final_text,
                reason,
                turn_token_stats,
                committed_next_seq: _,
                error_kind,
                cancelled_by,
            } => InternalEvent::TurnCompleted {
                turn_id,
                final_text,
                reason,
                turn_token_stats,
                committed_next_seq: self.turn_committed_next_seq,
                error_kind,
                cancelled_by,
            },
            other => other,
        };
        self.apply_internal_state(&ev, project, binding);
        match &ev {
            InternalEvent::CompactionLifecycle {
                trigger,
                stage: crate::runtime::observer::CompactionStage::Succeeded,
                ..
            } => {
                // Same wire as a step commit: history grew by one checkpoint item.
                self.last_turn_token_stats = None;
                self.bump_buffer_revision(project, binding);
                if *trigger == crate::runtime::observer::CompactionTrigger::Manual {
                    self.push_operation_ok(OperationKind::CompactSession, project, binding);
                }
            }
            InternalEvent::CompactionLifecycle {
                trigger: crate::runtime::observer::CompactionTrigger::Manual,
                stage: crate::runtime::observer::CompactionStage::Failed,
                fail_kind,
                error,
                ..
            } => {
                let code = crate::client_protocol::project::compact_fail_code(*fail_kind);
                self.push_operation_error(
                    OperationKind::CompactSession,
                    code,
                    error.clone().unwrap_or_else(|| "compaction failed".into()),
                    project,
                    binding,
                );
            }
            _ => {}
        }
        if let InternalEvent::BufferRestamp { seqs } = &ev {
            self.restamp_changed_seqs(seqs);
        }
        // Re-stamp the parent `subagent_launch` function_call so live FE item
        // stores see `child_session_id` without waiting for tool completion.
        if let InternalEvent::SubagentBound {
            call_id,
            child_session_id,
        } = &ev
            && let Some(event) = self
                .sessions
                .find_function_call_event(&self.session_id, call_id)
        {
            self.push_outgoing(project::buffer_log_row(
                &self.session_id,
                &event,
                Some(child_session_id.clone()),
            ));
        }
        let publish_live_meter = matches!(
            &ev,
            InternalEvent::LlmRequestBuilt { .. }
                | InternalEvent::LlmCompleted { .. }
                | InternalEvent::StepCommitted
        );
        let msg = if project::event_needs_snapshot(&ev) {
            project::project(&ev, &self.snapshot(project, binding))
        } else {
            project::project_live(&ev, &self.session_id, self.turn_id.as_deref())
        };
        if let Some(msg) = msg {
            self.push_outgoing(msg);
        }
        if publish_live_meter {
            // Request/completion events carry the exact server meter, and a
            // step commit refreshes it from the persisted bill+suffix. Push a
            // full snapshot so the browser never has to reconstruct thresholds.
            self.push_outgoing(project::session_snapshot(self.snapshot(project, binding)));
        }
    }

    pub fn on_event(
        &mut self,
        ev: InternalEvent,
        project: &str,
        binding: &SessionBindingProjection,
    ) {
        self.on_internal(
            InternalEnvelope {
                event: ev,
                parent_session_id: None,
            },
            project,
            binding,
        );
    }

    // ── buffer ──

    pub fn bump_buffer_revision(&mut self, project: &str, binding: &SessionBindingProjection) {
        self.buffer_revision = self.buffer_revision.saturating_add(1);
        let old_next = self.next_seq;
        // The cursor is a pair now: `last_seq` is the active tail, `next_seq` the
        // persisted high-water. New rows live in `[old_next, new_next)` because an
        // append always lands on the high-water (never in a hole left by a revert).
        let (new_last, new_next) = self.seq_cursor();
        self.next_seq = new_next;
        // Provider bill + uncovered suffix is recomputed on every durable
        // commit; a compact also lands here after clearing last-turn stats.
        self.refresh_context_estimate();
        if self.next_seq > old_next {
            match self
                .sessions
                .entry_load_events_range(&self.session_id, old_next, self.next_seq)
            {
                Ok(events) => {
                    let data_root = self.sessions.data_root_path();
                    for event in events {
                        let encoded = crate::session::event::item_from_event(&event)
                            .ok()
                            .map(|item| output::encode_client_item(item, &data_root))
                            .transpose();
                        let Ok(encoded) = encoded else {
                            tracing::error!(session_id = %self.session_id, seq = event.seq,
                                "bump_buffer_revision: failed to encode item body");
                            continue;
                        };
                        let child_session_id = self.child_session_id_for_encoded(encoded.as_ref());
                        self.push_buffer_log_row(&event, child_session_id);
                    }
                }
                Err(e) => {
                    tracing::error!(
                        session_id = %self.session_id,
                        error = %e,
                        "bump_buffer_revision: failed to load new log events"
                    );
                }
            }
        }
        self.on_event(
            InternalEvent::BufferChanged {
                last_seq: new_last,
                next_seq: new_next,
                revision: self.buffer_revision,
            },
            project,
            binding,
        );
    }

    // ── revert ──

    /// Wire-facing revert: truncates the transcript at a user row's `seq`.
    pub fn revert_to_user_anchor(
        &mut self,
        seq: u64,
        project: &str,
        binding: &SessionBindingProjection,
    ) -> anyhow::Result<()> {
        let lease = match self.sessions.try_begin_revert(&self.session_id) {
            Ok(lease) => lease,
            Err(LitecodeError::AgentAlreadyRunning) => {
                self.push_operation_error(
                    OperationKind::RevertToUserAnchor,
                    ErrorCode::AgentAlreadyRunning,
                    "session is busy".into(),
                    project,
                    binding,
                );
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let anchor = match i64::try_from(seq) {
            Ok(anchor) => anchor,
            Err(_) => {
                self.push_operation_error(
                    OperationKind::RevertToUserAnchor,
                    ErrorCode::InvalidRevertAnchor,
                    format!("seq={seq}"),
                    project,
                    binding,
                );
                return Ok(());
            }
        };
        match self
            .sessions
            .entry_revert_to_user_anchor(&self.session_id, anchor)
        {
            Ok(()) => {
                // The log is back at this user row: messages still queued for the
                // next seam belong to the state the user just discarded, so they
                // must not be delivered afterwards. Done under the same lease —
                // no turn can start and no end-of-turn flush can claim them in
                // between, and a failed revert never reaches this branch.
                if let Some(lease) = lease.as_ref() {
                    self.sessions.discard_pending_messages_for_revert(
                        &self.session_id,
                        lease.operation_id(),
                    );
                }
                self.bump_buffer_revision(project, binding);
                // Re-read after the bump: `last_seq` is the new (dropped) active
                // tail while `next_seq` is the persisted high-water that did not
                // move back.
                let (last_seq, next_seq) = self.seq_cursor();
                self.push_outgoing(serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "buffer/reverted",
                    "params": {
                        "session_id": self.session_id,
                        "last_seq": last_seq,
                        "next_seq": next_seq,
                    },
                }));
                self.push_operation_ok(OperationKind::RevertToUserAnchor, project, binding);
                Ok(())
            }
            Err(LitecodeError::InvalidRevertAnchor(msg)) => {
                self.push_operation_error(
                    OperationKind::RevertToUserAnchor,
                    ErrorCode::InvalidRevertAnchor,
                    msg,
                    project,
                    binding,
                );
                Ok(())
            }
            Err(e) => {
                self.push_operation_error(
                    OperationKind::RevertToUserAnchor,
                    ErrorCode::Internal,
                    e.to_string(),
                    project,
                    binding,
                );
                Ok(())
            }
        }
    }

    pub fn revert_files(
        &mut self,
        seq: u64,
        project: &str,
        binding: &SessionBindingProjection,
        workspace_root: &str,
        snapshots_dir: &std::path::Path,
    ) -> anyhow::Result<()> {
        let _lease = match self.sessions.try_begin_operation(
            &self.session_id,
            crate::session::manager::SessionOperationKind::Revert,
        ) {
            Ok(lease) => lease,
            Err(LitecodeError::AgentAlreadyRunning) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::AgentAlreadyRunning,
                    "session is busy".into(),
                    project,
                    binding,
                );
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let anchor = match i64::try_from(seq) {
            Ok(anchor) => anchor,
            Err(_) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::InvalidRevertAnchor,
                    format!("seq={seq}"),
                    project,
                    binding,
                );
                return Ok(());
            }
        };
        let stem = match self
            .sessions
            .entry_snapshot_stem_for_user_seq(&self.session_id, anchor)
        {
            Ok(stem) => stem,
            Err(LitecodeError::InvalidRevertAnchor(msg)) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::InvalidRevertAnchor,
                    msg,
                    project,
                    binding,
                );
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let workspace = std::path::PathBuf::from(workspace_root);
        let snaps = snapshots_dir.to_path_buf();
        let sid = self.session_id.clone();
        let restore_result = run_snapshot_restore_off_async(workspace, snaps, sid, stem);
        match restore_result {
            Ok(snapshot::RestoreOutcome::Restored { .. }) => {
                self.push_operation_ok(OperationKind::RevertFiles, project, binding);
                Ok(())
            }
            Ok(snapshot::RestoreOutcome::NothingToRevert) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::NothingToRevert,
                    "no file changes to revert".into(),
                    project,
                    binding,
                );
                Ok(())
            }
            Ok(snapshot::RestoreOutcome::Unavailable {
                reason: snapshot::RestoreUnavailable::MissingTrackRef,
            }) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::SnapshotUnavailable,
                    "no file snapshot for this anchor (track may have failed)".into(),
                    project,
                    binding,
                );
                Ok(())
            }
            Ok(snapshot::RestoreOutcome::Unavailable {
                reason: snapshot::RestoreUnavailable::TrackFailed,
            }) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::SnapshotUnavailable,
                    "file snapshot unavailable (track failed this turn)".into(),
                    project,
                    binding,
                );
                Ok(())
            }
            Err(LitecodeError::InvalidRevertAnchor(msg)) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::InvalidRevertAnchor,
                    msg,
                    project,
                    binding,
                );
                Ok(())
            }
            Err(e) => {
                self.push_operation_error(
                    OperationKind::RevertFiles,
                    ErrorCode::Internal,
                    e.to_string(),
                    project,
                    binding,
                );
                Ok(())
            }
        }
    }

    // ── materialize ──

    pub fn materialize_range(
        &self,
        from_seq: crate::session::event::Seq,
        to_seq: crate::session::event::Seq,
    ) -> anyhow::Result<MaterializedRange> {
        let events = self
            .sessions
            .entry_load_events_range(&self.session_id, from_seq, to_seq)?;
        let data_root = self.sessions.data_root_path();
        let mut out = Vec::with_capacity(events.len());
        for event in events {
            let encoded = crate::session::event::item_from_event(&event)
                .ok()
                .map(|item| output::encode_client_item(item, &data_root))
                .transpose()?;
            let child_session_id = self.child_session_id_for_encoded(encoded.as_ref());
            let hidden = crate::reminder::hidden_kind(event.event_type.as_str());
            out.push(crate::client_protocol::protocol::WireBufferEvent {
                seq: event.seq,
                event_type: event.event_type,
                body: event.data,
                result_data: event.result_data,
                cites: event.source_seqs.unwrap_or_default(),
                state: event.state,
                surface_op: event.surface_op,
                child_session_id,
                hidden,
            });
        }
        Ok(MaterializedRange { events: out })
    }

    // ── turn helpers ──

    pub fn clear_handle(&mut self) {
        self.phase = TurnPhase::Idle;
        self.turn_id = None;
    }

    pub fn deferred_mut(&mut self) -> &mut VecDeque<SessionRequest> {
        &mut self.deferred
    }

    pub fn push_operation_error(
        &mut self,
        op: OperationKind,
        code: ErrorCode,
        message: String,
        project: &str,
        binding: &SessionBindingProjection,
    ) {
        self.push_outgoing(project::operation_result(
            op,
            false,
            Some(StructuredError {
                code,
                message,
                retryable: false,
            }),
            self.snapshot(project, binding),
        ));
    }

    pub fn push_operation_ok(
        &mut self,
        op: OperationKind,
        project: &str,
        binding: &SessionBindingProjection,
    ) {
        self.push_outgoing(project::operation_result(
            op,
            true,
            None,
            self.snapshot(project, binding),
        ));
    }

    pub fn apply_turn_started(&mut self, turn_id: String, step_max: u32) {
        self.turn_id = Some(turn_id);
        self.phase = TurnPhase::Starting;
        self.step = 1;
        self.step_max = step_max;
        self.started_at_ms = chrono::Utc::now().timestamp_millis();
        self.turn_completed_emitted = false;
    }

    pub fn apply_internal_state(
        &mut self,
        ev: &crate::runtime::observer::InternalEvent,
        project: &str,
        binding: &SessionBindingProjection,
    ) {
        use crate::runtime::observer::TurnPhase;
        match ev {
            crate::runtime::observer::InternalEvent::TurnStarted {
                turn_id, step_max, ..
            } => {
                self.apply_turn_started(turn_id.clone(), *step_max);
            }
            crate::runtime::observer::InternalEvent::PhaseChanged { phase, step } => {
                self.phase = phase.clone();
                self.step = *step;
            }
            crate::runtime::observer::InternalEvent::StepStarted { step, step_max } => {
                self.step = *step;
                self.step_max = *step_max;
            }
            crate::runtime::observer::InternalEvent::StepCommitted => {
                self.bump_buffer_revision(project, binding);
            }
            crate::runtime::observer::InternalEvent::ProjectionLagged { skipped } => {
                tracing::warn!(
                    session_id = %self.session_id,
                    skipped,
                    next_seq = self.next_seq,
                    "session event subscriber lagged; re-bumping buffer for missing seals"
                );
                self.bump_buffer_revision(project, binding);
            }
            crate::runtime::observer::InternalEvent::LlmRequestBuilt {
                context_window,
                token_breakdown,
                context_envelope,
                context_meter,
                ..
            } => {
                // Local `token_estimate` is budget-only — never occupancy/ring truth.
                self.context_window = *context_window;
                self.context_token_breakdown = token_breakdown.clone();
                self.context_envelope = context_envelope.clone();
                self.context_meter = context_meter.clone();
                self.context_tokens_estimate = context_meter.used_tokens;
                self.context_unknown = false;
            }
            crate::runtime::observer::InternalEvent::LlmCompleted {
                prompt_tokens,
                completion_tokens,
                cache_hit_tokens,
                cache_miss_tokens,
                context_meter,
                ..
            } => {
                self.context_meter = context_meter.clone();
                self.context_tokens_estimate = context_meter.used_tokens;
                self.context_unknown = false;
                self.last_turn_token_stats =
                    Some(crate::client_protocol::protocol::TurnTokenStats {
                        prompt_tokens: *prompt_tokens,
                        completion_tokens: *completion_tokens,
                        cache_hit_tokens: *cache_hit_tokens,
                        cache_miss_tokens: *cache_miss_tokens,
                    });
                // Session-total accumulator: Σ per-request usage (same as persisted
                // cum_* which adds this turn's Σ at turn completion).
                let cum = self
                    .cumulative_token_stats
                    .get_or_insert_with(crate::client_protocol::protocol::TurnTokenStats::default);
                cum.prompt_tokens = cum.prompt_tokens.saturating_add(*prompt_tokens);
                cum.completion_tokens = cum.completion_tokens.saturating_add(*completion_tokens);
                cum.cache_hit_tokens = cum.cache_hit_tokens.saturating_add(*cache_hit_tokens);
                cum.cache_miss_tokens = cum.cache_miss_tokens.saturating_add(*cache_miss_tokens);
                let billed = prompt_tokens.saturating_add(*completion_tokens);
                self.provider_billed_tokens = Some(
                    self.provider_billed_tokens
                        .unwrap_or(0)
                        .saturating_add(billed),
                );
                self.provider_billed_requests = self.provider_billed_requests.saturating_add(1);
            }
            crate::runtime::observer::InternalEvent::TurnCompleted {
                turn_id: _,
                final_text: _,
                reason: _,
                turn_token_stats,
                committed_next_seq: _,
                error_kind: _,
                cancelled_by: _,
            } => {
                if self.turn_completed_emitted {
                    return;
                }
                self.turn_completed_emitted = true;
                self.phase = TurnPhase::Idle;
                self.turn_id = None;
                // Provider truth only — no usage this turn leaves prior hydrate intact.
                if turn_token_stats.has_provider_usage() {
                    self.last_turn_token_stats = Some(turn_token_stats.clone());
                }
                // Provider usage is now persisted on its request header; refresh
                // the same meter the next auto/manual gate will consume.
                self.refresh_context_estimate();
            }
            crate::runtime::observer::InternalEvent::BufferChanged {
                last_seq: _,
                next_seq,
                revision,
            } => {
                self.buffer_revision = *revision;
                self.next_seq = *next_seq;
            }
            crate::runtime::observer::InternalEvent::PermissionAsk {
                tool,
                rule_id,
                summary,
                ..
            } => {
                self.phase = TurnPhase::AwaitingPermission {
                    tool: tool.clone(),
                    rule_id: rule_id.clone(),
                    summary: summary.clone(),
                };
            }
            crate::runtime::observer::InternalEvent::PermissionResolved { .. } => {}
            crate::runtime::observer::InternalEvent::FileRevertUpdated { max_seq } => {
                self.max_file_revert_seq = *max_seq;
            }
            crate::runtime::observer::InternalEvent::CompactionLifecycle {
                trigger: crate::runtime::observer::CompactionTrigger::Auto,
                stage: crate::runtime::observer::CompactionStage::Started,
                ..
            } if self.turn_id.is_some() => {
                self.phase = TurnPhase::Compacting;
            }
            _ => {}
        }
    }
}

pub struct SessionController {
    pub runtime: RuntimeHandle,
    pub project: String,
    pub sessions: Arc<SessionManager>,
    pub projections: HashMap<String, Projection>,
    /// Sender for the merged event channel. Each `subscribe` spawns a task
    /// that forwards broadcast events as `(session_id, InternalEnvelope)` pairs.
    pub(super) merged_tx: mpsc::Sender<(String, InternalEnvelope)>,
    /// Receiver for the merged event channel, consumed by `run_session_loop`.
    pub(super) merged_rx: mpsc::Receiver<(String, InternalEnvelope)>,
    /// Temporary buffer for workspace-open frames (drained by take_outgoing).
    pub(super) _workspace_outgoing: Vec<serde_json::Value>,
    /// Deferred queue for requests that arrive while no projection is bound.
    pub(super) _dummy_deferred: VecDeque<SessionRequest>,
    /// Permission grants that arrived before the matching Ask was registered;
    /// consumed when that Ask is inserted into the pending-ask registry.
    pub(super) stray_grants: VecDeque<SessionRequest>,
}

impl SessionController {
    pub fn new(
        runtime: RuntimeHandle,
        session_id: Option<String>,
        sessions: Arc<SessionManager>,
    ) -> anyhow::Result<Self> {
        Self::with_turn_guard(runtime, session_id, sessions)
    }

    pub fn with_turn_guard(
        runtime: RuntimeHandle,
        _session_id: Option<String>,
        sessions: Arc<SessionManager>,
    ) -> anyhow::Result<Self> {
        let project = runtime
            .workspace
            .workspace_root
            .to_string_lossy()
            .to_string();
        // Bounded merged channel (FIX-2): a slow connection loop must not grow
        // memory without limit; the forwarder drops + signals lag on Full.
        let (merged_tx, merged_rx) = mpsc::channel::<(String, InternalEnvelope)>(1024);
        let ctrl = Self {
            runtime,
            project,
            sessions,
            projections: HashMap::new(),
            merged_tx,
            merged_rx,
            _workspace_outgoing: Vec::new(),
            _dummy_deferred: VecDeque::new(),
            stray_grants: VecDeque::new(),
        };
        // SessionRecord creation is handled by `subscribe()` or `open_session()`.
        // The session_id hint no longer pre-loads a Session here.
        Ok(ctrl)
    }

    // ── subscribe / unsubscribe ──

    /// Subscribe this connection to `session_id`'s turn-event broadcast.
    /// Creates a `Projection` with a fresh `broadcast_rx` and spawns a
    /// forwarding task that relays events to `merged_tx`.
    ///
    /// Persistence is owned by `SessionManager.entries` — Projection does NOT
    /// open its own SQLite connection.
    pub async fn subscribe_checked(&mut self, session_id: &str) -> anyhow::Result<()> {
        if self.projections.contains_key(session_id) {
            return Ok(());
        }

        // Ensure the SessionRecord exists in SessionManager (resume from SQLite if needed).
        self.sessions.ensure_entry(session_id).await?;

        // Ensure the session runtime exists in SessionManager (attach).
        let cached = self.sessions.attach(session_id);

        let envelope_rx = match self.sessions.subscribe(session_id) {
            Some(rx) => rx,
            None => {
                self.sessions.detach(session_id);
                return Err(anyhow::anyhow!(
                    "session {session_id} has no event receiver"
                ));
            }
        };

        let binding = self.session_binding(session_id);
        let context_window = binding.context_window;

        let sessions = self.sessions.clone();
        let request_model = self.resolved_model_for_binding(&binding);
        let mut proj = Projection::new_with_model(
            session_id.to_string(),
            sessions.clone(),
            context_window,
            request_model,
        );
        proj.max_file_revert_seq =
            snapshot::max_file_revert_seq(&self.runtime.workspace.paths.snapshots_dir, session_id);

        // Cold subscribe with no persisted request header: seed the full
        // server-owned envelope from the same main-turn shape a real request
        // would use. The browser never reconstructs instructions/tools/output.
        let has_persisted_envelope =
            crate::context_pipeline::latest_persisted_envelope(&sessions, session_id).is_some();
        let mut cold_seed_unknown = false;
        if !has_persisted_envelope {
            let agent_id = binding.agent_id.clone();
            let depth = self
                .sessions
                .data()
                .meta_blocking(session_id)
                .map(|meta| meta.subagent_depth)
                .unwrap_or(0);
            match self
                .cold_compaction_request(session_id, &agent_id, depth)
                .await
            {
                Ok(request) => {
                    proj.set_request_model(Some(Arc::clone(&request.target)));
                    proj.set_context_envelope(
                        crate::context_pipeline::ContextEnvelope::from_compaction_request(&request),
                    );
                }
                Err(error) => {
                    cold_seed_unknown = true;
                    tracing::warn!(
                        session_id = %session_id,
                        %error,
                        "subscribe could not cold-seed the server request envelope"
                    );
                }
            }
        }
        // If the cold runtime could not produce a request shape, every
        // snapshot emitted below (including buffered-event restore snapshots)
        // must omit the context fields rather than publish 0/0.
        proj.context_unknown = cold_seed_unknown;

        // R7: if a turn is already running for this session, restore state.
        if let Some(progress) = cached
            && self.sessions.is_turn_running_blocking(session_id)
        {
            let snapshot = project::turn_progress_to_snapshot(&progress);
            proj.turn_id = Some(snapshot.turn_id.clone());
            proj.phase = project::wire_phase_to_internal(&snapshot.phase);
            proj.step = snapshot.step;
            proj.step_max = snapshot.step_max;
            proj.started_at_ms = snapshot.started_at_ms;

            let events = self.sessions.event_buffer_snapshot(session_id);
            let project_str = self.project.clone();
            for envelope in events {
                proj.on_internal(envelope, &project_str, &binding);
            }
            proj.push_outgoing(project::session_attached(session_id, &snapshot));
        }

        let project_str = self.project.clone();
        let mut snapshot = proj.snapshot(&project_str, &binding);
        // (Re)subscribe is the client's sync point for transient bash state.
        snapshot.bash = Some(self.runtime.ide.terminal.jobs.wire_snapshot(session_id));
        snapshot.pending_messages = self.sessions.pending_messages_snapshot(session_id);
        // WP-186: re-assert after bash overlay (snapshot() already filled; keep explicit).
        let pending = self.sessions.pending_permissions_snapshot(session_id);
        snapshot.pending_permissions = project::pending_records_to_wire(&pending);
        if let Some(turn) = snapshot.turn.as_mut() {
            turn.awaiting_permission = !snapshot.pending_permissions.is_empty();
        }
        if let Some(live) = snapshot.live.as_mut() {
            live.awaiting_permission = !snapshot.pending_permissions.is_empty();
        }
        proj.push_outgoing(session_snapshot(snapshot));

        let sid_owned = session_id.to_string();
        let merged_tx = self.merged_tx.clone();
        let mut fwd_rx = envelope_rx;
        let forward_task = Some(tokio::spawn(async move {
            loop {
                match fwd_rx.recv().await {
                    Ok(envelope) => {
                        // Bounded merged channel (FIX-2): on Full, drop the frame
                        // and signal lag explicitly instead of blocking the
                        // session-side broadcast or growing memory unbounded.
                        match merged_tx.try_send((sid_owned.clone(), envelope)) {
                            Ok(()) => {}
                            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                tracing::warn!(
                                    session_id = %sid_owned,
                                    "merged event channel full; dropping frame (lagged resync)"
                                );
                                let _ = merged_tx.try_send((
                                    sid_owned.clone(),
                                    crate::runtime::observer::InternalEnvelope {
                                        event: crate::runtime::observer::InternalEvent::ProjectionLagged {
                                            skipped: 1,
                                        },
                                        parent_session_id: None,
                                    },
                                ));
                            }
                            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            session_id = %sid_owned,
                            skipped = n,
                            "session event broadcast lagged; requesting buffer resync"
                        );
                        let _ = merged_tx.try_send((
                            sid_owned.clone(),
                            crate::runtime::observer::InternalEnvelope {
                                event: crate::runtime::observer::InternalEvent::ProjectionLagged {
                                    skipped: n,
                                },
                                parent_session_id: None,
                            },
                        ));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
        proj.forward_task = forward_task;

        self.projections.insert(session_id.to_string(), proj);
        Ok(())
    }

    pub async fn subscribe(&mut self, session_id: &str) {
        if let Err(error) = self.subscribe_checked(session_id).await {
            tracing::warn!(error = %error, session_id, "subscribe: failed to ensure session entry");
        }
    }

    /// Unsubscribe from `session_id`. Removes the Projection (dropping its
    /// broadcast_rx). Does NOT cancel the turn, remove the session, or delete
    /// the SQLite row.
    pub fn unsubscribe(&mut self, session_id: &str) {
        if let Some(proj) = self.projections.remove(session_id)
            && let Some(handle) = proj.forward_task
        {
            handle.abort();
        }
        self.sessions.detach(session_id);
    }

    // ── accessors ──

    /// Get a mutable reference to the Projection for `session_id`.
    pub fn projection_mut(&mut self, session_id: &str) -> Option<&mut Projection> {
        self.projections.get_mut(session_id)
    }

    /// Get a reference to the Projection for `session_id`.
    pub fn projection(&self, session_id: &str) -> Option<&Projection> {
        self.projections.get(session_id)
    }

    /// Primary/active projection session id (lexicographically first bound
    /// projection — deterministic, never HashMap iteration order).
    pub fn first_session_id(&self) -> Option<String> {
        self.projections.keys().min().cloned()
    }

    /// Whether any projection is subscribed.
    pub fn has_projections(&self) -> bool {
        !self.projections.is_empty()
    }

    // ── snapshot / handshake ──

    /// Generate a snapshot for the given session_id.
    pub fn snapshot_for(&self, session_id: &str) -> Option<SessionSnapshot> {
        let binding = self.session_binding(session_id);
        self.projection(session_id)
            .map(|p| p.snapshot(&self.project, &binding))
    }

    /// Snapshot for the first (or only) bound projection, if any. Returns
    /// `None` when no projection is subscribed — callers surface an explicit
    /// error instead of fabricating an empty (0,0,0) snapshot.
    pub fn snapshot(&self) -> Option<SessionSnapshot> {
        self.first_session_id()
            .and_then(|sid| self.snapshot_for(&sid))
    }

    pub fn server_hello(&self) -> serde_json::Value {
        let primary_agents: Vec<crate::client_protocol::protocol::PrimaryAgentInfo> =
            crate::config::bridge::primary_agent_infos(&self.runtime.resolved)
                .into_iter()
                .map(
                    |(id, description)| crate::client_protocol::protocol::PrimaryAgentInfo {
                        id,
                        description,
                    },
                )
                .collect();
        let models: Vec<ModelInfo> = project::model_infos(&self.runtime.resolved);
        project::server_hello(
            crate::version::VERSION.into(),
            crate::version::channel().into(),
            self.first_session_id().unwrap_or_default(),
            self.project.clone(),
            self.runtime.workspace.workspace_id.clone(),
            self.runtime.settings_revision(),
            self.runtime.desired_primary_agent().to_string(),
            primary_agents,
            models,
        )
    }

    pub fn handshake_frames(&self) -> Vec<serde_json::Value> {
        vec![self.server_hello()]
    }

    // ── outgoing ──

    /// Take outgoing frames from all projections.
    pub fn take_all_outgoing(&mut self) -> Vec<serde_json::Value> {
        let mut result = Vec::new();
        result.extend(std::mem::take(&mut self._workspace_outgoing));
        for proj in self.projections.values_mut() {
            result.extend(proj.take_outgoing());
        }
        result
    }

    /// Take outgoing frames from all projections.
    pub fn take_outgoing(&mut self) -> Vec<serde_json::Value> {
        self.take_all_outgoing()
    }

    /// Take outgoing frames from a specific projection.
    pub fn take_outgoing_for(&mut self, session_id: &str) -> Vec<serde_json::Value> {
        self.projection_mut(session_id)
            .map(|p| p.take_outgoing())
            .unwrap_or_default()
    }

    /// `parent_call_id → child session id` for durable subagent children.
    pub fn child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> std::collections::HashMap<String, String> {
        self.sessions.child_bindings_for_parent(parent_session_id)
    }

    /// WP-091 R7 / P6: live bindings plus tombstones of deleted children.
    /// `subagent_bindings` keeps tombstoned entries too (compat).
    pub fn subagent_bindings_and_states(
        &self,
        parent_session_id: &str,
    ) -> (
        std::collections::HashMap<String, String>,
        std::collections::HashMap<String, crate::client_protocol::protocol::SubagentBindingState>,
    ) {
        use crate::client_protocol::protocol::SubagentBindingState;
        let mut bindings = self.sessions.child_bindings_for_parent(parent_session_id);
        let mut states: std::collections::HashMap<String, SubagentBindingState> = bindings
            .iter()
            .map(|(call, child)| {
                (
                    call.clone(),
                    SubagentBindingState {
                        child_session_id: child.clone(),
                        deleted: false,
                    },
                )
            })
            .collect();
        for (call, child) in self
            .sessions
            .deleted_child_bindings_for_parent(parent_session_id)
        {
            if states.contains_key(&call) {
                continue; // a live row wins (should not happen)
            }
            bindings.insert(call.clone(), child.clone());
            states.insert(
                call,
                SubagentBindingState {
                    child_session_id: child,
                    deleted: true,
                },
            );
        }
        (bindings, states)
    }

    // ── runtime helpers ──

    pub fn session_binding(&self, session_id: &str) -> SessionBindingProjection {
        project::binding_projection(
            &self.sessions,
            session_id,
            &self.runtime.resolved,
            self.runtime.desired_primary_agent(),
        )
    }

    pub(super) fn resolved_model_for_binding(
        &self,
        binding: &SessionBindingProjection,
    ) -> Option<Arc<crate::provider_catalog::ResolvedModel>> {
        binding
            .model_id
            .as_deref()
            .and_then(|id| self.runtime.resolved.catalog().model(id).cloned())
    }

    /// Build the same full server-owned request envelope a cold subscription
    /// uses. This is shared by `session/new` so its first snapshot never
    /// publishes a 0/0 instructions/tools placeholder.
    pub(super) async fn cold_compaction_request(
        &mut self,
        session_id: &str,
        agent_id: &str,
        depth: u32,
    ) -> anyhow::Result<crate::llm::CompactionRequestContext> {
        self.runtime.sync_workspace_tool_readiness();
        let mut runtime = self.runtime.build_runtime(
            session_id.to_string(),
            Arc::clone(&self.sessions),
            agent_id,
            depth,
            crate::permission::deny_permission_sink(),
            Arc::new(crate::runtime::observer::NoopObserver),
        )?;
        runtime.ensure_tooling_initialized().await?;
        Ok(runtime.compaction_request_context()?)
    }

    pub(super) fn runtime_step_max(&self, session_id: &str) -> u32 {
        let primary = self
            .sessions
            .agent_id(session_id)
            .unwrap_or_else(|| self.runtime.desired_primary_agent().to_string());
        crate::config::bridge::agent_config_for(&self.runtime.resolved, &primary)
            .map(|a| a.max_steps)
            .unwrap_or(50)
    }

    /// Permission sink for a specific session.
    pub fn permission_sink_for(
        &self,
        session_id: &str,
        perm_tx: &mpsc::UnboundedSender<PendingPermission>,
        turn_id: &str,
    ) -> Arc<dyn PermissionSink> {
        // 2.14: the caller generates the turn_id BEFORE the sink, so permission
        // wire frames carry a real turn_id (never the old "no-turn" fallback).
        permission_bridge::ws_permission_sink(
            session_id,
            turn_id,
            &self.runtime.agent_name,
            perm_tx,
        )
    }

    /// Clear turn handles for all projections.
    pub fn clear_handle(&mut self) {
        for proj in self.projections.values_mut() {
            proj.clear_handle();
        }
    }

    /// Clear the turn handle for a specific session.
    pub fn clear_handle_for(&mut self, session_id: &str) {
        if let Some(proj) = self.projection_mut(session_id) {
            proj.clear_handle();
        }
    }

    /// Whether any subscribed session has a running turn (SessionManager).
    pub fn is_turn_running(&self) -> bool {
        self.projections
            .keys()
            .any(|id| self.sessions.is_turn_running_blocking(id))
    }

    /// Whether a turn is running for a specific session (SessionManager only).
    pub fn is_turn_running_for(&self, session_id: &str) -> bool {
        self.sessions.is_turn_running_blocking(session_id)
    }

    /// Whether a turn or an exclusive standalone operation owns this session.
    pub fn is_session_busy_for(&self, session_id: &str) -> bool {
        self.sessions.is_session_busy_blocking(session_id)
    }
}

impl Drop for SessionController {
    fn drop(&mut self) {
        // 2.9: abort each projection's forward task and detach from its session
        // so no task or subscription outlives the controller. Turn lifecycle is
        // owned by the process-level `SessionManager` — dropping a connection
        // must NOT cancel a running turn.
        let sids: Vec<String> = self.projections.keys().cloned().collect();
        for sid in sids {
            if let Some(proj) = self.projections.get_mut(&sid)
                && let Some(handle) = proj.forward_task.take()
            {
                handle.abort();
            }
            self.sessions.detach(&sid);
        }
    }
}

/// Run [`snapshot::snapshot_restore`] on the blocking pool when inside a Tokio
/// runtime so git2/CLI disk I/O does not stall the async worker (Phase 3).
fn run_snapshot_restore_off_async(
    workspace: std::path::PathBuf,
    snapshots_dir: std::path::PathBuf,
    session_id: String,
    stem: i64,
) -> crate::types::Result<snapshot::RestoreOutcome> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| {
            handle.block_on(async move {
                tokio::task::spawn_blocking(move || {
                    snapshot::snapshot_restore(&workspace, &snapshots_dir, &session_id, stem)
                })
                .await
                .map_err(|e| {
                    LitecodeError::ToolExecution(format!(
                        "snapshot_restore spawn_blocking join: {e}"
                    ))
                })?
            })
        }),
        Err(_) => snapshot::snapshot_restore(&workspace, &snapshots_dir, &session_id, stem),
    }
}

#[cfg(test)]
mod merged_channel_tests {
    //! Acceptance for the FIX-2 / REV-8 merged event channel: the WS-session
    //! merged channel must be *bounded* and its overflow handled *explicitly*
    //! (drop + ProjectionLagged) — never growing memory without limit and never
    //! blocking the session-side broadcast. This mirrors the production
    //! forwarder in `subscribe_checked` (bounded `mpsc::channel(1024)` +
    //! `try_send` with `Full => drop + ProjectionLagged`).
}

#[cfg(test)]
mod compact_item_wire_tests {
    use super::*;

    use crate::runtime::observer::{
        CompactionFailKind, CompactionStage, CompactionTelemetry, CompactionTrigger,
    };
    use crate::types::user_text;
    use std::sync::Arc;

    fn binding() -> SessionBindingProjection {
        SessionBindingProjection::default()
    }

    fn setup_with_details(texts: &[&str]) -> (Projection, String, Arc<SessionManager>) {
        let sessions = Arc::new(SessionManager::ephemeral_registry());
        let sid = sessions
            .open_session_sync("/p", "default", Some("m"))
            .unwrap();
        sessions
            .insert_detail_rows(
                &sid,
                &texts.iter().map(|t| user_text(*t)).collect::<Vec<_>>(),
            )
            .unwrap();
        let proj = Projection::new(sid.clone(), sessions.clone(), 0);
        (proj, sid, sessions)
    }

    fn insert_detail(sessions: &SessionManager, sid: &str, text: &str) {
        sessions
            .insert_detail_rows(sid, &[user_text(text)])
            .unwrap();
    }

    fn buffer_item_frames(out: &[serde_json::Value]) -> Vec<&serde_json::Value> {
        out.iter()
            .filter(|msg| msg["method"] == crate::client_protocol::protocol::methods::BUFFER_ITEM)
            .collect()
    }

    fn meter(
        used: usize,
        source: crate::context_pipeline::ContextMeterSource,
    ) -> crate::context_pipeline::ContextMeter {
        crate::context_pipeline::ContextMeter::new(1_000, 1_000, 100, used, source)
    }

    fn request_built_event(
        context_meter: crate::context_pipeline::ContextMeter,
        context_envelope: crate::context_pipeline::ContextEnvelope,
    ) -> InternalEvent {
        InternalEvent::LlmRequestBuilt {
            model: "m".into(),
            endpoint: "responses".into(),
            token_estimate: context_meter.used_tokens,
            tools_count: 2,
            context_window: context_meter.context_window,
            token_breakdown: crate::session::estimate::ItemTokenBreakdown::default(),
            context_envelope,
            context_meter,
            sticky_model_id: String::new(),
            turn_model_ref: String::new(),
            turn_provider_id: String::new(),
            turn_endpoint_type: String::new(),
            bind_matches_sticky: false,
            origin_provider_id: String::new(),
            origin_endpoint_type: String::new(),
            origin_model_ref: String::new(),
            ciphertexts_stripped: 0,
            rebased_stripped: 0,
            reasoning_dropped: 0,
            reasoning_wholly_omitted: 0,
            rebased_true_count: 0,
            projection_rewrote: false,
            severity: String::new(),
            code: String::new(),
            message: String::new(),
        }
    }

    fn latest_snapshot(out: &[serde_json::Value]) -> &serde_json::Value {
        out.iter()
            .rev()
            .find(|msg| msg["method"] == "session/snapshot")
            .expect("server meter snapshot")
    }

    #[test]
    fn quality_compaction_failure_carries_subtype_on_operation_result_wire() {
        let (mut proj, _sid, _sessions) = setup_with_details(&["u0"]);
        proj.on_event(
            InternalEvent::CompactionLifecycle {
                trigger: CompactionTrigger::Manual,
                stage: CompactionStage::Failed,
                operation_id: Some("op-quality".into()),
                fail_kind: Some(CompactionFailKind::Quality),
                error: Some("summary quality rejected".into()),
                telemetry: Default::default(),
            },
            "/p",
            &binding(),
        );
        let out = proj.take_outgoing();
        let result = out
            .iter()
            .find(|msg| {
                msg["method"] == "agent/operation_result"
                    && msg["params"]["op"] == "compact_session"
            })
            .expect("manual compact failure must publish an operation result");
        assert_eq!(result["params"]["ok"], false);
        assert_eq!(result["params"]["error"]["code"], "compaction_quality");
        assert_eq!(
            result["params"]["error"]["message"],
            "summary quality rejected"
        );
    }

    #[test]
    fn compaction_lifecycle_wire_preserves_measured_telemetry() {
        let (mut proj, _sid, _sessions) = setup_with_details(&["u0"]);
        let telemetry = CompactionTelemetry {
            tokens_before: Some(100),
            tokens_pruned: Some(10),
            tokens_after: Some(70),
            summary_attempts: Some(2),
            overflow_retry: Some(0),
        };

        for stage in [CompactionStage::Started, CompactionStage::Failed] {
            proj.on_event(
                InternalEvent::CompactionLifecycle {
                    trigger: CompactionTrigger::Auto,
                    stage,
                    operation_id: Some("op-telemetry-wire".into()),
                    fail_kind: (stage == CompactionStage::Failed)
                        .then_some(CompactionFailKind::Quality),
                    error: (stage == CompactionStage::Failed)
                        .then(|| "summary quality rejected".into()),
                    telemetry: telemetry.clone(),
                },
                "/p",
                &binding(),
            );
            let out = proj.take_outgoing();
            let frame = out
                .iter()
                .find(|msg| msg["method"] == "session/compact_lifecycle")
                .expect("compaction lifecycle wire frame");
            let wire = &frame["params"]["telemetry"];
            assert_eq!(wire["tokens_before"], 100);
            assert_eq!(wire["tokens_pruned"], 10);
            assert_eq!(wire["tokens_after"], 70);
            assert_eq!(wire["summary_attempts"], 2);
            assert_eq!(wire["overflow_retry"], 0);
        }
    }

    #[test]
    fn request_completion_and_step_events_publish_the_live_server_meter() {
        let (mut proj, _sid, _sessions) = setup_with_details(&["u0"]);
        proj.apply_turn_started("t-meter".into(), 3);
        let envelope =
            crate::context_pipeline::ContextEnvelope::from_request_parts("system", &[], 100);
        let request_meter = meter(
            900,
            crate::context_pipeline::ContextMeterSource::ProviderPlusSuffix,
        );
        proj.on_event(
            request_built_event(request_meter.clone(), envelope.clone()),
            "/p",
            &binding(),
        );
        let out = proj.take_outgoing();
        let live = out
            .iter()
            .find(|msg| msg["method"] == "agent/turn_event")
            .expect("live request event");
        assert_eq!(
            live["params"]["event"]["context_meter"],
            serde_json::to_value(&request_meter).unwrap()
        );
        let snap = latest_snapshot(&out)["params"].clone();
        assert_eq!(
            snap["context_meter"],
            serde_json::to_value(&request_meter).unwrap()
        );
        assert_eq!(snap["context_tokens_estimate"], 900);
        assert_eq!(snap["compact_eligible"], true);

        let completed_meter = meter(1_000, crate::context_pipeline::ContextMeterSource::Provider);
        proj.on_event(
            InternalEvent::LlmCompleted {
                prompt_tokens: 1_000,
                completion_tokens: 10,
                cache_hit_tokens: 0,
                cache_miss_tokens: 1_000,
                stop_reason: "stop".into(),
                context_meter: completed_meter.clone(),
            },
            "/p",
            &binding(),
        );
        let out = proj.take_outgoing();
        let live = out
            .iter()
            .find(|msg| msg["method"] == "agent/turn_event")
            .expect("live completion event");
        assert_eq!(
            live["params"]["event"]["context_meter"],
            serde_json::to_value(&completed_meter).unwrap()
        );
        let snap = latest_snapshot(&out)["params"].clone();
        assert_eq!(
            snap["context_meter"],
            serde_json::to_value(&completed_meter).unwrap()
        );
        assert_eq!(snap["context_tokens_estimate"], 1_000);

        // Even with no resolved model in this projection-only test, the step
        // commit must not leave the ring on a stale meter: it still publishes
        // the authoritative current value.
        proj.on_event(InternalEvent::StepCommitted, "/p", &binding());
        let out = proj.take_outgoing();
        let snap = latest_snapshot(&out)["params"].clone();
        assert_eq!(
            snap["context_meter"],
            serde_json::to_value(&completed_meter).unwrap()
        );
    }

    #[test]
    fn cancel_seal_restamps_changed_seqs_without_next_seq_growth() {
        use crate::authority::responses::{
            AssistantRole, FunctionToolCall, MessageItem, OutputMessage, OutputMessageContent,
            OutputStatus, OutputTextContent,
        };
        use crate::types::Item;

        let (mut proj, sid, sessions) = setup_with_details(&["u0"]);
        sessions
            .begin_stream_item(
                &sid,
                &Item::Message(MessageItem::Output(OutputMessage {
                    id: "asst_live".into(),
                    role: AssistantRole::Assistant,
                    content: vec![OutputMessageContent::OutputText(OutputTextContent {
                        text: "hel".into(),
                        annotations: vec![],
                        logprobs: None,
                    })],
                    status: OutputStatus::InProgress,
                    phase: None,
                })),
                "t-live",
            )
            .unwrap();
        sessions
            .begin_stream_item(
                &sid,
                &Item::FunctionCall(FunctionToolCall {
                    arguments: "{\"cmd\":\"ls\"}".into(),
                    call_id: "call_live".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: Some("fc_live".into()),
                    status: Some(OutputStatus::InProgress),
                }),
                "t-live",
            )
            .unwrap();
        proj.bump_buffer_revision("/p", &binding());
        let _ = proj.take_outgoing();
        let next_before_seal = proj.next_seq;

        let seqs = sessions.seal_in_progress_items(&sid).unwrap();
        assert_eq!(seqs, vec![1, 2]);

        proj.bump_buffer_revision("/p", &binding());
        let after_bump = proj.take_outgoing();
        let after_next_only = buffer_item_frames(&after_bump);
        assert!(
            after_next_only.is_empty(),
            "seal must not be shipped by next_seq-only revision (next_seq={next_before_seal})"
        );
        assert_eq!(proj.next_seq, next_before_seal);

        proj.on_event(
            InternalEvent::BufferRestamp { seqs: seqs.clone() },
            "/p",
            &binding(),
        );
        let restamp_out = proj.take_outgoing();
        let restamp = buffer_item_frames(&restamp_out);
        assert_eq!(restamp.len(), 2);
        assert_eq!(restamp[0]["params"]["seq"], 1);
        assert_eq!(restamp[1]["params"]["seq"], 2);
        for frame in &restamp {
            let params = &frame["params"];
            assert!(params["kind"].as_str().unwrap().starts_with("item/"));
            assert_eq!(params["state"], "final");
            assert_eq!(params["body"]["status"], "incomplete");
            assert!(params["body"].is_object());
            assert!(params["cites"].is_array());
            assert!(params.get("item").is_none());
            assert!(params.get("type").is_none());
        }

        proj.on_event(
            InternalEvent::TurnCompleted {
                turn_id: "t-cancel".into(),
                final_text: None,
                reason: crate::runtime::observer::TurnEndReason::Cancelled,
                turn_token_stats: crate::runtime::observer::TurnTokenStats::default(),
                committed_next_seq: 0,
                error_kind: None,
                cancelled_by: None,
            },
            "/p",
            &binding(),
        );
        let after_end = proj.take_outgoing();
        assert!(
            buffer_item_frames(&after_end).is_empty(),
            "TurnCompleted must not be the restamp vehicle"
        );

        proj.on_event(InternalEvent::BufferRestamp { seqs }, "/p", &binding());
        let again_out = proj.take_outgoing();
        let again = buffer_item_frames(&again_out);
        assert_eq!(again.len(), 2);
        assert_eq!(again[0]["params"]["body"]["status"], "incomplete");
        assert_eq!(again[1]["params"]["body"]["status"], "incomplete");
    }

    #[test]
    fn revert_then_append_delivers_new_seq_using_persisted_high_water() {
        // Rows 0..8 (nine user messages); the persisted high-water is 9.
        let (mut proj, sid, sessions) =
            setup_with_details(&["m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7", "m8"]);
        proj.bump_buffer_revision("/p", &binding());
        let _ = proj.take_outgoing();
        assert_eq!(proj.next_seq, 9, "high-water after nine appends");
        assert_eq!(proj.snapshot("/p", &binding()).buffer.last_seq, 8);

        // Revert at user seq 6 → keeps seqs 0..5 (active MAX 5); the
        // allocator high-water stays at 9 because a truncate never rewinds it.
        proj.revert_to_user_anchor(6, "/p", &binding()).unwrap();
        let out = proj.take_outgoing();
        let reverted = out
            .iter()
            .find(|m| m["method"] == "buffer/reverted")
            .expect("buffer/reverted notification");
        assert_eq!(
            reverted["params"]["last_seq"], 5,
            "active tail dropped to 5"
        );
        assert_eq!(
            reverted["params"]["next_seq"], 9,
            "high-water did not move back"
        );
        assert_eq!(proj.next_seq, 9);
        assert_eq!(proj.snapshot("/p", &binding()).buffer.last_seq, 5);

        // Append must allocate the high-water seq 9, never the deleted seq 6, and
        // the next bump must deliver exactly that row.
        insert_detail(&sessions, &sid, "after-revert");
        proj.bump_buffer_revision("/p", &binding());
        let out = proj.take_outgoing();
        let items = buffer_item_frames(&out);
        assert_eq!(items.len(), 1, "the appended row must be delivered");
        assert_eq!(items[0]["params"]["seq"], 9);
        assert_eq!(proj.snapshot("/p", &binding()).buffer.last_seq, 9);
        assert_eq!(proj.next_seq, 10);
    }
}
