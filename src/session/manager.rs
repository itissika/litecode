use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::ResolvedConfig;
use crate::config::TurnGuard;
use crate::llm::reconnect::{LlmReconnect, LlmReconnectPhase};
use crate::runtime::RuntimeHandle;
use crate::runtime::TurnHandle;
use crate::runtime::observer::{InternalEnvelope, InternalEvent, TurnEndReason, TurnPhase};
use crate::session::data::command::{MutationId, ReadValue, SessionMutation, SessionRead};
use crate::session::data::{SessionData, SessionDataReader};
use crate::session::event::{EventDraft, EventType};
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::store::{
    ProviderUsage, ProviderUsageRollup, Session, SessionApply, SessionContextMeter,
};
use crate::session::task_state::{TaskReminders, prune_stale_active_plan};
use crate::types::{LitecodeError, Result};

/// One queued user message waiting for the turn's next request seam.
///
/// Process-local by contract: the queue is the live session's steering buffer,
/// it is never written as a SessionEvent, and a process restart drops it. The only
/// durable artifact of a queued message is the ordinary `item/user` row
/// written when the queue is consumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMessage {
    pub id: String,
    pub text: String,
    /// `litecode-media:` refs, in paste order. Empty for a text-only queue entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// One user message from a claimed queue: texts joined the way the client
/// seals the in-flight bubble (`"\n\n"`), images concatenated in order.
pub fn merge_pending(messages: &[PendingMessage]) -> crate::types::UserInput {
    crate::types::UserInput {
        text: messages
            .iter()
            .map(|message| message.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
        images: messages
            .iter()
            .flat_map(|message| message.images.iter().cloned())
            .collect(),
    }
}

/// Live turn bookkeeping without wire types.
pub struct LiveTurnState {
    pub turn_id: String,
    pub cancel: CancellationToken,
    pub progress: TurnProgress,
    /// True after a cancel request while the runtime is still winding down.
    pub stopping: bool,
    /// WP-098: who requested the cancel (first writer wins).
    pub cancelled_by: Option<crate::types::StopOrigin>,
}

/// Process-local exclusive activity for one session.
///
/// The activity is reserved while holding `SessionManager::records`, before any
/// async work starts. This is the single check-and-set boundary shared by turns,
/// manual compaction, and destructive session operations.
enum SessionActivity {
    Idle,
    StartingTurn {
        turn_id: String,
        progress: TurnProgress,
        /// WP-090 R1: a cancel arrived before `start_turn` installed the
        /// RunningTurn; `start_turn` cancels the handle token immediately.
        cancel_requested: bool,
        /// WP-098: origin of the Starting-window cancel.
        cancelled_by: Option<crate::types::StopOrigin>,
    },
    RunningTurn(LiveTurnState),
    Exclusive {
        operation_id: String,
        kind: SessionOperationKind,
    },
}

/// Result of a single-lock cancel attempt on one session (WP-090 R1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// A RunningTurn existed; its token was cancelled and `stopping=true`.
    Running,
    /// A StartingTurn reservation existed; `cancel_requested=true` was recorded.
    Starting,
    /// Idle / Exclusive / unknown session: nothing to stop. No side effects.
    Idle,
}

/// WP-098 R-Q result of `enqueue_follow_up` / `cancel_and_follow_up`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpOutcome {
    /// Child idle: nothing queued; the caller starts the turn directly.
    Idle,
    /// Queued behind the live turn; 1-based position.
    Queued { position: usize },
}

/// Single-lock cancel body shared by `cancel_turn_sync` and
/// `cancel_and_follow_up`. First cancel origin wins.
fn cancel_locked(record: &mut SessionRecord, by: crate::types::StopOrigin) -> CancelOutcome {
    match &mut record.activity {
        SessionActivity::RunningTurn(live) => {
            live.stopping = true;
            live.cancelled_by.get_or_insert(by);
            live.cancel.cancel();
            CancelOutcome::Running
        }
        SessionActivity::StartingTurn {
            cancel_requested,
            cancelled_by,
            ..
        } => {
            *cancel_requested = true;
            cancelled_by.get_or_insert(by);
            CancelOutcome::Starting
        }
        SessionActivity::Idle | SessionActivity::Exclusive { .. } => CancelOutcome::Idle,
    }
}

impl CancelOutcome {
    /// true for Running | Starting (what `agent/cancel {"cancelled"}` reports).
    pub fn cancelled(self) -> bool {
        matches!(self, Self::Running | Self::Starting)
    }
}

/// WP-090 / WP-091 R8 hook: Runtime calls it after a turn activity transition
/// it owns has been decided (reservation→RunningTurn, reservation released,
/// turn finished). Always called **outside** the `records` lock, after the
/// matching lifecycle event (if any) has been emitted, on the transitioning
/// thread. Fires for child sessions too; parent aggregate
/// (`RunningWithSubagent`) is Session's to recompute.
pub type TurnStatusHook = Arc<dyn Fn(&str, SessionStatus) + Send + Sync>;

/// Public session status. `RunningWithSubagent` is the aggregate view for an
/// otherwise-idle parent whose subtree still has active turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Idle,
    Running,
    Stopping,
    RunningWithSubagent,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::RunningWithSubagent => "running_with_subagent",
        }
    }
}

/// Live child-session counts for the post-compaction reminder.
///
/// Counts only — ids, responsibilities, and results stay behind
/// `subagent_list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildCounts {
    pub total: usize,
    pub running: usize,
}

/// One session's live activity, read from the process registry.
///
/// Durable session facts (last write, preview) live in `sessions.db`; only the
/// process knows which sessions hold a turn right now. Read-only projection for
/// the workspace panel: never written back, never a control channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionActivityRow {
    pub session_id: String,
    /// Own activity only. A parent whose child is running stays `Idle` here —
    /// the child row carries the running turn; use [`SessionManager::session_status`]
    /// for the promoted `RunningWithSubagent` view.
    pub status: SessionStatus,
    /// When the current turn started, while the session is occupied by one.
    pub turn_started_at_ms: Option<i64>,
    /// Current step of the running turn as `(step, step_max)`.
    pub step: Option<(u64, u32)>,
    pub agent_id: String,
    pub model_id: Option<String>,
    pub parent_session_id: Option<String>,
}

impl SessionActivity {
    fn status(&self) -> SessionStatus {
        match self {
            Self::Idle => SessionStatus::Idle,
            Self::StartingTurn { .. } => SessionStatus::Running,
            Self::RunningTurn(live) if live.stopping => SessionStatus::Stopping,
            Self::RunningTurn(_) => SessionStatus::Running,
            Self::Exclusive { .. } => SessionStatus::Running,
        }
    }

    fn is_turn(&self) -> bool {
        matches!(self, Self::StartingTurn { .. } | Self::RunningTurn(_))
    }

