use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::session::task_state::TodoItem;

/// L1 failure category; mapped to wire error codes in `client_protocol::project`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailReason {
    LlmHttp,
    LlmParse,
    Internal,
}

/// Runtime error surfaced on the L1 event bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnError {
    pub reason: FailReason,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionKind {
    FirstPass,
    SecondPass,
    AggressiveTruncate,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Manual,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStage {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionFailKind {
    NothingToCompact,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnPhase {
    Idle,
    Starting,
    Compacting,
    CallingLlm,
    Streaming,
    ExecutingTools,
    AwaitingPermission {
        tool: String,
        rule_id: String,
        summary: String,
    },
    Cancelling,
    Finalizing,
    Failed {
        reason: FailReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndReason {
    Completed,
    Cancelled,
    MaxSteps,
    Error,
    HookBlocked,
}

impl TurnEndReason {
    /// Durable `turn/end.reason` schema: `completed | cancelled | error | max_steps | hook_blocked`.
    pub fn as_log_reason(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
            Self::MaxSteps => "max_steps",
            Self::HookBlocked => "hook_blocked",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TurnTokenStats {
    /// Last LLM request in this turn (not summed across tool-loop steps).
    /// Provider usage only — never a local estimate.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cache_hit_tokens: u64,
    pub cache_miss_tokens: u64,
}

impl TurnTokenStats {
    /// True when any provider usage field is present (truth, not estimate).
    pub fn has_provider_usage(&self) -> bool {
        self.prompt_tokens > 0
            || self.completion_tokens > 0
            || self.cache_hit_tokens > 0
            || self.cache_miss_tokens > 0
    }
}

/// L1 runtime event; projected to wire envelopes by L2 `client_protocol`.
///
/// Historically `InternalEnvelope.parent_session_id` tagged forwarded subagent
/// events. Subagent loops now use their own session_id fanout — that field is
/// unused and left for wire compatibility only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InternalEvent {
    TurnStarted {
        turn_id: String,
        input: String,
        step_max: u32,
    },
    TurnCompleted {
        turn_id: String,
        final_text: Option<String>,
        reason: TurnEndReason,
        turn_token_stats: TurnTokenStats,
        committed_next_seq: u64,
    },
    PhaseChanged {
        phase: TurnPhase,
        step: u64,
    },
    StepStarted {
        step: u64,
        step_max: u32,
    },
    /// Authority Responses stream event (emitted when adapter streaming is wired).
    StreamEvent(crate::types::StreamEvents),
    TodoProgress {
        pending: usize,
        in_progress: usize,
        completed: usize,
        items: Vec<TodoItem>,
    },
    LlmRequestBuilt {
        model: String,
        endpoint: String,
        token_estimate: usize,
        tools_count: usize,
        context_window: usize,
    },
    LlmCompleted {
        prompt_tokens: u64,
        completion_tokens: u64,
        cache_hit_tokens: u64,
        cache_miss_tokens: u64,
        stop_reason: String,
    },
    Compaction {
        kind: CompactionKind,
        detail: Option<String>,
    },
    /// Unified compact lifecycle (manual + auto). History grows by a checkpoint
    /// item, projected like any other `buffer/item`.
    CompactionLifecycle {
        trigger: CompactionTrigger,
        stage: CompactionStage,
        operation_id: Option<String>,
        fail_kind: Option<CompactionFailKind>,
        error: Option<String>,
    },
    HookFired {
        phase: String,
        action: String,
    },
    PermissionAsk {
        session_id: String,
        turn_id: String,
        request_id: String,
        tool: String,
        rule_id: String,
        summary: String,
    },
    PermissionResolved {
        tool: String,
        approved: bool,
        always: bool,
    },
    /// Turn-level progress signal: whether the running turn is currently
    /// blocked awaiting a permission grant. Carried on the shared turn event
    /// stream (emitted by `PhasePermissionSink`) so `SessionManager` can track
    /// it in `TurnProgress` and surface it via `session/list` + `session/lifecycle`.
    /// Pure progress signal — not projected to a wire event.
    PermissionAwaiting {
        awaiting: bool,
    },
    /// Step delta persisted to DB; L2 bumps `buffer.revision` / `next_seq`.
    StepCommitted,
    /// Durable session preview (`last_message`) changed — fanout → lifecycle.
    SessionPreviewUpdated {
        preview: String,
        updated_at: i64,
    },
    /// One durable SessionLog row. Live `buffer/item` uses this same envelope
    /// (`kind`/`body`/`cites`/`state`); control-plane kinds are included here
    /// and excluded from HumanView/AgentView by spine fold.
    BufferItem {
        event: crate::session::event::SessionEvent,
        /// When this item is a `subagent_launch` function_call (or its re-stamp),
        /// the durable child session id for FE subscribe/load.
        child_session_id: Option<String>,
    },
    /// Same-seq restamp of already-allocated log rows (seal/cancel). Ordered,
    /// unique seqs. Empty means no-op. Does not imply `next_seq` growth.
    BufferRestamp {
        seqs: Vec<crate::session::event::Seq>,
    },
    /// Parent session: a subagent child was created for `call_id` (immediate bind).
    SubagentBound {
        call_id: String,
        child_session_id: String,
    },
    BufferChanged {
        last_seq: i64,
        next_seq: u64,
        revision: u64,
    },
    /// L2 broadcast subscriber lagged; session loop should re-bump buffer so
    /// dropped `StepCommitted` / seals can be healed from durable state.
    ProjectionLagged {
        skipped: u64,
    },
    WorkspaceChanged {
        paths: Vec<String>,
        kind: String,
    },
    /// Snapshot track/record degraded; projected to FE toast (not turn-fatal).
    SnapshotNotice {
        level: String,
        message: String,
    },
    /// Highest user-anchor with a nonempty file patch (file-revert button gate).
    /// Emitted after `snapshot_record_patch` so the wire snapshot is not stale
    /// relative to TurnCompleted (patch I/O runs after the turn goes idle).
    FileRevertUpdated {
        max_k: Option<i64>,
    },
    /// Session-scoped agent bash jobs (running + current waits). Projected to `bash/jobs`.
    BashJobs {
        snapshot: crate::terminal::BashJobsSnapshot,
    },
    Error(TurnError),
}

pub trait RuntimeObserver: Send + Sync {
    fn on_internal(&self, ev: InternalEvent);
}

/// Internal event with optional parent session context.
///
/// `parent_session_id` is unused after subagent isolation (child sessions fan out
/// on their own `session_id`). Kept on the envelope for wire/replay compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalEnvelope {
    pub event: InternalEvent,
    pub parent_session_id: Option<String>,
}

/// Forwards internal events to an unbounded channel (e.g. `TurnHandle.rx`).
pub struct ChannelObserver {
    tx: mpsc::UnboundedSender<InternalEnvelope>,
}

impl ChannelObserver {
    pub fn new(tx: mpsc::UnboundedSender<InternalEnvelope>) -> Arc<Self> {
        Arc::new(Self { tx })
    }
}

impl RuntimeObserver for ChannelObserver {
    fn on_internal(&self, ev: InternalEvent) {
        let _ = self.tx.send(InternalEnvelope {
            event: ev,
            parent_session_id: None,
        });
    }
}

/// Discards all events (tests / headless runs).
pub struct NoopObserver;

impl RuntimeObserver for NoopObserver {
    fn on_internal(&self, _ev: InternalEvent) {}
}

#[cfg(test)]
mod tests {
    use super::TurnEndReason;

    #[test]
    fn turn_end_reason_log_schema_is_snake_case_for_every_variant() {
        let cases = [
            (TurnEndReason::Completed, "completed"),
            (TurnEndReason::Cancelled, "cancelled"),
            (TurnEndReason::Error, "error"),
            (TurnEndReason::MaxSteps, "max_steps"),
            (TurnEndReason::HookBlocked, "hook_blocked"),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.as_log_reason(), expected);
            let v = serde_json::to_value(reason).unwrap();
            assert_eq!(v, expected);
        }
    }
}
