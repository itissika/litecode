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
use crate::session::gate::SessionGate;
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::store::Session;
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

/// One session: durable store + exclusive activity + L2 subscription fanout.
pub struct SessionRecord {
    pub store: Arc<SessionGate>,
    pub task_state: TaskReminders,
    activity: SessionActivity,
    /// Always present so L2 can subscribe before a turn starts.
    event_tx: broadcast::Sender<InternalEnvelope>,
    /// Ring buffer for reconnect replay (cleared on next start_turn).
    event_buffer: VecDeque<InternalEnvelope>,
    subscriber_count: usize,
    /// Sticky agent selection — isomorphic with `sessions.agent_id`.
    pub agent_id: String,
    /// Sticky model catalog id — isomorphic with `sessions.model_id` (NULL = unset).
    pub model_id: Option<String>,
    /// Platform thinking tier — isomorphic with `sessions.thinking_tier`.
    pub thinking_tier: crate::platform_knobs::ThinkingTier,
    /// Platform context mode — isomorphic with `sessions.context_mode`.
    pub context_mode: crate::platform_knobs::ContextMode,
    /// Parent session when this is a subagent child; `None` for root sessions.
    pub parent_session_id: Option<String>,
    /// Parent `function_call.call_id` that launched this child.
    pub parent_call_id: Option<String>,
    pub project: Option<String>,
    last_permission_sink: Option<Arc<dyn crate::permission::PermissionSink>>,
}

impl SessionRecord {
    fn new(store: Arc<SessionGate>, task_state: TaskReminders) -> Self {
        let (agent_id, model_id, thinking_tier, context_mode, parent_session_id, parent_call_id) =
            store.with(|s| {
                (
                    s.agent_id.clone(),
                    s.model_id.clone(),
                    s.thinking_tier,
                    s.context_mode,
                    s.parent_session_id.clone(),
                    s.parent_call_id.clone(),
                )
            });
        let (event_tx, _) = broadcast::channel(256);
        Self {
            store,
            task_state,
            activity: SessionActivity::Idle,
            event_tx,
            event_buffer: VecDeque::new(),
            subscriber_count: 0,
            agent_id,
            model_id,
            thinking_tier,
            context_mode,
            parent_session_id,
            parent_call_id,
            project: None,
            last_permission_sink: None,
        }
    }
}

/// Process-level manager: registry of sessions (durable + live). No wire types.
pub struct SessionManager {
    records: std::sync::Mutex<HashMap<String, SessionRecord>>,
    pub turn_guard: Arc<TurnGuard>,
    /// Workspace sessions DB path (fixed for the lifetime of this process).
    db_path: std::sync::Mutex<String>,
    lifecycle_tx: broadcast::Sender<LifecycleEvent>,
    /// In-flight subagent launches keyed by parent session id.
    subagent_slots: std::sync::Mutex<HashMap<String, u32>>,
}

const EVENT_BUFFER_CAPACITY: usize = 1024;
pub const EMPTY_SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