    fn is_busy(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    fn progress(&self) -> Option<&TurnProgress> {
        match self {
            Self::StartingTurn { progress, .. } => Some(progress),
            Self::RunningTurn(live) => Some(&live.progress),
            Self::Idle | Self::Exclusive { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOperationKind {
    Compact,
    Revert,
    Delete,
}

/// RAII lease for a non-turn operation that must own an idle session across
/// async work. Dropping the lease releases only the matching operation.
pub struct SessionOperationLease {
    manager: Arc<SessionManager>,
    session_id: String,
    operation_id: String,
}

impl SessionOperationLease {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

impl Drop for SessionOperationLease {
    fn drop(&mut self) {
        self.manager
            .release_operation(&self.session_id, &self.operation_id);
    }
}

/// One session: exclusive activity + L2 subscription fanout + committed revision.
pub struct SessionRecord {
    pub revision: u64,
    pub task_state: TaskReminders,
    activity: SessionActivity,
    /// Revert requested while a turn is winding down. `finish_turn` transfers
    /// ownership directly to this exclusive operation without exposing Idle.
    pending_revert: Option<String>,
    /// Always present so L2 can subscribe before a turn starts.
    event_tx: broadcast::Sender<InternalEnvelope>,
    /// Ring buffer for reconnect replay (cleared on next start_turn).
    event_buffer: VecDeque<InternalEnvelope>,
    /// Queued user messages for the live turn. Outlives a turn (a cancel keeps
    /// the queue for the follow-up flush) but never touches the session log.
    pending_messages: Vec<PendingMessage>,
    /// WP-186: durable user-gate asks mirrored from `session_pending_permissions`.
    pending_permissions: Vec<crate::session::pending::PendingPermissionRecord>,
    /// WP-098 R-Q: child follow-up turns. Separate from `pending_messages`,
    /// never claimed at a step seam, memory-only (ISSUE on restart loss).
    follow_ups: VecDeque<crate::types::UserInput>,
    subscriber_count: usize,
    /// Sticky agent selection 锟?isomorphic with `sessions.agent_id`.
    pub agent_id: String,
    /// Sticky model catalog id 锟?isomorphic with `sessions.model_id` (NULL = unset).
    pub model_id: Option<String>,
    /// Platform thinking tier 锟?isomorphic with `sessions.thinking_tier`.
    pub thinking_tier: crate::platform_knobs::ThinkingTier,
    /// Platform context mode 锟?isomorphic with `sessions.context_mode`.
    pub context_mode: crate::platform_knobs::ContextMode,
    /// Parent session when this is a subagent child; `None` for root sessions.
    pub parent_session_id: Option<String>,
    /// Parent `function_call.call_id` that launched this child.
    pub parent_call_id: Option<String>,
    pub project: Option<String>,
    last_permission_sink: Option<Arc<dyn crate::permission::PermissionSink>>,
    /// Files this process has seen the session touch. Not durable.
    file_tracker: crate::reminder::FileTracker,
    /// Last reconnect bubble that should survive a refresh. Not a log row.
    llm_reconnect: Option<LlmReconnect>,
}

impl SessionRecord {
    fn from_meta(
        meta: &crate::session::model::SessionMeta,
        task_state: TaskReminders,
        revision: u64,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            revision,
            task_state,
            activity: SessionActivity::Idle,
            pending_revert: None,
            event_tx,
            event_buffer: VecDeque::new(),
            pending_messages: Vec::new(),
            pending_permissions: Vec::new(),
            follow_ups: VecDeque::new(),
            subscriber_count: 0,
            agent_id: meta.agent_id.clone(),
            model_id: meta.model_id.clone(),
            thinking_tier: crate::platform_knobs::ThinkingTier::parse(&meta.thinking_tier)
                .unwrap_or_default(),
            context_mode: crate::platform_knobs::ContextMode::parse(&meta.context_mode)
                .unwrap_or_default(),
            parent_session_id: meta.parent_session_id.clone(),
            parent_call_id: meta.parent_call_id.clone(),
            project: Some(meta.project.clone()),
            last_permission_sink: None,
            file_tracker: crate::reminder::FileTracker::default(),
            llm_reconnect: None,
        }
    }

    fn note_internal(&mut self, event: &InternalEvent) {
        match event {
            InternalEvent::LlmReconnect(notice) => {
                self.llm_reconnect = notice.retained();
            }
            InternalEvent::TurnStarted { .. } => {
                self.llm_reconnect = None;
            }
            InternalEvent::TurnCompleted { reason, .. } => {
                let keep = *reason == TurnEndReason::Error
                    && matches!(
                        self.llm_reconnect,
                        Some(LlmReconnect {
                            phase: LlmReconnectPhase::Failed,
                            ..
                        })
                    );
                if !keep {
                    self.llm_reconnect = None;
                }
            }
            _ => {}
        }
    }
}

/// Process-level manager: registry of sessions (durable + live). No wire types.
pub struct SessionManager {
    records: std::sync::Mutex<HashMap<String, SessionRecord>>,
    activity_changed: Condvar,
    /// Process-wide settings write gate. `begin_turn` only from
    /// `reserve_turn` / `reserve_turn_and_claim_pending` / `begin_turn`;
    /// `end_turn` only from `release_turn_reservation` / `finish_turn` /
    /// `try_begin_revert`(StartingTurn) / `remove_session`. Fanout spawn
    /// failure must call `finish_turn` so this counter cannot stick.
    pub turn_guard: Arc<TurnGuard>,
    data: Arc<SessionData>,
    /// Keeps a test-created lease alive for the manager's writer lifetime.
    _test_lease: Option<crate::session::WorkspaceWriteLease>,
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,
    /// WP-090: optional turn status hook (default none = no-op). Session installs.
    turn_status_hook: std::sync::RwLock<Option<TurnStatusHook>>,
    /// WP-166: flips to `true` once, when the first `session/list` or
    /// `session/new` reply has been emitted on any connection. P2 startup work
    /// (session-index first tick, empty-session GC) waits on it.
    startup_settled: tokio::sync::watch::Sender<bool>,
    /// Test-only: next `start_turn` pretends fanout thread spawn failed.
    #[cfg(test)]
    fail_next_fanout_spawn: std::sync::atomic::AtomicBool,
}

/// WP-166 tests: park `resume_session` for one session id so a test can hold a
/// `session/subscribe` mid-replay. Compiled only under `cfg(test)`.
#[cfg(test)]
pub(crate) mod resume_gate_for_test {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    pub(crate) struct Gate {
        pub(crate) entered: tokio::sync::Notify,
        pub(crate) release: tokio::sync::Semaphore,
    }

    static GATES: LazyLock<Mutex<HashMap<String, Arc<Gate>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    /// Install a closed gate for `session_id`; the next resume parks on it.
    pub(crate) fn install(session_id: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Semaphore::new(0),
        });
        GATES
            .lock()
            .unwrap()
            .insert(session_id.to_string(), Arc::clone(&gate));
        gate
    }

    pub(crate) async fn wait(session_id: &str) {
        let gate = GATES.lock().unwrap().remove(session_id);
        if let Some(gate) = gate {
            gate.entered.notify_one();
            let _ = gate.release.acquire().await;
        }
    }
}

const EVENT_BUFFER_CAPACITY: usize = 1024;
pub const EMPTY_SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

impl SessionManager {
    /// Test-only convenience constructor. Production must inject `SessionData`
    /// through [`SessionManager::from_data`] so the workspace lease remains
    /// owned by the process root.
    pub fn new_for_test(turn_guard: Arc<TurnGuard>, db_path: String) -> Self {
        let (data, test_lease) = if db_path.is_empty() {
            (
                SessionData::open_ephemeral().expect("ephemeral sessions.db"),
                None,
            )
        } else {
            let root = std::path::Path::new(&db_path)
                .parent()
                .expect("sessions.db parent");
            let lease = crate::session::WorkspaceWriteLease::acquire(root)
                .expect("acquire test workspace lease");
            let data = SessionData::open(&lease, std::path::Path::new(&db_path))
                .expect("open sessions.db");
            (data, Some(lease))
        };
        let (lifecycle_tx, _) = broadcast::channel(256);
        Self {
            records: std::sync::Mutex::new(HashMap::new()),
            activity_changed: Condvar::new(),
            turn_guard,
            data,
            _test_lease: test_lease,
            lifecycle_tx,
            turn_status_hook: std::sync::RwLock::new(None),
            startup_settled: tokio::sync::watch::Sender::new(false),
            #[cfg(test)]
            fail_next_fanout_spawn: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn from_data(turn_guard: Arc<TurnGuard>, data: Arc<SessionData>) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(256);
        let manager = Self {
            records: std::sync::Mutex::new(HashMap::new()),
            activity_changed: Condvar::new(),
            turn_guard,
            data,
            _test_lease: None,
            lifecycle_tx,
            turn_status_hook: std::sync::RwLock::new(None),
            startup_settled: tokio::sync::watch::Sender::new(false),
            #[cfg(test)]
            fail_next_fanout_spawn: std::sync::atomic::AtomicBool::new(false),
        };
        let orphans = manager.remove_orphan_child_sessions();
        if orphans > 0 {
            tracing::warn!(removed = orphans, "removed orphan subagent sessions");
        }
        manager
    }

    pub fn data(&self) -> &Arc<SessionData> {
        &self.data
    }

    /// Test-only: next `start_turn` rolls back as if fanout thread spawn failed.
    #[cfg(test)]
    pub fn inject_fanout_spawn_failure(&self) {
        self.fail_next_fanout_spawn
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn reader(&self) -> SessionDataReader {
        self.data.reader()
    }

    /// Current workspace sessions DB path.
    pub fn db_path(&self) -> String {
        self.data.path().display().to_string()
    }

    fn expected_revision(&self, session_id: &str) -> u64 {
        // The durable revision is the CAS authority.  A manager may observe a
        // receipt produced by a different in-process adapter after a restart
        // or during transition, so its hot record is only a post-commit cache.
        self.data.revision_blocking(session_id).unwrap_or(0)
    }

    fn note_receipt(&self, receipt: &crate::session::data::command::CommitReceipt) {
        if receipt.session_id.is_empty() {
            return;
        }
        if let Some(record) = self
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&receipt.session_id)
        {
            record.revision = receipt.revision;
        }
    }

    pub fn mutate_blocking(
        &self,
        mutation: SessionMutation,
    ) -> Result<crate::session::data::command::CommitReceipt> {
        let receipt = self.data.mutate_blocking(mutation)?;
        self.note_receipt(&receipt);
        Ok(receipt)
    }

    /// WP-166: record that the first `session/list` / `session/new` reply went
    /// out. Idempotent; only the first call has an effect.
    pub fn mark_startup_settled(&self) {
        self.startup_settled.send_if_modified(|settled| {
            if *settled {
                false
            } else {
                *settled = true;
                true
            }
        });
    }

    pub fn is_startup_settled(&self) -> bool {
        *self.startup_settled.borrow()
    }

    /// WP-166: resolves once [`Self::mark_startup_settled`] has been called
    /// (immediately if it already was). No timeout: P2 work starts only after
    /// the client has its list / new reply.
    pub async fn wait_startup_settled(&self) {
        let mut rx = self.startup_settled.subscribe();
        let _ = rx.wait_for(|settled| *settled).await;
    }

    /// Private in-memory registry for tests that need an isolated manager
    /// without a workspace DB path.
    #[cfg(test)]
    pub fn ephemeral_registry() -> Self {
        Self::new_for_test(Arc::new(TurnGuard::new()), String::new())
    }

    pub fn insert_session_for_test(&self, session_id: &str) {
        let meta = self.data.meta_blocking(session_id).unwrap_or_else(|_| {
            crate::session::model::SessionMeta {
                id: session_id.to_string(),
                project: String::new(),
                created_at: 0,
                parent_session_id: None,
                parent_call_id: None,
                responsibility: String::new(),
                subagent_depth: 0,
                agent_id: "default".into(),
                model_id: None,
                thinking_tier: "medium".into(),
                context_mode: "standard".into(),
                updated_at: 0,
                compacted_seq: None,
                spine_from: 0,
                todos: Vec::new(),
                plan_slug: None,
                plan_revision: None,
                preview: String::new(),
            }
        });
        let revision = self.data.revision_blocking(session_id).unwrap_or(0);
        let record = SessionRecord::from_meta(
            &meta,
            crate::session::task_state::TaskReminders::default(),
            revision,
        );
        self.records
            .lock()
            .unwrap()
            .insert(session_id.to_string(), record);
    }

    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Whether `session_id` is a persisted subagent child (has a parent).
    pub fn is_child_session(&self, session_id: &str) -> bool {
        if let Some(record) = self.records.lock().unwrap().get(session_id) {
            return record.parent_session_id.is_some();
        }
        self.data
            .meta_blocking(session_id)
            .map(|m| m.parent_session_id.is_some())
            .unwrap_or(false)
    }

    fn emit_lifecycle(&self, event: LifecycleEvent) {
        let _ = self.lifecycle_tx.send(event);
    }

    pub async fn open_session(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> Result<String> {
        self.open_session_sync(project, agent_id, model_id)
    }

    pub fn open_session_sync(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
    ) -> Result<String> {
        let receipt = self.mutate_blocking(SessionMutation::Create {
            operation_id: MutationId::new(),
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            model_id: model_id.map(|s| s.to_string()),
            parent_session_id: None,
            parent_call_id: None,
            responsibility: String::new(),
        })?;
        let sid = receipt.session_id.clone();
        let meta = self.data.meta_blocking(&sid)?;
        let mut task_state = TaskReminders {
            todos: meta
                .todos
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            active_plan: meta.plan_slug.as_ref().map(|slug| {
                crate::session::task_state::PlanRef::with_revision(slug, meta.plan_revision.clone())
            }),
        };
        let mut revision = receipt.revision;
        let plan_dir = self.plan_dir_path();
        if prune_stale_active_plan(&mut task_state, &plan_dir) {
            match self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: sid.clone(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            }) {
                // Record is not in `records` yet, so note_receipt cannot sync it (S1).
                Ok(saved) => revision = saved.revision,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to persist pruned active plan on open")
                }
            }
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, revision);
        record.project = Some(project.to_string());
        self.records.lock().unwrap().insert(sid.clone(), record);
        Ok(sid)
    }

    /// Open a durable child session linked to a parent `subagent_launch` call.
    pub fn open_child_session(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: &str,
        parent_call_id: &str,
    ) -> Result<String> {
        self.open_child_session_with_responsibility(
            project,
            agent_id,
            model_id,
            parent_session_id,
            parent_call_id,
            "",
        )
    }

    pub fn open_child_session_with_responsibility(
        &self,
        project: &str,
        agent_id: &str,
        model_id: Option<&str>,
        parent_session_id: &str,
        parent_call_id: &str,
        responsibility: &str,
    ) -> Result<String> {
        if self.db_path().is_empty() || self.db_path() == ":memory:" {
            return Err(LitecodeError::ToolExecution(
                "open_child_session requires a workspace SessionManager".into(),
            ));
        }
        let receipt = self.mutate_blocking(SessionMutation::Create {
            operation_id: MutationId::new(),
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            model_id: model_id.map(|s| s.to_string()),
            parent_session_id: Some(parent_session_id.to_string()),
            parent_call_id: Some(parent_call_id.to_string()),
            responsibility: responsibility.to_string(),
        })?;
        let sid = receipt.session_id.clone();
        let meta = self.data.meta_blocking(&sid)?;
        let mut task_state = TaskReminders::default();
        let mut revision = receipt.revision;
        let plan_dir = self.plan_dir_path();
        if prune_stale_active_plan(&mut task_state, &plan_dir) {
            match self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: sid.clone(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            }) {
                // Record is not in `records` yet, so note_receipt cannot sync it (S1).
                Ok(saved) => revision = saved.revision,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to persist pruned active plan on open")
                }
            }
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, revision);
        record.project = Some(project.to_string());
        self.records.lock().unwrap().insert(sid.clone(), record);
        self.emit_lifecycle(LifecycleEvent::SessionAdded {
            session_id: sid.clone(),
            project: project.to_string(),
            agent_id: agent_id.to_string(),
            parent_session_id: Some(parent_session_id.to_string()),
            parent_call_id: Some(parent_call_id.to_string()),
            responsibility: responsibility.to_string(),
            updated_at: meta.updated_at,
        });
        Ok(sid)
    }

    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        #[cfg(test)]
        resume_gate_for_test::wait(session_id).await;
        let mut records = self.records.lock().unwrap();
        if records.contains_key(session_id) {
            return Ok(());
        }
        let meta = self.data.meta_blocking(session_id)?;
        let mut revision = self.data.revision_blocking(session_id).unwrap_or(0);
        let mut task_state = TaskReminders {
            todos: meta
                .todos
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            active_plan: meta.plan_slug.as_ref().map(|slug| {
                crate::session::task_state::PlanRef::with_revision(slug, meta.plan_revision.clone())
            }),
        };
        let plan_dir = self.plan_dir_path();
        if prune_stale_active_plan(&mut task_state, &plan_dir) {
            // `records` is locked here: call data directly (note_receipt would
            // re-lock `records` and self-deadlock). Revision synced below (S1).
            match self.data.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: session_id.to_string(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            }) {
                // Record is not in `records` yet, so note_receipt cannot sync it (S1).
                Ok(saved) => revision = saved.revision,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to persist pruned active plan on open")
                }
            }
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, revision);
        record.project = None;
        // WP-186: hydrate pending user-gate asks from the P1 sidecar.
        match self.data.pending_permissions_blocking(session_id) {
            Ok(pending) => record.pending_permissions = pending,
            Err(e) => tracing::warn!(
                error = %e,
                session_id,
                "failed to load pending_permissions on resume"
            ),
        }
        records.insert(session_id.to_string(), record);
        Ok(())
    }

    pub async fn ensure_entry(&self, session_id: &str) -> Result<()> {
        if self.records.lock().unwrap().contains_key(session_id) {
            return Ok(());
        }
        self.resume_session(session_id).await
    }

    pub fn entry_buffer_len(&self, session_id: &str) -> usize {
        self.data
            .working_set_blocking(session_id)
            .map(|rows| rows.len())
            .unwrap_or(0)
    }

    pub fn entry_buffer_len_blocking(&self, session_id: &str) -> usize {
        self.entry_buffer_len(session_id)
    }

    pub fn entry_wire_seq_cursor(&self, session_id: &str) -> (i64, u64) {
        match self.data.seq_cursor_blocking(session_id) {
            Ok(cursor) => cursor,
            Err(_) => (-1, 0),
        }
    }

    pub fn entry_load_events_range(
        &self,
        session_id: &str,
        from_seq: crate::session::event::Seq,
        to_seq: crate::session::event::Seq,
    ) -> Result<Vec<crate::session::event::SessionEvent>> {
        self.data
            .events_range_blocking(session_id, from_seq as i64, to_seq as i64)
    }

    pub fn entry_revert_to_user_anchor(&self, session_id: &str, seq: i64) -> Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::Apply {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            op: SessionApply::Truncate { anchor_seq: seq },
        })?;
        Ok(())
    }

    pub fn entry_user_anchor_window(
        &self,
        session_id: &str,
        anchor_seq: Option<i64>,
        before: i64,
        after: i64,
    ) -> Result<crate::session::data::command::UserAnchorWindow> {
        match self.data.read_blocking(SessionRead::UserAnchors {
            session_id: session_id.to_string(),
            anchor_seq,
            before,
            after,
        })? {
            ReadValue::UserAnchors(window) => Ok(window),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected user anchors".into(),
            )),
        }
    }

    pub fn entry_user_detail_count(&self, session_id: &str) -> Result<i64> {
        match self.data.read_blocking(SessionRead::UserDetailCount {
            session_id: session_id.to_string(),
        })? {
            ReadValue::Count(n) => Ok(n),
            _ => Err(LitecodeError::SessionStorage("unexpected count".into())),
        }
    }

    /// File-snapshot stem for a user row: `seq + 1` (`next_seq` at turn start).
    pub fn entry_snapshot_stem_for_user_seq(&self, session_id: &str, seq: i64) -> Result<i64> {
        match self.data.read_blocking(SessionRead::UserAnchorStem {
            session_id: session_id.to_string(),
            seq,
        })? {
            ReadValue::Count(n) => Ok(n),
            _ => Err(LitecodeError::InvalidRevertAnchor(format!("seq={seq}"))),
        }
    }

    #[doc(hidden)]
    pub fn records_lock_free(&self) -> bool {
        self.records.try_lock().is_ok()
    }

    pub fn save_task_state(&self, session_id: &str) -> anyhow::Result<()> {
        let state = {
            let records = self.records.lock().unwrap();
            let entry = records
                .get(session_id)
                .ok_or_else(|| anyhow::anyhow!("session entry not found: {}", session_id))?;
            entry.task_state.clone()
        };
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SaveTaskState {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            state,
        })?;
        Ok(())
    }

    /// Workspace-owned plan directory for this manager's `.litecode` root.
    pub fn plan_dir_path(&self) -> std::path::PathBuf {
        self.data_root_path().join("plan")
    }

    /// One settled `read` / `write` / `edit` of the active plan file.
    ///
    /// Direct and nested calls both reach this from the shared tool entry.
    /// `read` names the file `path`. `write` and `edit` name it `path` now, and
    /// still accept the historical `file_path` recorded in old transcripts.
    pub fn note_plan_tool_touch(&self, session_id: &str, call: &crate::types::FunctionToolCall) {
        let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.arguments) else {
            return;
        };
        let raw = match call.name.as_str() {
            "read" => args.get("path"),
            "write" | "edit" => args.get("path").or_else(|| args.get("file_path")),
            _ => return,
        };
        let Some(raw) = raw.and_then(|value| value.as_str()) else {
            return;
        };
        let Ok(Some(plan)) =
            self.with_entry_task_state(session_id, |state| Ok(state.active_plan.clone()))
        else {
            return;
        };
        let normalized = raw.replace('\\', "/");
        if !normalized
            .trim_start_matches("./")
            .ends_with(plan.relative_path.as_str())
        {
            return;
        }
        let path = self.plan_dir_path().join(format!("{}.md", plan.slug));
        if let Some(revision) = crate::session::task_state::plan_file_revision(&path) {
            let _ = self.update_active_plan_revision(session_id, &plan.slug, &revision);
        }
    }

    /// Update the content hash the session last saw for its active plan.
    pub fn update_active_plan_revision(
        &self,
        session_id: &str,
        slug: &str,
        revision: &str,
    ) -> anyhow::Result<()> {
        self.with_entry_task_state_mut(session_id, |state| {
            if let Some(plan) = state.active_plan.as_mut()
                && plan.slug == slug
            {
                plan.revision = Some(revision.to_string());
            }
            Ok(())
        })?;
        self.save_task_state(session_id)
    }

    /// Drop an active-plan pointer whose plan file is gone, just before a model call.
    ///
    /// The system owns file loss, not the model: when the pointer was stale the
    /// cleared state is persisted and a PlanChanged event is published so the UI
    /// stops showing a plan that no longer exists. Persistence and publish
    /// failures are logged, never fatal — the caller still gets the settled state.
    ///
    /// The plan directory is resolved from this manager's own data root, never
    /// from thread-local `active_paths()`: manual compaction runs on shared
    /// worker threads that have no workspace context.
    pub fn settle_stale_plan(&self, session_id: &str) -> anyhow::Result<TaskReminders> {
        let plan_dir = self.plan_dir_path();
        let (state, changed) = self.with_entry_task_state_mut(session_id, |s| {
            let changed = prune_stale_active_plan(s, &plan_dir);
            Ok((s.clone(), changed))
        })?;
        if !changed {
            return Ok(state);
        }
        if let Err(e) = self.save_task_state(session_id) {
            tracing::warn!(session_id, error = %e, "failed to persist cleared active plan");
        }
        if !self.publish_internal(
            session_id,
            InternalEvent::PlanChanged {
                active_plan_path: None,
            },
        ) {
            tracing::warn!(session_id, "failed to publish cleared active plan");
        }
        Ok(state)
    }

    pub fn with_entry_task_state_mut<F, R>(&self, session_id: &str, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&mut TaskReminders) -> anyhow::Result<R>,
    {
        let mut records = self
            .records
            .lock()
            .map_err(|e| anyhow::anyhow!("records lock poisoned: {}", e))?;
        let entry = records
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("session entry not found: {}", session_id))?;
        f(&mut entry.task_state)
    }

    pub fn with_entry_task_state<F, R>(&self, session_id: &str, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&TaskReminders) -> anyhow::Result<R>,
    {
        let records = self
            .records
            .lock()
            .map_err(|e| anyhow::anyhow!("records lock poisoned: {}", e))?;
        let entry = records
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("session entry not found: {}", session_id))?;
        f(&entry.task_state)
    }

    /// Attach a viewer; returns cached progress when a turn is running.
    pub fn attach(&self, session_id: &str) -> Option<TurnProgress> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id)?;
        record.subscriber_count += 1;
        record.activity.progress().cloned()
    }

    pub fn detach(&self, session_id: &str) {
        let mut records = self.records.lock().unwrap();
        if let Some(record) = records.get_mut(session_id) {
            record.subscriber_count = record.subscriber_count.saturating_sub(1);
        }
    }

    /// Atomically reserve an idle session before a runtime is spawned.
    pub fn reserve_turn(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<TurnProgress> {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let progress = TurnProgress {
            turn_id: turn_id.clone(),
            phase: TurnPhase::Starting,
            step: 1,
            step_max,
            started_at_ms,
            awaiting_permission: false,
        };

        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        if record.activity.is_busy() {
            return Err(LitecodeError::AgentAlreadyRunning);
        }
        record.agent_id = primary_agent.to_string();
        record.project = Some(project.to_string());
        record.event_buffer.clear();
        // Leave a failed reconnect in place until TurnStarted. A spawn that
        // never starts must still be retryable.
        record.activity = SessionActivity::StartingTurn {
            turn_id,
            progress: progress.clone(),
            cancel_requested: false,
            cancelled_by: None,
        };
        drop(records);

        self.turn_guard.begin_turn();
        Ok(progress)
    }

    /// Release a failed pre-spawn turn reservation.
    pub fn release_turn_reservation(&self, session_id: &str, turn_id: &str) -> bool {
        let released = {
            let mut records = self.records.lock().unwrap();
            let Some(record) = records.get_mut(session_id) else {
                return false;
            };
            let matches = matches!(
                &record.activity,
                SessionActivity::StartingTurn {
                    turn_id: reserved,
                    ..
                } if reserved == turn_id
            );
            if matches {
                record.activity = SessionActivity::Idle;
            }
            matches
        };
        if released {
            self.turn_guard.end_turn();
            self.fire_turn_status(session_id, SessionStatus::Idle);
        }
        released
    }

    /// Install the turn status hook (Session, WP-091 R8). Replaces any previous.
    pub fn set_turn_status_hook(&self, hook: TurnStatusHook) {
        *self.turn_status_hook.write().unwrap() = Some(hook);
    }

    /// Call the hook outside `records`. Never call while holding the lock.
    fn fire_turn_status(&self, session_id: &str, status: SessionStatus) {
        let hook = self.turn_status_hook.read().unwrap().clone();
        if let Some(hook) = hook {
            hook(session_id, status);
        }
    }

    /// Begin a turn directly. Used by tests and inline runtimes that cannot
    /// split reservation from activation.
    pub fn begin_turn(
        &self,
        session_id: &str,
        turn_id: String,
        cancel: CancellationToken,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<TurnProgress> {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let progress = TurnProgress {
            turn_id: turn_id.clone(),
            phase: TurnPhase::Starting,
            step: 1,
            step_max,
            started_at_ms,
            awaiting_permission: false,
        };

        {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            if record.activity.is_busy() {
                return Err(LitecodeError::AgentAlreadyRunning);
            }
            record.agent_id = primary_agent.to_string();
            record.project = Some(project.to_string());
            record.event_buffer.clear();
            record.activity = SessionActivity::RunningTurn(LiveTurnState {
                turn_id,
                cancel,
                progress: progress.clone(),
                stopping: false,
                cancelled_by: None,
            });
        }

        self.turn_guard.begin_turn();
        self.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: session_id.to_string(),
            progress: progress.clone(),
        });
        Ok(progress)
    }

    /// Update Running progress (turn_id must match).
    pub fn apply_progress(&self, session_id: &str, progress: TurnProgress) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        let SessionActivity::RunningTurn(live) = &mut record.activity else {
            return Ok(());
        };
        if live.turn_id != progress.turn_id {
            return Ok(());
        }
        live.progress = progress.clone();
        drop(records);
        self.emit_lifecycle(LifecycleEvent::TurnProgress {
            session_id: session_id.to_string(),
            progress,
        });
        Ok(())
    }

    /// Finish a turn. This is the sole path that clears `RunningTurn`; normally
    /// it becomes Idle, but a queued revert receives the exclusive lease directly.
    ///
    /// Idempotent when already Idle or turn_id mismatches a different live turn.
    pub fn finish_turn(&self, session_id: &str, turn_id: &str) -> Option<TurnProgress> {
        let progress = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id)?;
            let SessionActivity::RunningTurn(live) = &record.activity else {
                return None;
            };
            if live.turn_id != turn_id {
                return None;
            }
            let activity = std::mem::replace(&mut record.activity, SessionActivity::Idle);
            let SessionActivity::RunningTurn(live) = activity else {
                unreachable!("activity checked as RunningTurn");
            };
            if let Some(operation_id) = record.pending_revert.take() {
                record.activity = SessionActivity::Exclusive {
                    operation_id,
                    kind: SessionOperationKind::Revert,
                };
            }
            let mut progress = live.progress;
            progress.awaiting_permission = false;
            (progress, record.activity.status())
        };
        let (progress, status_after) = progress;
        self.turn_guard.end_turn();
        // WP-098: reason and cancelled_by both come from the durable turn/end,
        // so the live event carries exactly what was persisted.
        let (reason, cancelled_by) = self
            .data()
            .latest_turn_end_blocking(session_id)
            .ok()
            .flatten()
            .filter(|(turn_id, _, _)| *turn_id == progress.turn_id)
            .map_or((None, None), |(_, reason, by)| (Some(reason), by));
        self.emit_lifecycle(LifecycleEvent::TurnFinished {
            session_id: session_id.to_string(),
            progress: progress.clone(),
            reason,
            cancelled_by,
        });
        self.activity_changed.notify_all();
        self.fire_turn_status(session_id, status_after);
        Some(progress)
    }

    pub async fn start_turn(
        &self,
        session_id: &str,
        handle: TurnHandle,
        primary_agent: &str,
        project: &str,
        manager: Arc<SessionManager>,
    ) -> Result<()> {
        let turn_id = handle.turn_id.clone();
        let cancel = handle.cancel.clone();
        let (event_tx, progress, cancel_requested) = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            let activity = std::mem::replace(&mut record.activity, SessionActivity::Idle);
            let (reserved_turn_id, progress, cancel_requested, cancelled_by) = match activity {
                SessionActivity::StartingTurn {
                    turn_id,
                    progress,
                    cancel_requested,
                    cancelled_by,
                } => (turn_id, progress, cancel_requested, cancelled_by),
                other => {
                    record.activity = other;
                    cancel.cancel();
                    return Err(LitecodeError::AgentAlreadyRunning);
                }
            };
            if reserved_turn_id != turn_id {
                record.activity = SessionActivity::StartingTurn {
                    turn_id: reserved_turn_id,
                    progress,
                    cancel_requested,
                    cancelled_by,
                };
                cancel.cancel();
                return Err(LitecodeError::AgentAlreadyRunning);
            }
            record.agent_id = primary_agent.to_string();
            record.project = Some(project.to_string());
            record.activity = SessionActivity::RunningTurn(LiveTurnState {
                turn_id: turn_id.clone(),
                cancel: cancel.clone(),
                progress: progress.clone(),
                stopping: cancel_requested,
                cancelled_by,
            });
            if cancel_requested {
                // R1: cancel requested during the Starting window takes effect
                // the moment the token is installed; the turn winds down via
                // the normal cancel path (TurnFinished reason=Cancelled).
                cancel.cancel();
            }
            (record.event_tx.clone(), progress, cancel_requested)
        };

        self.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: session_id.to_string(),
            progress,
        });
        self.fire_turn_status(
            session_id,
            if cancel_requested {
                SessionStatus::Stopping
            } else {
                SessionStatus::Running
            },
        );

        // Fanout must outlive the caller's runtime. Idle bash auto-turn starts
        // the turn on a throwaway current-thread runtime that exits as soon as
        // `start_turn` returns; `tokio::spawn` on that runtime would abort
        // fanout and the agent panel would never see `agent/turn_started` /
        // `buffer/item` even though session-list lifecycle already flipped to
        // running.
        #[cfg(test)]
        if self
            .fail_next_fanout_spawn
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            cancel.cancel();
            let _ = self.finish_turn(session_id, &turn_id);
            return Err(LitecodeError::ToolExecution(
                "turn fanout spawn failed: injected".into(),
            ));
        }

        let session_id_owned = session_id.to_string();
        let turn_id_owned = turn_id.clone();
        std::thread::Builder::new()
            .name(format!("turn-fanout-{session_id}"))
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(error) => {
                        tracing::error!(error = %error, "turn fanout runtime failed");
                        let _ = manager.finish_turn(&session_id_owned, &turn_id_owned);
                        return;
                    }
                };
                rt.block_on(fanout_turn(session_id_owned, handle, event_tx, manager));
            })
            .map_err(|error| {
                cancel.cancel();
                // RunningTurn + TurnGuard were already taken; mirror the runtime
                // build failure path so the session and settings writes unblock.
                let _ = self.finish_turn(session_id, &turn_id);
                LitecodeError::ToolExecution(format!("turn fanout spawn failed: {error}"))
            })?;
        Ok(())
    }

    pub async fn cancel_turn(
        &self,
        session_id: &str,
        by: crate::types::StopOrigin,
    ) -> CancelOutcome {
        self.cancel_turn_sync(session_id, by)
    }

    /// Cancel a live turn without taking the exclusive lease. Decision and
    /// action happen under one `records` lock. Never touches pending queues,
    /// mailboxes or completion inboxes (K3), and emits no parent notice.
    pub fn cancel_turn_sync(
        &self,
        session_id: &str,
        by: crate::types::StopOrigin,
    ) -> CancelOutcome {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return CancelOutcome::Idle;
        };
        cancel_locked(record, by)
    }

    /// WP-098: who cancelled the live turn `turn_id`, if anyone. Read by the
    /// runtime right before it writes `turn/end`.
    pub fn turn_cancelled_by(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Option<crate::types::StopOrigin> {
        let records = self.records.lock().unwrap();
        match &records.get(session_id)?.activity {
            SessionActivity::RunningTurn(live) if live.turn_id == turn_id => live.cancelled_by,
            _ => None,
        }
    }

    // ── follow-up queue (WP-098 R-Q, children only, memory-only) ──

    /// Queue a follow-up turn for a busy child. Never claimed at a step seam;
    /// the end-of-turn flush runs one turn per message, FIFO. `Idle` means
    /// nothing was queued: the caller starts the turn itself.
    pub fn enqueue_follow_up(
        &self,
        session_id: &str,
        input: crate::types::UserInput,
    ) -> Result<FollowUpOutcome> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        if record
            .parent_session_id
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(LitecodeError::ToolExecution(
                "follow-up queue is only for child sessions".into(),
            ));
        }
        if !record.activity.is_busy() {
            return Ok(FollowUpOutcome::Idle);
        }
        record.follow_ups.push_back(input);
        Ok(FollowUpOutcome::Queued {
            position: record.follow_ups.len(),
        })
    }

    /// WP-098 R-SS: cancel the live turn and put `input` at the head of the
    /// follow-up queue under one `records` lock. Emits nothing.
    pub fn cancel_and_follow_up(
        &self,
        session_id: &str,
        input: crate::types::UserInput,
        by: crate::types::StopOrigin,
    ) -> Result<(CancelOutcome, FollowUpOutcome)> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        if record
            .parent_session_id
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err(LitecodeError::ToolExecution(
                "follow-up queue is only for child sessions".into(),
            ));
        }
        if !record.activity.is_busy() {
            return Ok((CancelOutcome::Idle, FollowUpOutcome::Idle));
        }
        let outcome = cancel_locked(record, by);
        record.follow_ups.push_front(input);
        Ok((outcome, FollowUpOutcome::Queued { position: 1 }))
    }

    pub fn has_follow_ups(&self, session_id: &str) -> bool {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|record| !record.follow_ups.is_empty())
    }

    pub fn follow_ups_len(&self, session_id: &str) -> usize {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .map_or(0, |record| record.follow_ups.len())
    }

    /// Reserve an idle turn and pop exactly one follow-up for it, in one
    /// critical section. `Ok(None)` = queue empty.
    pub(crate) fn reserve_turn_and_pop_follow_up(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<Option<(TurnProgress, crate::types::UserInput)>> {
        let progress = TurnProgress {
            turn_id: turn_id.clone(),
            phase: TurnPhase::Starting,
            step: 1,
            step_max,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            awaiting_permission: false,
        };
        let input = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            if record.follow_ups.is_empty() {
                return Ok(None);
            }
            if record.activity.is_busy() {
                return Err(LitecodeError::AgentAlreadyRunning);
            }
            let input = record.follow_ups.pop_front().expect("non-empty");
            record.agent_id = primary_agent.to_string();
            record.project = Some(project.to_string());
            record.event_buffer.clear();
            record.activity = SessionActivity::StartingTurn {
                turn_id,
                progress: progress.clone(),
                cancel_requested: false,
                cancelled_by: None,
            };
            input
        };
        self.turn_guard.begin_turn();
        Ok(Some((progress, input)))
    }

    pub(crate) fn restore_follow_up_front(&self, session_id: &str, input: crate::types::UserInput) {
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.follow_ups.push_front(input);
        }
    }

    /// WP-090 R11: in-memory child sessions that are not busy, as
    /// `(child_session_id, parent_session_id)`. Read-only.
    pub(crate) fn idle_child_session_ids(&self) -> Vec<(String, String)> {
        let records = self.records.lock().unwrap();
        records
            .iter()
            .filter(|(_, record)| !record.activity.is_busy())
            .filter_map(|(id, record)| {
                record
                    .parent_session_id
                    .as_ref()
                    .filter(|parent| !parent.is_empty())
                    .map(|parent| (id.clone(), parent.clone()))
            })
            .collect()
    }

    // ── queued user messages (memory-only) ──

    /// Queue a user message for the live session. Never persisted: only the
    /// consuming append writes an `item/user` row.
    pub fn enqueue_pending_message(&self, session_id: &str, text: &str) -> Result<PendingMessage> {
        self.enqueue_user_input(session_id, crate::types::UserInput::text(text))
    }

    /// Queue a composer payload. Image refs are stored as-is; the caller has
    /// already checked they are `litecode-media:` names.
    pub fn enqueue_user_input(
        &self,
        session_id: &str,
        input: crate::types::UserInput,
    ) -> Result<PendingMessage> {
        let message = PendingMessage {
            id: uuid::Uuid::new_v4().to_string(),
            text: input.text,
            images: input.images,
        };
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        record.pending_messages.push(message.clone());
        emit_pending_messages_locked(record);
        Ok(message)
    }

    /// Remove one queued message by id. Idempotent: `false` when the id was
    /// already consumed or never existed.
    pub fn remove_pending_message(&self, session_id: &str, id: &str) -> bool {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return false;
        };
        let before = record.pending_messages.len();
        record.pending_messages.retain(|message| message.id != id);
        if record.pending_messages.len() == before {
            return false;
        }
        emit_pending_messages_locked(record);
        true
    }

    pub fn pending_messages_snapshot(&self, session_id: &str) -> Vec<PendingMessage> {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .map(|record| record.pending_messages.clone())
            .unwrap_or_default()
    }

    /// WP-186: snapshot pending user-gate asks for wire `pending_permissions`.
    pub fn pending_permissions_snapshot(
        &self,
        session_id: &str,
    ) -> Vec<crate::session::pending::PendingPermissionRecord> {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .map(|record| record.pending_permissions.clone())
            .unwrap_or_default()
    }

    /// WP-186: upsert one pending ask in memory + durable sidecar.
    pub fn upsert_pending_permission(&self, rec: crate::session::pending::PendingPermissionRecord) {
        let session_id = rec.session_id.clone();
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(&session_id) else {
            tracing::warn!(
                session_id = %session_id,
                request_id = %rec.request_id,
                "upsert_pending_permission: no session record"
            );
            return;
        };
        if let Some(pos) = record
            .pending_permissions
            .iter()
            .position(|p| p.request_id == rec.request_id)
        {
            record.pending_permissions[pos] = rec.clone();
        } else {
            record.pending_permissions.push(rec.clone());
        }
        let pending = record.pending_permissions.clone();
        let expected = record.revision;
        drop(records);
        if let Err(e) = self
            .data
            .save_pending_permissions_blocking(&session_id, expected, pending)
        {
            tracing::warn!(error = %e, session_id = %session_id, "persist pending_permissions failed");
        } else if let Ok(rev) = self.data.revision_blocking(&session_id) {
            if let Some(record) = self.records.lock().unwrap().get_mut(&session_id) {
                record.revision = rev;
            }
        }
    }

    /// WP-186: remove one pending ask (user answered / cancelled that request).
    pub fn remove_pending_permission(&self, session_id: &str, request_id: &str) {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return;
        };
        let before = record.pending_permissions.len();
        record
            .pending_permissions
            .retain(|p| p.request_id != request_id);
        if record.pending_permissions.len() == before {
            return;
        }
        let pending = record.pending_permissions.clone();
        let expected = record.revision;
        drop(records);
        if let Err(e) = self
            .data
            .save_pending_permissions_blocking(session_id, expected, pending)
        {
            tracing::warn!(error = %e, session_id, "persist pending_permissions after remove failed");
        } else if let Ok(rev) = self.data.revision_blocking(session_id) {
            if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
                record.revision = rev;
            }
        }
    }

    /// WP-186: clear all pending asks (Cancel → Aborted path).
    pub fn clear_pending_permissions(&self, session_id: &str) {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return;
        };
        if record.pending_permissions.is_empty() {
            return;
        }
        record.pending_permissions.clear();
        let expected = record.revision;
        drop(records);
        if let Err(e) =
            self.data
                .save_pending_permissions_blocking(session_id, expected, Vec::new())
        {
            tracing::warn!(error = %e, session_id, "persist clear pending_permissions failed");
        } else if let Ok(rev) = self.data.revision_blocking(session_id) {
            if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
                record.revision = rev;
            }
        }
    }

    pub fn has_pending_messages(&self, session_id: &str) -> bool {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .is_some_and(|record| !record.pending_messages.is_empty())
    }

    /// Atomically take the queued messages for a running, non-stopping turn.
    ///
    /// `None` means the session is not running that turn (or a cancel already
    /// claimed the stopping flag): the queue is left untouched for the
    /// follow-up flush. `Some(vec![])` means the claim succeeded and nothing
    /// was queued.
    pub fn claim_pending_messages_for_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Option<Vec<PendingMessage>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id)?;
        match &record.activity {
            SessionActivity::RunningTurn(live) if live.turn_id == turn_id && !live.stopping => {}
            _ => return None,
        }
        let claimed = std::mem::take(&mut record.pending_messages);
        if !claimed.is_empty() {
            emit_pending_messages_locked(record);
        }
        Some(claimed)
    }

    /// Put claimed messages back at the head, in order, ahead of anything that
    /// was queued after the claim.
    pub fn restore_pending_messages(&self, session_id: &str, claimed: Vec<PendingMessage>) {
        if claimed.is_empty() {
            return;
        }
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return;
        };
        record.pending_messages.splice(0..0, claimed);
        emit_pending_messages_locked(record);
    }

    /// Drop the queue once a revert has taken the log back to anchor `k`.
    ///
    /// Only the revert that owns the session may call this: while that lease is
    /// held no turn can start and no end-of-turn flush can claim the queue, so a
    /// success here means nothing the user discarded can be delivered later.
    /// Returns false when the caller does not hold the revert lease.
    pub fn discard_pending_messages_for_revert(
        &self,
        session_id: &str,
        operation_id: &str,
    ) -> bool {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return false;
        };
        let owns_revert = matches!(
            &record.activity,
            SessionActivity::Exclusive {
                operation_id: active,
                kind: SessionOperationKind::Revert,
            } if active == operation_id
        );
        if !owns_revert {
            return false;
        }
        if record.pending_messages.is_empty() {
            return true;
        }
        record.pending_messages.clear();
        emit_pending_messages_locked(record);
        true
    }

    /// Take the queue only when the session is idle (no live turn or exclusive
    /// operation).
    ///
    /// `None` = busy: the queue is untouched and a live turn (or its end flush)
    /// owns it. `Some(vec![])` = idle, nothing was queued.
    pub fn claim_pending_if_idle(&self, session_id: &str) -> Option<Vec<PendingMessage>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id)?;
        if record.activity.is_busy() {
            return None;
        }
        let claimed = std::mem::take(&mut record.pending_messages);
        if !claimed.is_empty() {
            emit_pending_messages_locked(record);
        }
        Some(claimed)
    }

    /// Reserve an idle turn and take the queue for it in one critical section.
    ///
    /// `Ok(None)` = nothing queued, no state changed. `Err(AgentAlreadyRunning)`
    /// = a live turn owns the session; the running turn (or its own end flush)
    /// will consume the queue.
    #[allow(clippy::type_complexity)]
    pub fn reserve_turn_and_claim_pending(
        &self,
        session_id: &str,
        turn_id: String,
        step_max: u32,
        primary_agent: &str,
        project: &str,
    ) -> Result<Option<(TurnProgress, Vec<PendingMessage>)>> {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let progress = TurnProgress {
            turn_id: turn_id.clone(),
            phase: TurnPhase::Starting,
            step: 1,
            step_max,
            started_at_ms,
            awaiting_permission: false,
        };

        let claimed = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            if record.pending_messages.is_empty() {
                return Ok(None);
            }
            if record.activity.is_busy() {
                return Err(LitecodeError::AgentAlreadyRunning);
            }
            record.agent_id = primary_agent.to_string();
            record.project = Some(project.to_string());
            record.event_buffer.clear();
            record.activity = SessionActivity::StartingTurn {
                turn_id,
                progress: progress.clone(),
                cancel_requested: false,
                cancelled_by: None,
            };
            let claimed = std::mem::take(&mut record.pending_messages);
            emit_pending_messages_locked(record);
            claimed
        };

        self.turn_guard.begin_turn();
        Ok(Some((progress, claimed)))
    }

    /// Public own+descendant status for a session. Descendants only promote an
    /// idle session; they never demote the session's own `running`/`stopping`.
    pub fn session_status(&self, session_id: &str) -> Option<SessionStatus> {
        let own = {
            let records = self.records.lock().unwrap();
            records
                .get(session_id)
                .map(|record| record.activity.status())?
        };
        if own != SessionStatus::Idle {
            return Some(own);
        }
        let descendants = self.descendant_session_ids(session_id);
        if descendants.is_empty() {
            return Some(own);
        }
        let records = self.records.lock().unwrap();
        let has_active_descendant = descendants.iter().any(|id| {
            records
                .get(id)
                .is_some_and(|record| record.activity.status() != SessionStatus::Idle)
        });
        Some(if has_active_descendant {
            SessionStatus::RunningWithSubagent
        } else {
            SessionStatus::Idle
        })
    }

    /// Live activity of every session known to this process, sorted by id.
    ///
    /// Cheap: one lock, no I/O. Sessions that only exist on disk (from an
    /// earlier process) are not included — count those with
    /// `SessionDataReader::list_session_ids_blocking`.
    pub fn activity_snapshot(&self) -> Vec<SessionActivityRow> {
        let records = self.records.lock().unwrap();
        let mut rows: Vec<SessionActivityRow> = records
            .iter()
            .map(|(session_id, record)| SessionActivityRow {
                session_id: session_id.clone(),
                status: record.activity.status(),
                turn_started_at_ms: record.activity.progress().map(|p| p.started_at_ms),
                step: record.activity.progress().map(|p| (p.step, p.step_max)),
                agent_id: record.agent_id.clone(),
                model_id: record.model_id.clone(),
                parent_session_id: record.parent_session_id.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        rows
    }

    /// Public descendant ids for session-level tooling; root excluded.
    pub fn descendant_session_ids(&self, session_id: &str) -> Vec<String> {
        self.collect_child_ids_blocking(session_id)
    }

    /// Remember files a tool in this session just touched.
    pub fn note_touched_files(&self, session_id: &str, paths: &[std::path::PathBuf]) {
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if let Some(record) = records.get_mut(session_id) {
            record.file_tracker.record_paths(paths);
        }
    }

    /// Re-snapshot tracked files after this session's own tool batch.
    pub fn absorb_touched_files(&self, session_id: &str) {
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if let Some(record) = records.get_mut(session_id) {
            record.file_tracker.absorb();
        }
    }

    /// External changes since the last snapshot. Updates the snapshots.
    pub fn take_changed_files(&self, session_id: &str) -> Vec<std::path::PathBuf> {
        let Ok(mut records) = self.records.lock() else {
            return Vec::new();
        };
        records
            .get_mut(session_id)
            .map(|record| record.file_tracker.take_changes())
            .unwrap_or_default()
    }

    /// Undo a prior [`take_changed_files`] consumption so the next take can
    /// re-report `paths` (ISSUE-032). Thin wrap over `FileTracker::mark_dirty`.
    pub fn restore_changed_files(&self, session_id: &str, paths: &[std::path::PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if let Some(record) = records.get_mut(session_id) {
            record.file_tracker.mark_dirty(paths);
        }
    }

    /// Live child-session counts for the whole subtree: total and non-idle.
    ///
    /// Cheap read used by the post-compaction reminder so a window reset does
    /// not erase the fact that child sessions exist. Details stay behind
    /// `subagent_list`; this never spawns work.
    pub fn child_counts(&self, session_id: &str) -> ChildCounts {
        let ids = self.descendant_session_ids(session_id);
        if ids.is_empty() {
            return ChildCounts {
                total: 0,
                running: 0,
            };
        }
        let records = self.records.lock().unwrap();
        let running = ids
            .iter()
            .filter(|id| {
                records
                    .get(id.as_str())
                    .is_some_and(|record| record.activity.status() != SessionStatus::Idle)
            })
            .count();
        ChildCounts {
            total: ids.len(),
            running,
        }
    }

    /// Signal cancel to every descendant running turn. Revert uses this before
    /// truncating the parent log; the call is fire-and-forget and leaves child
    /// sessions and their transcripts intact.
    fn cancel_descendant_turns(&self, session_id: &str) -> usize {
        let descendants = self.collect_child_ids_blocking(session_id);
        let mut records = self.records.lock().unwrap();
        let mut cancelled = 0usize;
        for id in descendants {
            if let Some(record) = records.get_mut(&id)
                && let SessionActivity::RunningTurn(live) = &mut record.activity
            {
                live.stopping = true;
                live.cancel.cancel();
                cancelled += 1;
            }
        }
        cancelled
    }

    /// Acquire the revert lease. If a turn is running, signal cancellation and
    /// wait until `finish_turn` atomically hands this request exclusive ownership.
    /// No Idle state is observable between the old turn and the truncate.
    pub fn try_begin_revert(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<Option<SessionOperationLease>> {
        self.cancel_descendant_turns(session_id);
        let operation_id = uuid::Uuid::new_v4().to_string();
        let mut records = self.records.lock().unwrap();
        loop {
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            match &mut record.activity {
                SessionActivity::Idle => {
                    record.activity = SessionActivity::Exclusive {
                        operation_id: operation_id.clone(),
                        kind: SessionOperationKind::Revert,
                    };
                    return Ok(Some(SessionOperationLease {
                        manager: Arc::clone(self),
                        session_id: session_id.to_string(),
                        operation_id,
                    }));
                }
                SessionActivity::RunningTurn(live) => {
                    match &record.pending_revert {
                        Some(pending) if pending != &operation_id => {
                            return Err(LitecodeError::AgentAlreadyRunning);
                        }
                        Some(_) => {}
                        None => record.pending_revert = Some(operation_id.clone()),
                    }
                    live.stopping = true;
                    live.cancel.cancel();
                }
                SessionActivity::StartingTurn { .. } => {
                    record.activity = SessionActivity::Exclusive {
                        operation_id: operation_id.clone(),
                        kind: SessionOperationKind::Revert,
                    };
                    drop(records);
                    self.turn_guard.end_turn();
                    return Ok(Some(SessionOperationLease {
                        manager: Arc::clone(self),
                        session_id: session_id.to_string(),
                        operation_id,
                    }));
                }
                SessionActivity::Exclusive { .. } => {
                    return Err(LitecodeError::AgentAlreadyRunning);
                }
            }
            records = self
                .activity_changed
                .wait(records)
                .unwrap_or_else(|error| error.into_inner());
            let Some(record) = records.get(session_id) else {
                return Err(LitecodeError::ToolExecution(format!(
                    "session {session_id} not found"
                )));
            };
            if matches!(
                &record.activity,
                SessionActivity::Exclusive {
                    operation_id: active,
                    kind: SessionOperationKind::Revert,
                } if active == &operation_id
            ) {
                return Ok(Some(SessionOperationLease {
                    manager: Arc::clone(self),
                    session_id: session_id.to_string(),
                    operation_id,
                }));
            }
        }
    }

    pub async fn is_turn_running(&self, session_id: &str) -> bool {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .is_some_and(|r| r.activity.is_turn())
    }

    pub fn is_turn_running_blocking(&self, session_id: &str) -> bool {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .is_some_and(|r| r.activity.is_turn())
    }

    pub fn is_session_busy_blocking(&self, session_id: &str) -> bool {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .is_some_and(|r| r.activity.is_busy())
    }

    pub fn is_compacting_blocking(&self, session_id: &str) -> bool {
        let records = self.records.lock().unwrap();
        records.get(session_id).is_some_and(|r| {
            matches!(
                &r.activity,
                SessionActivity::Exclusive {
                    kind: SessionOperationKind::Compact,
                    ..
                }
            )
        })
    }

    /// Acquire a process-local exclusive lease while the session is idle.
    ///
    /// Manual compaction must hold this lease from snapshot load through the
    /// final checkpoint commit; destructive revert/delete should hold it for
    /// their complete operation as well.
    pub fn try_begin_operation(
        self: &Arc<Self>,
        session_id: &str,
        kind: SessionOperationKind,
    ) -> Result<SessionOperationLease> {
        if kind == SessionOperationKind::Delete && self.has_active_descendants(session_id) {
            return Err(LitecodeError::AgentAlreadyRunning);
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        if record.activity.is_busy() {
            return Err(LitecodeError::AgentAlreadyRunning);
        }
        record.activity = SessionActivity::Exclusive {
            operation_id: operation_id.clone(),
            kind,
        };
        drop(records);
        Ok(SessionOperationLease {
            manager: Arc::clone(self),
            session_id: session_id.to_string(),
            operation_id,
        })
    }

    fn release_operation(&self, session_id: &str, operation_id: &str) {
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return;
        };
        let matches = matches!(
            &record.activity,
            SessionActivity::Exclusive {
                operation_id: active,
                ..
            } if active == operation_id
        );
        if matches {
            record.activity = SessionActivity::Idle;
            self.activity_changed.notify_all();
        }
    }

    /// List / GC emptiness: no SessionEvent rows yet. Not AgentView.
    ///
    /// Fold / working-set load must not hide a session that still has transcript
    /// rows. A busy turn is treated as non-empty so a just-started session stays
    /// listed before the first append lands.
    pub async fn is_session_empty(&self, session_id: &str) -> bool {
        {
            let records = self.records.lock().unwrap();
            if let Some(record) = records.get(session_id)
                && record.activity.is_busy()
            {
                return false;
            }
        }
        match self.data.seq_cursor_blocking(session_id) {
            Ok((last_seq, _)) => last_seq < 0,
            Err(error) => {
                tracing::warn!(
                    session_id,
                    error = %error,
                    "session empty-check failed; treating as non-empty"
                );
                false
            }
        }
    }

    pub async fn subscriber_count(&self, session_id: &str) -> usize {
        self.subscriber_count_blocking(session_id)
    }

    pub fn subscriber_count_blocking(&self, session_id: &str) -> usize {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .map(|r| r.subscriber_count)
            .unwrap_or(0)
    }

    pub fn project(&self, session_id: &str) -> Option<String> {
        let records = self.records.lock().unwrap();
        records.get(session_id).and_then(|r| r.project.clone())
    }

    pub fn set_last_permission_sink(
        &self,
        session_id: &str,
        sink: Arc<dyn crate::permission::PermissionSink>,
    ) {
        let mut records = self.records.lock().unwrap();
        if let Some(record) = records.get_mut(session_id) {
            record.last_permission_sink = Some(sink);
        }
    }

    pub fn last_permission_sink(
        &self,
        session_id: &str,
    ) -> Option<Arc<dyn crate::permission::PermissionSink>> {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .and_then(|r| r.last_permission_sink.clone())
    }

    pub async fn shutdown_cleanup(&self) {
        let ids: Vec<String> = self.records.lock().unwrap().keys().cloned().collect();
        for id in &ids {
            let _ = self.cancel_turn(id, crate::types::StopOrigin::User).await;
        }
        self.data.shutdown();
    }

    /// Remove only stale, empty durable sessions. Closing a panel merely
    /// releases its subscription; it must never make a valid session ID stale.
    pub async fn gc_stale_empty_sessions(&self, max_age: Duration) {
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
        let rows = match self.data.list_gc_blocking() {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = %error, "session GC: failed to list sessions");
                return;
            }
        };

        for (session_id, updated_at) in rows {
            if self.is_child_session(&session_id)
                || updated_at >= cutoff
                || self.subscriber_count(&session_id).await > 0
                || self.is_turn_running(&session_id).await
                || self.has_active_descendants(&session_id)
                || !self.is_session_empty(&session_id).await
            {
                continue;
            }
            let _ = self.remove_session(&session_id);
        }
    }

    /// True when any descendant session is not idle. Used to fence durable
    /// subtree operations (delete) that must not run under a live child turn.
    fn has_active_descendants(&self, session_id: &str) -> bool {
        let children = self.collect_child_ids_blocking(session_id);
        if children.is_empty() {
            return false;
        }
        let records = self.records.lock().unwrap();
        children.iter().any(|id| {
            records
                .get(id)
                .is_some_and(|record| record.activity.is_busy())
        })
    }
    /// Delete child sessions whose durable parent row is gone. This is the
    /// startup repair for databases written by pre-convergence builds.
    pub fn remove_orphan_child_sessions(&self) -> usize {
        let ids = match self
            .data
            .read_blocking(SessionRead::ListOrphanChildSessions)
        {
            Ok(ReadValue::Ids(ids)) => ids,
            _ => return 0,
        };
        let mut removed = 0usize;
        for id in ids {
            if self.remove_session(&id).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Collect the durable subtree ids (children first order is not required)
    /// plus any in-memory child records so a delete cannot leak a live turn.
    fn collect_child_ids_blocking(&self, session_id: &str) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(session_id.to_string());
        let mut out = Vec::new();
        while let Some(parent) = queue.pop_front() {
            let mut children: Vec<String> =
                match self.data.read_blocking(SessionRead::ListChildIds {
                    parent_session_id: parent.clone(),
                }) {
                    Ok(ReadValue::Ids(ids)) => ids,
                    _ => Vec::new(),
                };
            {
                let records = self.records.lock().unwrap();
                for (id, record) in records.iter() {
                    if record.parent_session_id.as_deref() == Some(parent.as_str())
                        && !children.iter().any(|existing| existing == id)
                    {
                        children.push(id.clone());
                    }
                }
            }
            for child in children {
                if child == session_id || !seen.insert(child.clone()) {
                    continue;
                }
                out.push(child.clone());
                queue.push_back(child);
            }
        }
        out
    }

    /// Remove root + descendants from the in-memory registry and release exactly
    /// one turn-guard slot for each removed turn. Must be called only after the
    /// durable delete has been accepted (or was already gone).
    fn take_removed_sessions(&self, session_id: &str, known_children: &[String]) -> Vec<String> {
        let mut records = self.records.lock().unwrap();
        let mut remove_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        remove_ids.insert(session_id.to_string());
        remove_ids.extend(known_children.iter().cloned());

        // Close the small spawn/delete race: any in-memory child whose parent is
        // inside the removed set is removed with it.
        loop {
            let mut added = false;
            for (id, record) in records.iter() {
                if remove_ids.contains(id) {
                    continue;
                }
                if let Some(parent) = record.parent_session_id.as_deref()
                    && remove_ids.contains(parent)
                {
                    remove_ids.insert(id.clone());
                    added = true;
                }
            }
            if !added {
                break;
            }
        }

        let mut pending_turn_ends = 0usize;
        for id in &remove_ids {
            if let Some(record) = records.get_mut(id) {
                if matches!(
                    record.activity,
                    SessionActivity::StartingTurn { .. } | SessionActivity::RunningTurn(_)
                ) {
                    pending_turn_ends += 1;
                }
                record.activity = SessionActivity::Idle;
            }
        }
        for id in &remove_ids {
            records.remove(id);
        }
        drop(records);

        for _ in 0..pending_turn_ends {
            self.turn_guard.end_turn();
        }

        let mut removed_children: Vec<String> = remove_ids
            .into_iter()
            .filter(|id| id != session_id)
            .collect();
        removed_children.sort();
        removed_children
    }

    /// Returns every removed session id: descendants (sorted) first, root last
    /// (WP-090 interface v1 §2; implementation owned by Session / WP-091).
    /// The idempotent SessionNotFound path returns the ids it could still
    /// collect, root included. Each `SessionRemoved` carries its parent id.
    pub fn remove_session(&self, session_id: &str) -> Result<Vec<String>> {
        // Cancel live turns across the durable subtree before deleting rows.
        let known_children = self.collect_child_ids_blocking(session_id);
        {
            let records = self.records.lock().unwrap();
            if let Some(record) = records.get(session_id)
                && let SessionActivity::RunningTurn(live) = &record.activity
            {
                live.cancel.cancel();
            }
            for child_id in &known_children {
                if let Some(record) = records.get(child_id)
                    && let SessionActivity::RunningTurn(live) = &record.activity
                {
                    live.cancel.cancel();
                }
            }
        }
        // WP-091 R7: parents must be read before the rows go.
        let mut parents: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        parents.insert(
            session_id.to_string(),
            self.parent_session_id_of(session_id),
        );
        for child_id in &known_children {
            parents.insert(child_id.clone(), self.parent_session_id_of(child_id));
        }

        // Durable cascade (children first) via SessionData delete.
        let expected = self.expected_revision(session_id);
        match self.mutate_blocking(SessionMutation::Delete {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
        }) {
            // Second arm is idempotent: already gone from the durable store.
            Ok(_) | Err(LitecodeError::SessionNotFound(_)) => {
                let removed_children = self.take_removed_sessions(session_id, &known_children);
                for child_id in &removed_children {
                    self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                        session_id: child_id.clone(),
                        parent_session_id: parents.get(child_id).cloned().flatten(),
                    });
                }
                self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                    session_id: session_id.to_string(),
                    parent_session_id: parents.get(session_id).cloned().flatten(),
                });
                let mut removed = removed_children;
                removed.push(session_id.to_string());
                Ok(removed)
            }
            Err(e) => {
                tracing::error!(
                    session_id,
                    error = %e,
                    "remove_session: SQLite delete failed, keeping record"
                );
                Err(e)
            }
        }
    }

    /// Parent of a session from memory, else durable meta.
    pub fn parent_session_id_of(&self, session_id: &str) -> Option<String> {
        if let Some(record) = self.records.lock().unwrap().get(session_id) {
            return record.parent_session_id.clone();
        }
        self.data
            .meta_blocking(session_id)
            .ok()
            .and_then(|m| m.parent_session_id)
    }

    /// WP-091 R6: busy descendants (root excluded), sorted.
    pub fn running_descendant_ids(&self, session_id: &str) -> Vec<String> {
        let children = self.collect_child_ids_blocking(session_id);
        let records = self.records.lock().unwrap();
        let mut out: Vec<String> = children
            .into_iter()
            .filter(|id| {
                records
                    .get(id)
                    .is_some_and(|record| record.activity.is_busy())
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Test-only helper for creating child sessions in unit tests (protocol /
    /// delete / status tests). Production code must not call it; children are
    /// created only by the subagent launch path (architecture_layers).
    #[cfg(test)]
    pub(crate) fn test_child_session(
        &self,
        project: &str,
        parent_session_id: &str,
        call_id: &str,
    ) -> Result<String> {
        self.open_child_session(project, "default", None, parent_session_id, call_id)
    }

    /// WP-091 R8: broadcast an aggregate status change (Session's R8 hook).
    pub fn emit_status_changed(&self, session_id: &str, status: SessionStatus) {
        self.emit_lifecycle(LifecycleEvent::SessionStatusChanged {
            session_id: session_id.to_string(),
            status: status.as_str().to_string(),
        });
    }

    /// WP-091 R7: tombstones `{call_id: child_session_id}` for children
    /// deleted while this parent survived.
    pub fn deleted_child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> std::collections::HashMap<String, String> {
        match self.data.read_blocking(SessionRead::DeletedChildBindings {
            parent_session_id: parent_session_id.to_string(),
        }) {
            Ok(ReadValue::ChildBindings(pairs)) => pairs.into_iter().collect(),
            _ => std::collections::HashMap::new(),
        }
    }

    /// Subscribe to a session's internal envelope broadcast (works while idle).
    pub fn subscribe(&self, session_id: &str) -> Option<broadcast::Receiver<InternalEnvelope>> {
        let records = self.records.lock().unwrap();
        records.get(session_id).map(|r| r.event_tx.subscribe())
    }

    /// Publish an internal event onto a session's broadcast (and reconnect buffer).
    ///
    /// Used for cross-session signals such as `SubagentBound` on the parent while
    /// the child turn runs on its own channel.
    pub fn publish_internal(&self, session_id: &str, event: InternalEvent) -> bool {
        let envelope = InternalEnvelope {
            event,
            parent_session_id: None,
        };
        let mut records = self.records.lock().unwrap();
        let Some(record) = records.get_mut(session_id) else {
            return false;
        };
        if record.event_buffer.len() >= EVENT_BUFFER_CAPACITY {
            record.event_buffer.pop_front();
        }
        record.note_internal(&envelope.event);
        record.event_buffer.push_back(envelope.clone());
        let _ = record.event_tx.send(envelope);
        true
    }

    /// Reconnect bubble still showing for this session, if any.
    pub fn llm_reconnect(&self, session_id: &str) -> Option<LlmReconnect> {
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|record| record.llm_reconnect)
    }

    pub fn llm_reconnect_retryable(&self, session_id: &str) -> bool {
        self.llm_reconnect(session_id)
            .is_some_and(|notice| notice.is_failed())
    }

    pub fn child_session_id_for_call(
        &self,
        parent_session_id: &str,
        parent_call_id: &str,
    ) -> Option<String> {
        // Prefer in-memory records (same process as the launch).
        {
            let records = self.records.lock().unwrap();
            for (id, record) in records.iter() {
                if record.parent_session_id.as_deref() == Some(parent_session_id)
                    && record.parent_call_id.as_deref() == Some(parent_call_id)
                {
                    return Some(id.clone());
                }
            }
        }
        match self.data.read_blocking(SessionRead::ChildForCall {
            parent_session_id: parent_session_id.to_string(),
            parent_call_id: parent_call_id.to_string(),
        }) {
            Ok(ReadValue::OptionalId(id)) => id,
            _ => None,
        }
    }

    pub fn child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> std::collections::HashMap<String, String> {
        match self.data.read_blocking(SessionRead::ChildBindings {
            parent_session_id: parent_session_id.to_string(),
        }) {
            Ok(ReadValue::ChildBindings(pairs)) => pairs.into_iter().collect(),
            _ => std::collections::HashMap::new(),
        }
    }

    /// Find the persisted function_call event for `call_id`.
    pub fn find_function_call_event(
        &self,
        session_id: &str,
        call_id: &str,
    ) -> Option<crate::session::event::SessionEvent> {
        let events = self.data.events_blocking(session_id).ok()?;
        events.into_iter().find(|event| {
            crate::session::event::item_from_event(event)
                .ok()
                .is_some_and(|item| {
                    matches!(
                        item,
                        crate::types::Item::FunctionCall(ref fc) if fc.call_id == call_id
                    )
                })
        })
    }

    /// WP-186: latest unanswered FunctionCall matching `tool_name` from transcript.
    /// Used to link `pending_permissions.call_id` when the ask is upserted.
    pub fn latest_unanswered_call_id_for_tool(
        &self,
        session_id: &str,
        tool_name: &str,
    ) -> Option<String> {
        let events = self.data.events_blocking(session_id).ok()?;
        let mut answered = std::collections::HashSet::new();
        let mut calls: Vec<(String, String)> = Vec::new();
        for event in events {
            let Ok(item) = crate::session::event::item_from_event(&event) else {
                continue;
            };
            match item {
                crate::types::Item::FunctionCall(fc) if !fc.call_id.is_empty() => {
                    calls.push((fc.call_id, fc.name));
                }
                crate::types::Item::FunctionCallOutput(out) => {
                    answered.insert(out.call_id);
                }
                _ => {}
            }
        }
        calls
            .into_iter()
            .rev()
            .find(|(cid, name)| name == tool_name && !answered.contains(cid))
            .map(|(cid, _)| cid)
    }

    /// Snapshot of the replay buffer (bounded at EVENT_BUFFER_CAPACITY). Each
    /// subscriber gets the same recent events 锟?draining on first consume would
    /// lose replay for every later subscriber (6a-6j).
    pub fn event_buffer_snapshot(&self, session_id: &str) -> Vec<InternalEnvelope> {
        let records = self.records.lock().unwrap();
        if let Some(record) = records.get(session_id) {
            return record.event_buffer.iter().cloned().collect();
        }
        Vec::new()
    }

    pub fn get_cached_progress(&self, session_id: &str) -> Option<TurnProgress> {
        let records = self.records.lock().unwrap();
        records
            .get(session_id)
            .and_then(|r| r.activity.progress())
            .cloned()
    }

    /// Persist sticky session model catalog id. Does not touch `agent_id`.
    pub fn set_session_model_id(&self, session_id: &str, model_id: Option<String>) -> Result<()> {
        self.set_session_model_id_inner(session_id, model_id, None)?;
        Ok(())
    }

    /// Atomically persist the session model and its control-plane switch event.
    ///
    /// The model column and `model/changed` row share one writer transaction,
    /// so a failed event append rolls the model change back too.
    pub fn set_session_model_with_change(
        &self,
        session_id: &str,
        model_id: Option<String>,
        record: &serde_json::Value,
    ) -> Result<crate::session::event::Seq> {
        let event = EventDraft {
            time: chrono::Utc::now().timestamp_millis(),
            event_type: EventType::ModelChanged,
            data: record.clone(),
            surface_op: None,
            source_seqs: None,
            ignorable: false,
            state: crate::session::model::LogState::Final,
        };
        let receipt = self.set_session_model_id_inner(session_id, model_id, Some(event))?;
        match receipt.outcome {
            crate::session::data::command::CommitKind::Appended { seq } => Ok(seq),
            other => Err(LitecodeError::InvalidSessionEvent(format!(
                "set_session_model_with_change: unexpected commit outcome {other:?}"
            ))),
        }
    }

    fn set_session_model_id_inner(
        &self,
        session_id: &str,
        model_id: Option<String>,
        event: Option<EventDraft>,
    ) -> Result<crate::session::data::command::CommitReceipt> {
        let normalized = model_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let expected = self.expected_revision(session_id);
        let receipt = self.mutate_blocking(SessionMutation::SetModel {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            model_id: normalized.clone(),
            event,
        })?;
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.model_id = normalized;
        }
        Ok(receipt)
    }

    pub fn session_model_id(&self, session_id: &str) -> Option<String> {
        let records = self.records.lock().unwrap();
        records.get(session_id).and_then(|r| r.model_id.clone())
    }

    /// Persist sticky session agent id. Does not touch `model_id`.
    pub fn set_agent_id(&self, session_id: &str, agent_id: String) -> Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SetAgent {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            agent_id: agent_id.clone(),
        })?;
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.agent_id = agent_id;
        }
        Ok(())
    }

    pub fn agent_id(&self, session_id: &str) -> Option<String> {
        let records = self.records.lock().unwrap();
        records.get(session_id).map(|r| r.agent_id.clone())
    }

    pub fn thinking_tier(&self, session_id: &str) -> Option<crate::platform_knobs::ThinkingTier> {
        let records = self.records.lock().unwrap();
        records.get(session_id).map(|r| r.thinking_tier)
    }

    pub fn context_mode(&self, session_id: &str) -> Option<crate::platform_knobs::ContextMode> {
        let records = self.records.lock().unwrap();
        records.get(session_id).map(|r| r.context_mode)
    }

    pub fn set_thinking_tier(
        &self,
        session_id: &str,
        tier: crate::platform_knobs::ThinkingTier,
    ) -> Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SetThinkingTier {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            tier,
        })?;
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.thinking_tier = tier;
        }
        Ok(())
    }

    pub fn set_context_mode(
        &self,
        session_id: &str,
        mode: crate::platform_knobs::ContextMode,
    ) -> Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SetContextMode {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            mode,
        })?;
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.context_mode = mode;
        }
        Ok(())
    }

    /// Resolved primary for a turn: per-session binding, else workspace default.
    pub fn resolve_primary_agent(
        &self,
        session_id: &str,
        default_primary: &str,
        resolved: &ResolvedConfig,
    ) -> Result<String> {
        let agent_id = self
            .agent_id(session_id)
            .unwrap_or_else(|| default_primary.to_string());
        RuntimeHandle::validate_primary_agent(resolved, &agent_id)?;
        Ok(agent_id)
    }

    pub fn register_session(&self, session_id: &str, mut task_state: TaskReminders) {
        let meta = self.data.meta_blocking(session_id).ok();
        let mut revision = self.data.revision_blocking(session_id).unwrap_or(0);
        let plan_dir = self.plan_dir_path();
        if prune_stale_active_plan(&mut task_state, &plan_dir) {
            match self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: session_id.to_string(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            }) {
                // Record is not in `records` yet, so note_receipt cannot sync it (S1).
                Ok(saved) => revision = saved.revision,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to persist pruned active plan on open")
                }
            }
        }
        if let Some(meta) = meta {
            let mut record = SessionRecord::from_meta(&meta, task_state, revision);
            record.project = None;
            self.records
                .lock()
                .unwrap()
                .insert(session_id.to_string(), record);
        }
    }

    pub fn register_for_test(&self, session: Session) {
        let id = session.id.clone();
        drop(session);
        self.insert_session_for_test(&id);
    }

    pub fn data_root_path(&self) -> std::path::PathBuf {
        self.data.data_root().to_path_buf()
    }

    pub fn apply(
        &self,
        session_id: &str,
        op: SessionApply,
    ) -> anyhow::Result<crate::session::data::command::CommitReceipt> {
        let expected = self.expected_revision(session_id);
        Ok(self.mutate_blocking(SessionMutation::Apply {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            op,
        })?)
    }

    pub fn persist_item(&self, session_id: &str, item: &crate::types::Item) -> anyhow::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::PersistItem {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            item: item.clone(),
        })?;
        Ok(())
    }

    /// Open the log row for a streaming provider item; returns its `seq`.
    ///
    /// The returned seq is the item's identity for the rest of its life: every
    /// later update and the final seal address it, and the client orders by it.
    pub fn begin_stream_item(
        &self,
        session_id: &str,
        item: &crate::types::Item,
        turn_id: &str,
    ) -> crate::types::Result<crate::session::event::Seq> {
        let expected = self.expected_revision(session_id);
        let receipt = self.mutate_blocking(SessionMutation::BeginStreamItem {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            item: item.clone(),
            turn_id: turn_id.to_string(),
        })?;
        match receipt.outcome {
            crate::session::data::command::CommitKind::Appended { seq } => Ok(seq),
            other => Err(crate::types::LitecodeError::InvalidSessionEvent(format!(
                "begin_stream_item: unexpected commit outcome {other:?}"
            ))),
        }
    }

    /// Replace the payload of a row that is still in flight.
    pub fn update_stream_item(
        &self,
        session_id: &str,
        seq: crate::session::event::Seq,
        item: &crate::types::Item,
    ) -> crate::types::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::UpdateStreamItem {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            seq,
            item: item.clone(),
        })?;
        Ok(())
    }

    /// Settle a streaming row with its authoritative (terminal) payload.
    pub fn seal_stream_item(
        &self,
        session_id: &str,
        seq: crate::session::event::Seq,
        item: &crate::types::Item,
    ) -> crate::types::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SealStreamItem {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            seq,
            item: item.clone(),
        })?;
        Ok(())
    }

    pub fn append_reminder(
        &self,
        session_id: &str,
        reminder: &crate::reminder::Reminder,
    ) -> anyhow::Result<crate::session::event::Seq> {
        let expected = self.expected_revision(session_id);
        let receipt = self.mutate_blocking(SessionMutation::AppendReminder {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            reminder: reminder.clone(),
        })?;
        match receipt.outcome {
            crate::session::data::command::CommitKind::Appended { seq } => Ok(seq),
            other => Err(anyhow::anyhow!(
                "append_reminder: unexpected outcome {other:?}"
            )),
        }
    }

    pub fn spine_reminder_view(
        &self,
        session_id: &str,
    ) -> crate::types::Result<crate::reminder::SpineReminderView> {
        match self.data.read_blocking(SessionRead::SpineReminderView {
            session_id: session_id.to_string(),
        })? {
            crate::session::data::command::ReadValue::SpineReminders(view) => Ok(view),
            _ => Err(crate::types::LitecodeError::SessionStorage(
                "unexpected spine reminder view".into(),
            )),
        }
    }

    /// Record the origin of one LLM request (`request/header`).
    ///
    /// Control-plane only: the row never enters the spine, the UI transcript, or
    /// model input. It exists so a later request can ask "which provider produced
    /// this reasoning ciphertext?" without rewriting any existing item.
    ///
    /// Returns the seq the record landed on, which is what later items of the same
    /// request are attributed to.
    pub fn append_request_origin(
        &self,
        session_id: &str,
        record: &serde_json::Value,
    ) -> crate::types::Result<crate::session::event::Seq> {
        let receipt = self.apply(
            session_id,
            SessionApply::Append(EventDraft {
                time: chrono::Utc::now().timestamp_millis(),
                event_type: EventType::RequestHeader,
                data: record.clone(),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::model::LogState::Final,
            }),
        )?;
        match receipt.outcome {
            crate::session::data::command::CommitKind::Appended { seq } => Ok(seq),
            other => Err(crate::types::LitecodeError::InvalidSessionEvent(format!(
                "append_request_origin: unexpected commit outcome {other:?}"
            ))),
        }
    }

    /// `(seq, body)` of every recorded request origin, oldest first.
    pub fn request_origins(
        &self,
        session_id: &str,
    ) -> crate::types::Result<Vec<(crate::session::event::Seq, serde_json::Value)>> {
        match self.data.read_blocking(SessionRead::RequestOrigins {
            session_id: session_id.to_string(),
        })? {
            ReadValue::RequestOrigins(rows) => Ok(rows
                .into_iter()
                .map(|(seq, body)| (seq.max(0) as crate::session::event::Seq, body))
                .collect()),
            _ => Err(crate::types::LitecodeError::SessionStorage(
                "unexpected request origins read".into(),
            )),
        }
    }

    /// Store one request's provider bill on its header row.
    ///
    /// The update names only `provider_usage`. A missing row, a non-header seq,
    /// or a column that already has a value changes nothing. The session
    /// revision is left alone.
    pub fn record_provider_usage(
        &self,
        session_id: &str,
        seq: crate::session::event::Seq,
        usage: &ProviderUsage,
    ) -> Result<()> {
        let usage_json = serde_json::to_string(usage).map_err(|err| {
            LitecodeError::SessionStorage(format!("provider usage encode: {err}"))
        })?;
        self.mutate_blocking(SessionMutation::RecordProviderUsage {
            session_id: session_id.to_string(),
            operation_id: MutationId::new(),
            seq,
            usage_json,
        })?;
        Ok(())
    }

    /// Bills already stored on this session's request headers, oldest first.
    pub fn provider_usage_rollup(&self, session_id: &str) -> Result<ProviderUsageRollup> {
        match self.data.read_blocking(SessionRead::ProviderUsageRollup {
            session_id: session_id.to_string(),
        })? {
            ReadValue::ProviderUsageRollup(rollup) => Ok(rollup),
            _ => Err(LitecodeError::SessionStorage(
                "unexpected provider usage rollup".into(),
            )),
        }
    }

    /// Persist a consumed queue as one ordinary user message.
    ///
    /// The only durable artifact of the memory-only pending queue: it must be
    /// indistinguishable from a message typed into the composer — same
    /// `item/user` kind, same revert-anchor status, same rendering.
    pub fn append_user_message(
        &self,
        session_id: &str,
        input: impl Into<crate::types::UserInput>,
    ) -> anyhow::Result<crate::session::event::Seq> {
        let input = input.into();
        let item = crate::types::user_message(input.text, &input.images);
        let mut draft = EventDraft::surface_item(
            EventType::ItemUser,
            &item,
            crate::session::surface::SurfaceOp::Append,
        )?;
        draft.time = chrono::Utc::now().timestamp_millis();
        let receipt = self.apply(session_id, SessionApply::Append(draft))?;
        match receipt.outcome {
            crate::session::data::command::CommitKind::Appended { seq } => Ok(seq),
            other => Err(anyhow::anyhow!(
                "append_user_message: unexpected outcome {other:?}"
            )),
        }
    }

    /// Persist a user message, then the hidden mentions attachment when it has one.
    ///
    /// A failed attachment is logged and does not undo the user row.
    pub fn append_user_message_with_mentions(
        &self,
        session_id: &str,
        input: impl Into<crate::types::UserInput>,
        workspace: &std::path::Path,
    ) -> anyhow::Result<crate::session::event::Seq> {
        let input = input.into();
        let seq = self.append_user_message(session_id, input.clone())?;
        self.append_mentions_for(session_id, workspace, &input.text);
        self.append_knowledge_status_for(session_id, workspace);
        Ok(seq)
    }

    /// Snapshot digests of mention citations still on the model-visible surface.
    ///
    /// A compacted reminder is no longer on that surface, so its digest is not
    /// returned and a later citation attaches the snapshot again.
    fn live_mention_digests(&self, session_id: &str) -> HashMap<String, String> {
        let Ok(events) = self.data.events_blocking(session_id) else {
            return HashMap::new();
        };
        let Ok(surface) = crate::session::fold_surface(&events) else {
            return HashMap::new();
        };
        let by_seq: HashMap<_, _> = events.iter().map(|event| (event.seq, event)).collect();
        let mut delivered = HashMap::new();
        for seq in &surface.nodes {
            let Some(event) = by_seq.get(seq) else {
                continue;
            };
            let EventType::Reminder(crate::reminder::ReminderKind::Mentions) = &event.event_type
            else {
                continue;
            };
            let Ok(crate::reminder::Reminder::Mentions(body)) =
                serde_json::from_value(event.data.clone())
            else {
                continue;
            };
            for citation in body.refs {
                if let Some(digest) = citation.digest {
                    delivered.insert(citation.key, digest);
                }
            }
        }
        delivered
    }

    /// Write `reminder/mentions` for text that was just stored as a user row.
    ///
    /// Citations whose snapshot still matches one on the live surface are omitted.
    pub fn append_mentions_for(
        &self,
        session_id: &str,
        workspace: &std::path::Path,
        user_text: &str,
    ) {
        let mut stages = crate::telemetry::latency::Stages::new("mentions", None);
        let digests = self.live_mention_digests(session_id);
        stages.mark("live_mention_digests");
        let built = crate::reminder::mentions::build(workspace, user_text, &digests);
        stages.mark("build");
        let Some(reminder) = built else {
            return;
        };
        if let Err(error) = self.append_reminder(session_id, &reminder) {
            tracing::warn!(%error, "mentions reminder was not saved");
        }
        stages.mark("append");
    }

    /// Write `reminder/knowledge_status` after a human message.
    ///
    /// A library with no enabled node writes nothing. The same class still on
    /// the live surface is left as it is. A failed write does not undo the user row.
    pub fn append_knowledge_status_for(&self, session_id: &str, workspace: &std::path::Path) {
        // `build` only reads the resident knowledge snapshot; its parts
        // (lookup / wait_fresh / snapshot_read) are on the
        // `knowledge_status.note` line.
        let mut stages = crate::telemetry::latency::Stages::new("knowledge_status", None);
        let built = crate::reminder::knowledge_status::build(workspace);
        stages.mark("build");
        let Some(reminder) = built else {
            return;
        };
        let view = self.spine_reminder_view(session_id);
        stages.mark("spine_view");
        if let Ok(view) = view
            && let Some(previous) = view.latest(crate::reminder::ReminderKind::KnowledgeStatus)
            && previous.same_snapshot(&reminder)
        {
            return;
        }
        if let Err(error) = self.append_reminder(session_id, &reminder) {
            tracing::warn!(%error, "knowledge status reminder was not saved");
        }
        stages.mark("append");
    }

    /// Persist the plan-execution trigger as a dedicated `plan/execute` spine Item.
    ///
    /// System-issued on the human's behalf; deliberately not a revert anchor.
    pub fn append_plan_execute(
        &self,
        session_id: &str,
        item: &crate::types::Item,
    ) -> anyhow::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::AppendPlanExecute {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            item: item.clone(),
        })?;
        Ok(())
    }

    pub fn seal_in_progress_items(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<crate::session::event::Seq>> {
        let expected = self.expected_revision(session_id);
        match self.mutate_blocking(SessionMutation::SealInProgress {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
        }) {
            Ok(receipt) => match receipt.outcome {
                crate::session::data::command::CommitKind::Sealed { seqs } => Ok(seqs),
                crate::session::data::command::CommitKind::Idempotent => Ok(Vec::new()),
                _ => Ok(Vec::new()),
            },
            Err(e) => Err(e.into()),
        }
    }

    pub fn save_context_meter(
        &self,
        session_id: &str,
        meter: &SessionContextMeter,
    ) -> anyhow::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SaveContextMeter {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            meter: meter.clone(),
        })?;
        Ok(())
    }

    pub fn insert_detail_rows(
        &self,
        session_id: &str,
        items: &[crate::types::Item],
    ) -> anyhow::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::InsertDetails {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            items: items.to_vec(),
            turn_id: String::new(),
        })?;
        Ok(())
    }

    pub fn commit_turn_delta(
        &self,
        session_id: &str,
        rows: Vec<crate::session::working::WorkingRow>,
        expected_max_seq: i64,
        turn_id: &str,
        result_data: &[crate::types::ToolResultData],
    ) -> anyhow::Result<(
        crate::session::data::command::CommitKind,
        Vec<crate::session::working::WorkingRow>,
        Option<crate::session::data::command::SessionListPreview>,
    )> {
        let expected = self.expected_revision(session_id);
        let receipt = self.mutate_blocking(SessionMutation::CommitTurnDelta {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            rows,
            expected_max_seq,
            turn_id: turn_id.to_string(),
            result_data: result_data.to_vec(),
        })?;
        let working = receipt
            .working_set
            .ok_or_else(|| anyhow::anyhow!("CommitTurnDelta receipt missing writer working set"))?;
        let preview = match (receipt.preview, receipt.assistant_preview) {
            (None, None) => None,
            (user, assistant) => {
                let updated_at = user
                    .as_ref()
                    .map(|(_, ts)| *ts)
                    .or_else(|| assistant.as_ref().map(|(_, ts)| *ts))
                    .unwrap_or(0);
                Some(crate::session::data::command::SessionListPreview {
                    updated_at,
                    user: user.map(|(text, _)| text),
                    assistant: assistant.map(|(text, _)| text),
                })
            }
        };
        Ok((receipt.outcome, working, preview))
    }
}

