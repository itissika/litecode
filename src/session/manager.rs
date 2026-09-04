use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::config::ResolvedConfig;
use crate::config::TurnGuard;
use crate::runtime::RuntimeHandle;
use crate::runtime::TurnHandle;
use crate::runtime::observer::{InternalEnvelope, InternalEvent, TurnPhase};
use crate::session::data::command::{
    CommitKind, MutationId, ReadValue, SessionMutation, SessionRead,
};
use crate::session::data::{SessionData, SessionDataReader};
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::store::{Session, SessionApply, SessionContextMeter};
use crate::session::task_state::{TaskReminders, prune_stale_active_plan};
use crate::types::{LitecodeError, Result};

/// Live turn bookkeeping without wire types.
pub struct LiveTurnState {
    pub turn_id: String,
    pub cancel: CancellationToken,
    pub progress: TurnProgress,
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
    },
    RunningTurn(LiveTurnState),
    Exclusive {
        operation_id: String,
        kind: SessionOperationKind,
    },
}

impl SessionActivity {
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

/// Parent-turn scoped lease: at most one in-flight `subagent_launch` per parent.
pub const MAX_SUBAGENTS_PER_PARENT: u32 = 1;

pub struct SubagentSlotLease {
    manager: Arc<SessionManager>,
    parent_session_id: String,
}

impl Drop for SubagentSlotLease {
    fn drop(&mut self) {
        self.manager.release_subagent_slot(&self.parent_session_id);
    }
}

/// One session: exclusive activity + L2 subscription fanout + committed revision.
pub struct SessionRecord {
    pub revision: u64,
    pub task_state: TaskReminders,
    activity: SessionActivity,
    /// Always present so L2 can subscribe before a turn starts.
    event_tx: broadcast::Sender<InternalEnvelope>,
    /// Ring buffer for reconnect replay (cleared on next start_turn).
    event_buffer: VecDeque<InternalEnvelope>,
    subscriber_count: usize,
    /// Sticky agent selection �?isomorphic with `sessions.agent_id`.
    pub agent_id: String,
    /// Sticky model catalog id �?isomorphic with `sessions.model_id` (NULL = unset).
    pub model_id: Option<String>,
    /// Platform thinking tier �?isomorphic with `sessions.thinking_tier`.
    pub thinking_tier: crate::platform_knobs::ThinkingTier,
    /// Platform context mode �?isomorphic with `sessions.context_mode`.
    pub context_mode: crate::platform_knobs::ContextMode,
    /// Parent session when this is a subagent child; `None` for root sessions.
    pub parent_session_id: Option<String>,
    /// Parent `function_call.call_id` that launched this child.
    pub parent_call_id: Option<String>,
    pub project: Option<String>,
    last_permission_sink: Option<Arc<dyn crate::permission::PermissionSink>>,
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
            event_tx,
            event_buffer: VecDeque::new(),
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
        }
    }
}

