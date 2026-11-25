use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use crate::engines::code_search::SearchHit;
use crate::engines::code_search_ipc::CodeSearchWorkerClient;
use crate::engines::code_search_ipc::protocol::{RefreshResult, RefreshScope};
use crate::types::{LitecodeError, Result};

/// A code or session read found the worker busy (e.g. session index mutating
/// or a test gate held). Callers may degrade to text or report busy.
pub const SEARCH_WORKER_BUSY: &str = "code_search worker is busy; semantic search was skipped";
/// Bounded wait for a search already inside the code tool. Cancellation is
/// checked between polls, so a run_code deadline never waits for this budget.
pub const SEARCH_QUEUE_WAIT: Duration = Duration::from_secs(20);
const SEARCH_QUEUE_POLL: Duration = Duration::from_millis(20);
/// Returned when the execution token fires while waiting for search.
pub const SEARCH_CANCELLED: &str = "code_search cancelled";

const WORKER_NOT_RUNNING: &str = "code_search worker is not running; enable the retrieval engine or restart litecode with this workspace";
/// Coalescing window for parent-side FS dirty notifications before pushing a
/// single one-way batch to the worker.
const DIRTY_FLUSH_DEBOUNCE: Duration = Duration::from_millis(80);
/// Cap on buffered discrete FS paths before collapsing into a single
/// `reconcile_disk` notification (protects RAM and pipe during huge FS storms).
const MAX_DIRTY_FS_PATHS: usize = 4096;

pub fn is_search_worker_busy(err: &LitecodeError) -> bool {
    matches!(err, LitecodeError::ToolExecution(msg) if msg == SEARCH_WORKER_BUSY)
}

#[derive(Default)]
struct DirtyFsQueue {
    paths: HashSet<(String, bool)>,
    reconcile: bool,
    stop: bool,
}

impl DirtyFsQueue {
    fn is_empty(&self) -> bool {
        self.paths.is_empty() && !self.reconcile
    }

    fn record_paths(&mut self, paths: &[String], deleted: bool) {
        if self.reconcile {
            return;
        }
        for p in paths {
            self.paths.insert((p.clone(), deleted));
        }
        if self.paths.len() > MAX_DIRTY_FS_PATHS {
            self.paths.clear();
            self.reconcile = true;
        }
    }

    fn record_reconcile(&mut self) {
        self.paths.clear();
        self.reconcile = true;
    }

    fn clear(&mut self) {
        self.paths.clear();
        self.reconcile = false;
    }
}

struct DirtyFsDispatcher {
    state: Mutex<DirtyFsQueue>,
    cv: Condvar,
}

impl DirtyFsDispatcher {
    fn new() -> Self {
        Self {
            state: Mutex::new(DirtyFsQueue::default()),
            cv: Condvar::new(),
        }
    }

    fn record_paths(&self, paths: &[String], deleted: bool) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.record_paths(paths, deleted);
        self.cv.notify_one();
    }

    fn record_reconcile(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.record_reconcile();
        self.cv.notify_one();
    }

    fn wake(&self) {
        self.cv.notify_one();
    }

    fn clear(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }

    fn stop(&self) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.stop = true;
        guard.clear();
        self.cv.notify_all();
    }
}

pub struct CodeSearchEngine {
    /// Multiplexed worker handle. The RwLock is ONLY held for nanoseconds to
    /// clone the `Arc` or install/remove a client — NEVER across IPC I/O.
    client: Arc<RwLock<Option<Arc<CodeSearchWorkerClient>>>>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    session_db_path: Arc<RwLock<Option<PathBuf>>>,
    warmup_epoch: AtomicU64,
    /// OS PID of the worker child. Set at spawn (before warmup finishes) so
    /// status-bar RSS can attribute embed memory while indexing is in progress.
    worker_os_pid: AtomicU32,
    on_worker_failed: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    /// Coalesced parent-side dirty set, flushed asynchronously off caller threads.
    dirty_fs: Arc<DirtyFsDispatcher>,
    flusher_started: AtomicBool,
    flusher_handle: Mutex<Option<JoinHandle<()>>>,
    /// Last ping-reported inference device (`cuda-ort` / `cpu-ort` / `hash`).
    embed_device: Mutex<String>,
    /// Test-only gate preserving `hold_client_for_test` semantics without
    /// bringing back a coarse IPC mutex.
    #[cfg(test)]
    search_test_gate: Mutex<()>,
}

