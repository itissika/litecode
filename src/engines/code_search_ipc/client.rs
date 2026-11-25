//! Parent-process client: spawn worker, multiplexed JSON-RPC 2.0 over
//! newline-delimited stdin/stdout.
//!
//! Architecture:
//! - Dedicated writer thread (`code-search-ipc-tx`) owns `ChildStdin` and
//!   flushes outbound requests & one-way notifications without callers holding
//!   any lock across pipe I/O.
//! - Dedicated reader thread (`code-search-ipc-rx`) owns `BufReader<ChildStdout>`,
//!   routes out-of-order responses by `id`, marks `alive = false` immediately on
//!   EOF/error, and fails all pending waiters.
//! - All RPC/notification methods take `&self`, so `search`, `refresh`, `ping`,
//!   and `notify_fs_changes` never contend on a shared client mutex.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::protocol::{
    InitializeParams, JsonRpcRequest, JsonRpcResponse, NotifyFsChangesParams, PingResult,
    RefreshParams, RefreshResult, RefreshScope, SearchParams, SearchResult, SessionSearchParams,
    SessionSearchResult, SetSessionDbParams,
};
use crate::engines::code_search::SearchHit;
use crate::engines::code_search_engine::SEARCH_CANCELLED;
use crate::engines::session_search::SessionTextHit;
use crate::types::{LitecodeError, Result};

const CANCEL_POLL: Duration = Duration::from_millis(20);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

type PendingMap = Mutex<HashMap<u64, mpsc::Sender<Result<Value>>>>;
type ExitCallback = Arc<dyn Fn() + Send + Sync>;

pub struct CodeSearchWorkerClient {
    child: Mutex<Child>,
    pid: u32,
    tx: mpsc::Sender<String>,
    pending: Arc<PendingMap>,
    alive: Arc<AtomicBool>,
    next_id: AtomicU64,
    on_exit: Arc<Mutex<Option<ExitCallback>>>,
}

fn self_worker_binary() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_litecode") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe()
        .map_err(|e| LitecodeError::ToolExecution(format!("code_search worker: current_exe: {e}")))
}

fn fail_all_pending(pending: &PendingMap, message: &str) {
    let waiters: Vec<mpsc::Sender<Result<Value>>> = match pending.lock() {
        Ok(mut map) => map.drain().map(|(_, tx)| tx).collect(),
        Err(poisoned) => poisoned.into_inner().drain().map(|(_, tx)| tx).collect(),
    };
    for tx in waiters {
        let _ = tx.send(Err(LitecodeError::ToolExecution(message.into())));
    }
}

fn fire_exit(alive: &AtomicBool, on_exit: &Mutex<Option<ExitCallback>>) {
    if alive.swap(false, Ordering::AcqRel)
        && let Ok(guard) = on_exit.lock()
        && let Some(cb) = guard.as_ref()
    {
        cb();
    }
}

fn spawn_exe(exe: &Path, force_cpu_ort: bool) -> Result<CodeSearchWorkerClient> {
    let mut cmd = Command::new(exe);
    cmd.arg("code-search-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if force_cpu_ort {
        cmd.env("LITECODE_ORT_FORCE_CPU", "1");
    }
    let mut child = cmd.spawn().map_err(|e| {
        LitecodeError::ToolExecution(format!(
            "code_search worker spawn failed ({}): {e}",
            exe.display()
        ))
    })?;
    let pid = child.id();
    // Kernel-level tie so the worker cannot outlive this process (Windows
    // Job Object; no-op elsewhere).
    crate::proc_lifetime::bind_child_to_parent(pid);

    let mut stdin = child.stdin.take().expect("worker stdin piped");
    let stdout = child.stdout.take().expect("worker stdout piped");

    let (tx, rx) = mpsc::channel::<String>();
    let pending: Arc<PendingMap> = Arc::new(Mutex::new(HashMap::new()));
    let alive = Arc::new(AtomicBool::new(true));
    let on_exit: Arc<Mutex<Option<ExitCallback>>> = Arc::new(Mutex::new(None));

    // Writer thread: drains outbound JSON-RPC lines to child stdin.
    {
        let pending = Arc::clone(&pending);
        let alive = Arc::clone(&alive);
        let on_exit = Arc::clone(&on_exit);
        let _ = thread::Builder::new()
            .name("code-search-ipc-tx".into())
            .spawn(move || {
                while let Ok(line) = rx.recv() {
                    if stdin.write_all(line.as_bytes()).is_err() || stdin.flush().is_err() {
                        fail_all_pending(&pending, "code_search worker broken pipe");
                        fire_exit(&alive, &on_exit);
                        return;
                    }
                }
            });
    }

    // Reader thread: demultiplexes out-of-order responses by id.
    {
        let pending = Arc::clone(&pending);
        let alive = Arc::clone(&alive);
        let on_exit = Arc::clone(&on_exit);
        let _ = thread::Builder::new()
            .name("code-search-ipc-rx".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                let mut response_line = String::new();
                loop {
                    response_line.clear();
                    match reader.read_line(&mut response_line) {
                        Ok(0) | Err(_) => {
                            fail_all_pending(&pending, "code_search worker closed stdout");
                            fire_exit(&alive, &on_exit);
                            return;
                        }
                        Ok(_) => {
                            let trimmed = response_line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            let resp: JsonRpcResponse = match serde_json::from_str(trimmed) {
                                Ok(r) => r,
                                Err(e) => {
                                    tracing::warn!(error = %e, "code_search worker malformed response");
                                    continue;
                                }
                            };
                            let waiter = pending
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .remove(&resp.id);
                            if let Some(waiter) = waiter {
                                let outcome = if let Some(err) = resp.error {
                                    Err(LitecodeError::ToolExecution(format!(
                                        "code_search worker error ({}): {}",
                                        err.code, err.message
                                    )))
                                } else {
                                    Ok(resp.result.unwrap_or(Value::Null))
                                };
                                let _ = waiter.send(outcome);
                            }
                        }
                    }
                }
            });
    }

    Ok(CodeSearchWorkerClient {
        child: Mutex::new(child),
        pid,
        tx,
        pending,
        alive,
        next_id: AtomicU64::new(1),
        on_exit,
    })
}

