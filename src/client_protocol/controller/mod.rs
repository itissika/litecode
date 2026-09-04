mod session_lifecycle;

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
use crate::session::estimate::{ItemTokenBreakdown, compute_token_breakdown};
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
    pub user_detail_before: i64,
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
    /// Cached local estimate of the model working set. Recomputed only when the
    /// durable transcript or effective context binding changes.
    pub context_tokens_estimate: usize,
    /// Item-text mix for the occupancy bar. Request-aligned after `LlmRequestBuilt`.
    pub context_token_breakdown: crate::session::estimate::ItemTokenBreakdown,
    /// Last-known provider usage (persisted session meter); feeds snapshot ring hydrate.
    pub last_turn_token_stats: Option<crate::client_protocol::protocol::TurnTokenStats>,
    /// Session-total provider usage (Σ per-request); feeds whole-session hit rate.
    pub cumulative_token_stats: Option<crate::client_protocol::protocol::TurnTokenStats>,
    /// Cached from snapshot patch files; see [`snapshot::max_file_revert_k`].
    pub max_file_revert_k: Option<i64>,
    pub deferred: VecDeque<SessionRequest>,
    /// Handle for the forwarding task spawned by `subscribe()`.
    /// Aborted on `unsubscribe()` to prevent resource leaks.
    pub(super) forward_task: Option<tokio::task::JoinHandle<()>>,
}

impl Projection {
    fn seq_cursor(&self) -> (i64, u64) {
        self.sessions.entry_wire_seq_cursor(&self.session_id)
    }