async fn fanout_turn(
    session_id: String,
    mut handle: TurnHandle,
    event_tx: broadcast::Sender<InternalEnvelope>,
    manager: Arc<SessionManager>,
) {
    let mut progress = {
        let records = manager.records.lock().unwrap();
        records
            .get(&session_id)
            .and_then(|r| r.activity.progress())
            .cloned()
            .unwrap_or(TurnProgress {
                turn_id: handle.turn_id.clone(),
                phase: TurnPhase::Starting,
                step: 1,
                step_max: handle.step_max,
                started_at_ms: chrono::Utc::now().timestamp_millis(),
                awaiting_permission: false,
            })
    };
    // Dedupe discrete turn-step announces by stream item id (added + first delta).
    let mut announced_step_items = std::collections::HashSet::<String>::new();
    let mut saw_completed = false;

    loop {
        match handle.rx.recv().await {
            Some(envelope) => {
                let is_completed = matches!(&envelope.event, InternalEvent::TurnCompleted { .. });
                if is_completed {
                    saw_completed = true;
                    // Finish before forwarding so gates open at the same moment as end events.
                    let _ = manager.finish_turn(&session_id, &handle.turn_id);
                }

                let progress_changed = apply_event_to_progress(&mut progress, &envelope.event);
                if progress_changed && !is_completed {
                    let _ = manager.apply_progress(&session_id, progress.clone());
                }

                match &envelope.event {
                    InternalEvent::SessionPreviewUpdated {
                        preview,
                        assistant_preview,
                        updated_at,
                    } => {
                        manager.emit_lifecycle(LifecycleEvent::SessionPreviewUpdated {
                            session_id: session_id.clone(),
                            preview: preview.clone(),
                            assistant_preview: assistant_preview.clone(),
                            updated_at: *updated_at,
                        });
                    }
                    InternalEvent::StreamEvent(ev) => {
                        if let Some((item_id, kind)) = turn_step_from_stream(ev)
                            && announced_step_items.insert(item_id)
                        {
                            manager.emit_lifecycle(LifecycleEvent::TurnStep {
                                session_id: session_id.clone(),
                                kind,
                                progress: progress.clone(),
                            });
                        }
                    }
                    _ => {}
                }

                {
                    let mut records = manager.records.lock().unwrap();
                    if let Some(record) = records.get_mut(&session_id) {
                        record.note_internal(&envelope.event);
                        if record.event_buffer.len() >= EVENT_BUFFER_CAPACITY {
                            record.event_buffer.pop_front();
                        }
                        record.event_buffer.push_back(envelope.clone());
                    }
                }

                let _ = event_tx.send(envelope);
            }
            None => break,
        }
    }

    if !saw_completed {
        tracing::error!(
            session_id = %session_id,
            turn_id = %handle.turn_id,
            "turn event channel closed without TurnCompleted"
        );
    }
    if let Some(join) = handle.handle.take() {
        match join.join() {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => tracing::error!(
                session_id = %session_id,
                turn_id = %handle.turn_id,
                error = %error,
                "agent turn thread returned error"
            ),
            Err(_) => tracing::error!(
                session_id = %session_id,
                turn_id = %handle.turn_id,
                "agent turn thread panicked"
            ),
        }
    }

    // Safety net if the turn thread died without TurnCompleted.
    let _ = manager.finish_turn(&session_id, &handle.turn_id);
}

