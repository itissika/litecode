//! Single writer actor. All session mutations enter this queue.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;

use tokio::sync::{mpsc, oneshot};

use crate::types::{LitecodeError, Result};

use super::command::{CommitKind, CommitReceipt, SessionMutation};
use super::sqlite::conn::SharedDb;
use super::sqlite::ops;
use super::sqlite::read;
use super::sqlite::session::{ApplyOutcome, CommitDeltaOutcome, Session};

pub const WRITER_QUEUE_CAPACITY: usize = 256;

pub struct WriteRequest {
    pub mutation: SessionMutation,
    pub reply: oneshot::Sender<Result<CommitReceipt>>,
}

pub struct WriterHandle {
    tx: Mutex<Option<mpsc::Sender<WriteRequest>>>,
    shutdown: Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
    pub hooks: Arc<WriterHooks>,
}

pub struct WriterHooks {
    pub paused: Mutex<bool>,
    pub unpause: Condvar,
    parked: Mutex<bool>,
    parked_changed: Condvar,
    pub fault: Mutex<Option<FaultKind>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    BeforeBegin,
    BeforeCommit,
    AfterCommit,
}

impl Default for WriterHooks {
    fn default() -> Self {
        Self {
            paused: Mutex::new(false),
            unpause: Condvar::new(),
            parked: Mutex::new(false),
            parked_changed: Condvar::new(),
            fault: Mutex::new(None),
        }
    }
}

impl WriterHooks {
    pub fn pause(&self) {
        *self.paused.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }

    pub fn resume(&self) {
        let mut g = self.paused.lock().unwrap_or_else(|e| e.into_inner());
        *g = false;
        self.unpause.notify_all();
    }

