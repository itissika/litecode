//! Typed SessionData commands. Closed enums: adding a variant without a
//! writer/reader arm is a compile error.

use serde::{Deserialize, Serialize};

use crate::session::data::sqlite::session::{SessionApply, SessionContextMeter};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitReceipt {
    pub session_id: String,
    pub operation_id: String,
    pub revision: u64,
    pub change_id: i64,
    pub outcome: CommitKind,
    /// Live session-list preview when this mutation updated `last_message`.
    /// Not part of operation identity; omitted from durable receipt JSON when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<(String, i64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitKind {
    Created,
    Appended { seq: Seq },
    Sealed { seqs: Vec<Seq> },
    Truncated { from_seq: i64 },
    Compacted { seq: Seq },
    MetaUpdated,
    Deleted,
    Idempotent,
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
    AppendJobExit {
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
    },
    Compact {
        session_id: String,
        expected_revision: u64,
        operation_id: MutationId,
        summary: Item,
        token_estimate: i64,
        kept_from: Option<Seq>,
        expected_prefix: Option<usize>,
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
    ClearOrphanedModelIds {
        operation_id: MutationId,
        valid_ids: Vec<String>,
    },
    RebuildFts {
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
            | Self::AppendJobExit { operation_id, .. }
            | Self::SealInProgress { operation_id, .. }
            | Self::CommitTurnDelta { operation_id, .. }
            | Self::Compact { operation_id, .. }
            | Self::SaveTaskState { operation_id, .. }
            | Self::SaveContextMeter { operation_id, .. }
            | Self::SetAgent { operation_id, .. }
            | Self::SetModel { operation_id, .. }
            | Self::SetThinkingTier { operation_id, .. }
            | Self::SetContextMode { operation_id, .. }
            | Self::Delete { operation_id, .. }
            | Self::ClearOrphanedModelIds { operation_id, .. }
            | Self::RebuildFts { operation_id, .. } => &operation_id.0,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Create { .. } | Self::ClearOrphanedModelIds { .. } | Self::RebuildFts { .. } => {
                None
            }
            Self::Apply { session_id, .. }
            | Self::InsertDetails { session_id, .. }
            | Self::PersistItem { session_id, .. }
            | Self::AppendJobExit { session_id, .. }
            | Self::SealInProgress { session_id, .. }
            | Self::CommitTurnDelta { session_id, .. }
            | Self::Compact { session_id, .. }
            | Self::SaveTaskState { session_id, .. }
            | Self::SaveContextMeter { session_id, .. }
            | Self::SetAgent { session_id, .. }
            | Self::SetModel { session_id, .. }
            | Self::SetThinkingTier { session_id, .. }
            | Self::SetContextMode { session_id, .. }
            | Self::Delete { session_id, .. } => Some(session_id),
        }
    }

    pub fn expected_revision(&self) -> Option<u64> {
        match self {
            Self::Create { .. } | Self::ClearOrphanedModelIds { .. } | Self::RebuildFts { .. } => {
                None
            }
            Self::Apply {
                expected_revision, ..
            }
            | Self::InsertDetails {
                expected_revision, ..
            }
            | Self::PersistItem {
                expected_revision, ..
            }
            | Self::AppendJobExit {
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
            | Self::SaveTaskState {
                expected_revision, ..
            }
            | Self::SaveContextMeter {
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
    ContextMeter {
        session_id: String,
    },
    ListSessions,
    ListSessionIds,
    ListSessionsForGc,
    ListChildIds {
        parent_session_id: String,
    },
    ChildForCall {
        parent_session_id: String,
        parent_call_id: String,
    },
    ChildBindings {
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
    UserDetailBefore {
        session_id: String,
        from_seq: i64,
    },
    SnapshotStem {
        session_id: String,
        k: i64,
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
    FtsSearch {
        query: String,
        session_id: Option<String>,
        limit: usize,
    },
    ChangeLogSince {
        last_change_id: i64,
    },
    LatestChangeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionChange {
    pub change_id: i64,
    pub session_id: String,
    pub revision: u64,
    pub kind: String,
    pub from_seq: Option<i64>,
    pub to_seq: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum ReadValue {
    Meta(crate::session::model::SessionMeta),
    Transcript(crate::types::Transcript),
    WorkingSet(Vec<WorkingRow>),
    Events(Vec<crate::session::event::SessionEvent>),
    SeqCursor { last_seq: i64, next_seq: u64 },
    Meter(SessionContextMeter),
    List(Vec<(String, String, i64, String, String, Option<String>)>),
    Ids(Vec<String>),
    GcList(Vec<(String, i64)>),
    OptionalId(Option<String>),
    ChildBindings(Vec<(String, String)>),
    Depth(u32),
    Seqs(Vec<i64>),
    Count(i64),
    Revision(u64),
    Searchable(Vec<crate::session::transcript_file::SearchableRow>),
    FtsHits(Vec<(String, i64, String)>),
    Changes(Vec<SessionChange>),
    Empty,
}