/// Fan out the full pending-message list to this session's subscribers.
///
/// Must be called while the `records` lock is held: the list and the broadcast
/// then stay one sequence, so a later mutation can never be observed before an
/// earlier one.
fn emit_pending_messages_locked(record: &mut SessionRecord) {
    let envelope = InternalEnvelope {
        event: InternalEvent::PendingMessages {
            pending: record.pending_messages.clone(),
        },
        parent_session_id: None,
    };
    if record.event_buffer.len() >= EVENT_BUFFER_CAPACITY {
        record.event_buffer.pop_front();
    }
    record.event_buffer.push_back(envelope.clone());
    let _ = record.event_tx.send(envelope);
}

/// Map a stream event to a one-shot list step kind + stable item id (for dedupe).
fn turn_step_from_stream(
    ev: &crate::types::StreamEvents,
) -> Option<(String, crate::session::live::TurnStepKind)> {
    use crate::authority::responses::{OutputItem, ResponseStreamEvent};
    use crate::session::live::TurnStepKind;

    match ev {
        ResponseStreamEvent::ResponseOutputItemAdded(e) => match &e.item {
            OutputItem::Reasoning(r) => {
                let id =
                    r.id.clone()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("reasoning-{}", e.output_index));
                Some((id, TurnStepKind::Reasoning))
            }
            OutputItem::FunctionCall(fc) => {
                let id = fc
                    .id
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| fc.call_id.clone());
                if id.is_empty() {
                    None
                } else {
                    Some((id, TurnStepKind::Toolcall))
                }
            }
            OutputItem::Message(m) if !m.id.is_empty() => Some((m.id.clone(), TurnStepKind::Text)),
            _ => None,
        },
        ResponseStreamEvent::ResponseReasoningTextDelta(e) if !e.item_id.is_empty() => {
            Some((e.item_id.clone(), TurnStepKind::Reasoning))
        }
        ResponseStreamEvent::ResponseOutputTextDelta(e) if !e.item_id.is_empty() => {
            Some((e.item_id.clone(), TurnStepKind::Text))
        }
        ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(e) if !e.item_id.is_empty() => {
            Some((e.item_id.clone(), TurnStepKind::Toolcall))
        }
        _ => None,
    }
}

