use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::runtime::observer::{InternalEnvelope, TurnPhase};

/// Coarse turn progress for reconnect and lifecycle broadcasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnProgress {
    pub turn_id: String,
    pub phase: TurnPhase,
    pub step: u64,
    pub step_max: u32,
    pub started_at_ms: i64,
    /// Whether the running turn is currently blocked awaiting a permission grant.
    pub awaiting_permission: bool,
}

/// Active turn handle used while a fanout task is driving events.
#[derive(Debug)]
pub struct LiveTurn {
    pub turn_id: String,
    pub cancel: CancellationToken,
    pub progress: TurnProgress,
    pub event_tx: broadcast::Sender<InternalEnvelope>,
}

/// One-shot list activity for the current turn (not a continuous state).
///
/// Emitted when a reasoning / toolcall / text segment **starts**. Frontend
/// decides presentation; backend does not aggregate or attach content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStepKind {
    Reasoning,
    Toolcall,
    Text,
}

/// Process-level session lifecycle events (L2 projects to wire JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleEvent {
    SessionRemoved {
        session_id: String,
    },
    TurnStarted {
        session_id: String,
        progress: TurnProgress,
    },
    TurnProgress {
        session_id: String,
        progress: TurnProgress,
    },
    TurnFinished {
        session_id: String,
        progress: TurnProgress,
    },
    /// `sessions.last_message` changed — session list preview patch.
    SessionPreviewUpdated {
        session_id: String,
        preview: String,
        updated_at: i64,
    },
    /// Discrete turn-step start (reasoning / toolcall / text). Not merged.
    TurnStep {
        session_id: String,
        kind: TurnStepKind,
        progress: TurnProgress,
    },
}
