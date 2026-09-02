//! Worker-side request dispatch (single-threaded stdin loop).

use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use super::protocol::{
    InitializeParams, JsonRpcRequest, JsonRpcResponse, NotifyFsChangesParams, RefreshMode,
    RefreshResult, SearchParams, SearchResult, SessionSearchParams, SessionSearchResult,
    SetSessionDbParams,
};
use crate::engines::code_search::{
    CodeSearchRuntime, INDEX_RECONCILE_INTERVAL, SemanticEngine, SharedRuntime,
    init_workspace_index, open_production_embedder, queue_fs_changes, rebuild_index_in_runtime,
    refresh_index_incremental, should_full_rebuild, sync_index_with_disk, warmup_index,
};
use crate::types::{LitecodeError, Result};

const ERR_INVALID: i32 = -32602;
const ERR_INTERNAL: i32 = -32000;

fn reload_worker_excludes(root: &std::path::Path) {
    let _ = crate::workspace::filter::reload_workspace_excludes_from_disk(root);
}

struct WorkerState {
    workspace_root: Option<PathBuf>,
    session_reader: Option<crate::session::SessionDataReader>,
    runtime: SharedRuntime,
    warmed: bool,
    last_reconcile: Instant,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            workspace_root: None,
            session_reader: None,
            runtime: Arc::new(RwLock::new(None)),
            warmed: false,
            last_reconcile: Instant::now(),
        }
    }

    fn initialize(&mut self, root: PathBuf, session_db_path: Option<PathBuf>) -> Result<()> {
        let root = crate::config::path::canon_abs_lossy(&root);
        if self
            .workspace_root
            .as_ref()
            .is_some_and(|w| crate::config::path::canon_abs_lossy(w) == root)
        {
            if let Some(path) = session_db_path {
                self.set_session_db(path)?;
            }
            return Ok(());
        }
        *self.runtime.write().unwrap() = None;
        self.warmed = false;
        self.workspace_root = Some(root);
        self.session_reader = session_db_path.map(|path| {
            crate::session::SessionDataReader::from_worker_config(
                crate::session::data::SessionDataReaderConfig::from_path(path),
            )
        });
        Ok(())
    }

    fn set_session_db(&mut self, session_db_path: PathBuf) -> Result<()> {
        let reader = crate::session::SessionDataReader::from_worker_config(
            crate::session::data::SessionDataReaderConfig::from_path(session_db_path),
        );
        self.session_reader = Some(reader.clone());
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

    fn warmup(&mut self) -> Result<()> {
        let root = self
            .workspace_root
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("initialize required".into()))?;

        reload_worker_excludes(&root);
        init_workspace_index(&root)?;
        let mut embedder = open_production_embedder()?;
        let index = warmup_index(&root, &mut *embedder)?;
        let runtime = CodeSearchRuntime::new(
            root.clone(),
            index,
            Some(embedder),
            self.session_reader.clone(),
        );
        reload_worker_excludes(&root);
        sync_index_with_disk(&runtime);
        if self.session_reader.is_some()
            && let Err(e) = runtime.ensure_session_index()
        {
            tracing::warn!(error = %e, "session semantic index warmup skipped");
        }
        runtime.drop_embedder_for_cool();
        *self.runtime.write().unwrap() = Some(runtime);
        self.warmed = true;
        self.last_reconcile = Instant::now();
        crate::telemetry::release_heap_to_os();
        Ok(())
    }

    fn search(
        &self,
        query: &str,
        glob: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<crate::engines::code_search::SearchHit>> {
        if !self.warmed {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let guard = self.runtime.read().unwrap();
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
        if !self.warmed {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let guard = self.runtime.read().unwrap();
        let runtime = guard
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("runtime missing".into()))?;
        runtime.search_sessions(query, top_k, session_id)
    }

    /// Refresh index: full rebuild when incompatible/absent, else incremental sync.
    fn refresh(&mut self) -> Result<RefreshResult> {
        if !self.warmed {
            return Err(LitecodeError::ToolExecution(
                "code_search worker not warmed".into(),
            ));
        }
        let root = self
            .workspace_root
            .clone()
            .ok_or_else(|| LitecodeError::ToolExecution("initialize required".into()))?;
        reload_worker_excludes(&root);
        let rebuild = should_full_rebuild(&root);
        let guard = self.runtime.read().unwrap();
        let runtime = guard
            .as_ref()
            .ok_or_else(|| LitecodeError::ToolExecution("runtime missing".into()))?;
        if rebuild {
            rebuild_index_in_runtime(runtime)?;
            Ok(RefreshResult {
                mode: RefreshMode::Rebuild,
            })
        } else {
            refresh_index_incremental(runtime)?;
            Ok(RefreshResult {
                mode: RefreshMode::Incremental,
            })
        }
    }

    fn notify_fs_changes(&self, paths: Vec<String>, deleted: bool) {
        if !self.warmed {
            return;
        }
        queue_fs_changes(&self.runtime, &paths, deleted);
    }

    fn reconcile_disk(&mut self) {
        if !self.warmed {
            return;
        }
        if let Some(root) = self.workspace_root.as_ref() {
            reload_worker_excludes(root);
        }
        let guard = self.runtime.read().unwrap();
        if let Some(runtime) = guard.as_ref() {
            sync_index_with_disk(runtime);
        }
        self.last_reconcile = Instant::now();
    }

    /// Periodic disk↔index sync (dirty signals + shared flush). Best-effort.
    fn maybe_reconcile(&mut self) {
        if !self.warmed || self.last_reconcile.elapsed() < INDEX_RECONCILE_INTERVAL {
            return;
        }
        if let Some(root) = self.workspace_root.as_ref() {
            reload_worker_excludes(root);
        }
        let guard = self.runtime.read().unwrap();
        if let Some(runtime) = guard.as_ref() {
            sync_index_with_disk(runtime);
        }
        drop(guard);
        self.last_reconcile = Instant::now();
        // Incremental embed/flush can leave another allocator high-water; trim once.
        crate::telemetry::release_heap_to_os();
    }

    /// Two-tier cool: L1 drop Session, L2 unload RAM index (disk retained).
    fn maybe_cool_memory(&mut self) {
        if !self.warmed {
            return;
        }
        let guard = self.runtime.read().unwrap();
        if let Some(runtime) = guard.as_ref() {
            runtime.maybe_cool_memory();
        }
    }

    fn shutdown(&mut self) {
        *self.runtime.write().unwrap() = None;
        self.warmed = false;
    }
}

fn dispatch(state: &mut WorkerState, req: JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id;
    match req.method.as_str() {
        "initialize" => match serde_json::from_value::<InitializeParams>(req.params) {
            Ok(p) => {
                let root = PathBuf::from(p.workspace_root);
                let session_db_path = p
                    .session_db_path
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from);
                match state.initialize(root, session_db_path) {
                    Ok(()) => JsonRpcResponse::ok(id, serde_json::json!({})),
                    Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                }
            }
            Err(e) => JsonRpcResponse::err(id, ERR_INVALID, e.to_string()),
        },
        "set_session_db" => match serde_json::from_value::<SetSessionDbParams>(req.params) {
            Ok(p) => match state.set_session_db(PathBuf::from(p.session_db_path)) {
                Ok(()) => JsonRpcResponse::ok(id, serde_json::json!({})),
                Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
            },
            Err(e) => JsonRpcResponse::err(id, ERR_INVALID, e.to_string()),
        },
        "warmup" => match state.warmup() {
            Ok(()) => JsonRpcResponse::ok(id, serde_json::json!({})),
            Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
        },
        "search" => match serde_json::from_value::<SearchParams>(req.params) {
            Ok(p) => match state.search(&p.query, p.glob.as_deref(), p.top_k) {
                Ok(hits) => {
                    let result = SearchResult { hits };
                    match serde_json::to_value(result) {
                        Ok(v) => JsonRpcResponse::ok(id, v),
                        Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                    }
                }
                Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
            },
            Err(e) => JsonRpcResponse::err(id, ERR_INVALID, e.to_string()),
        },
        "session_search" => match serde_json::from_value::<SessionSearchParams>(req.params) {
            Ok(p) => match state.session_search(&p.query, p.top_k, p.session_id.as_deref()) {
                Ok(hits) => {
                    let result = SessionSearchResult { hits };
                    match serde_json::to_value(result) {
                        Ok(v) => JsonRpcResponse::ok(id, v),
                        Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
                    }
                }
                Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
            },
            Err(e) => JsonRpcResponse::err(id, ERR_INVALID, e.to_string()),
        },
        "refresh" => match state.refresh() {
            Ok(result) => match serde_json::to_value(result) {
                Ok(v) => JsonRpcResponse::ok(id, v),
                Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
            },
            Err(e) => JsonRpcResponse::err(id, ERR_INTERNAL, e.to_string()),
        },
        "notify_fs_changes" => match serde_json::from_value::<NotifyFsChangesParams>(req.params) {
            Ok(p) => {
                state.notify_fs_changes(p.paths, p.deleted);
                JsonRpcResponse::ok(id, serde_json::json!({}))
            }
            Err(e) => JsonRpcResponse::err(id, ERR_INVALID, e.to_string()),
        },
        "reconcile_disk" => {
            state.reconcile_disk();
            JsonRpcResponse::ok(id, serde_json::json!({}))
        }
        "ping" => {
            if state.warmed {
                JsonRpcResponse::ok(id, serde_json::json!({ "ready": true }))
            } else {
                JsonRpcResponse::ok(id, serde_json::json!({ "ready": false }))
            }
        }
        "shutdown" => {
            state.shutdown();
            JsonRpcResponse::ok(id, serde_json::json!({}))
        }
        other => JsonRpcResponse::err(id, ERR_INVALID, format!("unknown method: {other}")),
    }
}