impl CodeSearchEngine {
    pub fn new() -> Self {
        Self {
            client: Arc::new(RwLock::new(None)),
            workspace_root: Arc::new(RwLock::new(None)),
            session_db_path: Arc::new(RwLock::new(None)),
            warmup_epoch: AtomicU64::new(0),
            worker_os_pid: AtomicU32::new(0),
            on_worker_failed: Arc::new(Mutex::new(None)),
            dirty_fs: Arc::new(DirtyFsDispatcher::new()),
            flusher_started: AtomicBool::new(false),
            flusher_handle: Mutex::new(None),
            embed_device: Mutex::new(String::new()),
            #[cfg(test)]
            search_test_gate: Mutex::new(()),
        }
    }

    fn live_client(&self) -> Option<Arc<CodeSearchWorkerClient>> {
        self.client
            .read()
            .ok()
            .and_then(|g| g.as_ref().cloned())
            .filter(|c| c.is_alive())
    }

    fn ensure_flusher_started(&self) {
        if self
            .flusher_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let dirty_fs = Arc::clone(&self.dirty_fs);
        let client_slot = Arc::clone(&self.client);
        let handle = thread::Builder::new()
            .name("code-search-fs-flush".into())
            .spawn(move || run_dirty_flusher(dirty_fs, client_slot))
            .ok();
        if let Some(handle) = handle
            && let Ok(mut slot) = self.flusher_handle.lock()
        {
            *slot = Some(handle);
        }
    }

    fn set_embed_device(&self, device: &str) {
        if let Ok(mut guard) = self.embed_device.lock() {
            *guard = device.to_string();
        }
    }

    fn clear_embed_device(&self) {
        self.set_embed_device("");
    }

    fn capture_embed_device(&self, client: &CodeSearchWorkerClient) {
        match client.ping() {
            Ok(ping) => self.set_embed_device(&ping.embed_device),
            Err(e) => tracing::debug!(error = %e, "code_search ping embed_device skipped"),
        }
    }

    /// Live worker inference device, if the last ping reported one.
    pub fn embed_device(&self) -> Option<String> {
        let guard = self.embed_device.lock().ok()?;
        if guard.is_empty() {
            None
        } else {
            Some(guard.clone())
        }
    }

    fn set_worker_os_pid(&self, pid: Option<u32>) {
        self.worker_os_pid
            .store(pid.unwrap_or(0), Ordering::Release);
    }

