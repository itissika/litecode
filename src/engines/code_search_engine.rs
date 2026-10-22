use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::engines::code_search::SearchHit;
use crate::engines::code_search_ipc::CodeSearchWorkerClient;
use crate::engines::code_search_ipc::protocol::{RefreshResult, RefreshScope};
use crate::types::{LitecodeError, Result};

/// A code or session read found the worker client lock already held.
///
/// Refresh/embed keeps that lock for the whole pass. Callers must return
/// immediately; blocking here stalls the agent turn until embedding finishes.
pub const SEARCH_WORKER_BUSY: &str = "code_search worker is busy; semantic search was skipped";

const WORKER_NOT_RUNNING: &str = "code_search worker is not running; enable the retrieval engine or restart litecode with this workspace";

pub fn is_search_worker_busy(err: &LitecodeError) -> bool {
    matches!(err, LitecodeError::ToolExecution(msg) if msg == SEARCH_WORKER_BUSY)
}

pub struct CodeSearchEngine {
    client: Mutex<Option<CodeSearchWorkerClient>>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    session_db_path: Arc<RwLock<Option<PathBuf>>>,
    warmup_epoch: AtomicU64,
    /// OS PID of the worker child. Set at spawn (before warmup finishes) so
    /// status-bar RSS can attribute embed memory while indexing is in progress.
    worker_os_pid: AtomicU32,
    on_worker_failed: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// FS events received before the worker is warm; flushed after warmup.
    pending_fs: Mutex<HashSet<(String, bool)>>,
    /// Last ping-reported inference device (`cuda-ort` / `cpu-ort` / `hash`).
    embed_device: Mutex<String>,
}

impl CodeSearchEngine {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            workspace_root: Arc::new(RwLock::new(None)),
            session_db_path: Arc::new(RwLock::new(None)),
            warmup_epoch: AtomicU64::new(0),
            worker_os_pid: AtomicU32::new(0),
            on_worker_failed: Mutex::new(None),
            pending_fs: Mutex::new(HashSet::new()),
            embed_device: Mutex::new(String::new()),
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

