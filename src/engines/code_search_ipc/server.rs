//! Worker-side request dispatch (multi-lane concurrent stdin/stdout loop).
//!
//! Traffic is stratified across dedicated lanes so long maintenance passes
//! (`refresh`, `reconcile_disk`) never head-of-line block control (`ping`,
//! `shutdown`), FS dirty notifications (`notify_fs_changes`), or interactive
//! queries (`search`, `session_search`):
//!
//! - **Stdin Reader (main thread)**: parses JSON-RPC lines and routes in O(1);
//!   answers `ping` immediately and signals cancellation on `shutdown`/EOF.
//! - **Stdout Writer thread**: serializes and flushes responses out-of-order.
//! - **Notify Lane thread**: ingests `notify_fs_changes` into `pending_updates`
//!   without waiting for `refresh` or `reconcile_disk`.
//! - **Query Lane thread**: runs `search` / `session_search` with High-priority
//!   embedder preemption between indexing micro-batches.
//! - **Maintenance Lane thread**: runs `initialize`, `set_session_db`,
//!   `warmup`, `refresh`, coalesced `reconcile_disk`, and cross-platform 1s
//!   idle memory cooling (`maybe_cool_memory`).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, mpsc};
use std::thread;
use std::time::Duration;

use super::protocol::{
    InitializeParams, JsonRpcRequest, JsonRpcResponse, NotifyFsChangesParams, PingResult,
    RefreshMode, RefreshParams, RefreshResult, RefreshScope, SearchParams, SearchResult,
    SessionSearchParams, SessionSearchResult, SetSessionDbParams,
};
use crate::engines::code_search::{
    CodeSearchRuntime, SemanticEngine, SharedRuntime, decide_index_work, init_workspace_index,
    open_production_embedder, pending_work_counts, queue_fs_changes, queue_reconcile_dirty,
    rebuild_index_in_runtime, refresh_index_incremental, scannable_files, should_full_rebuild,
    warmup_index, write_code_queue,
};
use crate::types::{LitecodeError, Result};

const ERR_INVALID: i32 = -32602;
const ERR_INTERNAL: i32 = -32000;
const IDLE_COOL_TICK: Duration = Duration::from_secs(1);

fn reload_worker_excludes(root: &std::path::Path) {
    let _ = crate::workspace::filter::reload_workspace_excludes_from_disk(root);
}