impl CodeSearchWorkerClient {
    pub fn spawn() -> Result<Self> {
        spawn_exe(&self_worker_binary()?, false)
    }

    /// Second-chance spawn: skip CUDA EP so a native abort cannot repeat.
    pub fn spawn_cpu_ort() -> Result<Self> {
        spawn_exe(&self_worker_binary()?, true)
    }

    pub fn set_on_exit(&self, handler: ExitCallback) {
        if let Ok(mut guard) = self.on_exit.lock() {
            *guard = Some(handler);
        }
    }

    /// Lock-free liveness check driven by the IPC reader/writer threads.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        if !self.is_alive() {
            return Err(LitecodeError::ToolExecution(
                "code_search worker closed stdout".into(),
            ));
        }
        let req = JsonRpcRequest::notification(method, params);
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.tx
            .send(line)
            .map_err(|_| LitecodeError::ToolExecution("code_search worker broken pipe".into()))
    }

    fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_with(method, params, None, None)
    }

    fn request_with(
        &self,
        method: &str,
        params: Value,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        if !self.is_alive() {
            return Err(LitecodeError::ToolExecution(
                "code_search worker closed stdout".into(),
            ));
        }
        let id = self.next_id();
        let req = JsonRpcRequest::request(id, method, params);
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');

        let (reply_tx, reply_rx) = mpsc::channel();
        {
            let mut map = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !self.is_alive() {
                return Err(LitecodeError::ToolExecution(
                    "code_search worker closed stdout".into(),
                ));
            }
            map.insert(id, reply_tx);
        }

        if self.tx.send(line).is_err() {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(LitecodeError::ToolExecution(
                "code_search worker broken pipe".into(),
            ));
        }

        if cancel.is_none() && timeout.is_none() {
            return reply_rx.recv().map_err(|_| {
                LitecodeError::ToolExecution("code_search worker closed stdout".into())
            })?;
        }

        let deadline = timeout.and_then(|d| Instant::now().checked_add(d));
        loop {
            if cancel.is_some_and(|t| t.is_cancelled()) {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(LitecodeError::ToolExecution(SEARCH_CANCELLED.into()));
            }
            let step = match deadline {
                Some(dl) => {
                    let now = Instant::now();
                    if now >= dl {
                        self.pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&id);
                        return Err(LitecodeError::ToolExecution(format!(
                            "code_search worker timeout on {method}"
                        )));
                    }
                    dl.saturating_duration_since(now).min(CANCEL_POLL)
                }
                None => CANCEL_POLL,
            };
            match reply_rx.recv_timeout(step) {
                Ok(res) => return res,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(LitecodeError::ToolExecution(
                        "code_search worker closed stdout".into(),
                    ));
                }
            }
        }
    }

    pub fn initialize(
        &self,
        workspace_root: &std::path::Path,
        session_db_path: Option<&std::path::Path>,
    ) -> Result<()> {
        let params = serde_json::to_value(InitializeParams {
            workspace_root: workspace_root.to_string_lossy().into_owned(),
            session_db_path: session_db_path.map(|p| p.to_string_lossy().into_owned()),
        })?;
        self.request("initialize", params)?;
        Ok(())
    }

    pub fn set_session_db(&self, session_db_path: &std::path::Path) -> Result<()> {
        let params = serde_json::to_value(SetSessionDbParams {
            session_db_path: session_db_path.to_string_lossy().into_owned(),
        })?;
        self.request("set_session_db", params)?;
        Ok(())
    }

    /// Fire-and-forget session DB injection (never blocks caller behind maintenance).
    pub fn set_session_db_async(&self, session_db_path: &std::path::Path) -> Result<()> {
        let params = serde_json::to_value(SetSessionDbParams {
            session_db_path: session_db_path.to_string_lossy().into_owned(),
        })?;
        self.send_notification("set_session_db", params)
    }

    pub fn warmup(&self) -> Result<()> {
        self.request("warmup", serde_json::json!({}))?;
        Ok(())
    }

    pub fn ping(&self) -> Result<PingResult> {
        let result = self.request("ping", serde_json::json!({}))?;
        Ok(serde_json::from_value(result)?)
    }

    pub fn search(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        self.search_cancellable(query, glob_filter, top_k, None, None)
    }

    pub fn search_cancellable(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
        cancel: Option<&tokio_util::sync::CancellationToken>,
        timeout: Option<Duration>,
    ) -> Result<Vec<SearchHit>> {
        let params = serde_json::to_value(SearchParams {
            query: query.into(),
            glob: glob_filter.map(str::to_string),
            top_k,
        })?;
        let result = self.request_with("search", params, cancel, timeout)?;
        let parsed: SearchResult = serde_json::from_value(result)?;
        Ok(parsed.hits)
    }

    pub fn session_search(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<SessionTextHit>> {
        let params = serde_json::to_value(SessionSearchParams {
            query: query.into(),
            top_k,
            session_id: session_id.map(str::to_string),
        })?;
        let result = self.request("session_search", params)?;
        let parsed: SessionSearchResult = serde_json::from_value(result)?;
        Ok(parsed.hits)
    }

    pub fn refresh(&self) -> Result<RefreshResult> {
        self.refresh_scope(RefreshScope::All)
    }

    pub fn refresh_scope(&self, scope: RefreshScope) -> Result<RefreshResult> {
        let params = serde_json::to_value(RefreshParams { scope })?;
        let result = self.request("refresh", params)?;
        let parsed: RefreshResult = serde_json::from_value(result)?;
        Ok(parsed)
    }

    /// Forward workspace FS changes and wait for the worker notify lane to persist `queue.json`.
    pub fn notify_fs_changes(&self, paths: &[String], deleted: bool) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let params = serde_json::to_value(NotifyFsChangesParams {
            paths: paths.to_vec(),
            deleted,
        })?;
        self.request("notify_fs_changes", params)?;
        Ok(())
    }

    /// Fire-and-forget one-way FS change notification (zero roundtrip).
    pub fn notify_fs_changes_async(&self, paths: &[String], deleted: bool) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let params = serde_json::to_value(NotifyFsChangesParams {
            paths: paths.to_vec(),
            deleted,
        })?;
        self.send_notification("notify_fs_changes", params)
    }

    /// Force disk↔index dirty reconcile and wait for completion (used by `code_index_tick`).
    pub fn reconcile_disk(&self) -> Result<()> {
        self.request("reconcile_disk", serde_json::json!({}))?;
        Ok(())
    }

    /// Fire-and-forget disk↔index reconcile trigger (coalesced on worker).
    pub fn reconcile_disk_async(&self) -> Result<()> {
        self.send_notification("reconcile_disk", serde_json::json!({}))
    }

    pub fn shutdown(&self) -> Result<()> {
        let _ = self.request_with(
            "shutdown",
            serde_json::json!({}),
            None,
            Some(SHUTDOWN_TIMEOUT),
        );
        Ok(())
    }

    pub fn kill(&self) {
        self.alive.store(false, Ordering::Release);
        fail_all_pending(&self.pending, "code_search worker killed");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn try_wait(&self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let status = child.try_wait()?;
        if status.is_some() {
            self.alive.store(false, Ordering::Release);
        }
        Ok(status)
    }

    pub fn pid(&self) -> Option<u32> {
        Some(self.pid)
    }
}

impl Drop for CodeSearchWorkerClient {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        fail_all_pending(&self.pending, "code_search worker dropped");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}