#[cfg(test)]
mod request_origin_tests {
    use super::*;

    /// A switch is durable and control-plane: state and history share one
    /// revision, it never becomes model input, and it is not a request origin.
    #[test]
    fn model_change_commits_state_and_control_plane_row_together() {
        let manager = SessionManager::ephemeral_registry();
        let session_id = manager
            .open_session_sync("", "default", None)
            .expect("session opens");
        manager
            .set_session_model_id(&session_id, Some("p/a".into()))
            .expect("initial model");
        let revision = manager
            .data()
            .revision_blocking(&session_id)
            .expect("revision");
        let record = serde_json::json!({
            "schema": 1,
            "from": "p/a",
            "to": "p/b",
            "at_seq": 0,
            "report": { "reasoning_dropped": 3, "projected_parts": 1 },
        });
        let seq = manager
            .set_session_model_with_change(&session_id, Some("p/b".into()), &record)
            .expect("switch commits");

        assert_eq!(
            manager.session_model_id(&session_id).as_deref(),
            Some("p/b")
        );
        assert_eq!(
            manager
                .data()
                .revision_blocking(&session_id)
                .expect("revision"),
            revision + 1,
            "one user action must advance the session revision once"
        );
        let events = manager.data().events_blocking(&session_id).expect("events");
        let row = events
            .iter()
            .find(|event| event.event_type == EventType::ModelChanged)
            .expect("durable switch row");
        assert_eq!(row.seq, seq);
        assert_eq!(row.data["to"], "p/b");
        assert_eq!(row.data["report"]["reasoning_dropped"], 3);

        let messages = crate::session::surface::derive_messages(&events).expect("derive");
        assert!(
            messages.is_empty(),
            "a switch record must never reach the model: {messages:?}"
        );
        assert!(
            manager.request_origins(&session_id).unwrap().is_empty(),
            "a switch is not a request origin"
        );
    }