struct WorkerState {
    workspace_root: RwLock<Option<PathBuf>>,
    session_reader: RwLock<Option<crate::session::SessionDataReader>>,
    runtime: SharedRuntime,
    warmed: AtomicBool,
    reconciling: AtomicBool,
    shutting_down: AtomicBool,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            workspace_root: RwLock::new(None),
            session_reader: RwLock::new(None),
            runtime: Arc::new(RwLock::new(None)),
            warmed: AtomicBool::new(false),
            reconciling: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
        }
    }

    fn is_warmed(&self) -> bool {
        self.warmed.load(Ordering::Acquire)
    }

    fn request_shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        if let Ok(guard) = self.runtime.read()
            && let Some(rt) = guard.as_ref()
        {
            rt.cancel();
        }
    }

    fn initialize(&self, root: PathBuf, session_db_path: Option<PathBuf>) -> Result<()> {
        let root = crate::config::path::canon_abs_lossy(&root);
        let same_root = self
            .workspace_root
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|w| crate::config::path::canon_abs_lossy(w) == root);
        if same_root {
            if let Some(path) = session_db_path {
                self.set_session_db(path)?;
            }
            return Ok(());
        }
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.warmed.store(false, Ordering::Release);
        *self
            .workspace_root
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(root);
        *self
            .session_reader
            .write()
            .unwrap_or_else(|e| e.into_inner()) = session_db_path.map(|path| {
            crate::session::SessionDataReader::from_worker_config(
                crate::session::data::SessionDataReaderConfig::from_path(path),
            )
        });
        Ok(())
    }

    fn set_session_db(&self, session_db_path: PathBuf) -> Result<()> {
        let reader = crate::session::SessionDataReader::from_worker_config(
            crate::session::data::SessionDataReaderConfig::from_path(session_db_path),
        );
        *self
            .session_reader
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(reader.clone());
        if let Ok(guard) = self.runtime.read()
            && let Some(runtime) = guard.as_ref()
        {
            runtime.attach_session_reader(reader);
            if let Err(e) = runtime.ensure_session_index() {
                tracing::warn!(error = %e, "session semantic index inject skipped");
            }
        }
        Ok(())
    }

    fn warmup(&self) -> Result<()> {
        let root = self
            .workspace_root
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("initialize required".into()))?;
        let session_reader = self
            .session_reader
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();

        reload_worker_excludes(&root);
        init_workspace_index(&root)?;
        // Worker load: no task of ours runs yet, so a Building/Refreshing
        // job.json is left over from a dead worker — clear + re-queue it.
        if crate::engines::code_search::reclaim_stale_index_job(&root) {
            tracing::info!("code_search worker: reclaimed stale index job on load");
        }
        let mut embedder = open_production_embedder()?;
        let index = warmup_index(&root, &mut *embedder)?;
        let runtime =
            CodeSearchRuntime::new(root.clone(), index, Some(embedder), session_reader.clone());
        reload_worker_excludes(&root);
        // WP-143: a fresh (or partial) index is filled by the incremental
        // path; reconcile queues every file the index does not hold yet.
        queue_reconcile_dirty(&runtime);
        if session_reader.is_some() {
            if let Err(e) = runtime.ensure_session_index() {
                tracing::warn!(error = %e, "session semantic index warmup load skipped");
            } else if crate::engines::session_search::session_should_rebuild(&root) {
                if let Err(e) = runtime.consume_session_index() {
                    tracing::warn!(error = %e, "session semantic index first-desired rebuild skipped");
                }
            }
        }
        runtime.drop_embedder_for_cool();
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = Some(runtime);
        self.warmed.store(true, Ordering::Release);
        crate::telemetry::release_heap_to_os();
        Ok(())
    }

    fn search(
        &self,
        query: &str,
        glob: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<crate::engines::code_search::SearchHit>> {
        if !self.is_warmed() {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let guard = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        let runtime = guard
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("runtime missing".into()))?;
        SemanticEngine::search(runtime, query, glob, top_k)
    }

    fn session_search(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        if !self.is_warmed() {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let guard = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        let runtime = guard
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("runtime missing".into()))?;
        runtime.search_sessions(query, top_k, session_id)
    }

    /// Refresh: `All` (human) does code then session; tools pass only their corpus.
    fn refresh(&self, scope: RefreshScope) -> Result<RefreshResult> {
        if !self.is_warmed() {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let root = self
            .workspace_root
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("initialize required".into()))?;
        reload_worker_excludes(&root);
        let guard = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        let runtime = guard
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("runtime missing".into()))?;
        let do_code = matches!(scope, RefreshScope::All | RefreshScope::Code);
        let do_session = matches!(scope, RefreshScope::All | RefreshScope::Session);
        let mode = if do_code {
            refresh_code_corpus(&root, runtime)?
        } else {
            RefreshMode::Incremental
        };
        if do_session
            && !self.shutting_down.load(Ordering::Acquire)
            && let Err(e) = runtime.consume_session_index()
        {
            tracing::warn!(error = %e, "session index consume during refresh skipped");
        }
        if do_code && !self.shutting_down.load(Ordering::Acquire) {
            crate::engines::code_search::clear_index_job(&root);
        }
        Ok(RefreshResult { mode })
    }

    fn notify_fs_changes(&self, paths: Vec<String>, deleted: bool) {
        if !self.is_warmed() {
            return;
        }
        queue_fs_changes(&self.runtime, &paths, deleted);
    }

    fn reconcile_disk(&self) {
        if !self.is_warmed() {
            return;
        }
        if self
            .reconciling
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        if let Some(root) = self
            .workspace_root
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            reload_worker_excludes(root);
        }
        let guard = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        if let Some(runtime) = guard.as_ref() {
            queue_reconcile_dirty(runtime);
        }
        self.reconciling.store(false, Ordering::SeqCst);
    }

    /// Two-tier cool: L1 drop Session, L2 unload RAM index (disk retained).
    fn maybe_cool_memory(&self) {
        if !self.is_warmed() {
            return;
        }
        let guard = self.runtime.read().unwrap_or_else(|e| e.into_inner());
        if let Some(runtime) = guard.as_ref() {
            runtime.maybe_cool_memory();
        }
    }

    fn ping(&self) -> PingResult {
        let ready = self.is_warmed();
        let embed_device = if ready {
            self.runtime
                .read()
                .ok()
                .and_then(|g| g.as_ref().map(|r| r.embed_device()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        PingResult {
            ready,
            embed_device,
        }
    }

    fn shutdown(&self) {
        self.request_shutdown();
        self.warmed.store(false, Ordering::Release);
        *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn refresh_code_corpus(root: &std::path::Path, runtime: &CodeSearchRuntime) -> Result<RefreshMode> {
    let rebuild = should_full_rebuild(root);
    if rebuild {
        rebuild_index_in_runtime(runtime)?;
        return Ok(RefreshMode::Rebuild);
    }
    queue_reconcile_dirty(runtime);
    let (embed_dirty, dirty) = runtime
        .pending_updates
        .lock()
        .map(|g| pending_work_counts(&g))
        .unwrap_or((0, 0));
    let indexed = runtime
        .with_index(|index| Ok(index.indexed_paths().len()))
        .unwrap_or(0);
    let scannable = scannable_files(root).map(|f| f.len()).unwrap_or(indexed);
    match decide_index_work(true, true, embed_dirty, dirty, indexed, scannable) {
        crate::engines::code_search::IndexWork::Rebuild { .. } => {
            rebuild_index_in_runtime(runtime)?;
            Ok(RefreshMode::Rebuild)
        }
        crate::engines::code_search::IndexWork::None => {
            write_code_queue(root, &Default::default());
            Ok(RefreshMode::Incremental)
        }
        crate::engines::code_search::IndexWork::Update { .. } => {
            refresh_index_incremental(runtime)?;
            Ok(RefreshMode::Incremental)
        }
    }
}

enum QueryJob {
    Search {
        id: u64,
        params: SearchParams,
    },
    SessionSearch {
        id: u64,
        params: SessionSearchParams,
    },
}

struct NotifyJob {
    id: Option<u64>,
    paths: Vec<String>,
    deleted: bool,
}

enum MaintJob {
    Initialize {
        id: Option<u64>,
        root: PathBuf,
        session_db_path: Option<PathBuf>,
    },
    SetSessionDb {
        id: Option<u64>,
        session_db_path: PathBuf,
    },
    Warmup {
        id: Option<u64>,
    },
    Refresh {
        id: Option<u64>,
        scope: RefreshScope,
    },
    ReconcileDisk {
        id: u64,
    },
}

#[derive(Default)]
struct MaintQueueState {
    jobs: VecDeque<MaintJob>,
    pending_reconcile_notify: bool,
    closed: bool,
}

struct MaintQueue {
    state: Mutex<MaintQueueState>,
    cv: Condvar,
}

impl MaintQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(MaintQueueState::default()),
            cv: Condvar::new(),
        }
    }

    fn push(&self, job: MaintJob) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.jobs.push_back(job);
        self.cv.notify_one();
    }

    fn request_reconcile_notify(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return;
        }
        state.pending_reconcile_notify = true;
        self.cv.notify_one();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.closed = true;
        self.cv.notify_all();
    }

    fn pop_timeout(&self, timeout: Duration) -> MaintPoll {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(job) = state.jobs.pop_front() {
                return MaintPoll::Job(job);
            }
            if state.pending_reconcile_notify {
                state.pending_reconcile_notify = false;
                return MaintPoll::CoalescedReconcile;
            }
            if state.closed {
                return MaintPoll::Closed;
            }
            let (next, timed_out) = self
                .cv
                .wait_timeout(state, timeout)
                .unwrap_or_else(|e| e.into_inner());
            state = next;
            if timed_out.timed_out() {
                if let Some(job) = state.jobs.pop_front() {
                    return MaintPoll::Job(job);
                }
                if state.pending_reconcile_notify {
                    state.pending_reconcile_notify = false;
                    return MaintPoll::CoalescedReconcile;
                }
                if state.closed {
                    return MaintPoll::Closed;
                }
                return MaintPoll::IdleTick;
            }
        }
    }
}

