//! Typed SessionData commands. Closed enums: adding a variant without a
//! writer/reader arm is a compile error.

use serde::{Deserialize, Serialize};

use crate::session::data::sqlite::session::{
    ProviderUsageRollup, ProviderUsageSeries, SessionApply, SessionContextMeter,
};
use crate::session::event::{EventDraft, Seq};
use crate::session::task_state::TaskReminders;
use crate::session::working::WorkingRow;
use crate::types::Item;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MutationId(pub String);

impl MutationId {
    pub fn new() -> Self {
        Self(ulid::Ulid::new().to_string())
    }
}

impl Default for MutationId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for MutationId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRevision(pub u64);

/// Session-list preview patch written with a mutation (`last_message` / `last_assistant`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionListPreview {
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<String>,
}

/// One `session/list` SQL row (roots and children).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListRow {
    pub id: String,
    pub project: String,
    pub updated_at: i64,
    pub preview: String,
    pub assistant_preview: String,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub parent_session_id: Option<String>,
    pub parent_call_id: Option<String>,
    pub responsibility: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitReceipt {
    pub session_id: String,
    pub operation_id: String,
    pub revision: u64,
    pub outcome: CommitKind,
    /// Live session-list preview when this mutation updated `last_message`.
    /// Not part of operation identity; omitted from durable receipt JSON when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<(String, i64)>,
    /// Live session-list assistant preview when this mutation updated `last_assistant`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_preview: Option<(String, i64)>,
    /// After `CommitTurnDelta`, the writer projection window (same skip rules
    /// as reader fold). Never durable; ignored by receipt JSON.
    #[serde(skip)]
    pub working_set: Option<Vec<WorkingRow>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitKind {
    Created,
    Appended {
        seq: Seq,
    },
    Sealed {
        seqs: Vec<Seq>,
    },
    Truncated {
        from_seq: i64,
    },
    Compacted {
        seq: Seq,
    },
    /// Replay-safe surface replacements appended by mechanical pruning.
    Replaced {
        seqs: Vec<Seq>,
    },
    MetaUpdated,
    Deleted,
    Idempotent,
}

/// One append-only, replay-safe surface replacement. `start`/`end` identify
/// the shadowed surface node(s); the new event cites `source_seqs` so the
/// original log row remains intact.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceReplacement {
    pub start: Seq,
    pub end: Seq,
    pub item: Item,
    pub source_seqs: Vec<Seq>,
}

#[derive(Debug)]
pub enum SessionMutation {
    Create {
        operation_id: MutationId,
        project: String,
        agent_id: String,
        model_id: Option<String>,
        parent_session_id: Option<String>,
        parent_call_id: Option<String>,
        responsibility: String,
    },
    Apply {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        op: SessionApply,
    },
    InsertDetails {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        items: Vec<Item>,
        turn_id: String,
    },
    PersistItem {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        item: Item,
    },
    /// Open the log row for a streaming provider item and return its `seq`.
    ///
    /// This is the **only** way a row enters the log un-settled. The row's
    /// lifecycle state comes from this command, never from the item payload: a
    /// provider's optional `status` field is content, not storage state, and a
    /// dialect that omits it must not be mistaken for "already settled".
    BeginStreamItem {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        item: Item,
        turn_id: String,
    },
    /// Replace the payload of a row opened by [`Self::BeginStreamItem`].
    ///
    /// Refused once the row is settled: a final row is immutable, so late
    /// content has to be a new row or nothing at all.
    UpdateStreamItem {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        seq: Seq,
        item: Item,
    },
    /// Settle a row opened by [`Self::BeginStreamItem`] with its **authoritative**
    /// payload — the terminal response's copy of the item, which is the only
    /// version the provider will accept back on replay.
    SealStreamItem {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        seq: Seq,
        item: Item,
    },
    AppendReminder {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        reminder: crate::reminder::Reminder,
    },
    AppendPlanExecute {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        item: Item,
    },
    SealInProgress {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
    },
    CommitTurnDelta {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        rows: Vec<WorkingRow>,
        expected_max_seq: i64,
        turn_id: String,
        /// UI-only structured `run_code` result data, keyed by `call_id`.
        /// Persisted beside the matching `item/tool_result` row; never an Item.
        result_data: Vec<crate::types::ToolResultData>,
    },
    Compact {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        /// Raw, role-free checkpoint summary. Never a projected model Item.
        summary: String,
        token_estimate: i64,
        kept_from: Option<Seq>,
        expected_prefix: Option<usize>,
        /// Bill for the summarizer request that produced `summary`. Stored on the
        /// checkpoint row itself, so the call is never silently free.
        usage: Option<crate::session::store::ProviderUsage>,
    },
    /// One atomic batch of mechanical-prune surface replacements. Every
    /// replacement is a new log event; the original rows are never deleted.
    ReplaceSurface {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        replacements: Vec<SurfaceReplacement>,
    },
    SaveTaskState {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        state: TaskReminders,
    },
    SaveContextMeter {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        meter: SessionContextMeter,
    },
    /// Fill `provider_usage` on one `request/header` row. Does not bump revision.
    RecordProviderUsage {
        session_id: String,
        operation_id: MutationId,
        seq: Seq,
        usage_json: String,
    },
    /// Replace the durable pending-permissions list for a session (WP-186).
    SavePendingPermissions {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        pending: Vec<crate::session::pending::PendingPermissionRecord>,
    },
    SetAgent {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        agent_id: String,
    },
    SetModel {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        model_id: Option<String>,
        /// Optional control-plane event committed with the model state change.
        event: Option<EventDraft>,
    },
    SetThinkingTier {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        tier: crate::platform_knobs::ThinkingTier,
    },
    SetContextMode {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        mode: crate::platform_knobs::ContextMode,
    },
    Delete {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
    },
}