    #[test]
    fn failed_model_change_rolls_back_state_and_event_together() {
        let manager = SessionManager::ephemeral_registry();
        let session_id = manager
            .open_session_sync("", "default", None)
            .expect("session opens");
        manager
            .set_session_model_id(&session_id, Some("p/a".into()))
            .expect("initial model");
        let revision = manager
            .data()
            .revision_blocking(&session_id)
            .expect("revision");
        let record = serde_json::json!({
            "schema": 1,
            "from": "p/a",
            "to": "p/b",
            "at_seq": 0,
            "report": { "reasoning_dropped": 0 },
        });

        manager
            .data()
            .hooks()
            .inject(crate::session::FaultKind::BeforeCommit);
        let error = manager
            .set_session_model_with_change(&session_id, Some("p/b".into()), &record)
            .expect_err("injected commit failure must fail the switch");
        assert!(error.to_string().contains("before commit"), "{error}");

        assert_eq!(
            manager.session_model_id(&session_id).as_deref(),
            Some("p/a"),
            "the model change must roll back with the event"
        );
        let events = manager.data().events_blocking(&session_id).expect("events");
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::ModelChanged),
            "no switch row may survive a rolled-back model change"
        );
        assert_eq!(
            manager
                .data()
                .revision_blocking(&session_id)
                .expect("revision"),
            revision,
            "a failed switch must not advance the revision"
        );
    }

}