    pub fn set_worker_failed_handler(&self, handler: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut guard) = self.on_worker_failed.lock() {
            *guard = Some(handler);
        }
    }

    fn notify_worker_failed(&self) {
        self.kill_client();
        if let Ok(guard) = self.on_worker_failed.lock()
            && let Some(handler) = guard.as_ref()
        {
            handler();
        }
    }

    pub fn set_workspace(&self, root: PathBuf) {
        if let Ok(mut guard) = self.workspace_root.write() {
            *guard = Some(crate::config::path::canon_abs_lossy(&root));
        }
    }

    pub fn set_session_reader(&self, reader: crate::session::SessionDataReader) {
        let path = reader.path().to_path_buf();
        if let Ok(mut guard) = self.session_db_path.write() {
            *guard = Some(path.clone());
        }
        self.inject_session_db(&path);
    }

    pub fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root.read().ok().and_then(|g| g.clone())
    }

    /// SessionData nod: store path and push it to a live worker without blocking.
    pub fn inject_session_db(&self, session_db_path: &std::path::Path) {
        if let Ok(mut guard) = self.session_db_path.write() {
            *guard = Some(session_db_path.to_path_buf());
        }
        let Some(client) = self.live_client() else {
            return;
        };
        if let Err(e) = client.set_session_db_async(session_db_path) {
            tracing::warn!(error = %e, "code_search set_session_db failed");
        }
    }

    fn warmup_still_valid(&self, epoch: u64) -> bool {
        self.warmup_epoch.load(Ordering::SeqCst) == epoch
    }

    fn kill_client(&self) {
        // Kill by OS pid first. Warmup holds the Child locally until the
        // worker RPC returns, so `client` is often still None while the
        // embedder is burning CPU.
        self.clear_embed_device();
        let pid = self.worker_os_pid.swap(0, Ordering::AcqRel);
        crate::serve::shutdown::kill_process(pid);
        let prev = self
            .client
            .write()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(client) = prev {
            client.kill();
        }
    }

    /// Code-corpus semantic search. Compatibility/direct form: never waits on test gate.
    pub fn search(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        self.search_with_queue(query, glob_filter, top_k, Duration::ZERO, None)
    }

    /// Agent code-surface form. Runs over the multiplexed IPC pipe so it never
    /// waits behind an entire refresh pass; still respects cancellation tokens.
    pub fn search_cancellable(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<SearchHit>> {
        self.search_with_queue(query, glob_filter, top_k, SEARCH_QUEUE_WAIT, Some(cancel))
    }

    #[cfg(test)]
    fn acquire_search_test_gate(
        &self,
        wait: Duration,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<std::sync::MutexGuard<'_, ()>> {
        let deadline = Instant::now().checked_add(wait);
        loop {
            if cancel.is_some_and(|token| token.is_cancelled()) {
                return Err(LitecodeError::ToolExecution(SEARCH_CANCELLED.into()));
            }
            match self.search_test_gate.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    let now = Instant::now();
                    let Some(deadline) = deadline.filter(|deadline| now < *deadline) else {
                        return Err(LitecodeError::ToolExecution(SEARCH_WORKER_BUSY.into()));
                    };
                    std::thread::sleep(
                        deadline
                            .saturating_duration_since(now)
                            .min(SEARCH_QUEUE_POLL),
                    );
                }
                Err(std::sync::TryLockError::Poisoned(err)) => {
                    return Err(LitecodeError::Config(format!(
                        "code_search test gate lock: {err}"
                    )));
                }
            }
        }
    }

    fn search_with_queue(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
        wait: Duration,
        cancel: Option<&tokio_util::sync::CancellationToken>,
    ) -> Result<Vec<SearchHit>> {
        #[cfg(test)]
        let _test_guard = self.acquire_search_test_gate(wait, cancel)?;

        if cancel.is_some_and(|token| token.is_cancelled()) {
            return Err(LitecodeError::ToolExecution(SEARCH_CANCELLED.into()));
        }

        let Some(client) = self.live_client() else {
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(WORKER_NOT_RUNNING.into()));
        };

        let timeout = if wait.is_zero() { None } else { Some(wait) };
        match client.search_cancellable(query, glob_filter, top_k, cancel, timeout) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                if cancel.is_some_and(|t| t.is_cancelled()) {
                    return Err(e);
                }
                tracing::warn!(tool = "code_search", error = %e, "worker search failed");
                if !client.is_alive() {
                    self.notify_worker_failed();
                }
                Err(e)
            }
        }
    }

    /// Session corpus ANN-only search (requires warmed worker; same lifecycle as code search).
    ///
    /// Multiplexed onto the worker's Query lane — never blocked by code refresh.
    pub fn search_sessions(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        #[cfg(test)]
        let _test_guard = match self.search_test_gate.try_lock() {
            Ok(g) => g,
            Err(_) => return Err(LitecodeError::ToolExecution(SEARCH_WORKER_BUSY.into())),
        };

        let Some(client) = self.live_client() else {
            return Err(LitecodeError::Config(
                "code_search engine not warmed".into(),
            ));
        };
        match client.session_search(query, top_k, session_id) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                tracing::warn!(tool = "session_search", error = %e, "worker session_search failed");
                if !client.is_alive() {
                    self.notify_worker_failed();
                }
                Err(e)
            }
        }
    }

    /// True when the worker child process is still running.
    ///
    /// O(1) lock-free atomic check — safe to call from any async or sync path
    /// without ever stalling behind a running refresh.
    pub fn worker_alive(&self) -> bool {
        self.live_client().is_some()
    }

    /// Test-only: hold the search test gate so a search waits or returns busy.
    #[cfg(test)]
    pub fn hold_client_for_test(&self) -> std::sync::MutexGuard<'_, ()> {
        self.search_test_gate
            .lock()
            .expect("code_search test gate lock")
    }

    /// PID of the code-search worker child, if running.
    ///
    /// Available from spawn through warmup/indexing (not only after Warm), so
    /// telemetry can show embed RSS while the worker is still building.
    pub fn worker_pid(&self) -> Option<u32> {
        let pid = self.worker_os_pid.load(Ordering::Acquire);
        (pid != 0).then_some(pid)
    }

    /// Refresh index while Warm: rebuild or incremental (worker decides).
    /// Human Refresh: both corpora. Agent tools use [`Self::refresh_scope`].
    pub fn refresh(&self) -> Result<RefreshResult> {
        self.refresh_scope(RefreshScope::All)
    }

    pub fn refresh_scope(&self, scope: RefreshScope) -> Result<RefreshResult> {
        let Some(client) = self.live_client() else {
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(
                "code_search worker is not running; start the retrieval engine first".into(),
            ));
        };
        match client.refresh_scope(scope) {
            Ok(result) => Ok(result),
            Err(e) => {
                tracing::warn!(tool = "code_search", error = %e, "worker refresh failed");
                if !client.is_alive() {
                    self.notify_worker_failed();
                }
                // Do not kill on non-fatal refresh errors — index may still be usable.
                Err(e)
            }
        }
    }

    /// Non-blocking, coalesced forward of workspace FS events to the worker.
    ///
    /// Records paths in a local dirty set and wakes the background flusher
    /// thread. Never blocks the caller (safe on Tokio async worker threads),
    /// never waits for `refresh`, and collapses extreme bursts into a single
    /// `reconcile_disk` notification.
    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) {
        if paths.is_empty() {
            return;
        }
        self.ensure_flusher_started();
        self.dirty_fs.record_paths(paths, deleted);
    }

    /// Non-blocking fire-and-forget disk↔index reconcile trigger (used after
    /// watcher lag, rescan, or excludes update).
    pub fn request_reconcile(&self) {
        self.ensure_flusher_started();
        self.dirty_fs.record_reconcile();
    }

    /// Blocking disk↔index reconcile over the multiplexed pipe (used only from
    /// `spawn_blocking` inside `code_index_tick` so `queue.json` reflects disk
    /// before deciding whether to spawn an incremental refresh).
    pub fn reconcile_disk_blocking(&self) {
        let Some(client) = self.live_client() else {
            return;
        };
        if let Err(e) = client.reconcile_disk() {
            tracing::debug!(error = %e, "code_search reconcile_disk failed");
        }
    }
}

