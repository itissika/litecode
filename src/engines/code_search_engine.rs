use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::engines::code_search::SearchHit;
use crate::engines::code_search_ipc::CodeSearchWorkerClient;
use crate::engines::code_search_ipc::protocol::RefreshResult;
use crate::types::{LitecodeError, Result};

pub struct CodeSearchEngine {
    client: Mutex<Option<CodeSearchWorkerClient>>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    warmup_epoch: AtomicU64,
    /// OS PID of the worker child. Set at spawn (before warmup finishes) so
    /// status-bar RSS can attribute embed memory while indexing is in progress.
    worker_os_pid: AtomicU32,
    on_worker_failed: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// FS events received before the worker is warm; flushed after warmup.
    pending_fs: Mutex<HashSet<(String, bool)>>,
}

impl CodeSearchEngine {
    pub fn new() -> Self {
        Self {
            client: Mutex::new(None),
            workspace_root: Arc::new(RwLock::new(None)),
            warmup_epoch: AtomicU64::new(0),
            worker_os_pid: AtomicU32::new(0),
            on_worker_failed: Mutex::new(None),
            pending_fs: Mutex::new(HashSet::new()),
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

    fn warmup_still_valid(&self, epoch: u64) -> bool {
        self.warmup_epoch.load(Ordering::SeqCst) == epoch
    }

    fn kill_client(&self) {
        self.set_worker_os_pid(None);
        if let Ok(mut guard) = self.client.lock()
            && let Some(mut client) = guard.take()
        {
            let _ = client.shutdown();
            client.kill();
        }
    }

    pub fn search(
        &self,
        query: &str,
        glob_filter: Option<&str>,
        top_k: usize,
    ) -> Result<Vec<SearchHit>> {
        if !self.worker_alive() {
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(
                "code_search worker is not running; re-enable the tool catalog or restart litecode with this workspace"
                    .into(),
            ));
        }

        let mut guard = self
            .client
            .lock()
            .map_err(|e| LitecodeError::Config(format!("code_search client lock: {e}")))?;
        let client = guard
            .as_mut()
            .ok_or_else(|| LitecodeError::Config("code_search engine not warmed".into()))?;
        match client.search(query, glob_filter, top_k) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                tracing::warn!(tool = "code_search", error = %e, "worker exited");
                drop(guard);
                self.notify_worker_failed();
                Err(e)
            }
        }
    }

    /// Session corpus ANN-only search (requires warmed worker; same lifecycle as code search).
    pub fn search_sessions(
        &self,
        query: &str,
        top_k: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<crate::engines::session_search::SessionTextHit>> {
        if !self.worker_alive() {
            self.notify_worker_failed();
            return Err(LitecodeError::ToolExecution(
                "code_search worker is not running; re-enable the tool catalog or restart litecode with this workspace"
                    .into(),
            ));
        }

        let mut guard = self
            .client
            .lock()
            .map_err(|e| LitecodeError::Config(format!("code_search client lock: {e}")))?;
        let client = guard
            .as_mut()
            .ok_or_else(|| LitecodeError::Config("code_search engine not warmed".into()))?;
        match client.session_search(query, top_k, session_id) {
            Ok(hits) => Ok(hits),
            Err(e) => {
                tracing::warn!(tool = "session_search", error = %e, "worker session_search failed");
                drop(guard);
                self.notify_worker_failed();
                Err(e)
            }
        }
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

    /// PID of the code-search worker child, if running.
    ///
    /// Available from spawn through warmup/indexing (not only after Warm), so
    /// telemetry can show embed RSS while the worker is still building.
    pub fn worker_pid(&self) -> Option<u32> {
        let pid = self.worker_os_pid.load(Ordering::Acquire);
        (pid != 0).then_some(pid)
    }

    /// Refresh index while Warm: rebuild or incremental (worker decides).
    pub fn refresh(&self) -> Result<RefreshResult> {
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
        match client.refresh() {
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

        if !self.warmup_still_valid(epoch) {
            return Ok(());
        }

        self.kill_client();

        if !self.warmup_still_valid(epoch) {
            return Ok(());
        }

        let mut client = CodeSearchWorkerClient::spawn()?;
        self.set_worker_os_pid(client.pid());

        if !self.warmup_still_valid(epoch) {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        if let Err(e) = client.initialize(&root) {
            client.kill();
            self.set_worker_os_pid(None);
            return Err(e);
        }

        if !self.warmup_still_valid(epoch) {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        if let Err(e) = client.warmup() {
            client.kill();
            self.set_worker_os_pid(None);
            return Err(e);
        }

        if !self.warmup_still_valid(epoch) {
            client.kill();
            self.set_worker_os_pid(None);
            return Ok(());
        }

        self.flush_pending_fs(&mut client);

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

impl Default for CodeSearchEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::code_search::init_workspace_index;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn notify_buffers_while_cold() {
        let engine = CodeSearchEngine::new();
        engine.notify_fs_changes(&["src/a.rs".into()], false);
        let pending = engine.pending_fs.lock().unwrap();
        assert!(pending.contains(&("src/a.rs".into(), false)));
    }

    #[test]
    fn stop_during_warmup_kills_worker() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        init_workspace_index(&root).unwrap();
        for i in 0..80 {
            std::fs::write(
                root.join(format!("slow_{i}.rs")),
                format!("pub fn fn_{i}() {{\n{}\n}}\n", "let _ = 1;\n".repeat(200)),
            )
            .unwrap();
        }

        let engine = Arc::new(CodeSearchEngine::new());
        engine.set_workspace(root);

        let worker = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || {
                let _ = engine.warmup();
            })
        };

        std::thread::sleep(Duration::from_millis(5));
        engine.stop();
        worker.join().expect("warmup thread");

        assert!(!engine.worker_alive());
        let guard = engine.client.lock().unwrap();
        assert!(guard.is_none());
    }

    #[test]
    fn worker_pid_available_during_warmup() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        init_workspace_index(&root).unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

        let engine = Arc::new(CodeSearchEngine::new());
        engine.set_workspace(root);

        let worker = {
            let engine = Arc::clone(&engine);
            std::thread::spawn(move || {
                let _ = engine.warmup();
            })
        };

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut saw_pid = false;
        while std::time::Instant::now() < deadline {
            if engine.worker_pid().is_some() {
                saw_pid = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        worker.join().expect("warmup thread");
        assert!(
            saw_pid,
            "worker_pid should be set once the child is spawned"
        );
        if engine.worker_alive() {
            assert!(engine.worker_pid().is_some());
        }
    }
}