#[cfg(target_os = "linux")]
mod poll_ffi {
    #[repr(C)]
    pub struct PollFd {
        pub fd: i32,
        pub events: i16,
        pub revents: i16,
    }
    unsafe extern "C" {
        pub fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    }
}

/// Polling stdin loop: periodic disk reconcile + request dispatch.
#[cfg(target_os = "linux")]
pub fn run_worker_loop() -> Result<()> {
    use poll_ffi::*;

    let mut state = WorkerState::new();
    let stdin = std::io::stdin();
    let stdin_fd = stdin.as_raw_fd();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();

    loop {
        // Poll stdin with 1s timeout so reconcile runs even without incoming requests.
        let ret = unsafe {
            let mut pfd = PollFd {
                fd: stdin_fd,
                events: 1,
                revents: 0,
            }; // events=1 = POLLIN
            poll(&mut pfd, 1, 1000)
        };
        if ret < 0 {
            break; // poll error
        }

        // Dirty-signal reconcile + flush (reuses pending_updates → update_files).
        state.maybe_reconcile();
        // After reconcile, allow L1/L2 cool so peaks always fall back.
        state.maybe_cool_memory();

        if ret == 0 {
            continue; // timeout, no data yet
        }

        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break; // EOF
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(req) => {
                if req.method == "shutdown" {
                    let resp = dispatch(&mut state, req);
                    let out = serde_json::to_string(&resp)?;
                    writeln!(stdout, "{out}")?;
                    stdout.flush()?;
                    break;
                }
                dispatch(&mut state, req)
            }
            Err(e) => JsonRpcResponse::err(0, ERR_INVALID, e.to_string()),
        };
        let out = serde_json::to_string(&response)?;
        writeln!(stdout, "{out}")?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn run_worker_loop() -> Result<()> {
    let mut state = WorkerState::new();
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<JsonRpcRequest>(trimmed) {
            Ok(req) => {
                if req.method == "shutdown" {
                    let resp = dispatch(&mut state, req);
                    let out = serde_json::to_string(&resp)?;
                    writeln!(stdout, "{out}")?;
                    stdout.flush()?;
                    break;
                }
                dispatch(&mut state, req)
            }
            Err(e) => JsonRpcResponse::err(0, ERR_INVALID, e.to_string()),
        };
        let out = serde_json::to_string(&response)?;
        writeln!(stdout, "{out}")?;
        stdout.flush()?;
    }
    Ok(())
}