impl SessionMutation {
    pub fn operation_id(&self) -> &str {
        match self {
            Self::Create { operation_id, .. }
            | Self::Apply { operation_id, .. }
            | Self::InsertDetails { operation_id, .. }
            | Self::PersistItem { operation_id, .. }
            | Self::BeginStreamItem { operation_id, .. }
            | Self::UpdateStreamItem { operation_id, .. }
            | Self::SealStreamItem { operation_id, .. }
            | Self::AppendReminder { operation_id, .. }
            | Self::AppendPlanExecute { operation_id, .. }
            | Self::SealInProgress { operation_id, .. }
            | Self::CommitTurnDelta { operation_id, .. }
            | Self::Compact { operation_id, .. }
            | Self::ReplaceSurface { operation_id, .. }
            | Self::SaveTaskState { operation_id, .. }
            | Self::SaveContextMeter { operation_id, .. }
            | Self::RecordProviderUsage { operation_id, .. }
            | Self::SavePendingPermissions { operation_id, .. }
            | Self::SetAgent { operation_id, .. }
            | Self::SetModel { operation_id, .. }
            | Self::SetThinkingTier { operation_id, .. }
            | Self::SetContextMode { operation_id, .. }
            | Self::Delete { operation_id, .. } => &operation_id.0,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Create { .. } => None,
            Self::Apply { session_id, .. }
            | Self::InsertDetails { session_id, .. }
            | Self::PersistItem { session_id, .. }
            | Self::BeginStreamItem { session_id, .. }
            | Self::UpdateStreamItem { session_id, .. }
            | Self::SealStreamItem { session_id, .. }
            | Self::AppendReminder { session_id, .. }
            | Self::AppendPlanExecute { session_id, .. }
            | Self::SealInProgress { session_id, .. }
            | Self::CommitTurnDelta { session_id, .. }
            | Self::Compact { session_id, .. }
            | Self::ReplaceSurface { session_id, .. }
            | Self::SaveTaskState { session_id, .. }
            | Self::SaveContextMeter { session_id, .. }
            | Self::RecordProviderUsage { session_id, .. }
            | Self::SavePendingPermissions { session_id, .. }
            | Self::SetAgent { session_id, .. }
            | Self::SetModel { session_id, .. }
            | Self::SetThinkingTier { session_id, .. }
            | Self::SetContextMode { session_id, .. }
            | Self::Delete { session_id, .. } => Some(session_id),
        }
    }