    /// Test synchronization point: the writer has dequeued a command and is
    /// blocked before executing it, so subsequent sends observe queue capacity.
    pub fn wait_until_parked(&self) {
        let mut parked = self.parked.lock().unwrap_or_else(|e| e.into_inner());
        while !*parked {
            parked = self
                .parked_changed
                .wait(parked)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn wait_if_paused(&self) {
        let mut g = self.paused.lock().unwrap_or_else(|e| e.into_inner());
        while *g {
            {
                let mut parked = self.parked.lock().unwrap_or_else(|e| e.into_inner());
                *parked = true;
                self.parked_changed.notify_all();
            }
            g = self.unpause.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        let mut parked = self.parked.lock().unwrap_or_else(|e| e.into_inner());
        *parked = false;
        self.parked_changed.notify_all();
    }

    fn take_fault_if(&self, expected: FaultKind) -> bool {
        let mut fault = self.fault.lock().unwrap_or_else(|e| e.into_inner());
        if *fault == Some(expected) {
            *fault = None;
            true
        } else {
            false
        }
    }

    pub fn inject(&self, kind: FaultKind) {
        *self.fault.lock().unwrap_or_else(|e| e.into_inner()) = Some(kind);
    }
}

struct WriterState {
    db: Rc<SharedDb>,
    data_root: PathBuf,
    live: HashMap<String, Session>,
    hooks: Arc<WriterHooks>,
    /// Session directories and snapshots to remove **after** the commit lands.
    /// Deleting them during the transaction would leave a rolled-back session
    /// row pointing at files that are already gone.
    deleted_files: Vec<(String, Option<String>)>,
}

impl WriterHandle {
    pub fn spawn(path: &Path, data_root: PathBuf) -> Result<Self> {
        let path = path.to_path_buf();
        let (tx, rx) = mpsc::channel(WRITER_QUEUE_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let hooks = Arc::new(WriterHooks::default());
        let hooks_thread = Arc::clone(&hooks);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new()
            .name("session-data-writer".into())
            .spawn(move || writer_loop(path, data_root, rx, hooks_thread, ready_tx))
            .map_err(|e| LitecodeError::SessionStorage(format!("spawn writer: {e}")))?;
        ready_rx
            .recv()
            .map_err(|_| LitecodeError::SessionStorage("writer died before ready".into()))??;
        Ok(Self {
            tx: Mutex::new(Some(tx)),
            shutdown,
            join: Mutex::new(Some(join)),
            hooks,
        })
    }

    pub fn spawn_ephemeral() -> Result<(Self, PathBuf)> {
        let uri = format!(
            "file:session-mem-{}?mode=memory&cache=shared",
            ulid::Ulid::new()
        );
        let (tx, rx) = mpsc::channel(WRITER_QUEUE_CAPACITY);
        let shutdown = Arc::new(AtomicBool::new(false));
        let hooks = Arc::new(WriterHooks::default());
        let hooks_thread = Arc::clone(&hooks);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let uri_thread = uri.clone();
        let join = std::thread::Builder::new()
            .name("session-data-writer-mem".into())
            .spawn(move || writer_loop_ephemeral(uri_thread, rx, hooks_thread, ready_tx))
            .map_err(|e| LitecodeError::SessionStorage(format!("spawn writer: {e}")))?;
        ready_rx.recv().map_err(|_| {
            LitecodeError::SessionStorage("ephemeral writer died before ready".into())
        })??;
        Ok((
            Self {
                tx: Mutex::new(Some(tx)),
                shutdown,
                join: Mutex::new(Some(join)),
                hooks,
            },
            PathBuf::from(uri),
        ))
    }

    fn sender(&self) -> Result<mpsc::Sender<WriteRequest>> {
        self.tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .cloned()
            .ok_or(LitecodeError::SessionDataClosed)
    }

    pub async fn submit(&self, mutation: SessionMutation) -> Result<CommitReceipt> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LitecodeError::SessionDataClosed);
        }
        let (reply, rx) = oneshot::channel();
        self.sender()?
            .send(WriteRequest { mutation, reply })
            .await
            .map_err(|_| LitecodeError::SessionDataClosed)?;
        rx.await.map_err(|_| LitecodeError::SessionDataClosed)?
    }

    pub fn submit_blocking(&self, mutation: SessionMutation) -> Result<CommitReceipt> {
        // Legacy synchronous manager adapters are still invoked from async
        // controller paths. Tokio forbids `blocking_send` on a runtime worker;
        // bridge that compatibility path through a short helper thread until
        // those callers use the async typed SessionData interface directly.
        if tokio::runtime::Handle::try_current().is_ok() {
            return std::thread::scope(|scope| {
                scope
                    .spawn(|| self.submit_blocking_inner(mutation))
                    .join()
                    .map_err(|_| {
                        LitecodeError::SessionStorage("writer submit thread panicked".into())
                    })?
            });
        }
        self.submit_blocking_inner(mutation)
    }

    fn submit_blocking_inner(&self, mutation: SessionMutation) -> Result<CommitReceipt> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LitecodeError::SessionDataClosed);
        }
        let (reply, rx) = oneshot::channel();
        self.sender()?
            .blocking_send(WriteRequest { mutation, reply })
            .map_err(|_| LitecodeError::SessionDataClosed)?;
        rx.blocking_recv()
            .map_err(|_| LitecodeError::SessionDataClosed)?
    }

    pub fn try_submit_nowait(
        &self,
        mutation: SessionMutation,
    ) -> Result<oneshot::Receiver<Result<CommitReceipt>>> {
        if self.shutdown.load(Ordering::SeqCst) {
            return Err(LitecodeError::SessionDataClosed);
        }
        let (reply, rx) = oneshot::channel();
        self.sender()?
            .try_send(WriteRequest { mutation, reply })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => LitecodeError::SessionBackpressure,
                mpsc::error::TrySendError::Closed(_) => LitecodeError::SessionDataClosed,
            })?;
        Ok(rx)
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.hooks.resume();
        drop(self.tx.lock().unwrap_or_else(|e| e.into_inner()).take());
        if let Some(join) = self.join.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = join.join();
        }
    }
}

fn writer_loop(
    path: PathBuf,
    data_root: PathBuf,
    mut rx: mpsc::Receiver<WriteRequest>,
    hooks: Arc<WriterHooks>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    let db = match SharedDb::open_rw(&path) {
        Ok(db) => Rc::new(db),
        Err(e) => {
            tracing::error!(error = %e, "session writer failed to open db");
            let _ = ready.send(Err(LitecodeError::SessionStorage(e.to_string())));
            while let Some(req) = rx.blocking_recv() {
                let _ = req
                    .reply
                    .send(Err(LitecodeError::SessionStorage(e.to_string())));
            }
            return;
        }
    };
    let mut state = WriterState {
        db,
        data_root,
        live: HashMap::new(),
        hooks,
        deleted_files: Vec::new(),
    };
    cleanup_orphan_blob_files(&state);
    if let Err(error) = seal_orphaned_in_progress(&state) {
        // Not fatal: a session that cannot be sealed yet is a session that will
        // be sealed when it is next resumed. Refusing to serve writes over it
        // would turn a cosmetic problem into an outage.
        tracing::warn!(error = %error, "could not seal rows left in flight by a previous run");
    }
    // Ready only once the startup repair has run: callers treat a successful
    // open as "the store is in a servable state", and the repair is part of
    // that state.
    let _ = ready.send(Ok(()));
    while let Some(req) = rx.blocking_recv() {
        state.hooks.wait_if_paused();
        let result = execute(&mut state, req.mutation);
        let _ = req.reply.send(result);
    }
    let _ = state.db.checkpoint();
}