/// Process-level manager: registry of sessions (durable + live). No wire types.
pub struct SessionManager {
    records: std::sync::Mutex<HashMap<String, SessionRecord>>,
    pub turn_guard: Arc<TurnGuard>,
    data: Arc<SessionData>,
    /// Keeps a test-created lease alive for the manager's writer lifetime.
    _test_lease: Option<crate::session::WorkspaceWriteLease>,
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,
    /// In-flight subagent launches keyed by parent session id.
    subagent_slots: std::sync::Mutex<HashMap<String, u32>>,
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
            turn_guard,
            data,
            _test_lease: test_lease,
            lifecycle_tx,
            subagent_slots: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn from_data(turn_guard: Arc<TurnGuard>, data: Arc<SessionData>) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(256);
        Self {
            records: std::sync::Mutex::new(HashMap::new()),
            turn_guard,
            data,
            _test_lease: None,
            lifecycle_tx,
            subagent_slots: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn data(&self) -> &Arc<SessionData> {
        &self.data
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

    /// Fail-closed capacity gate: one in-flight child per parent session.
    pub fn try_acquire_subagent_slot(
        self: &Arc<Self>,
        parent_session_id: &str,
    ) -> Result<SubagentSlotLease> {
        let mut slots = self
            .subagent_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = slots.entry(parent_session_id.to_string()).or_insert(0);
        if *n >= MAX_SUBAGENTS_PER_PARENT {
            return Err(LitecodeError::ToolExecution(format!(
                "subagent capacity exceeded for parent {parent_session_id}: \
                 at most {MAX_SUBAGENTS_PER_PARENT} in-flight launch"
            )));
        }
        *n += 1;
        Ok(SubagentSlotLease {
            manager: Arc::clone(self),
            parent_session_id: parent_session_id.to_string(),
        })
    }

    fn release_subagent_slot(&self, parent_session_id: &str) {
        let mut slots = self
            .subagent_slots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(n) = slots.get_mut(parent_session_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                slots.remove(parent_session_id);
            }
        }
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

    /// Workspace-list lifecycle: child turn noise must not reach the session list.
    /// `SessionRemoved` is always broadcast so clients can drop stale rows.
    fn emit_lifecycle(&self, event: LifecycleEvent) {
        let session_id = match &event {
            LifecycleEvent::SessionRemoved { session_id } => {
                let _ = self.lifecycle_tx.send(LifecycleEvent::SessionRemoved {
                    session_id: session_id.clone(),
                });
                return;
            }
            LifecycleEvent::TurnStarted { session_id, .. }
            | LifecycleEvent::TurnProgress { session_id, .. }
            | LifecycleEvent::TurnFinished { session_id, .. }
            | LifecycleEvent::SessionPreviewUpdated { session_id, .. }
            | LifecycleEvent::TurnStep { session_id, .. } => session_id.as_str(),
        };
        if self.is_child_session(session_id) {
            return;
        }
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
        })?;
        let sid = receipt.session_id.clone();
        let meta = self.data.meta_blocking(&sid)?;
        let mut task_state = TaskReminders {
            todos: meta
                .todos
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            active_plan: meta
                .plan_slug
                .as_ref()
                .map(|slug| crate::session::task_state::PlanRef::new(slug)),
        };
        if prune_stale_active_plan(&mut task_state) {
            let _ = self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: sid.clone(),
                expected_revision: receipt.revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            });
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, receipt.revision);
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
        })?;
        let sid = receipt.session_id.clone();
        let meta = self.data.meta_blocking(&sid)?;
        let mut task_state = TaskReminders::default();
        if prune_stale_active_plan(&mut task_state) {
            let _ = self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: sid.clone(),
                expected_revision: receipt.revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            });
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, receipt.revision);
        record.project = Some(project.to_string());
        self.records.lock().unwrap().insert(sid.clone(), record);
        Ok(sid)
    }

    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        if records.contains_key(session_id) {
            return Ok(());
        }
        let meta = self.data.meta_blocking(session_id)?;
        let revision = self.data.revision_blocking(session_id).unwrap_or(0);
        let mut task_state = TaskReminders {
            todos: meta
                .todos
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect(),
            active_plan: meta
                .plan_slug
                .as_ref()
                .map(|slug| crate::session::task_state::PlanRef::new(slug)),
        };
        if prune_stale_active_plan(&mut task_state) {
            let _ = self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: session_id.to_string(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            });
        }
        let mut record = SessionRecord::from_meta(&meta, task_state, revision);
        record.project = None;
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

    pub fn entry_revert_to_user_anchor(&self, session_id: &str, k: i64) -> Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::Apply {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            op: SessionApply::Truncate { user_k: k },
        })?;
        Ok(())
    }

    pub fn entry_user_detail_count(&self, session_id: &str) -> Result<i64> {
        match self.data.read_blocking(SessionRead::UserDetailBefore {
            session_id: session_id.to_string(),
            from_seq: i64::MAX,
        })? {
            ReadValue::Count(n) => Ok(n),
            _ => Err(LitecodeError::SessionStorage("unexpected count".into())),
        }
    }

    pub fn entry_user_detail_before_seq(
        &self,
        session_id: &str,
        from_seq: crate::session::event::Seq,
    ) -> Result<i64> {
        match self.data.read_blocking(SessionRead::UserDetailBefore {
            session_id: session_id.to_string(),
            from_seq: from_seq as i64,
        })? {
            ReadValue::Count(n) => Ok(n),
            _ => Err(LitecodeError::SessionStorage("unexpected count".into())),
        }
    }

    pub fn entry_snapshot_stem_for_user_k(&self, session_id: &str, k: i64) -> Result<i64> {
        match self.data.read_blocking(SessionRead::SnapshotStem {
            session_id: session_id.to_string(),
            k,
        })? {
            ReadValue::Count(n) => Ok(n.saturating_add(1)),
            _ => Err(LitecodeError::InvalidRevertAnchor(format!("k={k}"))),
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
        record.activity = SessionActivity::StartingTurn {
            turn_id,
            progress: progress.clone(),
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
        }
        released
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

    /// Finish a turn (Running �?Idle). Sole path that clears running; emits lifecycle TurnFinished.
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
            let mut progress = live.progress;
            progress.awaiting_permission = false;
            progress
        };
        self.turn_guard.end_turn();
        self.emit_lifecycle(LifecycleEvent::TurnFinished {
            session_id: session_id.to_string(),
            progress: progress.clone(),
        });
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
        let (event_tx, progress) = {
            let mut records = self.records.lock().unwrap();
            let record = records.get_mut(session_id).ok_or_else(|| {
                LitecodeError::ToolExecution(format!("session {session_id} not found"))
            })?;
            let activity = std::mem::replace(&mut record.activity, SessionActivity::Idle);
            let (reserved_turn_id, progress) = match activity {
                SessionActivity::StartingTurn { turn_id, progress } => (turn_id, progress),
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
            });
            (record.event_tx.clone(), progress)
        };

        self.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: session_id.to_string(),
            progress,
        });

        // Fanout must outlive the caller's runtime. Idle bash auto-turn starts
        // the turn on a throwaway current-thread runtime that exits as soon as
        // `start_turn` returns; `tokio::spawn` on that runtime would abort
        // fanout and the agent panel would never see `agent/turn_started` /
        // `buffer/item` even though session-list lifecycle already flipped to
        // running.
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
                LitecodeError::ToolExecution(format!("turn fanout spawn failed: {error}"))
            })?;
        Ok(())
    }

    pub async fn cancel_turn(&self, session_id: &str) {
        self.cancel_turn_sync(session_id);
    }

    /// Cancel a live `RunningTurn` without taking the exclusive lease.
    /// Returns true when a cancel token was signalled.
    pub fn cancel_turn_sync(&self, session_id: &str) -> bool {
        let records = self.records.lock().unwrap();
        if let Some(record) = records.get(session_id)
            && let SessionActivity::RunningTurn(live) = &record.activity
        {
            live.cancel.cancel();
            return true;
        }
        false
    }

    /// Exclusive compact (or other exclusive ops) is the only busy reject.
    /// Idle takes a lease; a running turn is cancelled and truncate proceeds
    /// without one; StartingTurn becomes Exclusive Revert under the same lock.
    pub fn try_begin_revert(
        self: &Arc<Self>,
        session_id: &str,
    ) -> Result<Option<SessionOperationLease>> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        match &record.activity {
            SessionActivity::Idle => {}
            SessionActivity::RunningTurn(live) => {
                live.cancel.cancel();
                return Ok(None);
            }
            SessionActivity::StartingTurn { .. } => {
                let operation_id = uuid::Uuid::new_v4().to_string();
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
        drop(records);
        self.try_begin_operation(session_id, SessionOperationKind::Revert)
            .map(Some)
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
        }
    }

    pub async fn is_session_empty(&self, session_id: &str) -> bool {
        {
            let records = self.records.lock().unwrap();
            if let Some(record) = records.get(session_id) {
                if record.activity.is_busy() {
                    return false;
                }
                return self
                    .data
                    .working_set_blocking(session_id)
                    .map(|rows| rows.is_empty())
                    .unwrap_or(true);
            }
        }
        self.data
            .working_set_blocking(session_id)
            .map(|rows| rows.is_empty())
            .unwrap_or(true)
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
            self.cancel_turn(id).await;
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
                || !self.is_session_empty(&session_id).await
            {
                continue;
            }
            let _ = self.remove_session(&session_id);
        }
    }

    pub fn remove_session(&self, session_id: &str) -> Result<()> {
        // Cancel live turns on this session and any in-memory children first.
        let child_ids = match self.data.read_blocking(SessionRead::ListChildIds {
            parent_session_id: session_id.to_string(),
        }) {
            Ok(ReadValue::Ids(ids)) => ids,
            _ => Vec::new(),
        };
        {
            let records = self.records.lock().unwrap();
            if let Some(record) = records.get(session_id)
                && let SessionActivity::RunningTurn(live) = &record.activity
            {
                live.cancel.cancel();
            }
            for child_id in &child_ids {
                if let Some(record) = records.get(child_id)
                    && let SessionActivity::RunningTurn(live) = &record.activity
                {
                    live.cancel.cancel();
                }
            }
        }

        // Durable cascade (children first) via SessionData delete.
        let expected = self.expected_revision(session_id);
        match self.mutate_blocking(SessionMutation::Delete {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
        }) {
            Ok(_) => {
                {
                    let mut records = self.records.lock().unwrap();
                    for child_id in &child_ids {
                        records.remove(child_id);
                    }
                    records.remove(session_id);
                }
                for child_id in &child_ids {
                    self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                        session_id: child_id.clone(),
                    });
                }
                self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                    session_id: session_id.to_string(),
                });
                Ok(())
            }
            Err(LitecodeError::SessionNotFound(_)) => {
                // Idempotent: already gone from durable store.
                {
                    let mut records = self.records.lock().unwrap();
                    for child_id in &child_ids {
                        records.remove(child_id);
                    }
                    records.remove(session_id);
                }
                for child_id in &child_ids {
                    self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                        session_id: child_id.clone(),
                    });
                }
                self.emit_lifecycle(LifecycleEvent::SessionRemoved {
                    session_id: session_id.to_string(),
                });
                Ok(())
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
        record.event_buffer.push_back(envelope.clone());
        let _ = record.event_tx.send(envelope);
        true
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

    /// Snapshot of the replay buffer (bounded at EVENT_BUFFER_CAPACITY). Each
    /// subscriber gets the same recent events �?draining on first consume would
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
        let normalized = model_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::SetModel {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            model_id: normalized.clone(),
        })?;
        if let Some(record) = self.records.lock().unwrap().get_mut(session_id) {
            record.model_id = normalized;
        }
        Ok(())
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

    /// After catalog replace: clear sticky `model_id` when it no longer exists.
    /// Updates DB for all sessions and in-memory records for loaded ones.
    /// Does not touch `agent_id`. Returns cleared session ids.
    pub fn clear_orphaned_model_ids(
        &self,
        valid_model_ids: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>> {
        let receipt = self.mutate_blocking(SessionMutation::ClearOrphanedModelIds {
            operation_id: MutationId::new(),
            valid_ids: valid_model_ids.iter().cloned().collect(),
        })?;
        let _ = receipt;
        let mut records = self.records.lock().unwrap();
        let mut cleared = Vec::new();
        for (id, record) in records.iter_mut() {
            if let Some(mid) = record.model_id.as_ref()
                && !valid_model_ids.contains(mid)
            {
                record.model_id = None;
                cleared.push(id.clone());
            }
        }
        Ok(cleared)
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
        let revision = self.data.revision_blocking(session_id).unwrap_or(0);
        if prune_stale_active_plan(&mut task_state) {
            let _ = self.mutate_blocking(SessionMutation::SaveTaskState {
                session_id: session_id.to_string(),
                expected_revision: revision,
                operation_id: MutationId::new(),
                state: task_state.clone(),
            });
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

    pub fn append_job_exit(
        &self,
        session_id: &str,
        item: &crate::types::Item,
    ) -> anyhow::Result<()> {
        let expected = self.expected_revision(session_id);
        self.mutate_blocking(SessionMutation::AppendJobExit {
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
    ) -> anyhow::Result<(
        crate::session::data::command::CommitKind,
        Vec<crate::session::working::WorkingRow>,
        Option<(String, i64)>,
    )> {
        let expected = self.expected_revision(session_id);
        let receipt = self.mutate_blocking(SessionMutation::CommitTurnDelta {
            session_id: session_id.to_string(),
            expected_revision: expected,
            operation_id: MutationId::new(),
            rows,
            expected_max_seq,
            turn_id: turn_id.to_string(),
        })?;
        let working = receipt.working_set.ok_or_else(|| {
            anyhow::anyhow!("CommitTurnDelta receipt missing writer working set")
        })?;
        Ok((receipt.outcome, working, receipt.preview))
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

    loop {
        match handle.rx.recv().await {
            Some(envelope) => {
                let is_completed = matches!(&envelope.event, InternalEvent::TurnCompleted { .. });
                if is_completed {
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
                        updated_at,
                    } => {
                        manager.emit_lifecycle(LifecycleEvent::SessionPreviewUpdated {
                            session_id: session_id.clone(),
                            preview: preview.clone(),
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

    // Safety net if the turn thread died without TurnCompleted.
    let _ = manager.finish_turn(&session_id, &handle.turn_id);
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
mod turn_step_tests {
    use super::turn_step_from_stream;
    use crate::authority::responses::{
        FunctionToolCall, OutputItem, ReasoningItem, ResponseFunctionCallArgumentsDeltaEvent,
        ResponseOutputItemAddedEvent, ResponseStreamEvent, ResponseTextDeltaEvent,
    };
    use crate::session::live::TurnStepKind;

    #[test]
    fn maps_added_and_dedupe_keys() {
        let added = ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
            sequence_number: 1,
            output_index: 0,
            item: OutputItem::FunctionCall(FunctionToolCall {
                arguments: "{}".into(),
                call_id: "call_1".into(),
                name: "bash".into(),
                namespace: None,
                id: Some("fc_1".into()),
                status: None,
            }),
        });
        let (id, kind) = turn_step_from_stream(&added).unwrap();
        assert_eq!(id, "fc_1");
        assert_eq!(kind, TurnStepKind::Toolcall);

        let delta = ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
            ResponseFunctionCallArgumentsDeltaEvent {
                sequence_number: 2,
                item_id: "fc_1".into(),
                output_index: 0,
                delta: "x".into(),
            },
        );
        let (id2, kind2) = turn_step_from_stream(&delta).unwrap();
        assert_eq!(id2, "fc_1");
        assert_eq!(kind2, TurnStepKind::Toolcall);
    }

    #[test]
    fn maps_text_delta_and_reasoning_added() {
        let text = ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
            sequence_number: 1,
            item_id: "msg_1".into(),
            output_index: 0,
            content_index: 0,
            delta: "hi".into(),
            logprobs: None,
        });
        assert_eq!(
            turn_step_from_stream(&text).unwrap(),
            ("msg_1".into(), TurnStepKind::Text)
        );

        let reasoning =
            ResponseStreamEvent::ResponseOutputItemAdded(ResponseOutputItemAddedEvent {
                sequence_number: 2,
                output_index: 1,
                item: OutputItem::Reasoning(ReasoningItem {
                    id: Some("rs_1".into()),
                    summary: vec![],
                    content: None,
                    encrypted_content: None,
                    status: None,
                }),
            });
        assert_eq!(
            turn_step_from_stream(&reasoning).unwrap(),
            ("rs_1".into(), TurnStepKind::Reasoning)
        );
    }
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

    #[tokio::test]
    async fn open_child_and_remove_parent_cascades_in_memory() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap().to_string();
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db_path,
        ));

        let parent_id = mgr
            .open_session("/proj", "default", None)
            .await
            .expect("parent");
        let child_id = mgr
            .open_child_session("/proj", "reviewer", None, &parent_id, "call_xyz")
            .expect("child");

        assert!(mgr.records.lock().unwrap().contains_key(&child_id));
        let resumed = mgr.data().meta_blocking(&child_id).unwrap();
        assert_eq!(
            resumed.parent_session_id.as_deref(),
            Some(parent_id.as_str())
        );
        assert_eq!(resumed.parent_call_id.as_deref(), Some("call_xyz"));

        mgr.remove_session(&parent_id).expect("remove parent");
        assert!(!mgr.records.lock().unwrap().contains_key(&parent_id));
        assert!(!mgr.records.lock().unwrap().contains_key(&child_id));
        assert!(mgr.data().meta_blocking(&child_id).is_err());
    }

    #[tokio::test]
    async fn child_turn_lifecycle_is_filtered() {
        use crate::runtime::observer::TurnPhase;
        use crate::session::live::TurnProgress;

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap().to_string();
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db_path,
        ));

        let parent_id = mgr.open_session("/proj", "default", None).await.unwrap();
        let child_id = mgr
            .open_child_session("/proj", "reviewer", None, &parent_id, "call_life")
            .unwrap();

        let mut rx = mgr.subscribe_lifecycle();
        // Drain open noise.
        while rx.try_recv().is_ok() {}

        mgr.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: child_id.clone(),
            progress: TurnProgress {
                turn_id: "t1".into(),
                phase: TurnPhase::Starting,
                step: 1,
                step_max: 5,
                started_at_ms: 1,
                awaiting_permission: false,
            },
        });
        mgr.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: parent_id.clone(),
            progress: TurnProgress {
                turn_id: "t2".into(),
                phase: TurnPhase::Starting,
                step: 1,
                step_max: 5,
                started_at_ms: 2,
                awaiting_permission: false,
            },
        });

        let mut got_parent = false;
        let mut got_child = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                LifecycleEvent::TurnStarted { session_id, .. } if session_id == parent_id => {
                    got_parent = true;
                }
                LifecycleEvent::TurnStarted { session_id, .. } if session_id == child_id => {
                    got_child = true;
                }
                _ => {}
            }
        }
        assert!(got_parent, "parent turn_started must broadcast");
        assert!(!got_child, "child turn_started must be filtered");
    }

    #[test]
    fn channel_observer_never_tags_parent_session_id() {
        use crate::runtime::observer::{
            ChannelObserver, InternalEvent, RuntimeObserver, TurnPhase,
        };
        use tokio::sync::mpsc;

        let (tx, mut rx) = mpsc::unbounded_channel();
        let observer = ChannelObserver::new(tx);
        observer.on_internal(InternalEvent::PhaseChanged {
            phase: TurnPhase::Starting,
            step: 1,
        });
        let env = rx.try_recv().expect("event");
        assert!(env.parent_session_id.is_none());
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
    async fn revert_during_running_turn_cancels_without_exclusive_lease() {
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

        let lease = mgr
            .try_begin_revert(&sid)
            .expect("revert during turn must proceed");
        assert!(
            lease.is_none(),
            "running turn keeps activity; revert must not take Exclusive"
        );
        assert!(
            cancel.is_cancelled(),
            "revert must signal the turn cancel token"
        );
        assert!(mgr.is_turn_running_blocking(&sid));
        assert_eq!(mgr.entry_user_detail_count(&sid).unwrap(), 3);
        assert_eq!(mgr.entry_snapshot_stem_for_user_k(&sid, 0).unwrap(), 1);

        mgr.entry_revert_to_user_anchor(&sid, 1).unwrap();
        let len = mgr.data().transcript_blocking(&sid).unwrap().len();
        assert_eq!(len, 1, "running-turn revert must truncate the log");
    }

    #[tokio::test]
    async fn revert_during_starting_turn_releases_reservation() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        ));
        let sid = mgr.open_session("/proj", "default", None).await.unwrap();
        mgr.insert_detail_rows(
            &sid,
            &[crate::types::user_text("u0"), crate::types::user_text("u1")],
        )
        .unwrap();
        mgr.reserve_turn(&sid, "reserved".into(), 5, "default", "/proj")
            .expect("reserve turn");

        let lease = mgr
            .try_begin_revert(&sid)
            .expect("starting turn must not block revert");
        assert!(
            lease.is_some(),
            "starting turn takes exclusive revert lease"
        );
        assert!(!mgr.is_turn_running_blocking(&sid));
        assert!(mgr.is_session_busy_blocking(&sid));
        let sneak = mgr
            .reserve_turn(&sid, "sneak".into(), 5, "default", "/proj")
            .unwrap_err();
        assert!(matches!(sneak, LitecodeError::AgentAlreadyRunning));

        mgr.entry_revert_to_user_anchor(&sid, 1).unwrap();
        let len = mgr.data().transcript_blocking(&sid).unwrap().len();
        assert_eq!(len, 1);
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

    #[test]
    fn subagent_slot_is_one_per_parent_fail_closed() {
        let mgr = Arc::new(SessionManager::ephemeral_registry());
        let first = mgr
            .try_acquire_subagent_slot("parent-1")
            .expect("first slot");
        let second = mgr.try_acquire_subagent_slot("parent-1");
        assert!(
            second.is_err(),
            "second in-flight launch for the same parent must fail-closed"
        );
        let other = mgr
            .try_acquire_subagent_slot("parent-2")
            .expect("other parent is independent");
        drop(first);
        let retry = mgr
            .try_acquire_subagent_slot("parent-1")
            .expect("slot frees on drop");
        drop(retry);
        drop(other);
    }

    #[test]
    fn start_turn_fanout_survives_caller_runtime_drop() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let mgr = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            db.to_str().unwrap().to_string(),
        ));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let sid = rt
            .block_on(mgr.open_session("/proj", "default", None))
            .unwrap();
        let _ = mgr.attach(&sid);
        let mut sub = mgr.subscribe(&sid).expect("subscribe");
        mgr.reserve_turn(&sid, "t-idle".into(), 5, "default", "/proj")
            .unwrap();

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let handle = crate::runtime::TurnHandle {
            handle: None,
            rx,
            cancel: CancellationToken::new(),
            turn_id: "t-idle".into(),
            step_max: 5,
        };
        let mgr_start = Arc::clone(&mgr);
        let sid_start = sid.clone();
        std::thread::spawn(move || {
            let nested = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            nested
                .block_on(mgr_start.start_turn(
                    &sid_start,
                    handle,
                    "default",
                    "/proj",
                    Arc::clone(&mgr_start),
                ))
                .expect("start_turn");
        })
        .join()
        .expect("starter thread");

        tx.send(InternalEnvelope {
            event: InternalEvent::TurnStarted {
                turn_id: "t-idle".into(),
                input: "from-mailbox".into(),
                step_max: 5,
            },
            parent_session_id: None,
        })
        .expect("fanout still holding the turn event channel");

        let got = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), sub.recv())
                .await
                .expect("timed out waiting for fanout")
                .expect("broadcast lagged")
        });
        drop(tx);
        match got.event {
            InternalEvent::TurnStarted { input, .. } => {
                assert_eq!(input, "from-mailbox");
            }
            other => panic!("expected TurnStarted, got {other:?}"),
        }
    }
}
