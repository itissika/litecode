//! Global optional tool services (webfetch/websearch).
//!
//! Workspace infrastructure engines (LSP, retrieval) live in [`crate::engines`].

pub mod exa_mcp;
mod stub_engine;
mod webfetch_engine;
mod websearch_engine;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
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
}

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
        }
    }

    /// Demote a warmed engine to Idle (worker crash path).
    pub fn note_engine_idle(&self, tool_id: &str) {
        let mut states = self.states.write().expect("engine states lock");
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
            .expect("engine states lock")
            .get(tool_id)
            .copied()
            == Some(EngineWarmupState::Warm)
    }

    pub fn state(&self, tool_id: &str) -> Option<EngineWarmupState> {
        self.states
            .read()
            .expect("engine states lock")
            .get(tool_id)
            .copied()
    }

    pub fn last_error(&self, tool_id: &str) -> Option<String> {
        self.last_errors
            .read()
            .expect("engine errors lock")
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

    fn start_warmup(&self, resolved: &ResolvedConfig, tool_id: &str) {
        if tool_id == "websearch" {
            self.websearch.configure(resolved.websearch());
        }

        {
            let states = self.states.read().expect("engine states lock");
            if matches!(
                states.get(tool_id),
                Some(EngineWarmupState::Warm | EngineWarmupState::Warming)
            ) {
                return;
            }
        }

        {
            let mut states = self.states.write().expect("engine states lock");
            states.insert(tool_id.to_string(), EngineWarmupState::Warming);
        }

        let Some(engine) = self.engines.get(tool_id).cloned() else {
            return;
        };
        let tool_id_owned = tool_id.to_string();

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let states = Arc::clone(&self.states);
            let errors = Arc::clone(&self.last_errors);
            let tool_id_for_log = tool_id_owned.clone();
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || engine.warmup()).await;
                let (warm, err_msg) = match result {
                    Ok(Ok(())) => (true, None),
                    Ok(Err(e)) => (false, Some(e.to_string())),
                    Err(e) => (false, Some(format!("warmup task failed: {e}"))),
                };
                let mut states = states.write().expect("engine states lock");
                if states.get(&tool_id_owned) == Some(&EngineWarmupState::Warming) {
                    states.insert(
                        tool_id_owned.clone(),
                        if warm {
                            EngineWarmupState::Warm
                        } else {
                            EngineWarmupState::Failed
                        },
                    );
                }
                let mut errors = errors.write().expect("engine errors lock");
                if warm {
                    errors.remove(&tool_id_owned);
                } else if let Some(msg) = err_msg {
                    errors.insert(tool_id_owned, msg);
                }
                tracing::debug!(tool = %tool_id_for_log, warm, "optional engine warmup finished");
            });
        } else {
            let result = engine.warmup();
            let warm = result.is_ok();
            let mut states = self.states.write().expect("engine states lock");
            if states.get(tool_id) == Some(&EngineWarmupState::Warming) {
                states.insert(
                    tool_id_owned.clone(),
                    if warm {
                        EngineWarmupState::Warm
                    } else {
                        EngineWarmupState::Failed
                    },
                );
            }
            let mut errors = self.last_errors.write().expect("engine errors lock");
            if warm {
                errors.remove(tool_id);
            } else if let Err(e) = result {
                errors.insert(tool_id.to_string(), e.to_string());
            }
        }
    }

    fn stop_engine(&self, tool_id: &str) {
        if let Some(engine) = self.engines.get(tool_id) {
            engine.stop();
        }
        let mut states = self.states.write().expect("engine states lock");
        states.insert(tool_id.to_string(), EngineWarmupState::Stopped);
        let mut errors = self.last_errors.write().expect("engine errors lock");
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
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::GlobalSettings;

    #[test]
    fn reconcile_starts_warmup_for_network_core() {
        let global = GlobalSettings::default();
        let resolved = resolve(global, WorkspaceState::new("/tmp"));
        let mgr = EngineManager::new();
        mgr.reconcile(&resolved);
        assert!(mgr.is_warmed("webfetch", &resolved));
    }

    #[test]
    fn stop_clears_warm_state() {
        let global = GlobalSettings::default();
        let resolved = resolve(global, WorkspaceState::new("/tmp"));
        let mgr = EngineManager::new();
        mgr.reconcile(&resolved);
        mgr.stop_all();
        assert!(!mgr.is_warmed("webfetch", &resolved));
    }
}