/// Seal rows the previous process left in flight.
///
/// The session layer promises that a session which has stopped has no
/// `in_progress` rows: a cancelled or failed turn seals its own rows as
/// `Incomplete` + `Final` (`runtime/mod.rs`). A process that *died* cannot keep
/// that promise, so the promise is restored here, at startup, before a single
/// mutation is served. At this moment nothing can be in flight, so every
/// `in_progress` row in the database is an orphan by definition — no age
/// heuristic, no guessing.
///
/// This is precisely the repair the projection refuses to do. Searching only
/// settles history, so a crashed turn's rows are invisible to search until
/// something seals them; leaving them alone would make them invisible forever.
/// Sealing makes them visible, incomplete, and honest about it.
///
/// Nothing has to be told to the index: sealing makes the rows searchable, and
/// the next reconciliation finds them by comparing key sets. That is the whole
/// reason the reconcile does not need change notifications.
fn seal_orphaned_in_progress(state: &WriterState) -> Result<usize> {
    let ids: Vec<String> = {
        let conn = state.db.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT session_id FROM transcript_items
             WHERE state = 'in_progress' ORDER BY session_id",
        )?;
        stmt.query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut sealed = 0usize;
    for id in ids {
        let session = Session::resume_shared(Rc::clone(&state.db), state.data_root.clone(), &id)?;
        let seqs = session.seal_in_progress_items()?;
        if !seqs.is_empty() {
            // "This session was interrupted by something that did not get to
            // clean up" is a fact worth being able to look up later.
            tracing::warn!(
                session_id = %id,
                rows = seqs.len(),
                seqs = ?seqs,
                "sealed rows a previous run left in flight"
            );
        }
        sealed += seqs.len();
    }
    if sealed > 0 {
        tracing::info!(rows = sealed, "sealed orphaned in-progress rows at startup");
    }
    Ok(sealed)
}

fn cleanup_orphan_blob_files(state: &WriterState) {
    let live_ids = ops::referenced_blob_ids(state.db.conn()).unwrap_or_default();
    if let Err(error) = super::blob::gc_unreferenced(&state.data_root, &live_ids) {
        tracing::warn!(error = %error, "session blob orphan cleanup failed");
    }
}

fn writer_loop_ephemeral(
    uri: String,
    mut rx: mpsc::Receiver<WriteRequest>,
    hooks: Arc<WriterHooks>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    let db = match SharedDb::open_shared_memory(&uri) {
        Ok(db) => {
            let _ = ready.send(Ok(()));
            Rc::new(db)
        }
        Err(e) => {
            let _ = ready.send(Err(LitecodeError::SessionStorage(e.to_string())));
            while let Some(req) = rx.blocking_recv() {
                let _ = req
                    .reply
                    .send(Err(LitecodeError::SessionStorage(e.to_string())));
            }
            return;
        }
    };
    let mut state = WriterState {
        db,
        data_root: std::env::temp_dir().join("litecode"),
        live: HashMap::new(),
        hooks,
        deleted_files: Vec::new(),
    };
    while let Some(req) = rx.blocking_recv() {
        state.hooks.wait_if_paused();
        let result = execute(&mut state, req.mutation);
        let _ = req.reply.send(result);
    }
}