#[cfg(test)]
mod turn_step_tests {
    
    
    

}

fn apply_event_to_progress(progress: &mut TurnProgress, ev: &InternalEvent) -> bool {
    match ev {
        InternalEvent::TurnStarted {
            turn_id, step_max, ..
        } => {
            progress.turn_id = turn_id.clone();
            progress.phase = TurnPhase::Starting;
            progress.step = 1;
            progress.step_max = *step_max;
            progress.started_at_ms = chrono::Utc::now().timestamp_millis();
            progress.awaiting_permission = false;
            true
        }
        InternalEvent::PhaseChanged { phase, step } => {
            progress.phase = phase.clone();
            progress.step = *step;
            true
        }
        InternalEvent::StepStarted { step, step_max } => {
            progress.step = *step;
            progress.step_max = *step_max;
            true
        }
        InternalEvent::PermissionAwaiting { awaiting } => {
            progress.awaiting_permission = *awaiting;
            true
        }
        InternalEvent::CompactionLifecycle {
            trigger: crate::runtime::observer::CompactionTrigger::Auto,
            stage: crate::runtime::observer::CompactionStage::Started,
            ..
        } => {
            progress.phase = TurnPhase::Compacting;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod child_session_tests {
    use super::*;
    use crate::config::TurnGuard;
    
    
    use std::sync::Arc;

    fn wp090_mgr(dir: &tempfile::TempDir) -> (Arc<TurnGuard>, Arc<SessionManager>) {
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::clone(&guard),
            dir.path().join("sessions.db").to_str().unwrap().to_string(),
        ));
        (guard, mgr)
    }

    #[tokio::test]
    async fn cancel_during_starting_turn_cancels_on_start() {
        let dir = tempfile::tempdir().unwrap();
        let (_guard, mgr) = wp090_mgr(&dir);
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.reserve_turn(&sid, "t-start".into(), 5, "default", "/proj")
            .unwrap();
        assert_eq!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User),
            CancelOutcome::Starting
        );
        assert_eq!(
            mgr.cancel_turn(&sid, crate::types::StopOrigin::User).await,
            CancelOutcome::Starting
        );
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let token = CancellationToken::new();
        let handle = crate::runtime::TurnHandle {
            handle: None,
            rx,
            cancel: token.clone(),
            turn_id: "t-start".into(),
            step_max: 5,
        };
        mgr.start_turn(&sid, handle, "default", "/proj", Arc::clone(&mgr))
            .await
            .unwrap();
        assert!(
            token.is_cancelled(),
            "token cancelled the moment it installs"
        );
        assert_eq!(mgr.session_status(&sid), Some(SessionStatus::Stopping));
        assert_eq!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User),
            CancelOutcome::Running
        );
        assert!(mgr.finish_turn(&sid, "t-start").is_some());
        assert_eq!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User),
            CancelOutcome::Idle
        );
    }

    #[tokio::test]
    async fn cancel_turn_sync_reports_idle_when_nothing_running() {
        let dir = tempfile::tempdir().unwrap();
        let (_guard, mgr) = wp090_mgr(&dir);
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        assert_eq!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User),
            CancelOutcome::Idle
        );
        assert!(!CancelOutcome::Idle.cancelled());
        assert_eq!(
            mgr.cancel_turn_sync("missing", crate::types::StopOrigin::User),
            CancelOutcome::Idle
        );
        assert_eq!(mgr.session_status(&sid), Some(SessionStatus::Idle));
        assert!(CancelOutcome::Running.cancelled() && CancelOutcome::Starting.cancelled());
    }

    #[tokio::test]
    async fn cancel_leaves_pending_queue_intact() {
        let dir = tempfile::tempdir().unwrap();
        let (_guard, mgr) = wp090_mgr(&dir);
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.begin_turn(
            &sid,
            "t-run".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        mgr.enqueue_pending_message(&sid, "queued").unwrap();
        assert_eq!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User),
            CancelOutcome::Running
        );
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 1);
        assert!(mgr.finish_turn(&sid, "t-run").is_some());
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 1);
    }

    #[tokio::test]
    async fn turn_status_hook_fires_on_start_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let (_guard, mgr) = wp090_mgr(&dir);
        let seen: Arc<std::sync::Mutex<Vec<(String, SessionStatus)>>> = Arc::default();
        let sink = Arc::clone(&seen);
        mgr.set_turn_status_hook(Arc::new(move |sid, status| {
            sink.lock().unwrap().push((sid.to_string(), status));
        }));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.reserve_turn(&sid, "t-rel".into(), 5, "default", "/proj")
            .unwrap();
        assert!(mgr.release_turn_reservation(&sid, "t-rel"));
        mgr.reserve_turn(&sid, "t-go".into(), 5, "default", "/proj")
            .unwrap();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = crate::runtime::TurnHandle {
            handle: None,
            rx,
            cancel: CancellationToken::new(),
            turn_id: "t-go".into(),
            step_max: 5,
        };
        mgr.start_turn(&sid, handle, "default", "/proj", Arc::clone(&mgr))
            .await
            .unwrap();
        assert!(mgr.finish_turn(&sid, "t-go").is_some());
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (sid.clone(), SessionStatus::Idle),
                (sid.clone(), SessionStatus::Running),
                (sid.clone(), SessionStatus::Idle),
            ]
        );
    }

    #[tokio::test]
    async fn begin_turn_rejects_second_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap().to_string();
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(Arc::clone(&guard), db_path));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();

        mgr.begin_turn(
            &sid,
            "t1".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        assert!(mgr.is_turn_running_blocking(&sid));
        assert!(guard.is_turn_in_progress());

        let err = mgr
            .begin_turn(
                &sid,
                "t2".into(),
                CancellationToken::new(),
                5,
                "default",
                "/proj",
            )
            .unwrap_err();
        assert!(matches!(err, LitecodeError::AgentAlreadyRunning));
    }

    fn append_turn_end(mgr: &SessionManager, sid: &str, turn: &str, reason: &str) {
        mgr.apply(
            sid,
            crate::session::store::SessionApply::Append(crate::session::EventDraft {
                time: 1,
                event_type: crate::session::EventType::TurnEnd,
                data: serde_json::json!({"turn": turn, "reason": reason, "final_text": "llm error: boom"}),
                surface_op: None,
                source_seqs: None,
                ignorable: false,
                state: crate::session::LogState::Final,
            }),
        )
        .unwrap();
    }

    /// WP-024 G3: error termination (reconnect exhausted / auto compact failed)
    /// persists turn/end before finish_turn, so TurnFinished carries a non-empty reason.
    #[tokio::test]
    async fn g3_turn_finished_reason_non_empty_on_error_termination() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sessions.db").to_str().unwrap().to_string();
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db_path,
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.begin_turn(
            &sid,
            "t1".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        let mut rx = mgr.subscribe_lifecycle();
        append_turn_end(&mgr, &sid, "t1", "error");
        assert!(mgr.finish_turn(&sid, "t1").is_some());
        let mut reason = None;
        while let Ok(ev) = rx.try_recv() {
            if let LifecycleEvent::TurnFinished {
                session_id,
                reason: r,
                ..
            } = ev
            {
                if session_id == sid {
                    reason = Some(r);
                }
            }
        }
        let reason = reason
            .expect("TurnFinished emitted")
            .expect("reason present");
        assert!(!reason.is_empty());
        assert_eq!(reason, "error");
    }

    /// WP-024 G4 (manager half): while a turn runs, a Compact operation is
    /// rejected with AgentAlreadyRunning; the wire mapping is asserted in
    /// client_protocol::connection::compact_error_tests.
    /// WP-024 G5: after error termination the session is Idle and a new turn
    /// starts (no stuck RunningTurn), and compact is admitted again.
    #[tokio::test]
    async fn g4_g5_compact_rejected_while_running_and_turn_restarts_after_error() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sessions.db").to_str().unwrap().to_string();
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(Arc::clone(&guard), db_path));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.begin_turn(
            &sid,
            "t1".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        assert!(matches!(
            mgr.try_begin_operation(&sid, SessionOperationKind::Compact),
            Err(LitecodeError::AgentAlreadyRunning)
        ));
        append_turn_end(&mgr, &sid, "t1", "error");
        mgr.finish_turn(&sid, "t1").unwrap();
        assert!(!mgr.is_turn_running_blocking(&sid));
        assert!(!guard.is_turn_in_progress());
        {
            let _lease = mgr
                .try_begin_operation(&sid, SessionOperationKind::Compact)
                .expect("compact admitted after error termination");
        }
        mgr.begin_turn(
            &sid,
            "t2".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .expect("new turn after error termination");
        assert!(mgr.is_turn_running_blocking(&sid));
    }

    /// WP-024 S1 repro: reopening a session whose active plan file is gone
    /// prunes it with SaveTaskState; in-memory revision must follow the DB,
    /// otherwise the next CAS write conflicts.
    #[tokio::test]
    async fn s1_reopen_prune_keeps_revision_in_sync() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("sessions.db").to_str().unwrap().to_string();
        let sid = {
            let mgr = SessionManager::new_for_test(Arc::new(TurnGuard::new()), db_path.clone());
            let sid = mgr.open_session("/proj", "default", None).await.unwrap();
            mgr.records
                .lock()
                .unwrap()
                .get_mut(&sid)
                .unwrap()
                .task_state
                .active_plan = Some(crate::session::task_state::PlanRef::new("gone"));
            mgr.save_task_state(&sid).unwrap();
            sid
        };
        let mgr = SessionManager::new_for_test(Arc::new(TurnGuard::new()), db_path);
        mgr.ensure_entry(&sid).await.unwrap();
        assert!(
            mgr.records.lock().unwrap()[&sid]
                .task_state
                .active_plan
                .is_none()
        );
        assert_eq!(
            mgr.expected_revision(&sid),
            mgr.data().revision_blocking(&sid).unwrap(),
            "in-memory revision must match DB after prune-on-open"
        );
        mgr.save_task_state(&sid)
            .expect("next CAS write must not conflict");
    }

    #[tokio::test]
    async fn finish_turn_clears_running_immediately_and_allows_rebegin() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap().to_string();
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(Arc::clone(&guard), db_path));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();

        mgr.begin_turn(
            &sid,
            "t1".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        assert!(mgr.finish_turn(&sid, "t1").is_some());
        assert!(!mgr.is_turn_running_blocking(&sid));
        assert!(!guard.is_turn_in_progress());

        // Second finish is a no-op (no double TurnGuard end).
        assert!(mgr.finish_turn(&sid, "t1").is_none());

        mgr.begin_turn(
            &sid,
            "t2".into(),
            CancellationToken::new(),
            5,
            "default",
            "/proj",
        )
        .unwrap();
        assert!(mgr.is_turn_running_blocking(&sid));
        assert_eq!(
            mgr.get_cached_progress(&sid).map(|p| p.turn_id),
            Some("t2".into())
        );
    }

    #[tokio::test]
    async fn compact_lease_atomically_blocks_turn_and_revert() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();

        let compact = mgr
            .try_begin_operation(&sid, SessionOperationKind::Compact)
            .expect("reserve compact");
        assert!(mgr.is_session_busy_blocking(&sid));
        assert!(mgr.is_compacting_blocking(&sid));
        assert!(!mgr.is_turn_running_blocking(&sid));

        let turn_err = mgr
            .reserve_turn(&sid, "t1".into(), 5, "default", "/proj")
            .unwrap_err();
        assert!(matches!(turn_err, LitecodeError::AgentAlreadyRunning));
        let revert_err = match mgr.try_begin_operation(&sid, SessionOperationKind::Revert) {
            Ok(_) => panic!("revert must not overlap compact"),
            Err(error) => error,
        };
        assert!(matches!(revert_err, LitecodeError::AgentAlreadyRunning));
        let revert_busy = match mgr.try_begin_revert(&sid) {
            Ok(_) => panic!("try_begin_revert must not overlap compact"),
            Err(error) => error,
        };
        assert!(matches!(revert_busy, LitecodeError::AgentAlreadyRunning));

        drop(compact);
        assert!(!mgr.is_session_busy_blocking(&sid));
        let revert = mgr
            .try_begin_operation(&sid, SessionOperationKind::Revert)
            .expect("lease releases on drop");
        drop(revert);
    }

    #[tokio::test]
    async fn revert_during_running_turn_waits_for_finish_and_holds_exclusive_lease() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.insert_detail_rows(
            &sid,
            &[
                crate::types::user_text("u0"),
                crate::types::user_text("u1"),
                crate::types::user_text("u2"),
            ],
        )
        .unwrap();
        let cancel = CancellationToken::new();
        mgr.begin_turn(
            &sid,
            "t-revert".into(),
            cancel.clone(),
            5,
            "default",
            "/proj",
        )
        .unwrap();

        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let waiter = {
            let mgr = Arc::clone(&mgr);
            let sid = sid.clone();
            std::thread::spawn(move || {
                let lease = mgr
                    .try_begin_revert(&sid)
                    .expect("revert during turn must proceed")
                    .expect("revert must acquire a lease");
                acquired_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                drop(lease);
            })
        };

        tokio::time::timeout(Duration::from_secs(1), cancel.cancelled())
            .await
            .expect("revert must signal cancellation");
        assert_eq!(mgr.session_status(&sid), Some(SessionStatus::Stopping));
        assert!(
            acquired_rx.try_recv().is_err(),
            "revert must not acquire before the old turn finishes"
        );

        assert!(mgr.finish_turn(&sid, "t-revert").is_some());
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("finish_turn must hand ownership to revert");
        assert!(mgr.is_session_busy_blocking(&sid));
        let sneak = mgr
            .reserve_turn(&sid, "sneak".into(), 5, "default", "/proj")
            .unwrap_err();
        assert!(matches!(sneak, LitecodeError::AgentAlreadyRunning));

        mgr.entry_revert_to_user_anchor(&sid, 1).unwrap();
        assert_eq!(mgr.data().transcript_blocking(&sid).unwrap().len(), 1);
        release_tx.send(()).unwrap();
        waiter.join().unwrap();
        assert!(!mgr.is_session_busy_blocking(&sid));
    }

    #[tokio::test]
    async fn turn_reservation_closes_pre_spawn_idle_window() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::clone(&guard),
            db.to_str().unwrap().to_string(),
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();

        mgr.reserve_turn(&sid, "reserved".into(), 5, "default", "/proj")
            .expect("reserve turn");
        assert!(mgr.is_turn_running_blocking(&sid));
        assert!(mgr.is_session_busy_blocking(&sid));
        assert!(guard.is_turn_in_progress());
        assert_eq!(
            mgr.get_cached_progress(&sid).map(|p| p.turn_id),
            Some("reserved".into())
        );
        assert!(matches!(
            mgr.try_begin_operation(&sid, SessionOperationKind::Compact),
            Err(LitecodeError::AgentAlreadyRunning)
        ));

        assert!(mgr.release_turn_reservation(&sid, "reserved"));
        assert!(!mgr.is_session_busy_blocking(&sid));
        assert!(!guard.is_turn_in_progress());
    }

    #[tokio::test]
    async fn start_turn_fanout_spawn_failure_rolls_back_running_and_turn_guard() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::clone(&guard),
            db.to_str().unwrap().to_string(),
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        let _ = mgr.attach(&sid);
        mgr.reserve_turn(&sid, "t-spawn-fail".into(), 5, "default", "/proj")
            .expect("reserve");
        assert!(guard.is_turn_in_progress());
        assert!(mgr.is_turn_running_blocking(&sid));

        mgr.inject_fanout_spawn_failure();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = crate::runtime::TurnHandle {
            handle: None,
            rx,
            cancel: CancellationToken::new(),
            turn_id: "t-spawn-fail".into(),
            step_max: 5,
        };
        let err = mgr
            .start_turn(&sid, handle, "default", "/proj", Arc::clone(&mgr))
            .await
            .expect_err("injected fanout spawn failure must surface");
        assert!(err.to_string().contains("fanout spawn failed"), "got {err}");
        drop(tx);

        assert!(
            !mgr.is_turn_running_blocking(&sid),
            "RunningTurn must roll back to Idle"
        );
        assert!(
            !guard.is_turn_in_progress(),
            "TurnGuard must end_turn on spawn failure"
        );
        // Session must accept a new reservation (not stuck AgentAlreadyRunning).
        mgr.reserve_turn(&sid, "t-after".into(), 5, "default", "/proj")
            .expect("re-reserve after spawn failure rollback");
        assert!(mgr.release_turn_reservation(&sid, "t-after"));
    }

    #[tokio::test]
    async fn delete_operation_is_blocked_by_active_descendant_turns() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        ));
        let parent = mgr.open_session("/proj", "default", None).await.unwrap();
        let child = mgr
            .open_child_session("/proj", "reviewer", None, &parent, "call_gate")
            .unwrap();

        mgr.reserve_turn(&child, "t-child-start".into(), 5, "reviewer", "/proj")
            .expect("reserve child turn");
        assert!(
            matches!(
                mgr.try_begin_operation(&parent, SessionOperationKind::Delete),
                Err(LitecodeError::AgentAlreadyRunning)
            ),
            "starting descendant must fence parent delete"
        );
        assert!(mgr.release_turn_reservation(&child, "t-child-start"));

        let cancel = CancellationToken::new();
        mgr.begin_turn(&child, "t-child-run".into(), cancel, 5, "reviewer", "/proj")
            .expect("begin child turn");
        assert!(
            matches!(
                mgr.try_begin_operation(&parent, SessionOperationKind::Delete),
                Err(LitecodeError::AgentAlreadyRunning)
            ),
            "running descendant must fence parent delete"
        );
        assert!(mgr.finish_turn(&child, "t-child-run").is_some());

        let lease = mgr
            .try_begin_operation(&parent, SessionOperationKind::Delete)
            .expect("idle descendant must permit parent delete");
        drop(lease);
    }

}