enum MaintPoll {
    Job(MaintJob),
    CoalescedReconcile,
    IdleTick,
    Closed,
}

fn send_reply(stdout_tx: &mpsc::Sender<JsonRpcResponse>, id: Option<u64>, resp: JsonRpcResponse) {
    if id.is_some() {
        let _ = stdout_tx.send(resp);
    }
}

fn run_query_lane(
    state: Arc<WorkerState>,
    query_rx: mpsc::Receiver<QueryJob>,
    stdout_tx: mpsc::Sender<JsonRpcResponse>,
) {
    while let Ok(job) = query_rx.recv() {
        if state.shutting_down.load(Ordering::Acquire) {
            break;
        }
        let resp = match job {
            QueryJob::Search { id, params } => {
                match state.search(&params.query, params.glob.as_deref(), params.top_k) {
                    Ok(hits) => match serde_json::to_value(SearchResult { hits }) {
                        Ok(v) => JsonRpcResponse::ok(id, v),
                        Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                    },
                    Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                }
            }
            QueryJob::SessionSearch { id, params } => {
                match state.session_search(&params.query, params.top_k, params.session_id.as_deref())
                {
                    Ok(hits) => match serde_json::to_value(SessionSearchResult { hits }) {
                        Ok(v) => JsonRpcResponse::ok(id, v),
                        Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                    },
                    Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                }
            }
        };
        if stdout_tx.send(resp).is_err() {
            break;
        }
    }
}