    fn capture_embed_device(&self, client: &mut CodeSearchWorkerClient) {
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

    /// SessionData nod: store path and push it to a live worker without restarting.
    pub fn inject_session_db(&self, session_db_path: &std::path::Path) {
        if let Ok(mut guard) = self.session_db_path.write() {
            *guard = Some(session_db_path.to_path_buf());
        }
        let Ok(mut guard) = self.client.lock() else {
            return;
        };
        let Some(client) = guard.as_mut() else {
            return;
        };
        if let Err(e) = client.set_session_db(session_db_path) {
            tracing::warn!(error = %e, "code_search set_session_db failed");
        }
    }

    fn warmup_still_valid(&self, epoch: u64) -> bool {
        self.warmup_epoch.load(Ordering::SeqCst) == epoch
    }

    fn kill_client(&self) {
        // Kill by OS pid first. Warmup holds the Child locally until the
        // worker RPC returns, so `client` is often still None while the
        // embedder is burning CPU. A graceful shutdown RPC also cannot run
        // until that in-flight request finishes — wait and the process leaks.
        self.clear_embed_device();
        let pid = self.worker_os_pid.swap(0, Ordering::AcqRel);
        crate::serve::shutdown::kill_process(pid);
        if let Ok(mut guard) = self.client.try_lock()
            && let Some(mut client) = guard.take()
        {
            client.kill();
        }
    }

    /// Code-corpus semantic search.
    ///
    /// Never waits on the worker. A refresh holds the client lock for its whole
    /// embed pass; blocking behind it is the stall [`SEARCH_WORKER_BUSY`] exists
    /// to avoid. A dead or missing worker is still reported as not running.
    pub fn search(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        let mut guard = match self.client.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(LitecodeError::ToolExecution(SEARCH_WORKER_BUSY.into()));
            }
            Err(std::sync::TryLockError::Poisoned(err)) => {
                return Err(LitecodeError::Config(format!(
                    "code_search client lock: {err}"
                )));
            }
        };
        let Some(client) = guard.as_mut() else {
            drop(guard);
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(WORKER_NOT_RUNNING.into()));
        };
        if !matches!(client.try_wait(), Ok(None)) {
            drop(guard);
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(WORKER_NOT_RUNNING.into()));
        }
        match client.search(query, glob_filter, top_k) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                tracing::warn!(tool = "code_search", error = %e, "worker search failed");
                drop(guard);
                if self.worker_gone_try() {
                    self.notify_worker_failed();
                }
                Err(e)
            }
        }
    }

    /// Session corpus ANN-only search (requires warmed worker; same lifecycle as code search).
    ///
    /// Reads never wait on the worker: a refresh holds the client lock for its
    /// whole pass, and a search queued behind one would be the very stall this
    /// lane avoids. When the lock is taken the lane is skipped and the caller
    /// degrades to text — the same answer a cold worker gets.
    pub fn search_sessions(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        let Ok(mut guard) = self.client.try_lock() else {
            return Err(LitecodeError::ToolExecution(SEARCH_WORKER_BUSY.into()));
        };
        let Some(client) = guard.as_mut() else {
            return Err(LitecodeError::Config(
                "code_search engine not warmed".into(),
            ));
        };
        match client.session_search(query, top_k, session_id) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                tracing::warn!(tool = "session_search", error = %e, "worker session_search failed");
                drop(guard);
                if self.worker_gone_try() {
                    self.notify_worker_failed();
                }
                Err(e)
            }
        }
    }

    /// Whether the worker child is known to be gone, answered without waiting on
    /// the client lock: a held lock means a call is in flight (the refresh that
    /// must not stall us), not a dead worker.
    fn worker_gone_try(&self) -> bool {
        let Ok(mut guard) = self.client.try_lock() else {
            return false;
        };
        guard
            .as_mut()
            .is_some_and(|client| !matches!(client.try_wait(), Ok(None)))
    }

    /// True when the worker child process is still running.
    pub fn worker_alive(&self) -> bool {
        let Ok(mut guard) = self.client.lock() else {
            return false;
        };
        let Some(client) = guard.as_mut() else {
            return false;
        };
        match client.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    /// Test-only: hold the worker client lock so a search must return busy.
    #[cfg(test)]
    pub fn hold_client_for_test(
        &self,
    ) -> std::sync::MutexGuard<'_, Option<CodeSearchWorkerClient>> {
        self.client.lock().expect("code_search client lock")
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
        if !self.worker_alive() {
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(
                "code_search worker is not running; start the retrieval engine first".into(),
            ));
        }

        let mut guard = self
            .client
            .lock()
            .map_err(|e| LitecodeError::Config(format!("code_search client lock: {e}")))?;
        let client = guard
            .as_mut()
            .ok_or_else(|| LitecodeError::Config("code_search engine not warmed".into()))?;
        match client.refresh_scope(scope) {
            Ok(result) => Ok(result),
            Err(e) => {
                tracing::warn!(tool = "code_search", error = %e, "worker refresh failed");
                // Do not kill on refresh errors — index may still be usable.
                Err(e)
            }
        }
    }

    /// Best-effort forward of workspace FS events to the worker Index queue.
    ///
    /// If the worker is not yet warm, paths are buffered and flushed after warmup.
    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) {
        if paths.is_empty() {
            return;
        }
        if !self.worker_alive() {
            if let Ok(mut pending) = self.pending_fs.lock() {
                for p in paths {
                    pending.insert((p.clone(), deleted));
                }
            }
            return;
        }
        let Ok(mut guard) = self.client.lock() else {
            return;
        };
        let Some(client) = guard.as_mut() else {
            return;
        };
        if let Err(e) = client.notify_fs_changes(paths, deleted) {
            tracing::debug!(error = %e, "code_search notify_fs_changes failed");
            // Buffer on transport failure so a later reconcile/warmup can recover.
            drop(guard);
            if let Ok(mut pending) = self.pending_fs.lock() {
                for p in paths {
                    pending.insert((p.clone(), deleted));
                }
            }
        }
    }

    /// After broadcast lag, force a disk↔index reconcile on the worker.
    pub fn request_reconcile(&self) {
        if !self.worker_alive() {
            return;
        }
        let Ok(mut guard) = self.client.lock() else {
            return;
        };
        let Some(client) = guard.as_mut() else {
            return;
        };
        if let Err(e) = client.reconcile_disk() {
            tracing::debug!(error = %e, "code_search reconcile_disk failed");
        }
    }

    fn flush_pending_fs(&self, client: &mut CodeSearchWorkerClient) {
        let pending: Vec<(String, bool)> = match self.pending_fs.lock() {
            Ok(mut g) => g.drain().collect(),
            Err(_) => return,
        };
        if pending.is_empty() {
            return;
        }
        let (deleted, modified): (Vec<_>, Vec<_>) = pending.into_iter().partition(|(_, d)| *d);
        if !deleted.is_empty() {
            let paths: Vec<String> = deleted.into_iter().map(|(p, _)| p).collect();
            let _ = client.notify_fs_changes(&paths, true);
        }
        if !modified.is_empty() {
            let paths: Vec<String> = modified.into_iter().map(|(p, _)| p).collect();
            let _ = client.notify_fs_changes(&paths, false);
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

        let Some(mut client) = self.spawn_ready_worker(&root, session_db_path.as_deref(), epoch)?
        else {
            return Ok(());
        };
        if !self.warmup_still_valid(epoch) {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        self.flush_pending_fs(&mut client);

        // SessionData may have nodded during warmup (client not installed yet).
        let inject_path = self.session_db_path.read().ok().and_then(|g| g.clone());
        if let Some(ref path) = inject_path
            && let Err(e) = client.set_session_db(path)
        {
            tracing::warn!(error = %e, "code_search set_session_db after warmup skipped");
        }

        if let Ok(mut guard) = self.client.lock() {
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
        }

        Ok(())
    }

    fn handshake_worker(
        &self,
        client: &mut CodeSearchWorkerClient,
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
            let mut client = if force_cpu_ort {
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
            match self.handshake_worker(&mut client, root, session_db_path, epoch) {
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
                    let died = handshake_failure_is_worker_death(&mut client, &e);
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
        if let Ok(mut pending) = self.pending_fs.lock() {
            pending.clear();
        }
        if let Ok(mut root) = self.workspace_root.write() {
            *root = None;
        }
        crate::telemetry::release_heap_to_os();
    }
}

/// True when warmup IPC failed because the child is gone (CUDA native abort,
/// delay-load crash, stdout EOF). JSON-RPC errors from a live worker are false.
fn handshake_failure_is_worker_death(
    client: &mut CodeSearchWorkerClient,
    err: &LitecodeError,
) -> bool {
    if !matches!(client.try_wait(), Ok(None)) {
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
    
    use std::process::{Command, Stdio};
    
    use std::time::{Duration, Instant};
    

    fn spawn_stub_child() -> std::process::Child {
        #[cfg(windows)]
        {
            Command::new("ping")
                .args(["-t", "127.0.0.1"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("ping")
        }
        #[cfg(unix)]
        {
            Command::new("sleep")
                .arg("60")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("sleep")
        }
    }

    fn wait_until_dead(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !crate::serve::shutdown::is_process_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        !crate::serve::shutdown::is_process_alive(pid)
    }

    #[test]
    fn stop_kills_os_process_when_client_not_installed() {
        // Warmup holds the Child locally until the worker RPC returns. Turning
        // the engine off must still kill that OS process; worker_alive() is
        // false whenever `client` is None, so it cannot catch this leak.
        let mut child = spawn_stub_child();
        let pid = child.id();
        assert!(
            crate::serve::shutdown::is_process_alive(pid),
            "stub child {pid} should be running"
        );

        let engine = CodeSearchEngine::new();
        engine.set_worker_os_pid(Some(pid));
        engine.stop();
        // Reap before liveness probe: on Unix kill(0) still succeeds for zombies
        // held by this test's Child (ISSUE-009). SIGKILL is delivered above but
        // the exit is asynchronous: a single try_wait() can run before the child
        // has exited, leave it unreaped, and the zombie then reads as alive
        // (flake, ISSUE-049 notes). Poll the reap within the same 2 s budget.
        let reap_deadline = Instant::now() + Duration::from_secs(2);
        while !matches!(child.try_wait(), Ok(Some(_))) && Instant::now() < reap_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            wait_until_dead(pid, Duration::from_secs(2)),
            "code-search worker pid {pid} still alive after engine stop"
        );
    }

    #[test]
    fn search_returns_busy_without_waiting_when_client_lock_held() {
        let engine = CodeSearchEngine::new();
        let _hold = engine.hold_client_for_test();
        let started = Instant::now();
        let err = engine
            .search("auth", None, 5)
            .expect_err("held client lock");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "search must not wait on an in-flight embed"
        );
        assert!(is_search_worker_busy(&err), "{err}");
    }

}