fn execute(state: &mut WriterState, mutation: SessionMutation) -> Result<CommitReceipt> {
    if state.hooks.take_fault_if(FaultKind::BeforeBegin) {
        return Err(LitecodeError::SessionStorage(
            "injected fault before begin".into(),
        ));
    }
    if mutation.session_id().is_none()
        && let Some(receipt) = ops::load_operation_by_id(state.db.conn(), mutation.operation_id())?
    {
        return Ok(receipt);
    }
    if let Some(sid) = mutation.session_id()
        && let Some(expected) = mutation.expected_revision()
    {
        if let Some(receipt) = ops::load_operation(state.db.conn(), sid, mutation.operation_id())? {
            return Ok(receipt);
        }
        let actual = read::load_revision(state.db.conn(), sid).unwrap_or(0);
        if actual != expected {
            return Err(LitecodeError::SessionConflict { expected, actual });
        }
    }
    let affected_session = mutation.session_id().map(str::to_owned);
    state
        .db
        .conn()
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| LitecodeError::SessionStorage(format!("BEGIN IMMEDIATE: {e}")))?;
    let result = dispatch(state, mutation);
    match result {
        Ok(receipt) => {
            if state.hooks.take_fault_if(FaultKind::BeforeCommit) {
                let _ = state.db.conn().execute_batch("ROLLBACK");
                state.deleted_files.clear();
                if let Some(session_id) = affected_session {
                    state.live.remove(&session_id);
                }
                cleanup_orphan_blob_files(state);
                return Err(LitecodeError::SessionStorage(
                    "injected fault before commit".into(),
                ));
            }
            let orphan_ids = ops::unreferenced_blob_ids(state.db.conn()).unwrap_or_default();
            let _ = ops::delete_blob_rows(state.db.conn(), &orphan_ids);
            state
                .db
                .conn()
                .execute_batch("COMMIT")
                .map_err(|e| LitecodeError::SessionStorage(format!("COMMIT: {e}")))?;
            for id in orphan_ids {
                let path = state.data_root.join(super::blob::rel_path_for(&id));
                let _ = std::fs::remove_file(path);
            }
            for (session_id, project) in std::mem::take(&mut state.deleted_files) {
                Session::cleanup_deleted_files(&state.data_root, &session_id, project.as_deref());
            }
            if state.hooks.take_fault_if(FaultKind::AfterCommit) {
                return Err(LitecodeError::SessionStorage(
                    "injected fault after commit".into(),
                ));
            }
            Ok(receipt)
        }
        Err(e) => {
            let _ = state.db.conn().execute_batch("ROLLBACK");
            state.deleted_files.clear();
            // The legacy live projection is updated while dispatching.  A
            // rolled-back command must never leave that projection ahead of
            // disk; lazily hydrate it again on the next accepted command.
            if let Some(session_id) = affected_session {
                state.live.remove(&session_id);
            }
            cleanup_orphan_blob_files(state);
            Err(e)
        }
    }
}

/// Record a receipt **in the caller's transaction**.
///
/// The receipt is written inside the same `BEGIN IMMEDIATE` as the mutation it
/// describes, so a failure rolls the whole mutation back rather than committing
/// content with no record of it.
fn bump_receipt(
    state: &WriterState,
    session_id: &str,
    operation_id: &str,
    previous: u64,
    outcome: CommitKind,
) -> Result<CommitReceipt> {
    let receipt = CommitReceipt {
        session_id: session_id.to_string(),
        operation_id: operation_id.to_string(),
        revision: previous + 1,
        change_id: 0,
        outcome,
        preview: None,
        assistant_preview: None,
        working_set: None,
    };
    ops::persist_receipt(state.db.conn(), &receipt)?;
    let mut receipt = receipt;
    receipt.change_id = ops::latest_change_id(state.db.conn())?;
    Ok(receipt)
}

fn ensure_live<'a>(state: &'a mut WriterState, session_id: &str) -> Result<&'a mut Session> {
    if !state.live.contains_key(session_id) {
        let session =
            Session::resume_shared(Rc::clone(&state.db), state.data_root.clone(), session_id)?;
        state.live.insert(session_id.to_string(), session);
    }
    Ok(state.live.get_mut(session_id).expect("just inserted"))
}