fn run_notify_lane(
    state: Arc<WorkerState>,
    notify_rx: mpsc::Receiver<NotifyJob>,
    stdout_tx: mpsc::Sender<JsonRpcResponse>,
) {
    while let Ok(job) = notify_rx.recv() {
        if state.shutting_down.load(Ordering::Acquire) {
            break;
        }
        state.notify_fs_changes(job.paths, job.deleted);
        if let Some(id) = job.id {
            let _ = stdout_tx.send(JsonRpcResponse::ok(id, serde_json::json!({})));
        }
    }
}

fn run_maint_lane(
    state: Arc<WorkerState>,
    queue: Arc<MaintQueue>,
    stdout_tx: mpsc::Sender<JsonRpcResponse>,
) {
    loop {
        match queue.pop_timeout(IDLE_COOL_TICK) {
            MaintPoll::Closed => break,
            MaintPoll::IdleTick => {
                if state.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                state.maybe_cool_memory();
            }
            MaintPoll::CoalescedReconcile => {
                if state.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                state.reconcile_disk();
                state.maybe_cool_memory();
            }
            MaintPoll::Job(job) => {
                if state.shutting_down.load(Ordering::Acquire) {
                    break;
                }
                match job {
                    MaintJob::Initialize {
                        id,
                        root,
                        session_db_path,
                    } => {
                        let resp = match state.initialize(root, session_db_path) {
                            Ok(()) => JsonRpcResponse::ok(id.unwrap_or(0), serde_json::json!({})),
                            Err(e) => {
                                JsonRpcResponse::err(id.unwrap_or(0), ERR_INTERNAL, e.to_string())
                            }
                        };
                        send_reply(&stdout_tx, id, resp);
                    }
                    MaintJob::SetSessionDb {
                        id,
                        session_db_path,
                    } => {
                        let resp = match state.set_session_db(session_db_path) {
                            Ok(()) => JsonRpcResponse::ok(id.unwrap_or(0), serde_json::json!({})),
                            Err(e) => {
                                JsonRpcResponse::err(id.unwrap_or(0), ERR_INTERNAL, e.to_string())
                            }
                        };
                        send_reply(&stdout_tx, id, resp);
                    }
                    MaintJob::Warmup { id } => {
                        let resp = match state.warmup() {
                            Ok(()) => JsonRpcResponse::ok(id.unwrap_or(0), serde_json::json!({})),
                            Err(e) => {
                                JsonRpcResponse::err(id.unwrap_or(0), ERR_INTERNAL, e.to_string())
                            }
                        };
                        send_reply(&stdout_tx, id, resp);
                    }
                    MaintJob::Refresh { id, scope } => {
                        let resp = match state.refresh(scope) {
                            Ok(result) => match serde_json::to_value(result) {
                                Ok(v) => JsonRpcResponse::ok(id.unwrap_or(0), v),
                                Err(e) => JsonRpcResponse::err(
                                    id.unwrap_or(0),
                                    ERR_INTERNAL,
                                    e.to_string(),
                                ),
                            },
                            Err(e) => {
                                JsonRpcResponse::err(id.unwrap_or(0), ERR_INTERNAL, e.to_string())
                            }
                        };
                        send_reply(&stdout_tx, id, resp);
                        state.maybe_cool_memory();
                    }
                    MaintJob::ReconcileDisk { id } => {
                        state.reconcile_disk();
                        let _ = stdout_tx.send(JsonRpcResponse::ok(id, serde_json::json!({})));
                        state.maybe_cool_memory();
                    }
                }
            }
        }
    }
}

