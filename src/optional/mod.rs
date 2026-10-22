//! Global optional tool services (webfetch/websearch).
//!
//! Workspace infrastructure engines (LSP, retrieval) live in [`crate::engines`].

pub mod exa_mcp;
mod stub_engine;
mod webfetch_engine;
mod websearch_engine;

use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::global_db::tools::optional_builtin_ids;
use crate::config::resolved::ResolvedConfig;
use crate::types::Result;

pub use stub_engine::StubEngine;
use webfetch_engine::WebfetchEngine;
use websearch_engine::WebsearchEngine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineWarmupState {
    Idle,
    Warming,
    Warm,
    Failed,
    Stopped,
}

pub trait ToolEngine: Send + Sync {
    fn id(&self) -> &str;
    fn warmup(&self) -> Result<()>;
    fn stop(&self);
}

#[derive(Clone)]
pub struct EngineManager {
    states: Arc<RwLock<HashMap<String, EngineWarmupState>>>,
    last_errors: Arc<RwLock<HashMap<String, String>>>,
    engines: Arc<HashMap<String, Arc<dyn ToolEngine>>>,
    webfetch: Arc<WebfetchEngine>,
    websearch: Arc<WebsearchEngine>,
    /// WP-167: one client build at a time per engine (see [`Self::ensure_warm`]).
    warm_locks: Arc<HashMap<String, Mutex<()>>>,
    #[cfg(test)]
    builds: Arc<AtomicUsize>,
}

/// Called by a web tool at the start of every call (WP-167): builds the
/// client on first use; `Err` when the build failed this time.
pub type WarmHook = Arc<dyn Fn() -> std::result::Result<(), String> + Send + Sync>;