fn run_dirty_flusher(
    dirty_fs: Arc<DirtyFsDispatcher>,
    client_slot: Arc<RwLock<Option<Arc<CodeSearchWorkerClient>>>>,
) {
    loop {
        // Wait until there is dirty work (or shutdown).
        {
            let mut guard = dirty_fs.state.lock().unwrap_or_else(|e| e.into_inner());
            while !guard.stop && guard.is_empty() {
                guard = dirty_fs.cv.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
            if guard.stop {
                return;
            }
        }

        // Short coalescing window so rapid FS bursts (e.g. Unity asset imports)
        // merge into a single one-way IPC frame.
        {
            let guard = dirty_fs.state.lock().unwrap_or_else(|e| e.into_inner());
            if !guard.stop && !guard.reconcile {
                let _ = dirty_fs
                    .cv
                    .wait_timeout(guard, DIRTY_FLUSH_DEBOUNCE)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }

        let client = client_slot
            .read()
            .ok()
            .and_then(|g| g.as_ref().cloned())
            .filter(|c| c.is_alive());

        let Some(client) = client else {
            // Worker not warm yet: keep paths buffered and wait until warmup
            // wakes us (or more events arrive / engine stops).
            let guard = dirty_fs.state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.stop {
                return;
            }
            let _ = dirty_fs
                .cv
                .wait_timeout(guard, Duration::from_millis(250))
                .unwrap_or_else(|e| e.into_inner());
            continue;
        };

        let (reconcile, drained): (bool, Vec<(String, bool)>) = {
            let mut guard = dirty_fs.state.lock().unwrap_or_else(|e| e.into_inner());
            if guard.stop {
                return;
            }
            let reconcile = guard.reconcile;
            guard.reconcile = false;
            let drained = guard.paths.drain().collect();
            (reconcile, drained)
        };

        if reconcile {
            if let Err(e) = client.reconcile_disk_async() {
                tracing::debug!(error = %e, "code_search async reconcile_disk failed");
                dirty_fs.record_reconcile();
            }
            continue;
        }

        if drained.is_empty() {
            continue;
        }
        let (deleted, modified): (Vec<_>, Vec<_>) = drained.into_iter().partition(|(_, d)| *d);
        if !deleted.is_empty() {
            let paths: Vec<String> = deleted.into_iter().map(|(p, _)| p).collect();
            if let Err(e) = client.notify_fs_changes_async(&paths, true) {
                tracing::debug!(error = %e, "code_search async notify_fs_changes (del) failed");
                dirty_fs.record_paths(&paths, true);
            }
        }
        if !modified.is_empty() {
            let paths: Vec<String> = modified.into_iter().map(|(p, _)| p).collect();
            if let Err(e) = client.notify_fs_changes_async(&paths, false) {
                tracing::debug!(error = %e, "code_search async notify_fs_changes (mod) failed");
                dirty_fs.record_paths(&paths, false);
            }
        }
    }
}

impl CodeSearchEngine {
    pub fn warmup(&self) -> Result<()> {
        let epoch = self.warmup_epoch.load(Ordering::SeqCst);

        let root = self
            .workspace_root
            .read()
            .map_err(|e| LitecodeError::Config(format!("workspace lock: {e}")))?
            .clone()
            .ok_or_else(|| LitecodeError::Config("code_search: workspace not set".into()))?;
        let session_db_path = self
            .session_db_path
            .read()
            .map_err(|e| LitecodeError::Config(format!("session reader lock: {e}")))?
            .clone();

        if !self.warmup_still_valid(epoch) {
            return Ok(());
        }

        self.kill_client();

        if !self.warmup_still_valid(epoch) {
            return Ok(());
        }

        let Some(client) = self.spawn_ready_worker(&root, session_db_path.as_deref(), epoch)?
        else {
            return Ok(());
        };
        if !self.warmup_still_valid(epoch) {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        // SessionData may have nodded during warmup (client not installed yet).
        let inject_path = self.session_db_path.read().ok().and_then(|g| g.clone());
        if let Some(ref path) = inject_path
            && let Err(e) = client.set_session_db(path)
        {
            tracing::warn!(error = %e, "code_search set_session_db after warmup skipped");
        }

        let client = Arc::new(client);
        // Wire asynchronous worker-death detection so external kills flip state
        // immediately without polling under a mutex.
        {
            let client_slot = Arc::clone(&self.client);
            let on_failed = Arc::clone(&self.on_worker_failed);
            let installed = Arc::downgrade(&client);
            client.set_on_exit(Arc::new(move || {
                let still_current = client_slot
                    .read()
                    .ok()
                    .and_then(|g| g.as_ref().cloned())
                    .is_some_and(|cur| {
                        installed
                            .upgrade()
                            .is_some_and(|inst| Arc::ptr_eq(&cur, &inst))
                    });
                if !still_current {
                    return;
                }
                if let Ok(mut guard) = client_slot.write()
                    && guard.as_ref().is_some_and(|cur| {
                        installed
                            .upgrade()
                            .is_some_and(|inst| Arc::ptr_eq(cur, &inst))
                    })
                {
                    *guard = None;
                }
                if let Ok(guard) = on_failed.lock()
                    && let Some(handler) = guard.as_ref()
                {
                    handler();
                }
            }));
        }

        if let Ok(mut guard) = self.client.write() {
            // Slot-time epoch re-check: `stop()` may have bumped the epoch and
            // killed the old client between our last check and acquiring this
            // lock. Never install a client that belongs to a stale warmup.
            if !self.warmup_still_valid(epoch) {
                drop(guard);
                client.kill();
                self.set_worker_os_pid(None);
                return Ok(());
            }
            *guard = Some(client);
        } else {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        // Wake the dirty flusher so any FS events buffered before warmup drain now.
        self.ensure_flusher_started();
        self.dirty_fs.wake();

        Ok(())
    }

    fn handshake_worker(
        &self,
        client: &CodeSearchWorkerClient,
        root: &std::path::Path,
        session_db_path: Option<&std::path::Path>,
        epoch: u64,
    ) -> Result<bool> {
        client.initialize(root, session_db_path)?;
        if !self.warmup_still_valid(epoch) {
            return Ok(false);
        }
        client.warmup()?;
        if !self.warmup_still_valid(epoch) {
            return Ok(false);
        }
        self.capture_embed_device(client);
        Ok(true)
    }

    fn spawn_ready_worker(
        &self,
        root: &std::path::Path,
        session_db_path: Option<&std::path::Path>,
        epoch: u64,
    ) -> Result<Option<CodeSearchWorkerClient>> {
        let mut force_cpu_ort = false;
        loop {
            let client = if force_cpu_ort {
                CodeSearchWorkerClient::spawn_cpu_ort()?
            } else {
                CodeSearchWorkerClient::spawn()?
            };
            self.set_worker_os_pid(client.pid());
            if !self.warmup_still_valid(epoch) {
                client.kill();
                self.set_worker_os_pid(None);
                return Ok(None);
            }
            match self.handshake_worker(&client, root, session_db_path, epoch) {
                Ok(true) => {
                    if force_cpu_ort {
                        tracing::info!("code_search worker recovered on CPU EP");
                    }
                    return Ok(Some(client));
                }
                Ok(false) => {
                    client.kill();
                    self.set_worker_os_pid(None);
                    return Ok(None);
                }
                Err(e) => {
                    let died = handshake_failure_is_worker_death(&client, &e);
                    client.kill();
                    self.set_worker_os_pid(None);
                    if died && !force_cpu_ort && self.warmup_still_valid(epoch) {
                        tracing::warn!(
                            error = %e,
                            "code_search worker died during warmup; retrying with CPU EP"
                        );
                        force_cpu_ort = true;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
    }

    pub fn stop(&self) {
        self.warmup_epoch.fetch_add(1, Ordering::SeqCst);
        self.kill_client();
        self.dirty_fs.clear();
        if let Ok(mut root) = self.workspace_root.write() {
            *root = None;
        }
        crate::telemetry::release_heap_to_os();
    }
}

impl Drop for CodeSearchEngine {
    fn drop(&mut self) {
        self.dirty_fs.stop();
        if let Ok(mut slot) = self.flusher_handle.lock()
            && let Some(handle) = slot.take()
        {
            let _ = handle.join();
        }
    }
}

/// True when warmup IPC failed because the child is gone (CUDA native abort,
/// delay-load crash, stdout EOF). JSON-RPC errors from a live worker are false.
fn handshake_failure_is_worker_death(
    client: &CodeSearchWorkerClient,
    err: &LitecodeError,
) -> bool {
    if !client.is_alive() || !matches!(client.try_wait(), Ok(None)) {
        return true;
    }
    handshake_error_looks_like_death(err)
}

fn handshake_error_looks_like_death(err: &LitecodeError) -> bool {
    match err {
        LitecodeError::Io(e) if e.kind() == std::io::ErrorKind::BrokenPipe => true,
        LitecodeError::ToolExecution(msg) if msg.contains("closed stdout") => true,
        other => {
            let msg = other.to_string();
            msg.contains("closed stdout")
                || msg.contains("Broken pipe")
                || msg.contains("broken pipe")
        }
    }
}

impl Default for CodeSearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_fs_queue_coalesces_duplicates_and_collapses_overflow_to_reconcile() {
        let mut q = DirtyFsQueue::default();
        q.record_paths(&["a.rs".into(), "a.rs".into(), "b.rs".into()], false);
        assert_eq!(q.paths.len(), 2);
        assert!(!q.reconcile);

        let burst: Vec<String> = (0..=MAX_DIRTY_FS_PATHS)
            .map(|i| format!("f{i}.rs"))
            .collect();
        q.record_paths(&burst, false);
        assert!(q.reconcile, "overflow must collapse to reconcile");
        assert!(q.paths.is_empty(), "paths cleared after collapse");

        // Subsequent paths remain folded under reconcile.
        q.record_paths(&["after.rs".into()], false);
        assert!(q.reconcile);
        assert!(q.paths.is_empty());
    }

    #[test]
    fn notify_and_worker_alive_never_block_without_worker() {
        let engine = CodeSearchEngine::new();
        let started = Instant::now();
        assert!(!engine.worker_alive());
        engine.notify_fs_changes(&["a.rs".into(), "b.rs".into()], false);
        engine.request_reconcile();
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "notify and worker_alive must be O(1) non-blocking"
        );
    }
}