fn dispatch(state: &mut WriterState, mutation: SessionMutation) -> Result<CommitReceipt> {
    match mutation {
        SessionMutation::Create {
            operation_id,
            project,
            agent_id,
            model_id,
            parent_session_id,
            parent_call_id,
            responsibility,
        } => {
            let session = Session::open_shared(
                Rc::clone(&state.db),
                state.data_root.clone(),
                &project,
                &agent_id,
                model_id.as_deref(),
                parent_session_id.as_deref(),
                parent_call_id.as_deref(),
                &responsibility,
            )?;
            let id = session.id.clone();
            state.live.insert(id.clone(), session);
            bump_receipt(state, &id, &operation_id.0, 0, CommitKind::Created)
        }
        SessionMutation::Apply {
            session_id,
            expected_revision,
            operation_id,
            op,
        } => {
            let commit_kind = {
                let session = ensure_live(state, &session_id)?;
                match session.apply(op)? {
                    ApplyOutcome::Appended(seq) => CommitKind::Appended { seq },
                    ApplyOutcome::Sealed { seq } => CommitKind::Sealed { seqs: vec![seq] },
                    ApplyOutcome::Truncated { anchor } => {
                        CommitKind::Truncated { from_seq: anchor }
                    }
                }
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                commit_kind,
            )
        }
        SessionMutation::InsertDetails {
            session_id,
            expected_revision,
            operation_id,
            items,
            turn_id,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.insert_detail_rows_with_turn(&items, &turn_id)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::PersistItem {
            session_id,
            expected_revision,
            operation_id,
            item,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.persist_item(&item)?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Appended { seq },
            )
        }
        SessionMutation::BeginStreamItem {
            session_id,
            expected_revision,
            operation_id,
            item,
            turn_id,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.begin_stream_item(&item, &turn_id)?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Appended { seq },
            )
        }
        SessionMutation::UpdateStreamItem {
            session_id,
            expected_revision,
            operation_id,
            seq,
            item,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.update_stream_item(seq, &item)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SealStreamItem {
            session_id,
            expected_revision,
            operation_id,
            seq,
            item,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.seal_stream_item(seq, &item)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Sealed { seqs: vec![seq] },
            )
        }
        SessionMutation::AppendJobExit {
            session_id,
            expected_revision,
            operation_id,
            item,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.append_job_exit(&item)?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Appended { seq },
            )
        }
        SessionMutation::AppendPlanReminder {
            session_id,
            expected_revision,
            operation_id,
            item,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.append_plan_reminder(&item)?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Appended { seq },
            )
        }
        SessionMutation::AppendPlanExecute {
            session_id,
            expected_revision,
            operation_id,
            item,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.append_plan_execute(&item)?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Appended { seq },
            )
        }
        SessionMutation::SealInProgress {
            session_id,
            expected_revision,
            operation_id,
        } => {
            let seqs = {
                let session = ensure_live(state, &session_id)?;
                session.seal_in_progress_items()?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Sealed { seqs },
            )
        }
        SessionMutation::CommitTurnDelta {
            session_id,
            expected_revision,
            operation_id,
            mut rows,
            expected_max_seq,
            turn_id,
        } => {
            let (kind, preview, working) = {
                let session = ensure_live(state, &session_id)?;
                let outcome = session.commit_turn_delta_with_orphan_cleanup(
                    &mut rows,
                    &[],
                    expected_max_seq,
                    &turn_id,
                )?;
                let working = session.load_working_set()?;
                match outcome {
                    CommitDeltaOutcome::Discarded => (CommitKind::Idempotent, None, working),
                    CommitDeltaOutcome::Applied {
                        sealed_seqs,
                        preview,
                        ..
                    } => {
                        let kind = if sealed_seqs.is_empty() {
                            CommitKind::MetaUpdated
                        } else {
                            CommitKind::Sealed { seqs: sealed_seqs }
                        };
                        (kind, preview, working)
                    }
                }
            };
            let mut receipt =
                bump_receipt(state, &session_id, &operation_id.0, expected_revision, kind)?;
            receipt.preview = preview
                .as_ref()
                .and_then(|patch| patch.user.clone().map(|user| (user, patch.updated_at)));
            receipt.assistant_preview = preview.as_ref().and_then(|patch| {
                patch
                    .assistant
                    .clone()
                    .map(|assistant| (assistant, patch.updated_at))
            });
            receipt.working_set = Some(working);
            Ok(receipt)
        }
        SessionMutation::Compact {
            session_id,
            expected_revision,
            operation_id,
            summary,
            token_estimate,
            kept_from,
            expected_prefix,
        } => {
            let seq = {
                let session = ensure_live(state, &session_id)?;
                session.apply_compact_checkpoint_checked(
                    &summary,
                    kept_from.map(|s| s as i64),
                    token_estimate,
                    expected_prefix,
                )?
            };
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Compacted { seq: seq as u64 },
            )
        }
        SessionMutation::SaveTaskState {
            session_id,
            expected_revision,
            operation_id,
            state: task_state,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.save_task_state(&task_state)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SaveContextMeter {
            session_id,
            expected_revision,
            operation_id,
            meter,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.save_context_meter(&meter)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SetAgent {
            session_id,
            expected_revision,
            operation_id,
            agent_id,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.set_agent_id(&agent_id)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SetModel {
            session_id,
            expected_revision,
            operation_id,
            model_id,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.set_model_id(model_id.as_deref())?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SetThinkingTier {
            session_id,
            expected_revision,
            operation_id,
            tier,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.set_thinking_tier(tier)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::SetContextMode {
            session_id,
            expected_revision,
            operation_id,
            mode,
        } => {
            {
                let session = ensure_live(state, &session_id)?;
                session.set_context_mode(mode)?;
            }
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::MetaUpdated,
            )
        }
        SessionMutation::Delete {
            session_id,
            expected_revision,
            operation_id,
        } => {
            let deleted = Session::delete_on(state.db.conn(), &session_id)?;
            state.live.remove(&session_id);
            // The files go after the commit, not during it.
            state.deleted_files.extend(deleted);
            bump_receipt(
                state,
                &session_id,
                &operation_id.0,
                expected_revision,
                CommitKind::Deleted,
            )
        }
    }
}