impl SessionManager {
    pub fn new(turn_guard: Arc<TurnGuard>, db_path: String) -> Self {
        let (lifecycle_tx, _) = broadcast::channel(256);
        Self {
            records: std::sync::Mutex::new(HashMap::new()),
            turn_guard,
            db_path: std::sync::Mutex::new(db_path),
            lifecycle_tx,
            subagent_slots: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Current workspace sessions DB path.
    pub fn db_path(&self) -> String {
        self.db_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
        Self::new(Arc::new(TurnGuard::new()), String::new())
    }

    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    /// Whether `session_id` is a persisted subagent child (has a parent).
    pub fn is_child_session(&self, session_id: &str) -> bool {
        if let Some(record) = self.records.lock().unwrap().get(session_id) {
            return record.parent_session_id.is_some();
        }
        let db = self.db_path();
        Session::resume(&db, session_id)
            .map(|s| s.parent_session_id.is_some())
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
        let store = Session::open(&self.db_path(), project, agent_id, model_id)
            .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        let sid = store.id.clone();
        let mut task_state = store.load_task_state().unwrap_or_default();
        if prune_stale_active_plan(&mut task_state) {
            let _ = store.save_task_state(&task_state);
        }
        let mut record = SessionRecord::new(Arc::new(SessionGate::new(store)), task_state);
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
        if self.db_path().is_empty() {
            return Err(LitecodeError::ToolExecution(
                "open_child_session requires a workspace SessionManager".into(),
            ));
        }
        let db = self.db_path();
        let store = Session::open_with_parent(
            &db,
            project,
            agent_id,
            model_id,
            Some(parent_session_id),
            Some(parent_call_id),
        )
        .map_err(|e| LitecodeError::ToolExecution(e.to_string()))?;
        let sid = store.id.clone();
        let mut task_state = store.load_task_state().unwrap_or_default();
        if prune_stale_active_plan(&mut task_state) {
            let _ = store.save_task_state(&task_state);
        }
        let mut record = SessionRecord::new(Arc::new(SessionGate::new(store)), task_state);
        record.project = Some(project.to_string());
        self.records.lock().unwrap().insert(sid.clone(), record);
        Ok(sid)
    }

    pub async fn resume_session(&self, session_id: &str) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        if records.contains_key(session_id) {
            return Ok(());
        }
        let store = Session::resume(&self.db_path(), session_id)?;
        let mut task_state = store.load_task_state().unwrap_or_default();
        if prune_stale_active_plan(&mut task_state) {
            let _ = store.save_task_state(&task_state);
        }
        let mut record = SessionRecord::new(Arc::new(SessionGate::new(store)), task_state);
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
        self.records
            .lock()
            .unwrap()
            .get(session_id)
            .map(|e| e.store.with(|s| s.buffer_len()))
            .unwrap_or(0)
    }

    pub fn entry_buffer_len_blocking(&self, session_id: &str) -> usize {
        self.entry_buffer_len(session_id)
    }

    pub fn entry_load_range(
        &self,
        session_id: &str,
        start: usize,
        end: usize,
    ) -> Result<Vec<crate::types::Item>> {
        let records = self.records.lock().unwrap();
        let entry = records.get(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        entry.store.with(|s| s.load_by_buffer_index(start, end))
    }

    /// [`Self::entry_load_range`] plus each row's DB `kind` (REV-11 wire).
    pub fn entry_load_range_with_kinds(
        &self,
        session_id: &str,
        start: usize,
        end: usize,
    ) -> Result<(Vec<crate::types::Item>, Vec<String>)> {
        let records = self.records.lock().unwrap();
        let entry = records.get(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        entry
            .store
            .with(|s| s.load_by_buffer_index_with_kinds(start, end))
    }

    pub fn entry_user_detail_before_buffer_index(
        &self,
        session_id: &str,
        start: usize,
    ) -> Result<usize> {
        let records = self.records.lock().unwrap();
        let entry = records.get(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        entry
            .store
            .with(|s| s.user_detail_before_buffer_index(start))
    }

    pub fn entry_revert_to_user_anchor(&self, session_id: &str, k: i64) -> Result<()> {
        let records = self.records.lock().unwrap();
        let entry = records.get(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        entry.store.with(|s| s.revert_to_user_anchor(k))
    }

    pub fn entry_user_detail_count(&self, session_id: &str) -> Result<i64> {
        let records = self.records.lock().unwrap();
        let entry = records.get(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        entry.store.with(|s| s.user_detail_count())
    }

    /// Test probe (2.15 REV-5): true when the records lock is currently FREE.
    /// Lets a test assert that `with_entry_store` runs its closure outside the
    /// records lock (the lock may only be held by this thread mid-closure).
    #[doc(hidden)]
    pub fn records_lock_free(&self) -> bool {
        self.records.try_lock().is_ok()
    }

    pub fn with_entry_store<F, R>(&self, session_id: &str, f: F) -> anyhow::Result<R>
    where
        F: FnOnce(&Session) -> anyhow::Result<R>,
    {
        // REV-5: only clone the Arc<SessionGate> while holding the records lock,
        // then run the (potentially SQLite-I/O-bound) closure OUTSIDE the lock —
        // no DB I/O happens while the records lock is held.
        let gate = {
            let records = self
                .records
                .lock()
                .map_err(|e| anyhow::anyhow!("records lock poisoned: {}", e))?;
            records
                .get(session_id)
                .map(|e| Arc::clone(&e.store))
                .ok_or_else(|| anyhow::anyhow!("session entry not found: {}", session_id))?
        };
        gate.with(f)
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

    pub fn save_task_state(&self, session_id: &str) -> anyhow::Result<()> {
        let records = self.records.lock().unwrap();
        let entry = records
            .get(session_id)
            .ok_or_else(|| anyhow::anyhow!("session entry not found: {}", session_id))?;
        let state = entry.task_state.clone();
        entry.store.with(|s| s.save_task_state(&state))?;
        Ok(())
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

    /// Finish a turn (Running → Idle). Sole path that clears running; emits lifecycle TurnFinished.
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
                cancel,
                progress: progress.clone(),
            });
            (record.event_tx.clone(), progress)
        };

        self.emit_lifecycle(LifecycleEvent::TurnStarted {
            session_id: session_id.to_string(),
            progress,
        });

        let session_id_owned = session_id.to_string();
        tokio::spawn(async move {
            fanout_turn(session_id_owned, handle, event_tx, manager).await;
        });
        Ok(())
    }

    pub async fn cancel_turn(&self, session_id: &str) {
        let records = self.records.lock().unwrap();
        if let Some(record) = records.get(session_id) {
            if let SessionActivity::RunningTurn(live) = &record.activity {
                live.cancel.cancel();
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
                return record.store.with(|s| s.buffer_len()) == 0;
            }
        }
        match Session::resume(&self.db_path(), session_id) {
            Ok(session) => session.buffer_len() == 0,
            Err(_) => true,
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
            self.cancel_turn(id).await;
        }
    }

    /// Remove only stale, empty durable sessions. Closing a panel merely
    /// releases its subscription; it must never make a valid session ID stale.
    pub async fn gc_stale_empty_sessions(&self, max_age: Duration) {
        let cutoff = chrono::Utc::now().timestamp_millis()
            - i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
        let rows = match Session::list_sessions_for_gc(&self.db_path()) {
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
        let child_ids =
            Session::list_child_session_ids(&self.db_path(), session_id).unwrap_or_default();
        {
            let records = self.records.lock().unwrap();
            if let Some(record) = records.get(session_id) {
                if let SessionActivity::RunningTurn(live) = &record.activity {
                    live.cancel.cancel();
                }
            }
            for child_id in &child_ids {
                if let Some(record) = records.get(child_id) {
                    if let SessionActivity::RunningTurn(live) = &record.activity {
                        live.cancel.cancel();
                    }
                }
            }
        }

        // Durable cascade (children first) via Session::delete.
        match Session::delete(&self.db_path(), session_id) {
            Ok(()) => {
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
            for record in records.values() {
                if record.parent_session_id.as_deref() == Some(parent_session_id)
                    && record.parent_call_id.as_deref() == Some(parent_call_id)
                {
                    return record.store.with(|s| Some(s.id.clone()));
                }
            }
        }
        Session::child_session_id_for_call(&self.db_path(), parent_session_id, parent_call_id)
            .ok()
            .flatten()
    }

    pub fn child_bindings_for_parent(
        &self,
        parent_session_id: &str,
    ) -> std::collections::HashMap<String, String> {
        Session::child_bindings_for_parent(&self.db_path(), parent_session_id).unwrap_or_default()
    }

    /// Find buffer index + item for a function_call `call_id` in a session transcript.
    pub fn find_function_call_buffer_item(
        &self,
        session_id: &str,
        call_id: &str,
    ) -> Option<(usize, crate::types::Item)> {
        let records = self.records.lock().unwrap();
        let record = records.get(session_id)?;
        record.store.with(|s| {
            let len = s.buffer_len();
            let items = s.load_by_buffer_index(0, len).ok()?;
            for (idx, item) in items.into_iter().enumerate() {
                if let crate::types::Item::FunctionCall(ref fc) = item {
                    if fc.call_id == call_id {
                        return Some((idx, item));
                    }
                }
            }
            None
        })
    }

    /// Snapshot of the replay buffer (bounded at EVENT_BUFFER_CAPACITY). Each
    /// subscriber gets the same recent events — draining on first consume would
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
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        let normalized = model_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        record
            .store
            .with_mut(|s| s.set_model_id(normalized.as_deref()))?;
        record.model_id = normalized;
        Ok(())
    }

    pub fn session_model_id(&self, session_id: &str) -> Option<String> {
        let records = self.records.lock().unwrap();
        records.get(session_id).and_then(|r| r.model_id.clone())
    }

    /// Persist sticky session agent id. Does not touch `model_id`.
    pub fn set_agent_id(&self, session_id: &str, agent_id: String) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        record.store.with_mut(|s| s.set_agent_id(&agent_id))?;
        record.agent_id = agent_id;
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
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        record.store.with_mut(|s| s.set_thinking_tier(tier))?;
        record.thinking_tier = tier;
        Ok(())
    }

    pub fn set_context_mode(
        &self,
        session_id: &str,
        mode: crate::platform_knobs::ContextMode,
    ) -> Result<()> {
        let mut records = self.records.lock().unwrap();
        let record = records.get_mut(session_id).ok_or_else(|| {
            LitecodeError::ToolExecution(format!("session {session_id} not found"))
        })?;
        record.store.with_mut(|s| s.set_context_mode(mode))?;
        record.context_mode = mode;
        Ok(())
    }

    /// After catalog replace: clear sticky `model_id` when it no longer exists.
    /// Updates DB for all sessions and in-memory records for loaded ones.
    /// Does not touch `agent_id`. Returns cleared session ids.
    pub fn clear_orphaned_model_ids(
        &self,
        valid_model_ids: &std::collections::HashSet<String>,
    ) -> Result<Vec<String>> {
        let cleared = Session::clear_orphaned_model_ids(&self.db_path(), valid_model_ids)?;
        if cleared.is_empty() {
            return Ok(cleared);
        }
        let cleared_set: std::collections::HashSet<&str> =
            cleared.iter().map(String::as_str).collect();
        let mut records = self.records.lock().unwrap();
        for (id, record) in records.iter_mut() {
            if cleared_set.contains(id.as_str()) {
                record.model_id = None;
            } else if let Some(mid) = record.model_id.as_ref() {
                if !valid_model_ids.contains(mid) {
                    let _ = record.store.with_mut(|s| s.set_model_id(None));
                    record.model_id = None;
                }
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

    pub fn register_session(&self, session: Session, mut task_state: TaskReminders) {
        let id = session.id.clone();
        if prune_stale_active_plan(&mut task_state) {
            let _ = session.save_task_state(&task_state);
        }
        let mut record = SessionRecord::new(Arc::new(SessionGate::new(session)), task_state);
        record.project = None;
        self.records.lock().unwrap().insert(id, record);
    }

    pub fn register_for_test(&self, session: Session) {
        let task = session.load_task_state().unwrap_or_default();
        self.register_session(session, task);
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
                        if let Some((item_id, kind)) = turn_step_from_stream(ev) {
                            if announced_step_items.insert(item_id) {
                                manager.emit_lifecycle(LifecycleEvent::TurnStep {
                                    session_id: session_id.clone(),
                                    kind,
                                    progress: progress.clone(),
                                });
                            }
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
        let mgr = Arc::new(SessionManager::new(Arc::new(TurnGuard::new()), db_path));

        let parent_id = mgr
            .open_session("/proj", "default", None)
            .await
            .expect("parent");
        let child_id = mgr
            .open_child_session("/proj", "reviewer", None, &parent_id, "call_xyz")
            .expect("child");

        assert!(mgr.records.lock().unwrap().contains_key(&child_id));
        let resumed = Session::resume(&mgr.db_path(), &child_id).unwrap();
        assert_eq!(
            resumed.parent_session_id.as_deref(),
            Some(parent_id.as_str())
        );
        assert_eq!(resumed.parent_call_id.as_deref(), Some("call_xyz"));

        mgr.remove_session(&parent_id).expect("remove parent");
        assert!(!mgr.records.lock().unwrap().contains_key(&parent_id));
        assert!(!mgr.records.lock().unwrap().contains_key(&child_id));
        assert!(Session::resume(&mgr.db_path(), &child_id).is_err());
    }

    #[tokio::test]
    async fn child_turn_lifecycle_is_filtered() {
        use crate::runtime::observer::TurnPhase;
        use crate::session::live::TurnProgress;

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let db_path = db.to_str().unwrap().to_string();
        let mgr = Arc::new(SessionManager::new(Arc::new(TurnGuard::new()), db_path));

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
        let mgr = Arc::new(SessionManager::new(Arc::clone(&guard), db_path));
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
        let mgr = Arc::new(SessionManager::new(Arc::clone(&guard), db_path));
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
        let mgr = Arc::new(SessionManager::new(
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

        drop(compact);
        assert!(!mgr.is_session_busy_blocking(&sid));
        let revert = mgr
            .try_begin_operation(&sid, SessionOperationKind::Revert)
            .expect("lease releases on drop");
        drop(revert);
    }

    #[tokio::test]
    async fn turn_reservation_closes_pre_spawn_idle_window() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("sessions.db");
        let guard = Arc::new(TurnGuard::new());
        let mgr = Arc::new(SessionManager::new(
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
}