/// Multi-lane stdin/stdout worker loop (cross-platform: Windows, macOS, Linux).
pub fn run_worker_loop() -> Result<()> {
    let state = Arc::new(WorkerState::new());
    let (stdout_tx, stdout_rx) = mpsc::channel::<JsonRpcResponse>();
    let (query_tx, query_rx) = mpsc::channel::<QueryJob>();
    let (notify_tx, notify_rx) = mpsc::channel::<NotifyJob>();
    let maint_queue = Arc::new(MaintQueue::new());

    let stdout_handle = thread::Builder::new()
        .name("code-search-stdout".into())
        .spawn(move || {
            let mut stdout = std::io::stdout().lock();
            while let Ok(resp) = stdout_rx.recv() {
                let Ok(out) = serde_json::to_string(&resp) else {
                    continue;
                };
                if writeln!(stdout, "{out}").is_err() || stdout.flush().is_err() {
                    break;
                }
            }
        })
        .map_err(|e| LitecodeError::ToolExecution(format!("worker stdout thread: {e}")))?;

    let query_handle = {
        let state = Arc::clone(&state);
        let stdout_tx = stdout_tx.clone();
        thread::Builder::new()
            .name("code-search-query".into())
            .spawn(move || run_query_lane(state, query_rx, stdout_tx))
            .map_err(|e| LitecodeError::ToolExecution(format!("worker query thread: {e}")))?
    };

    let notify_handle = {
        let state = Arc::clone(&state);
        let stdout_tx = stdout_tx.clone();
        thread::Builder::new()
            .name("code-search-notify".into())
            .spawn(move || run_notify_lane(state, notify_rx, stdout_tx))
            .map_err(|e| LitecodeError::ToolExecution(format!("worker notify thread: {e}")))?
    };

    let maint_handle = {
        let state = Arc::clone(&state);
        let maint_queue = Arc::clone(&maint_queue);
        let stdout_tx = stdout_tx.clone();
        thread::Builder::new()
            .name("code-search-maint".into())
            .spawn(move || run_maint_lane(state, maint_queue, stdout_tx))
            .map_err(|e| LitecodeError::ToolExecution(format!("worker maint thread: {e}")))?
    };

    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // Parent closed stdin — exit cleanly.
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(req) => req,
            Err(e) => {
                let _ = stdout_tx.send(JsonRpcResponse::err(0, ERR_INVALID, e.to_string()));
                continue;
            }
        };
        let id = req.id;

        match req.method.as_str() {
            "ping" => {
                if let Some(id) = id {
                    let resp = match serde_json::to_value(state.ping()) {
                        Ok(v) => JsonRpcResponse::ok(id, v),
                        Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                    };
                    let _ = stdout_tx.send(resp);
                }
            }
            "shutdown" => {
                state.request_shutdown();
                if let Some(id) = id {
                    let _ = stdout_tx.send(JsonRpcResponse::ok(id, serde_json::json!({})));
                }
                break;
            }
            "notify_fs_changes" => match serde_json::from_value::<NotifyFsChangesParams>(req.params)
            {
                Ok(p) => {
                    let _ = notify_tx.send(NotifyJob {
                        id,
                        paths: p.paths,
                        deleted: p.deleted,
                    });
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            "reconcile_disk" => match id {
                Some(id) => maint_queue.push(MaintJob::ReconcileDisk { id }),
                None => maint_queue.request_reconcile_notify(),
            },
            "search" => match serde_json::from_value::<SearchParams>(req.params) {
                Ok(params) => {
                    if let Some(id) = id {
                        let _ = query_tx.send(QueryJob::Search { id, params });
                    }
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            "session_search" => match serde_json::from_value::<SessionSearchParams>(req.params) {
                Ok(params) => {
                    if let Some(id) = id {
                        let _ = query_tx.send(QueryJob::SessionSearch { id, params });
                    }
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            "initialize" => match serde_json::from_value::<InitializeParams>(req.params) {
                Ok(p) => {
                    let root = PathBuf::from(p.workspace_root);
                    let session_db_path = p
                        .session_db_path
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from);
                    maint_queue.push(MaintJob::Initialize {
                        id,
                        root,
                        session_db_path,
                    });
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            "set_session_db" => match serde_json::from_value::<SetSessionDbParams>(req.params) {
                Ok(p) => {
                    maint_queue.push(MaintJob::SetSessionDb {
                        id,
                        session_db_path: PathBuf::from(p.session_db_path),
                    });
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            "warmup" => {
                maint_queue.push(MaintJob::Warmup { id });
            }
            "refresh" => match serde_json::from_value::<RefreshParams>(req.params) {
                Ok(p) => {
                    maint_queue.push(MaintJob::Refresh { id, scope: p.scope });
                }
                Err(e) => {
                    send_reply(
                        &stdout_tx,
                        id,
                        JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, e.to_string()),
                    );
                }
            },
            other => {
                send_reply(
                    &stdout_tx,
                    id,
                    JsonRpcResponse::err(id.unwrap_or(0), ERR_INVALID, format!("unknown method: {other}")),
                );
            }
        }
    }

    // Signal shutdown, close all lane inputs, and drain stdout so any final
    // `shutdown` response is flushed before the process terminates.
    state.request_shutdown();
    drop(query_tx);
    drop(notify_tx);
    maint_queue.close();
    drop(stdout_tx);
    let _ = query_handle.join();
    let _ = notify_handle.join();
    let _ = maint_handle.join();
    state.shutdown();
    let _ = stdout_handle.join();
    Ok(())
}