#[cfg(test)]
mod plan_settle_tests {
    
    
    
    
    
    

}

#[cfg(test)]
mod pending_message_tests {
    use super::*;
    use crate::config::TurnGuard;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn mgr() -> Arc<SessionManager> {
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ))
    }

    #[tokio::test]
    async fn enqueue_remove_and_claim_never_touch_the_log() {
        let mgr = mgr();
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        let revision_before = mgr.data().revision_blocking(&sid).unwrap();
        let events_before = mgr.data().events_blocking(&sid).unwrap().len();

        let first = mgr.enqueue_pending_message(&sid, "first").unwrap();
        mgr.enqueue_pending_message(&sid, "second").unwrap();
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 2);
        assert!(mgr.has_pending_messages(&sid));

        assert!(mgr.remove_pending_message(&sid, &first.id));
        assert!(!mgr.remove_pending_message(&sid, &first.id));
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 1);

        // Memory-only by contract: nothing here is a log write.
        assert_eq!(mgr.data().revision_blocking(&sid).unwrap(), revision_before);
        assert_eq!(
            mgr.data().events_blocking(&sid).unwrap().len(),
            events_before
        );
    }

    #[tokio::test]
    async fn claim_only_for_the_running_turn_and_restore_prepends() {
        let mgr = mgr();
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.enqueue_pending_message(&sid, "a").unwrap();
        mgr.enqueue_pending_message(&sid, "b").unwrap();

        // Idle and wrong-turn-id claims are refused without touching the queue.
        assert!(mgr.claim_pending_messages_for_turn(&sid, "t1").is_none());
        let cancel = CancellationToken::new();
        mgr.begin_turn(&sid, "t1".into(), cancel.clone(), 5, "default", "/proj")
            .unwrap();
        assert!(mgr.claim_pending_messages_for_turn(&sid, "t2").is_none());
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 2);

        let claimed = mgr.claim_pending_messages_for_turn(&sid, "t1").unwrap();
        assert_eq!(claimed.len(), 2);
        assert!(!mgr.has_pending_messages(&sid));
        assert!(
            mgr.claim_pending_messages_for_turn(&sid, "t1")
                .unwrap()
                .is_empty()
        );

        // A message queued after the claim survives a restore and lands after
        // the claimed block.
        mgr.enqueue_pending_message(&sid, "c").unwrap();
        mgr.restore_pending_messages(&sid, claimed);
        let restored: Vec<String> = mgr
            .pending_messages_snapshot(&sid)
            .into_iter()
            .map(|message| message.text)
            .collect();
        assert_eq!(restored, vec!["a", "b", "c"]);

        // Cancel marks the turn stopping: it may no longer claim the queue.
        assert!(
            mgr.cancel_turn_sync(&sid, crate::types::StopOrigin::User)
                .cancelled()
        );
        assert!(mgr.claim_pending_messages_for_turn(&sid, "t1").is_none());
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 3);
    }

    #[tokio::test]
    async fn a_reverted_log_drops_the_queue_it_made_meaningless() {
        let mgr = mgr();
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.insert_detail_rows(
            &sid,
            &[crate::types::user_text("u0"), crate::types::user_text("u1")],
        )
        .unwrap();
        mgr.enqueue_pending_message(&sid, "discarded").unwrap();

        // Only the revert that owns the session may drop the queue: another
        // exclusive operation blocks it and a foreign caller is refused.
        let compact = mgr
            .try_begin_operation(&sid, SessionOperationKind::Compact)
            .expect("compact lease");
        let busy = match mgr.try_begin_revert(&sid) {
            Ok(_) => panic!("compact must block the revert"),
            Err(error) => error,
        };
        assert!(matches!(busy, LitecodeError::AgentAlreadyRunning));
        assert!(!mgr.discard_pending_messages_for_revert(&sid, "not-the-owner"));
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 1);
        drop(compact);

        // A revert that does own the session clears it, and the end-of-turn
        // flush then has nothing left to deliver.
        let lease = mgr
            .try_begin_revert(&sid)
            .expect("revert after compact releases the lease")
            .expect("revert acquires a lease");
        assert!(mgr.discard_pending_messages_for_revert(&sid, lease.operation_id()));
        assert!(!mgr.has_pending_messages(&sid));
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 0);
        // Idempotent, and still owned.
        assert!(mgr.discard_pending_messages_for_revert(&sid, lease.operation_id()));
        drop(lease);

        // The cleared queue is not re-created: an idle claim finds nothing.
        assert_eq!(mgr.claim_pending_if_idle(&sid).map(|c| c.len()), Some(0));
        assert!(
            mgr.reserve_turn_and_claim_pending(&sid, "t1".into(), 5, "default", "/proj")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_failed_revert_keeps_the_queue() {
        let mgr = mgr();
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.insert_detail_rows(&sid, &[crate::types::user_text("u0")])
            .unwrap();
        mgr.enqueue_pending_message(&sid, "still mine").unwrap();

        let lease = mgr
            .try_begin_revert(&sid)
            .expect("revert acquires a lease")
            .expect("lease");
        // A bad anchor truncates nothing, and the controller only clears on the
        // success branch — so the queue must survive the attempt.
        assert!(mgr.entry_revert_to_user_anchor(&sid, 99).is_err());
        assert!(mgr.has_pending_messages(&sid));
        drop(lease);
        assert_eq!(mgr.pending_messages_snapshot(&sid).len(), 1);
    }
}