    pub fn expected_revision(&self) -> Option<u64> {
        match self {
            Self::Create { .. } | Self::RecordProviderUsage { .. } => None,
            Self::Apply {
                expected_revision, ..
            }
            | Self::InsertDetails {
                expected_revision, ..
            }
            | Self::PersistItem {
                expected_revision, ..
            }
            | Self::BeginStreamItem {
                expected_revision, ..
            }
            | Self::UpdateStreamItem {
                expected_revision, ..
            }
            | Self::SealStreamItem {
                expected_revision, ..
            }
            | Self::AppendReminder {
                expected_revision, ..
            }
            | Self::AppendPlanExecute {
                expected_revision, ..
            }
            | Self::SealInProgress {
                expected_revision, ..
            }
            | Self::CommitTurnDelta {
                expected_revision, ..
            }
            | Self::Compact {
                expected_revision, ..
            }
            | Self::ReplaceSurface {
                expected_revision, ..
            }
            | Self::SaveTaskState {
                expected_revision, ..
            }
            | Self::SaveContextMeter {
                expected_revision, ..
            }
            | Self::SavePendingPermissions {
                expected_revision, ..
            }
            | Self::SetAgent {
                expected_revision, ..
            }
            | Self::SetModel {
                expected_revision, ..
            }
            | Self::SetThinkingTier {
                expected_revision, ..
            }
            | Self::SetContextMode {
                expected_revision, ..
            }
            | Self::Delete {
                expected_revision, ..
            } => Some(*expected_revision),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionRead {
    Meta {
        session_id: String,
    },
    /// WP-186 durable pending user-gate asks.
    PendingPermissions {
        session_id: String,
    },
    Transcript {
        session_id: String,
    },
    WorkingSet {
        session_id: String,
    },
    Events {
        session_id: String,
    },
    EventsRange {
        session_id: String,
        from: i64,
        to: i64,
    },
    SeqCursor {
        session_id: String,
    },
    TruncateEpoch {
        session_id: String,
    },
    ContextMeter {
        session_id: String,
    },
    ListSessions,
    ListSessionIds,
    ListSessionsForGc,
    /// Sessions whose `updated_at` is at or after `since_ms` (activity report).
    ListSessionActivity {
        since_ms: i64,
    },
    ListChildIds {
        parent_session_id: String,
    },
    ListOrphanChildSessions,
    ChildForCall {
        parent_session_id: String,
        parent_call_id: String,
    },
    ChildBindings {
        parent_session_id: String,
    },
    /// WP-091 R7: tombstones `(parent_call_id, child_session_id)` of children
    /// deleted while this parent survived.
    DeletedChildBindings {
        parent_session_id: String,
    },
    SubagentDepth {
        session_id: String,
    },
    ResolveRef {
        refer: String,
    },
    SurfaceSeqs {
        session_id: String,
    },
    UserDetailCount {
        session_id: String,
    },
    /// Snapshot stem (`seq + 1`) when `seq` is an `item/user` row.
    UserAnchorStem {
        session_id: String,
        seq: i64,
    },
    /// `item/user` seqs around an anchor. `anchor_seq: None` means the latest user row.
    UserAnchors {
        session_id: String,
        anchor_seq: Option<i64>,
        before: i64,
        after: i64,
    },
    CheckpointSeq {
        session_id: String,
    },
    Revision {
        session_id: String,
    },
    SearchableRows {
        session_id: Option<String>,
    },
    /// `(session_id, seq)` of every final searchable row. The cheap half of a
    /// reconciliation: integers only, no bodies.
    SearchableKeys {
        session_id: Option<String>,
    },
    /// `request/header` control-plane rows: `(seq, body)` per recorded request.
    /// The body is the request's origin record — the only durable answer to
    /// "which service minted the items in this turn/step".
    RequestOrigins {
        session_id: String,
    },
    /// Recorded provider bills on `request/header` rows. Empty when none exist.
    ProviderUsageRollup {
        session_id: String,
    },
    /// Last `limit` bills for the usage chart, plus whole-session totals.
    ProviderUsageSeries {
        session_id: String,
        limit: i64,
    },
    /// Authoritative workspace spend: per natural day, per session, plus totals.
    /// No session filter — this is the one read that spans the whole workspace.
    WorkspaceUsage,
    /// Diff/Restore baseline for the reminder engine, folded from the live spine.
    SpineReminderView {
        session_id: String,
    },
    /// Bodies for exactly these rows, so an incremental refresh decodes only what
    /// is new instead of the whole corpus.
    SearchableRowsFor {
        keys: Vec<(String, i64)>,
    },
    DirtySessionsSince {
        last_change_id: i64,
    },
    LatestChangeId,
}

/// One session whose searchable keys changed after a watermark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtySession {
    pub change_id: i64,
    pub session_id: String,
}

/// One page of persisted user-message anchors (`item/user` seqs only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAnchorWindow {
    /// Ascending seqs: up to `before` strictly older, the anchor when it is a user row, then up to `after` newer.
    pub seqs: Vec<i64>,
    /// Set when the split point itself is an `item/user` row.
    pub anchor: Option<i64>,
    pub has_more_before: bool,
    pub has_more_after: bool,
}

#[derive(Debug, Clone)]
pub enum ReadValue {
    Meta(crate::session::model::SessionMeta),
    Transcript(crate::types::Transcript),
    WorkingSet(Vec<WorkingRow>),
    Events(Vec<crate::session::event::SessionEvent>),
    SeqCursor {
        last_seq: i64,
        next_seq: u64,
    },
    TruncateEpoch(u64),
    Meter(SessionContextMeter),
    List(Vec<SessionListRow>),
    Ids(Vec<String>),
    GcList(Vec<(String, i64)>),
    /// `(id, parent_session_id, updated_at, agent_id, last_message)` activity rows.
    SessionActivity(Vec<(String, Option<String>, i64, String, String)>),
    OptionalId(Option<String>),
    ChildBindings(Vec<(String, String)>),
    Depth(u32),
    Seqs(Vec<i64>),
    UserAnchors(UserAnchorWindow),
    Count(i64),
    Revision(u64),
    Searchable(Vec<crate::session::transcript_file::SearchableRow>),
    SearchableKeys(Vec<(String, i64)>),
    /// `request/header` rows as `(seq, body)` — the durable origin record of
    /// each LLM request, used to decide whether a replayed item identity still
    /// belongs to the endpoint being called.
    RequestOrigins(Vec<(i64, serde_json::Value)>),
    ProviderUsageRollup(ProviderUsageRollup),
    ProviderUsageSeries(ProviderUsageSeries),
    WorkspaceUsage(crate::session::store::WorkspaceUsage),
    SpineReminders(crate::reminder::SpineReminderView),
    DirtySessions(Vec<DirtySession>),
    /// WP-186 durable pending user-gate asks.
    PendingPermissions(Vec<crate::session::pending::PendingPermissionRecord>),
    Empty,
}