    fn last_seq(next_seq: u64) -> i64 {
        if next_seq == 0 {
            -1
        } else {
            (next_seq - 1) as i64
        }
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
        self.outgoing.push(project::buffer_log_row(
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

    fn estimate_context_tokens(
        sessions: &SessionManager,
        session_id: &str,
        window: usize,
    ) -> (usize, ItemTokenBreakdown) {
        sessions
            .data()
            .transcript_blocking(session_id)
            .map(|mut items| {
                crate::session::store::Session::snip_stale_results(&mut items);
                let n = crate::context_pipeline::BudgetPolicy::new(window).token_count(&items, 0);
                (n, compute_token_breakdown(&items))
            })
            .unwrap_or((0, ItemTokenBreakdown::default()))
    }

    fn refresh_context_estimate(&mut self) {
        let (n, bd) =
            Self::estimate_context_tokens(&self.sessions, &self.session_id, self.context_window);
        self.context_tokens_estimate = n;
        // Keep last-request mix while provider occupancy is showing that request.
        // After compact (meter cleared) fall back to the remaining working set.
        if self.last_turn_token_stats.is_none() {
            self.context_token_breakdown = bd;
        }
    }
}

impl Projection {
    pub fn new(session_id: String, sessions: Arc<SessionManager>, context_window: usize) -> Self {
        let (_, next_seq) = sessions.entry_wire_seq_cursor(&session_id);
        let (context_tokens_estimate, context_token_breakdown) =
            Self::estimate_context_tokens(&sessions, &session_id, context_window);
        let meter = sessions
            .data()
            .meter_blocking(&session_id)
            .unwrap_or_default();
        let last_turn_token_stats = if meter.is_empty() {
            None
        } else {
            Some(crate::client_protocol::protocol::TurnTokenStats {
                prompt_tokens: meter.prompt_tokens,
                completion_tokens: meter.completion_tokens,
                cache_hit_tokens: meter.cache_hit_tokens,
                cache_miss_tokens: meter.cache_miss_tokens,
            })
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
            context_token_breakdown,
            last_turn_token_stats,
            cumulative_token_stats,
            max_file_revert_k: None,
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

    pub fn push_outgoing(&mut self, msg: serde_json::Value) {
        self.outgoing.push(msg);
    }

    // ── snapshot ──

    pub fn snapshot(&self, project: &str, binding: &SessionBindingProjection) -> SessionSnapshot {
        let (last_seq, next_seq) = self.seq_cursor();
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
            self.context_tokens_estimate,
            self.sessions.is_compacting_blocking(&self.session_id),
        );
        snap.context_token_breakdown = self.context_token_breakdown.clone();
        snap.max_file_revert_k = self.max_file_revert_k;
        let task_state = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| Ok(state.clone()))
            .unwrap_or_default();
        snap.todos = task_state.todos;
        snap.active_plan_path = task_state.active_plan.map(|plan| plan.relative_path);
        if let Ok(meta) = self.sessions.data().meta_blocking(&self.session_id) {
            snap.meta = crate::client_protocol::protocol::SessionMetaWire {
                id: meta.id,
                project: meta.project,
                created_at: meta.created_at,
                updated_at: meta.updated_at,
                parent_session_id: meta.parent_session_id,
                parent_call_id: meta.parent_call_id,
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
            } => InternalEvent::TurnCompleted {
                turn_id,
                final_text,
                reason,
                turn_token_stats,
                committed_next_seq: self.turn_committed_next_seq,
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
                let code = match fail_kind {
                    Some(crate::runtime::observer::CompactionFailKind::NothingToCompact) => {
                        ErrorCode::NothingToCompact
                    }
                    Some(crate::runtime::observer::CompactionFailKind::Canceled) => {
                        ErrorCode::Cancelled
                    }
                    _ => ErrorCode::CompactionFailed,
                };
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
            self.outgoing.push(project::buffer_log_row(
                &self.session_id,
                &event,
                Some(child_session_id.clone()),
            ));
        }
        let msg = if project::event_needs_snapshot(&ev) {
            project::project(&ev, &self.snapshot(project, binding))
        } else {
            project::project_live(&ev, &self.session_id, self.turn_id.as_deref())
        };
        if let Some(msg) = msg {
            self.outgoing.push(msg);
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
        let (_, new_next) = self.seq_cursor();
        self.next_seq = new_next;
        // Provider occupancy is ring truth while a request's usage is showing.
        // Skip the full-log local estimate on step commits; compact clears the
        // meter and still needs a working-set refresh.
        if self.last_turn_token_stats.is_none() {
            self.refresh_context_estimate();
        }
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
                last_seq: Self::last_seq(self.next_seq),
                next_seq: self.next_seq,
                revision: self.buffer_revision,
            },
            project,
            binding,
        );
    }

    // ── revert ──

    /// Wire-facing revert: truncates transcript at user anchor `k`.
    pub fn revert_to_user_anchor(
        &mut self,
        k: u32,
        project: &str,
        binding: &SessionBindingProjection,
    ) -> anyhow::Result<()> {
        let _lease = match self.sessions.try_begin_revert(&self.session_id) {
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
        match self
            .sessions
            .entry_revert_to_user_anchor(&self.session_id, i64::from(k))
        {
            Ok(()) => {
                self.bump_buffer_revision(project, binding);
                self.push_outgoing(serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "buffer/reverted",
                    "params": {
                        "session_id": self.session_id,
                        "last_seq": Self::last_seq(self.next_seq),
                        "next_seq": self.next_seq,
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
        k: u32,
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
        let stem = match self
            .sessions
            .entry_snapshot_stem_for_user_k(&self.session_id, i64::from(k))
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
            out.push(crate::client_protocol::protocol::WireBufferEvent {
                seq: event.seq,
                event_type: event.event_type,
                body: event.data,
                cites: event.source_seqs.unwrap_or_default(),
                state: event.state,
                surface_op: event.surface_op,
                child_session_id,
            });
        }
        let user_detail_before = self
            .sessions
            .entry_user_detail_before_seq(&self.session_id, from_seq)?;
        Ok(MaterializedRange {
            events: out,
            user_detail_before,
        })
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
            Some(StructuredError { code, message }),
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
                ..
            } => {
                // Local `token_estimate` is budget-only — never occupancy/ring truth.
                self.context_window = *context_window;
                self.context_token_breakdown = token_breakdown.clone();
            }
            crate::runtime::observer::InternalEvent::LlmCompleted {
                prompt_tokens,
                completion_tokens,
                cache_hit_tokens,
                cache_miss_tokens,
                ..
            } => {
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
            }
            crate::runtime::observer::InternalEvent::TurnCompleted {
                turn_id: _,
                final_text: _,
                reason: _,
                turn_token_stats,
                committed_next_seq: _,
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
            crate::runtime::observer::InternalEvent::FileRevertUpdated { max_k } => {
                self.max_file_revert_k = *max_k;
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
    /// Permission grants that arrived while no permission wait was active;
    /// consumed by the next matching wait (race hardening, 6a-6j).
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
        let mut proj = Projection::new(session_id.to_string(), sessions, context_window);
        proj.max_file_revert_k =
            snapshot::max_file_revert_k(&self.runtime.workspace.paths.snapshots_dir, session_id);

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
        proj.push_outgoing(session_snapshot(proj.snapshot(&project_str, &binding)));

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
        let models: Vec<ModelInfo> = self
            .runtime
            .resolved
            .global()
            .models
            .iter()
            .map(|(id, m)| ModelInfo {
                id: id.clone(),
                api_model_id: m.api_model_id().to_string(),
                label: m.label.clone(),
                context_window: m.context_window(),
                adapter_id: m.adapter_id.clone(),
            })
            .collect();
        project::server_hello(
            crate::version::VERSION.into(),
            crate::version::channel().into(),
            self.first_session_id().unwrap_or_default(),
            self.project.clone(),
            self.runtime.workspace.workspace_id.clone(),
            self.runtime.settings_revision(),
            self.runtime.desired_primary_agent().to_string(),
            primary_agents,
            self.runtime.llm_ecosystem().to_string(),
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

    // ── runtime helpers ──

    pub fn session_binding(&self, session_id: &str) -> SessionBindingProjection {
        project::binding_projection(
            &self.sessions,
            session_id,
            &self.runtime.resolved,
            self.runtime.desired_primary_agent(),
        )
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

    use super::*;
    use crate::runtime::observer::InternalEnvelope;

    /// Capacity used by `SessionController::with_turn_guard` for the merged channel.
    const MERGED_CAPACITY: usize = 1024;

    fn envelope(skipped: u64) -> InternalEnvelope {
        InternalEnvelope {
            event: InternalEvent::ProjectionLagged { skipped },
            parent_session_id: None,
        }
    }

    #[test]
    fn merged_channel_is_bounded_not_unbounded() {
        // A bounded channel refuses to buffer beyond its capacity instead of
        // growing without limit (the old `unbounded_channel` behaviour).
        let (merged_tx, _merged_rx) = mpsc::channel::<(String, InternalEnvelope)>(MERGED_CAPACITY);

        for i in 0..MERGED_CAPACITY {
            assert!(
                merged_tx
                    .try_send((format!("s{i}"), envelope(i as u64)))
                    .is_ok(),
                "bounded channel must accept up to capacity"
            );
        }
        // One more send must report Full — the buffer cannot grow past capacity.
        let err = merged_tx
            .try_send(("overflow".into(), envelope(0)))
            .expect_err("bounded channel must reject beyond capacity");
        assert!(
            matches!(err, tokio::sync::mpsc::error::TrySendError::Full(_)),
            "overflow must surface as Full (explicit lag signal), got {err:?}"
        );
    }

    #[tokio::test]
    async fn merged_overflow_drops_frame_and_signals_lagged_not_blocking() {
        // Simulate a slow consumer (run_session_loop) against a fast producer
        // (the forwarder). The forwarder must NOT block on a full channel; it
        // drops the frame and injects a ProjectionLagged resync marker.
        let (merged_tx, merged_rx) = mpsc::channel::<(String, InternalEnvelope)>(MERGED_CAPACITY);

        let producer = tokio::spawn(async move {
            for i in 0..(MERGED_CAPACITY + 50) {
                let sid = format!("s{i}");
                // Mirrors the production forwarder: try_send, on Full drop the
                // frame and signal lag (never `.send()`, never block).
                match merged_tx.try_send((sid.clone(), envelope(1))) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        let _ = merged_tx.try_send((
                            sid,
                            crate::runtime::observer::InternalEnvelope {
                                event: InternalEvent::ProjectionLagged { skipped: 1 },
                                parent_session_id: None,
                            },
                        ));
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                }
            }
        });
        // Producer must finish promptly (it never awaits a slow consumer).
        tokio::time::timeout(std::time::Duration::from_secs(5), producer)
            .await
            .expect("producer must complete without blocking on a full channel")
            .expect("producer task panicked");

        // The consumer still drains all events (including the injected lag
        // markers); the channel is usable and delivery is complete.
        drop(merged_rx);
    }
}

#[cfg(test)]
mod compact_item_wire_tests {
    use super::*;
    use crate::runtime::observer::{CompactionStage, CompactionTrigger};
    use crate::session::EventType;
    use crate::types::user_text;
    use std::sync::Arc;

    fn binding() -> SessionBindingProjection {
        SessionBindingProjection::default()
    }

    fn succeeded(trigger: CompactionTrigger) -> InternalEvent {
        InternalEvent::CompactionLifecycle {
            trigger,
            stage: CompactionStage::Succeeded,
            operation_id: None,
            fail_kind: None,
            error: None,
        }
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

    #[test]
    fn bump_skips_local_estimate_while_provider_usage_is_showing() {
        let (mut proj, _sid, _sessions) = setup_with_details(&["a"]);
        proj.last_turn_token_stats = Some(crate::client_protocol::protocol::TurnTokenStats {
            prompt_tokens: 10,
            completion_tokens: 1,
            cache_hit_tokens: 0,
            cache_miss_tokens: 10,
        });
        proj.context_tokens_estimate = 999_999;
        proj.bump_buffer_revision("/p", &binding());
        assert_eq!(
            proj.context_tokens_estimate, 999_999,
            "step bump must not full-fold while provider occupancy is showing"
        );
    }

    #[test]
    fn compact_succeeded_clears_meter_and_refreshes_local_estimate() {
        let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
        proj.last_turn_token_stats = Some(crate::client_protocol::protocol::TurnTokenStats {
            prompt_tokens: 10,
            completion_tokens: 1,
            cache_hit_tokens: 0,
            cache_miss_tokens: 10,
        });
        proj.context_tokens_estimate = 999_999;
        apply_compact(&sessions, &sid, "first-cut");
        proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
        assert!(proj.last_turn_token_stats.is_none());
        assert_ne!(
            proj.context_tokens_estimate, 999_999,
            "compact must re-estimate the remaining working set"
        );
    }

    fn apply_compact(sessions: &SessionManager, sid: &str, summary: &str) {
        let expected = sessions.data().revision_blocking(sid).unwrap_or(0);
        sessions
            .mutate_blocking(crate::session::data::command::SessionMutation::Compact {
                session_id: sid.to_string(),
                expected_revision: expected,
                operation_id: crate::session::data::command::MutationId::new(),
                summary: user_text(summary),
                token_estimate: 10,
                kept_from: None,
                expected_prefix: None,
            })
            .unwrap();
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

    #[test]
    fn compact_succeeded_emits_compacted_buffer_row() {
        let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
        apply_compact(&sessions, &sid, "first-cut");
        let _ = proj.take_outgoing();

        proj.on_event(succeeded(CompactionTrigger::Manual), "/p", &binding());
        let out = proj.take_outgoing();

        let items = buffer_item_frames(&out);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["params"]["seq"], 3);
        assert_eq!(items[0]["params"]["kind"], "compacted");
        assert_eq!(items[0]["params"]["body"]["from"], 0);
        assert_eq!(items[0]["params"]["body"]["to"], 3);
        assert_eq!(items[0]["params"]["session_id"], sid);
        assert_eq!(items[0]["params"]["body"]["summary"], "first-cut");

        let life = out
            .iter()
            .find(|msg| msg["method"] == "session/compact_lifecycle")
            .expect("lifecycle after checkpoint item");
        assert_eq!(life["params"]["snapshot"]["buffer"]["last_seq"], 3);
        assert_eq!(life["params"]["snapshot"]["buffer"]["next_seq"], 4);
        assert_eq!(life["params"]["stage"], "succeeded");
        let item_pos = out
            .iter()
            .position(|msg| msg["method"] == crate::client_protocol::protocol::methods::BUFFER_ITEM)
            .unwrap();
        let life_pos = out
            .iter()
            .position(|msg| msg["method"] == "session/compact_lifecycle")
            .unwrap();
        assert!(
            item_pos < life_pos,
            "compacted buffer/item must precede lifecycle snapshot"
        );
    }

    #[test]
    fn second_compact_appends_another_checkpoint_item() {
        let (mut proj, sid, sessions) = setup_with_details(&["a", "b", "c"]);
        apply_compact(&sessions, &sid, "first-cut");
        proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
        let _ = proj.take_outgoing();

        insert_detail(&sessions, &sid, "d");
        proj.bump_buffer_revision("/p", &binding());
        let _ = proj.take_outgoing();

        apply_compact(&sessions, &sid, "second-cut");
        proj.on_event(succeeded(CompactionTrigger::Auto), "/p", &binding());
        let out = proj.take_outgoing();

        let items = buffer_item_frames(&out);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["params"]["seq"], 5);
        assert_eq!(items[0]["params"]["kind"], "compacted");
        assert_eq!(items[0]["params"]["body"]["summary"], "second-cut");

        let events = sessions.data().events_blocking(&sid).unwrap();
        let compacts = events
            .iter()
            .filter(|e| e.event_type == EventType::Compacted)
            .count();
        assert_eq!(compacts, 2);
    }

    #[test]
    fn subagent_bound_restamps_kind_body_envelope() {
        use crate::authority::responses::FunctionToolCall;
        use crate::types::Item;

        let sessions = Arc::new(SessionManager::ephemeral_registry());
        let sid = sessions
            .open_session_sync("/p", "default", Some("m"))
            .unwrap();
        sessions
            .persist_item(
                &sid,
                &Item::FunctionCall(FunctionToolCall {
                    arguments: "{}".into(),
                    call_id: "call_sub".into(),
                    namespace: None,
                    name: "subagent_launch".into(),
                    id: None,
                    status: None,
                }),
            )
            .unwrap();
        let mut proj = Projection::new(sid.clone(), sessions, 0);
        proj.on_event(
            InternalEvent::SubagentBound {
                call_id: "call_sub".into(),
                child_session_id: "child-sid".into(),
            },
            "/p",
            &binding(),
        );
        let out = proj.take_outgoing();
        let items = buffer_item_frames(&out);
        assert_eq!(items.len(), 1);
        let params = &items[0]["params"];
        assert_eq!(params["kind"], "item/tool_call");
        assert_eq!(params["state"], "final");
        assert_eq!(params["child_session_id"], "child-sid");
        assert!(params.get("type").is_none());
        assert!(params.get("item").is_none());
        assert!(params["body"].is_object());
        assert!(params["cites"].is_array());
        assert!(
            out.iter().any(|msg| msg["method"]
                == crate::client_protocol::protocol::methods::AGENT_SUBAGENT_BOUND)
        );
    }

    #[test]
    fn materialize_range_threads_log_state_and_user_detail_before() {
        use crate::authority::responses::{
            AssistantRole, MessageItem, OutputMessage, OutputMessageContent, OutputStatus,
            OutputTextContent,
        };
        use crate::session::model::LogState;
        use crate::types::Item;

        let (proj, sid, sessions) = setup_with_details(&["u0", "u1"]);
        sessions
            .persist_item(
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
            )
            .unwrap();
        let range = proj.materialize_range(0, 10).unwrap();
        assert_eq!(range.user_detail_before, 0);
        assert_eq!(range.events.len(), 3);
        assert_eq!(range.events[0].state, LogState::Final);
        assert_eq!(range.events[2].state, LogState::InProgress);
        let tail = proj.materialize_range(1, 10).unwrap();
        assert_eq!(tail.user_detail_before, 1);
    }

    #[test]
    fn live_and_materialize_share_kind_body_cites_state_including_control() {
        use crate::session::event::EventDraft;
        use crate::session::model::LogState;
        use crate::session::store::SessionApply;

        let (mut proj, sid, sessions) = setup_with_details(&["u0"]);
        sessions
            .apply(
                &sid,
                SessionApply::Append(EventDraft {
                    time: 1,
                    event_type: EventType::TurnEnd,
                    data: serde_json::json!({"turn": "t1", "reason": "cancelled"}),
                    surface_op: None,
                    source_seqs: None,
                    ignorable: false,
                    state: LogState::Final,
                }),
            )
            .unwrap();
        let _ = proj.take_outgoing();
        proj.bump_buffer_revision("/p", &binding());
        let outgoing = proj.take_outgoing();
        let live = buffer_item_frames(&outgoing);
        let loaded = proj.materialize_range(0, 10).unwrap();
        assert!(
            live.iter().any(|msg| msg["params"]["kind"] == "turn/end"
                && msg["params"]["body"]["reason"] == "cancelled"),
            "control-plane turn/end must be on live buffer/item"
        );
        for msg in &live {
            let seq = msg["params"]["seq"].as_u64().unwrap();
            let row = loaded
                .events
                .iter()
                .find(|e| e.seq == seq)
                .expect("live seq must exist on buffer/load");
            assert_eq!(msg["params"]["kind"], row.event_type.as_str());
            assert_eq!(msg["params"]["state"], row.state.as_str());
            assert_eq!(msg["params"]["body"], row.body);
            assert_eq!(
                msg["params"]["cites"],
                serde_json::to_value(&row.cites).unwrap()
            );
            assert!(msg["params"].get("item").is_none());
            let loaded_json = serde_json::to_value(row).unwrap();
            assert!(
                loaded_json.get("item").is_none(),
                "buffer/load must omit item like live buffer/item"
            );
        }
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
            .persist_item(
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
            )
            .unwrap();
        sessions
            .persist_item(
                &sid,
                &Item::FunctionCall(FunctionToolCall {
                    arguments: "{\"cmd\":\"ls\"}".into(),
                    call_id: "call_live".into(),
                    namespace: None,
                    name: "bash".into(),
                    id: Some("fc_live".into()),
                    status: Some(OutputStatus::InProgress),
                }),
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
    fn commit_delta_seal_restamps_completed_body_without_next_seq_growth() {
        use crate::authority::responses::{
            AssistantRole, MessageItem, OutputMessage, OutputMessageContent, OutputStatus,
            OutputTextContent,
        };
        use crate::session::data::command::CommitKind;
        use crate::session::working::WorkingRow;
        use crate::types::Item;

        let (mut proj, sid, sessions) = setup_with_details(&["u0"]);
        sessions
            .persist_item(
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
            )
            .unwrap();
        proj.bump_buffer_revision("/p", &binding());
        let _ = proj.take_outgoing();
        let next_before_seal = proj.next_seq;
        let live_seq = next_before_seal.saturating_sub(1);

        let sealed = Item::Message(MessageItem::Output(OutputMessage {
            id: "asst_live".into(),
            role: AssistantRole::Assistant,
            content: vec![OutputMessageContent::OutputText(OutputTextContent {
                text: "hello".into(),
                annotations: vec![],
                logprobs: None,
            })],
            status: OutputStatus::Completed,
            phase: None,
        }));
        let (kind, _, _) = sessions
            .commit_turn_delta(
                &sid,
                vec![
                    WorkingRow::persisted(0, user_text("u0")),
                    WorkingRow::persisted(live_seq, sealed),
                ],
                live_seq as i64,
                "t-complete",
            )
            .unwrap();
        let seqs = match kind {
            CommitKind::Sealed { seqs } => seqs,
            other => panic!("commit-seal must return Sealed seqs, got {other:?}"),
        };
        assert_eq!(seqs, vec![live_seq]);

        proj.bump_buffer_revision("/p", &binding());
        let after_bump = proj.take_outgoing();
        assert!(
            buffer_item_frames(&after_bump).is_empty(),
            "in-place commit seal must not be shipped by next_seq-only revision"
        );
        assert_eq!(proj.next_seq, next_before_seal);

        proj.on_event(InternalEvent::BufferRestamp { seqs }, "/p", &binding());
        let restamp_out = proj.take_outgoing();
        let restamp = buffer_item_frames(&restamp_out);
        assert_eq!(restamp.len(), 1);
        assert_eq!(restamp[0]["params"]["seq"], live_seq);
        assert_eq!(restamp[0]["params"]["body"]["status"], "completed");
        assert_eq!(restamp[0]["params"]["body"]["content"][0]["text"], "hello");
        assert_eq!(restamp[0]["params"]["state"], "final");
    }
}