impl EngineManager {
    pub fn new() -> Self {
        let states = Arc::new(RwLock::new(HashMap::new()));
        let last_errors = Arc::new(RwLock::new(HashMap::new()));
        let webfetch = Arc::new(WebfetchEngine::new());
        let websearch = Arc::new(WebsearchEngine::new());

        let mut engines: HashMap<String, Arc<dyn ToolEngine>> = HashMap::new();
        engines.insert("webfetch".to_string(), webfetch.clone());
        engines.insert("websearch".to_string(), websearch.clone());
        for id in optional_builtin_ids() {
            if *id == "webfetch" || *id == "websearch" || *id == "code_search" || *id == "lsp" {
                continue;
            }
            tracing::error!(
                tool = id,
                "unknown optional builtin tool id — no engine registered; \
                 add a real engine implementation or remove it from optional_builtin_ids"
            );
        }
        Self {
            states,
            last_errors,
            engines: Arc::new(engines),
            webfetch,
            websearch,
            warm_locks: Arc::new(
                ["webfetch", "websearch"]
                    .into_iter()
                    .map(|id| (id.to_string(), Mutex::new(())))
                    .collect(),
            ),
            #[cfg(test)]
            builds: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Demote a warmed engine to Idle (worker crash path).
    pub fn note_engine_idle(&self, tool_id: &str) {
        let mut states = self
            .states
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if states.get(tool_id) == Some(&EngineWarmupState::Warm) {
            states.insert(tool_id.to_string(), EngineWarmupState::Idle);
        }
    }

    pub fn webfetch_client(&self) -> Arc<RwLock<Option<reqwest::blocking::Client>>> {
        self.webfetch.client_handle()
    }

    pub fn websearch_client(&self) -> Arc<RwLock<Option<reqwest::blocking::Client>>> {
        self.websearch.client_handle()
    }

    pub fn websearch_endpoint(&self) -> Arc<RwLock<Option<String>>> {
        self.websearch.endpoint_handle()
    }

    pub async fn wait_until_warmed(&self, tool_id: &str, max_wait: Duration) -> bool {
        let start = std::time::Instant::now();
        loop {
            if self.state(tool_id) == Some(EngineWarmupState::Warm) {
                return true;
            }
            if start.elapsed() >= max_wait {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub fn is_warmed(&self, tool_id: &str, _resolved: &ResolvedConfig) -> bool {
        if tool_id != "webfetch" && tool_id != "websearch" {
            return true;
        }
        self.states
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tool_id)
            .copied()
            == Some(EngineWarmupState::Warm)
    }

    pub fn state(&self, tool_id: &str) -> Option<EngineWarmupState> {
        self.states
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tool_id)
            .copied()
    }

    pub fn last_error(&self, tool_id: &str) -> Option<String> {
        self.last_errors
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(tool_id)
            .cloned()
    }

    pub fn engine_status(&self, tool_id: &str) -> EngineStatus {
        EngineStatus {
            state: self.state(tool_id),
            error: self.last_error(tool_id),
        }
    }

    pub fn reconcile(&self, resolved: &ResolvedConfig) {
        for id in ["webfetch", "websearch"] {
            if should_run_engine(resolved, id) {
                self.start_warmup(resolved, id);
            } else {
                self.stop_engine(id);
            }
        }
    }

    pub fn stop_all(&self) {
        for id in ["webfetch", "websearch"] {
            self.stop_engine(id);
        }
    }

    /// Settings changed (or CLI startup): rebuild now. The old client is
    /// dropped and a new one is built with the current settings, on the
    /// blocking pool when a runtime exists, inline otherwise.
    fn start_warmup(&self, resolved: &ResolvedConfig, tool_id: &str) {
        if tool_id == "websearch" {
            self.websearch.configure(resolved.websearch());
        }
        // WP-167: only a client that already exists is rebuilt; otherwise the
        // new settings wait for the first call.
        // Only reached for an enabled engine (`reconcile` → `should_run_engine`;
        // disabling goes through `stop_engine`). Clear a previous Stopped /
        // Failed record so the next `ensure_warm` builds.
        self.states
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(tool_id);
        self.last_errors
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(tool_id);
        if !self.has_client(tool_id) {
            return;
        }
        {
            let Some(lock) = self.warm_locks.get(tool_id) else {
                return;
            };
            let _guard = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(engine) = self.engines.get(tool_id) {
                engine.stop();
            }
            self.states
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(tool_id);
        }
        let manager = self.clone();
        let tool_id_owned = tool_id.to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn_blocking(move || {
                let warm = manager.ensure_warm(&tool_id_owned).is_ok();
                tracing::debug!(tool = %tool_id_owned, warm, "optional engine warmup finished");
            });
        } else {
            let _ = manager.ensure_warm(&tool_id_owned);
        }
    }

    /// WP-167: build the engine's client if it is missing; reuse it otherwise.
    /// Idempotent and safe under concurrent calls (one build at a time, and a
    /// waiter that finds a client returns without building). A failure is
    /// recorded as `Failed` + error for this call only: the next call tries
    /// again. `Stopped` (user disabled) returns `Ok` without building, so
    /// callers keep the existing "not warmed" message.
    pub fn ensure_warm(&self, tool_id: &str) -> std::result::Result<(), String> {
        let Some(engine) = self.engines.get(tool_id).cloned() else {
            return Ok(());
        };
        if self.has_client(tool_id) {
            return Ok(());
        }
        let Some(lock) = self.warm_locks.get(tool_id) else {
            return Ok(());
        };
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.has_client(tool_id) {
            return Ok(());
        }
        // Stopped (user disabled): nothing to build; the caller finds no
        // client and returns its existing "not warmed" message.
        if self.state(tool_id) == Some(EngineWarmupState::Stopped) {
            return Ok(());
        }
        self.states
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(tool_id.to_string(), EngineWarmupState::Warming);
        #[cfg(test)]
        self.builds.fetch_add(1, Ordering::SeqCst);
        let result = engine.warmup();
        let mut states = self
            .states
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut errors = self
            .last_errors
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(()) => {
                states.insert(tool_id.to_string(), EngineWarmupState::Warm);
                errors.remove(tool_id);
                Ok(())
            }
            Err(e) => {
                states.insert(tool_id.to_string(), EngineWarmupState::Failed);
                errors.insert(tool_id.to_string(), e.to_string());
                Err(e.to_string())
            }
        }
    }

    fn has_client(&self, tool_id: &str) -> bool {
        let handle = match tool_id {
            "webfetch" => self.webfetch.client_handle(),
            "websearch" => self.websearch.client_handle(),
            _ => return false,
        };
        handle
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// Entry hook handed to the web tools (called once per tool call).
    pub fn warm_hook(&self, tool_id: &'static str) -> WarmHook {
        let manager = self.clone();
        Arc::new(move || manager.ensure_warm(tool_id))
    }

    /// WP-167: serve startup. Record the websearch endpoint only (cheap);
    /// no client is built until the first web tool call.
    pub fn defer_reconcile(&self, resolved: &ResolvedConfig) {
        self.websearch.configure(resolved.websearch());
    }

    fn stop_engine(&self, tool_id: &str) {
        if let Some(engine) = self.engines.get(tool_id) {
            engine.stop();
        }
        let mut states = self
            .states
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        states.insert(tool_id.to_string(), EngineWarmupState::Stopped);
        let mut errors = self
            .last_errors
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        errors.remove(tool_id);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub state: Option<EngineWarmupState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for EngineManager {
    fn default() -> Self {
        Self::new()
    }
}

fn should_run_engine(_resolved: &ResolvedConfig, tool_id: &str) -> bool {
    matches!(tool_id, "webfetch" | "websearch")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::GlobalSettings;

    #[test]
    fn concurrent_first_calls_build_once() {
        let mgr = EngineManager::new();
        let hook = mgr.warm_hook("webfetch");
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let hook = Arc::clone(&hook);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    hook()
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), Ok(()));
        }
        assert_eq!(mgr.builds.load(Ordering::SeqCst), 1);
    }

    /// WP-167 contract (tools.md (b)): `reconcile` on a manager with no client
    /// builds nothing; the first tool call (`ensure_warm`) builds it. Before
    /// WP-167 reconcile warmed eagerly, which is what this test used to assert.
    #[test]
    fn reconcile_starts_warmup_for_network_core() {
        let global = GlobalSettings::default();
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let mgr = EngineManager::new();
        mgr.reconcile(&resolved);
        assert!(!mgr.is_warmed("webfetch", &resolved), "reconcile is lazy");
        assert_eq!(mgr.builds.load(Ordering::SeqCst), 0);
        assert_eq!(mgr.ensure_warm("webfetch"), Ok(()));
        assert!(mgr.is_warmed("webfetch", &resolved), "first use warms");
    }

    /// B12: a panic while holding the states lock must not poison every later call.
    #[test]
    fn poisoned_state_lock_does_not_cascade_panics() {
        let mgr = EngineManager::new();
        let states = Arc::clone(&mgr.states);
        let _ = std::thread::spawn(move || {
            let _g = states.write().unwrap();
            panic!("poison engine states");
        })
        .join();
        assert!(mgr.states.is_poisoned());
        assert_eq!(mgr.state("webfetch"), None);
        mgr.note_engine_idle("webfetch");
        assert!(mgr.last_error("webfetch").is_none());
    }
}
