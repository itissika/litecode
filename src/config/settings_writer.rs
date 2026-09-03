//! Shared settings writer for REST and CLI (`litecode config set`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::global_db::{self, store, tools};
use super::log_filter;
use super::manager::ConfigManager;
use super::schema::{
    AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, LogSettings,
    McpServerDefinition, McpTransport, ModelDefinition, PROTECTED_AGENT_IDS, ProviderDefinition,
    ToolPreset, WebSearchSettings,
};
use super::gate::{CommitAck, DocId};
use super::turn_guard::TurnGuard;
use super::workspace;
use crate::optional::EngineManager;
use crate::tool::agent_bindings::normalize_agent_profile;
use crate::types::{LitecodeError, Result};

/// One toast-ready string covering provider → model → required agent bindings.
fn setup_guidance(settings: &GlobalSettings) -> Option<String> {
    let mut steps = Vec::new();
    if !settings.providers.values().any(crate::llm::provider_ready) {
        steps.push(
            "add a Provider in Settings → Connection (adapter, endpoint, API key) and Save"
                .to_string(),
        );
    }
    if settings.models.is_empty() {
        steps.push("add at least one Model in Settings → Models and Save".to_string());
    }
    let mut missing = Vec::new();
    for id in ["default", "compaction"] {
        match settings.agents.get(id) {
            Some(profile)
                if !profile.model_ref.is_empty()
                    && settings.models.contains_key(&profile.model_ref) => {}
            _ => missing.push(id),
        }
    }
    if !missing.is_empty() {
        let labels: Vec<String> = missing
            .into_iter()
            .map(|id| match id {
                "default" => "default (primary)".to_string(),
                "compaction" => "compaction (hidden — required for context compaction)".to_string(),
                other => other.to_string(),
            })
            .collect();
        steps.push(format!(
            "assign a model to agents ({}) in Settings → Agents and Save",
            labels.join(", ")
        ));
    }
    if steps.is_empty() {
        None
    } else {
        Some(format!(
            "AI setup incomplete — {}. Agent runs will fail until this is fixed.",
            steps.join("; then ")
        ))
    }
}

fn fill_closed_provider_endpoint(def: &mut ProviderDefinition) {
    if !def.config.endpoint.trim().is_empty() {
        return;
    }
    if let Some(endpoint) = crate::llm::closed_default_endpoint(&def.adapter_id) {
        def.config.endpoint = endpoint.to_string();
    }
}

/// Sanitized settings view (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSummary {
    pub revision: u64,
    pub ready_provider_count: usize,
    pub provider_endpoint: Option<String>,
    pub websearch_endpoint: Option<String>,
    pub model_count: usize,
    pub agent_count: usize,
    pub catalog_count: usize,
    pub log_level: Option<String>,
    /// Most settings apply on the next agent turn without restarting serve.
    pub effective_next_turn: bool,
    /// True when the server process must be restarted for a setting to take effect.
    pub restart_required: bool,
    /// When set, AI setup is incomplete — FE toasts this guidance (no hard gate).
    pub setup_guidance: Option<String>,
}

/// Provider view with masked api_key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderView {
    pub id: String,
    pub adapter_id: String,
    pub label: String,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub auth: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchView {
    pub search_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsChangedEvent {
    pub revision: u64,
    pub docs: Vec<DocId>,
    pub summary: SettingsSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsWriteError {
    TurnInProgress,
}

impl std::fmt::Display for SettingsWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TurnInProgress => write!(f, "turn_in_progress"),
        }
    }
}

impl std::error::Error for SettingsWriteError {}

pub struct SettingsWriter {
    db_path: PathBuf,
    turn_guard: Arc<TurnGuard>,
    revision: Arc<AtomicU64>,
    broadcast: broadcast::Sender<SettingsChangedEvent>,
    engine_manager: Option<Arc<EngineManager>>,
    /// Live runtime handle. When unset (CLI), readiness judgements fall back to an
    /// empty default so behavior is unchanged (CLI never judges global readiness).
    runtime: OnceLock<Arc<std::sync::RwLock<crate::runtime::RuntimeHandle>>>,
}

impl SettingsWriter {
    pub fn new(turn_guard: Arc<TurnGuard>) -> Self {
        Self::with_path(global_db::default_db_path(), turn_guard)
    }

    pub fn with_path(db_path: impl Into<PathBuf>, turn_guard: Arc<TurnGuard>) -> Self {
        let (broadcast, _) = broadcast::channel(32);
        Self {
            db_path: db_path.into(),
            turn_guard,
            revision: Arc::new(AtomicU64::new(0)),
            broadcast,
            engine_manager: None,
            runtime: OnceLock::new(),
        }
    }

    pub fn set_engine_manager(&mut self, engine_manager: Arc<EngineManager>) {
        self.engine_manager = Some(engine_manager);
    }

    /// Inject the live runtime handle. Safe to call once before the writer is wrapped in `Arc`.
    /// Subsequent calls are ignored (OnceLock), which is fine because the runtime is built once.
    pub fn set_runtime(&self, runtime: Arc<std::sync::RwLock<crate::runtime::RuntimeHandle>>) {
        let _ = self.runtime.set(runtime);
    }

    pub fn reconcile_engines(&self, workspace: &super::resolved::WorkspaceState) -> Result<()> {
        let Some(engine_manager) = &self.engine_manager else {
            return Ok(());
        };
        let settings = self.load()?;
        let workspace = workspace::workspace_with_disk_readiness(workspace);
        let resolved = ConfigManager::resolve(settings, workspace);
        engine_manager.reconcile(&resolved);
        Ok(())
    }

    pub fn revision_handle(&self) -> Arc<AtomicU64> {
        self.revision.clone()
    }

    pub fn current_revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SettingsChangedEvent> {
        self.broadcast.subscribe()
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn turn_guard(&self) -> &Arc<TurnGuard> {
        &self.turn_guard
    }

    fn ensure_writable(&self) -> std::result::Result<(), SettingsWriteError> {
        if self.turn_guard.is_turn_in_progress() {
            return Err(SettingsWriteError::TurnInProgress);
        }
        Ok(())
    }

    fn load(&self) -> Result<GlobalSettings> {
        global_db::load_global_from_path(&self.db_path)
    }

    pub fn load_settings(&self) -> Result<GlobalSettings> {
        self.load()
    }

    fn commit_lock() -> std::sync::MutexGuard<'static, ()> {
        static COMMIT_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        COMMIT_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn commit_partial<F>(&self, docs: &[DocId], mutate: F) -> Result<CommitAck>
    where
        F: FnOnce(&mut GlobalSettings) -> Result<bool>,
    {
        let _guard = Self::commit_lock();
        self.ensure_writable()
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut settings = self.load()?;
        let restart_required = mutate(&mut settings)?;
        ConfigManager::validate(&settings)?;
        let conn = global_db::open(&self.db_path)?;
        store::replace_all(&conn, &settings)?;
        let generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let docs = docs.to_vec();
        let summary = Self::summary_from(&settings, generation, restart_required);
        let _ = self.broadcast.send(SettingsChangedEvent {
            revision: generation,
            docs: docs.clone(),
            summary,
        });
        Ok(CommitAck {
            generation,
            docs,
            restart_required,
        })
    }

    /// Workspace-file commit: write file then advance generation under the same lock.
    fn commit_workspace_file<F>(&self, doc: DocId, write: F) -> Result<CommitAck>
    where
        F: FnOnce() -> Result<()>,
    {
        let _guard = Self::commit_lock();
        self.ensure_writable()
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        write()?;
        let generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let settings = self.load()?;
        let docs = vec![doc];
        let summary = Self::summary_from(&settings, generation, false);
        let _ = self.broadcast.send(SettingsChangedEvent {
            revision: generation,
            docs: docs.clone(),
            summary,
        });
        Ok(CommitAck {
            generation,
            docs,
            restart_required: false,
        })
    }

    fn commit_mixed<W, G>(&self, docs: &[DocId], write: W, mutate: G) -> Result<CommitAck>
    where
        W: FnOnce() -> Result<()>,
        G: FnOnce(&mut GlobalSettings) -> Result<bool>,
    {
        let _guard = Self::commit_lock();
        self.ensure_writable()
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        write()?;
        let mut settings = self.load()?;
        let restart_required = mutate(&mut settings)?;
        ConfigManager::validate(&settings)?;
        let conn = global_db::open(&self.db_path)?;
        store::replace_all(&conn, &settings)?;
        let generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let docs = docs.to_vec();
        let summary = Self::summary_from(&settings, generation, restart_required);
        let _ = self.broadcast.send(SettingsChangedEvent {
            revision: generation,
            docs: docs.clone(),
            summary,
        });
        Ok(CommitAck {
            generation,
            docs,
            restart_required,
        })
    }

    pub fn summary(&self) -> Result<SettingsSummary> {
        let settings = self.load()?;
        Ok(Self::summary_from(
            &settings,
            self.current_revision(),
            false,
        ))
    }

    pub fn summary_from(
        settings: &GlobalSettings,
        revision: u64,
        restart_required: bool,
    ) -> SettingsSummary {
        let ready: Vec<_> = settings
            .providers
            .values()
            .filter(|p| crate::llm::provider_ready(p))
            .collect();
        SettingsSummary {
            revision,
            ready_provider_count: ready.len(),
            provider_endpoint: ready
                .first()
                .map(|p| p.config.endpoint.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            websearch_endpoint: settings.websearch.search_endpoint.clone(),
            model_count: settings.models.len(),
            agent_count: settings.agents.len(),
            catalog_count: 0,
            log_level: settings.log.level.clone(),
            effective_next_turn: !restart_required,
            restart_required,
            setup_guidance: setup_guidance(settings),
        }
    }

    pub fn mask_api_key(key: Option<&str>) -> Option<String> {
        key.filter(|k| !k.is_empty()).map(|k| {
            if k.len() <= 8 {
                "*".repeat(k.len())
            } else {
                format!("{}***{}", &k[..3], &k[k.len().saturating_sub(4)..])
            }
        })
    }

    fn provider_view_entry(def: &ProviderDefinition) -> ProviderView {
        ProviderView {
            id: def.id.clone(),
            adapter_id: def.adapter_id.clone(),
            label: def.label.clone(),
            endpoint: {
                let filled = if def.config.endpoint.trim().is_empty() {
                    crate::llm::closed_default_endpoint(&def.adapter_id)
                        .unwrap_or("")
                        .to_string()
                } else {
                    def.config.endpoint.trim().to_string()
                };
                if filled.is_empty() {
                    None
                } else {
                    Some(filled)
                }
            },
            api_key: Self::mask_api_key(Some(def.config.api_key.as_str())),
            auth: match def.config.auth {
                crate::config::schema::ProviderAuth::Bearer => "bearer".into(),
                crate::config::schema::ProviderAuth::ApiKey => "api_key".into(),
            },
        }
    }

    pub fn providers_view(&self) -> Result<HashMap<String, ProviderView>> {
        let settings = self.load()?;
        Ok(settings
            .providers
            .iter()
            .map(|(id, def)| (id.clone(), Self::provider_view_entry(def)))
            .collect())
    }

    /// First ready provider view (legacy convenience).
    pub fn provider_view(&self) -> Result<ProviderView> {
        let settings = self.load()?;
        let ready = settings
            .providers
            .values()
            .find(|p| crate::llm::provider_ready(p))
            .or_else(|| settings.providers.values().next());
        Ok(ready
            .map(Self::provider_view_entry)
            .unwrap_or(ProviderView {
                id: String::new(),
                adapter_id: String::new(),
                label: String::new(),
                endpoint: None,
                api_key: None,
                auth: "bearer".into(),
            }))
    }

    pub fn write_provider(&self, provider: ProviderDefinition) -> Result<CommitAck> {
        self.commit_partial(&[DocId::Providers], |settings| {
            let id = provider.id.clone();
            if id.is_empty() {
                return Err(LitecodeError::Config(
                    "provider id must not be empty".into(),
                ));
            }
            settings.providers.insert(id.clone(), {
                let mut next = ProviderDefinition { id, ..provider };
                fill_closed_provider_endpoint(&mut next);
                next
            });
            Ok(false)
        })
    }

    pub fn write_providers(&self, providers: HashMap<String, ProviderDefinition>) -> Result<CommitAck> {
        self.commit_partial(&[DocId::Providers], |settings| {
            let mut merged = HashMap::new();
            for (map_key, mut def) in providers {
                let id = if def.id.is_empty() {
                    map_key.clone()
                } else {
                    def.id.clone()
                };
                if id.is_empty() {
                    return Err(LitecodeError::Config(
                        "provider id must not be empty".into(),
                    ));
                }
                def.id = id.clone();
                if let Some(existing) = settings.providers.get(&id) {
                    let key = def.config.api_key.trim();
                    if key.is_empty() || key.contains('*') {
                        def.config.api_key = existing.config.api_key.clone();
                    }
                }
                fill_closed_provider_endpoint(&mut def);
                merged.insert(id, def);
            }
            settings.providers = merged;
            Ok(false)
        })
    }

    pub fn websearch_view(&self) -> Result<WebSearchView> {
        let settings = self.load()?;
        Ok(WebSearchView {
            search_endpoint: settings.websearch.search_endpoint.clone(),
        })
    }

    pub fn write_websearch(&self, websearch: WebSearchSettings) -> Result<CommitAck> {
        self.commit_partial(&[DocId::Websearch], |settings| {
            settings.websearch = websearch;
            Ok(false)
        })
    }

    pub fn write_models(&self, models: HashMap<String, ModelDefinition>) -> Result<CommitAck> {
        if models.is_empty() {
            let settings = self.load()?;
            let agents_need_models = settings
                .agents
                .values()
                .any(|profile| !profile.model_ref.is_empty());
            if agents_need_models {
                return Err(LitecodeError::Config(
                    "refusing to wipe models registry while agents reference models; clear agent model_ref first or send at least one model entry".into(),
                ));
            }
        }
        // Orphan session model_id clear runs in serve/settings.rs after write
        // (SessionManager has the DB handle; SettingsWriter stays config-only).
        // Closed adapters are adapter-owned: their modality capabilities are the
        // vendor's official matrix (e.g. mimo-v2.5 = full text/image/video/audio),
        // so UI payloads cannot silently downgrade them to the ["text"] default.
        let normalized: HashMap<String, ModelDefinition> = models
            .into_iter()
            .map(|(id, mut model)| {
                crate::llm::apply_owned_modality_capabilities(&mut model);
                (id, model)
            })
            .collect();
        self.commit_partial(&[DocId::Models], |settings| {
            // Full replace: PUT body is the desired registry (guards above + validate block orphans).
            settings.models = normalized;
            Ok(false)
        })
    }

    pub fn write_agent(
        &self,
        id: &str,
        mut profile: AgentProfile,
        _workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        validate_agent_id(id)?;
        expand_binding_presets(&mut profile.tools);
        normalize_agent_profile(id, &mut profile);
        let id = id.to_string();
        self.commit_partial(&[DocId::Agents], |settings| {
            settings.agents.insert(id.clone(), profile);
            Ok(false)
        })
    }

    pub fn delete_agent(&self, id: &str) -> Result<CommitAck> {
        if PROTECTED_AGENT_IDS.contains(&id) {
            return Err(LitecodeError::Config(format!(
                "agent '{id}' is protected and cannot be deleted"
            )));
        }
        self.commit_partial(&[DocId::Agents], |settings| {
            if settings.agents.remove(id).is_none() {
                return Err(LitecodeError::Config(format!("agent not found: {id}")));
            }
            for profile in settings.agents.values_mut() {
                profile.allowed_subagents.retain(|s| s != id);
            }
            Ok(false)
        })
    }

    pub fn apply_agent_tool_preset(
        &self,
        agent_id: &str,
        preset: ToolPreset,
        _workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        validate_agent_id(agent_id)?;
        if !self.load()?.agents.contains_key(agent_id) {
            return Err(LitecodeError::Config(format!(
                "agent not found: {agent_id}"
            )));
        }
        let id = agent_id.to_string();
        self.commit_partial(&[DocId::Agents], move |settings| {
            let profile = settings
                .agents
                .get_mut(&id)
                .expect("agent exists after load check");
            for (tool_id, binding) in profile.tools.iter_mut() {
                if tools::core_none_tools().contains(&tool_id.as_str())
                    || tools::is_mcp_catalog_id(tool_id)
                {
                    binding.last_applied_preset = None;
                    continue;
                }
                apply_preset_to_binding(tool_id, binding, preset);
            }
            Ok(false)
        })
    }

    pub fn list_custom_tools(&self) -> Result<Vec<CustomToolDefinition>> {
        let settings = self.load()?;
        let mut tools = settings.custom_tools;
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    pub fn get_custom_tool(&self, id: &str) -> Result<Option<CustomToolDefinition>> {
        let settings = self.load()?;
        Ok(settings.custom_tools.into_iter().find(|t| t.name == id))
    }

    pub fn write_custom_tool(&self, id: &str, mut def: CustomToolDefinition) -> Result<CommitAck> {
        validate_tool_id(id)?;
        if tools::is_core_tool(id) || tools::is_optional_builtin(id) {
            return Err(LitecodeError::Config(format!(
                "custom tool id '{id}' conflicts with a builtin tool"
            )));
        }
        if def.name != id {
            if def.name.is_empty() {
                def.name = id.to_string();
            } else {
                return Err(LitecodeError::Config(format!(
                    "custom tool body name '{}' must match path id '{id}'",
                    def.name
                )));
            }
        }
        if def.command.trim().is_empty() {
            return Err(LitecodeError::Config(
                "custom tool command must not be empty".into(),
            ));
        }
        if def.schema.schema_type.trim().is_empty() {
            def.schema.schema_type = "object".into();
        }
        if def.timeout == 0 {
            def.timeout = 120;
        }

        self.commit_partial(&[DocId::CustomToolsGlobal], |settings| {
            if let Some(existing) = settings.custom_tools.iter_mut().find(|t| t.name == id) {
                *existing = def.clone();
            } else {
                settings.custom_tools.push(def.clone());
            }
            Ok(false)
        })
    }

    pub fn delete_custom_tool(
        &self,
        id: &str,
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        let keep_binding = workspace.workspace_custom_tools.contains_key(id);
        let docs = if keep_binding {
            vec![DocId::CustomToolsGlobal]
        } else {
            vec![DocId::CustomToolsGlobal, DocId::Agents]
        };
        self.commit_partial(&docs, |settings| {
            let before = settings.custom_tools.len();
            settings.custom_tools.retain(|t| t.name != id);
            if settings.custom_tools.len() == before {
                return Err(LitecodeError::Config(format!(
                    "custom tool not found: {id}"
                )));
            }
            if !keep_binding {
                for profile in settings.agents.values_mut() {
                    profile.tools.remove(id);
                }
            }
            Ok(false)
        })
    }

    pub fn list_mcp_servers(&self) -> Result<Vec<(String, McpServerDefinition)>> {
        let settings = self.load()?;
        let mut servers: Vec<_> = settings.mcp_servers.into_iter().collect();
        servers.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(servers)
    }

    pub fn get_mcp_server(&self, id: &str) -> Result<Option<McpServerDefinition>> {
        let settings = self.load()?;
        Ok(settings.mcp_servers.get(id).cloned())
    }

    pub fn write_mcp_server(&self, id: &str, mut def: McpServerDefinition) -> Result<CommitAck> {
        validate_mcp_definition(id, &mut def)?;
        self.commit_partial(&[DocId::McpGlobal], move |settings| {
            settings.mcp_servers.insert(id.to_string(), def.clone());
            Ok(false)
        })
    }

    pub fn delete_mcp_server(
        &self,
        id: &str,
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        let catalog_id = tools::mcp_catalog_id(id);
        let keep_binding = workspace.workspace_mcp_servers.contains_key(id);
        let docs = if keep_binding {
            vec![DocId::McpGlobal]
        } else {
            vec![DocId::McpGlobal, DocId::Agents]
        };
        self.commit_partial(&docs, |settings| {
            if settings.mcp_servers.remove(id).is_none() {
                return Err(LitecodeError::Config(format!("MCP server not found: {id}")));
            }
            if !keep_binding {
                for profile in settings.agents.values_mut() {
                    profile.tools.remove(&catalog_id);
                }
            }
            Ok(false)
        })
    }

    pub fn list_workspace_custom_tools(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<CustomToolDefinition>> {
        let mut tools: Vec<_> = workspace::read_workspace_custom_tools(workspace_root)?
            .tools
            .into_values()
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tools)
    }

    pub fn get_workspace_custom_tool(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<Option<CustomToolDefinition>> {
        Ok(workspace::read_workspace_custom_tools(workspace_root)?
            .tools
            .get(id)
            .cloned())
    }

    pub fn write_workspace_custom_tool(
        &self,
        workspace_root: &Path,
        id: &str,
        mut def: CustomToolDefinition,
    ) -> Result<CommitAck> {
        validate_tool_id(id)?;
        if tools::is_core_tool(id) || tools::is_optional_builtin(id) {
            return Err(LitecodeError::Config(format!(
                "custom tool id '{id}' conflicts with a builtin tool"
            )));
        }
        if def.name != id {
            if def.name.is_empty() {
                def.name = id.to_string();
            } else {
                return Err(LitecodeError::Config(format!(
                    "custom tool body name '{}' must match path id '{id}'",
                    def.name
                )));
            }
        }
        if def.command.trim().is_empty() {
            return Err(LitecodeError::Config(
                "custom tool command must not be empty".into(),
            ));
        }
        if def.schema.schema_type.trim().is_empty() {
            def.schema.schema_type = "object".into();
        }
        if def.timeout == 0 {
            def.timeout = 120;
        }
        let root = workspace_root.to_path_buf();
        self.commit_workspace_file(DocId::CustomToolsWorkspace, move || {
            workspace::upsert_workspace_custom_tool(&root, def)
        })
    }

    pub fn delete_workspace_custom_tool(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<CommitAck> {
        let file = workspace::read_workspace_custom_tools(workspace_root)?;
        if !file.tools.contains_key(id) {
            return Err(LitecodeError::Config(format!(
                "custom tool not found: {id}"
            )));
        }
        let keep_binding = self.load()?.custom_tools.iter().any(|t| t.name == id);
        let root = workspace_root.to_path_buf();
        let id_owned = id.to_string();
        if keep_binding {
            return self.commit_workspace_file(DocId::CustomToolsWorkspace, move || {
                workspace::delete_workspace_custom_tool(&root, &id_owned).map(|_| ())
            });
        }
        let strip_id = id.to_string();
        self.commit_mixed(
            &[DocId::CustomToolsWorkspace, DocId::Agents],
            move || workspace::delete_workspace_custom_tool(&root, &id_owned).map(|_| ()),
            move |settings| {
                for profile in settings.agents.values_mut() {
                    profile.tools.remove(&strip_id);
                }
                Ok(false)
            },
        )
    }

    pub fn list_workspace_mcp_servers(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<(String, McpServerDefinition)>> {
        let mut servers: Vec<_> = workspace::read_workspace_mcp(workspace_root)?
            .servers
            .into_iter()
            .collect();
        servers.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(servers)
    }

    pub fn get_workspace_mcp_server(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<Option<McpServerDefinition>> {
        Ok(workspace::read_workspace_mcp(workspace_root)?
            .servers
            .get(id)
            .cloned())
    }

    pub fn write_workspace_mcp_server(
        &self,
        workspace_root: &Path,
        id: &str,
        mut def: McpServerDefinition,
    ) -> Result<CommitAck> {
        validate_mcp_definition(id, &mut def)?;
        let root = workspace_root.to_path_buf();
        let id_owned = id.to_string();
        self.commit_workspace_file(DocId::McpWorkspace, move || {
            workspace::upsert_workspace_mcp(&root, &id_owned, def)
        })
    }

    pub fn delete_workspace_mcp_server(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<CommitAck> {
        let file = workspace::read_workspace_mcp(workspace_root)?;
        if !file.servers.contains_key(id) {
            return Err(LitecodeError::Config(format!("MCP server not found: {id}")));
        }
        let keep_binding = self.load()?.mcp_servers.contains_key(id);
        let root = workspace_root.to_path_buf();
        let id_owned = id.to_string();
        if keep_binding {
            return self.commit_workspace_file(DocId::McpWorkspace, move || {
                workspace::delete_workspace_mcp(&root, &id_owned).map(|_| ())
            });
        }
        let catalog_id = tools::mcp_catalog_id(id);
        self.commit_mixed(
            &[DocId::McpWorkspace, DocId::Agents],
            move || workspace::delete_workspace_mcp(&root, &id_owned).map(|_| ()),
            move |settings| {
                for profile in settings.agents.values_mut() {
                    profile.tools.remove(&catalog_id);
                }
                Ok(false)
            },
        )
    }

    pub fn get_engines(&self, workspace_root: &Path) -> Result<workspace::WorkspaceEnginesFile> {
        workspace::read_workspace_engines(workspace_root)
    }

    pub fn write_engines(
        &self,
        workspace_root: &Path,
        file: workspace::WorkspaceEnginesFile,
    ) -> Result<CommitAck> {
        if file.lsp.desired && file.lsp.servers.is_empty() {
            return Err(LitecodeError::Config(
                "lsp engine requires at least one language server".into(),
            ));
        }
        let root = workspace_root.to_path_buf();
        self.commit_workspace_file(DocId::Engines, move || {
            workspace::write_workspace_engines(&root, &file)
        })
    }

    pub fn get_excludes(
        &self,
        workspace_root: &Path,
    ) -> Result<crate::workspace::filter::WorkspaceExcludesFile> {
        crate::workspace::filter::ensure_workspace_excludes(workspace_root)
    }

    pub fn write_excludes(
        &self,
        workspace_root: &Path,
        file: crate::workspace::filter::WorkspaceExcludesFile,
    ) -> Result<CommitAck> {
        let root = workspace_root.to_path_buf();
        self.commit_workspace_file(DocId::Excludes, move || {
            crate::workspace::filter::write_workspace_excludes(&root, file).map(|_| ())
        })
    }

    pub fn write_log(&self, log: LogSettings) -> Result<CommitAck> {
        let ack = self.commit_partial(&[DocId::Log], |settings| {
            settings.log = log;
            Ok(false)
        })?;
        log_filter::reload_from_path(&self.db_path);
        Ok(ack)
    }

    /// CLI `config set <key> <value>` — keys mirror REST resources.
    pub fn set_key(&self, key: &str, value: &str) -> Result<(u64, bool)> {
        match key {
            "provider.endpoint" | "provider.api_key" => Err(LitecodeError::Config(
                "deprecated settings key; configure providers via Web Settings (PUT /api/settings/providers) or providers.<id>.endpoint in the settings UI".into(),
            )),
            "log.level" => self
                .write_log(LogSettings {
                    level: Some(value.to_string()),
                })
                .map(|ack| (ack.generation, false)),
            "websearch.search_endpoint" => self
                .write_websearch(WebSearchSettings {
                    search_endpoint: Some(value.to_string()),
                })
                .map(|ack| (ack.generation, false)),
            "auth.token" => Err(LitecodeError::Config(
                "auth.token removed: serve auth is host-injected via LITECODE_TOKEN only".into(),
            )),
            other => Err(LitecodeError::Config(format!(
                "unknown settings key: {other}"
            ))),
        }
    }
}

fn validate_agent_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(LitecodeError::Config(format!(
            "invalid agent id '{id}': use lowercase letters, digits, and underscores"
        )));
    }
    Ok(())
}

/// Expand `last_applied_preset` into policy/path_mode for configurable tools.
/// NONE tools (`plan` / `todo` / `subagent_launch`) are left untouched.
fn expand_binding_presets(tools: &mut HashMap<String, AgentToolBinding>) {
    for (tool_id, binding) in tools.iter_mut() {
        if tools::is_mcp_catalog_id(tool_id) {
            binding.last_applied_preset = None;
            continue;
        }
        binding.allowed_tools = None;
        if tools::core_none_tools().contains(&tool_id.as_str()) {
            binding.last_applied_preset = None;
            continue;
        }
        if let Some(preset) = binding.last_applied_preset {
            apply_preset_to_binding(tool_id, binding, preset);
        }
    }
}

fn apply_preset_to_binding(tool_id: &str, binding: &mut AgentToolBinding, preset: ToolPreset) {
    let (policy, path_mode) = if tools::is_core_tool(tool_id) || tools::is_optional_builtin(tool_id)
    {
        crate::permission::presets::binding_for_tool(tool_id, preset)
    } else {
        crate::permission::presets::binding_for_tool("custom", preset)
    };
    binding.policy = policy;
    binding.path_mode = path_mode;
    binding.last_applied_preset = Some(preset);
}

fn validate_mcp_definition(id: &str, def: &mut McpServerDefinition) -> Result<()> {
    validate_tool_id(id)?;
    if tools::is_core_tool(id) || tools::is_optional_builtin(id) {
        return Err(LitecodeError::Config(format!(
            "MCP server id '{id}' conflicts with a builtin tool"
        )));
    }
    def.command = def.command.trim().to_string();
    def.timeout = def.call_timeout_secs();
    match &def.transport {
        McpTransport::Stdio => {
            if def.command.is_empty() {
                return Err(LitecodeError::Config(
                    "MCP stdio server command must not be empty".into(),
                ));
            }
        }
        McpTransport::Remote { url, .. } => {
            if cfg!(not(feature = "remote-mcp")) {
                return Err(LitecodeError::Config(
                    "remote MCP transport requires a build with the remote-mcp feature".into(),
                ));
            }
            if url.trim().is_empty() {
                return Err(LitecodeError::Config(
                    "MCP remote server url must not be empty".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_tool_id(id: &str) -> Result<()> {
    let valid = !id.is_empty()
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(LitecodeError::Config(format!(
            "invalid id '{id}': use [a-z][a-z0-9_]*"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{
        ADAPTER_OPENAI_RESPONSES, LogSettings, ModelAdapterConfig, ModelCapability,
        ModelDefinition, ProviderAuth, ProviderConnectionConfig, ProviderDefinition,
    };
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn ready_provider(id: &str) -> ProviderDefinition {
        ProviderDefinition {
            id: id.into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            label: id.into(),
            config: ProviderConnectionConfig {
                endpoint: "https://api.example.com/v1".into(),
                api_key: "sk-test".into(),
                auth: ProviderAuth::Bearer,
            },
        }
    }

    fn sample_model(id: &str, provider_ref: &str) -> ModelDefinition {
        ModelDefinition {
            id: id.into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            provider_ref: provider_ref.into(),
            label: id.into(),
            config: ModelAdapterConfig {
                api_model_id: "gpt-4".into(),
                context_window: 128_000,
                max_tokens: 4096,
                thinking_mode: None,
                reasoning_effort: None,
                json_output: false,
                capabilities: vec![ModelCapability::Text],
            },
        }
    }

    #[test]
    fn mask_api_key_hides_middle() {
        assert_eq!(
            SettingsWriter::mask_api_key(Some("sk-abcdefghij")),
            Some("sk-***ghij".into())
        );
    }

    #[test]
    fn write_models_replaces_registry_and_drops_removed_entries() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = global_db::load_global_from_path(&db).unwrap();
        settings
            .providers
            .insert("main".into(), ready_provider("main"));
        settings
            .models
            .insert("default".into(), sample_model("default", "main"));
        settings
            .models
            .insert("extra".into(), sample_model("extra", "main"));
        settings.agents.get_mut("default").unwrap().model_ref = "default".into();
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));

        let mut kept = settings.models.clone();
        kept.remove("extra");
        writer.write_models(kept).unwrap();

        let loaded = global_db::load_global_from_path(&db).unwrap();
        assert!(!loaded.models.contains_key("extra"));
        assert!(loaded.models.contains_key("default"));
    }

    #[test]
    fn write_models_normalizes_closed_adapter_capabilities() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = global_db::load_global_from_path(&db).unwrap();
        settings
            .providers
            .insert("main".into(), ready_provider("main"));
        let mut mimo_provider = ready_provider("mimo");
        mimo_provider.adapter_id = crate::config::schema::ADAPTER_MIMO_RESPONSES.into();
        settings.providers.insert("mimo".into(), mimo_provider);

        let mut mimo_v25 = sample_model("mimo25", "mimo");
        mimo_v25.adapter_id = crate::config::schema::ADAPTER_MIMO_RESPONSES.into();
        mimo_v25.config.api_model_id = "mimo-v2.5".into();
        mimo_v25.config.capabilities = vec![crate::config::schema::ModelCapability::Text];

        let mut mimo_pro = sample_model("mimopro", "mimo");
        mimo_pro.adapter_id = crate::config::schema::ADAPTER_MIMO_RESPONSES.into();
        mimo_pro.config.api_model_id = "mimo-v2.5-pro".into();
        mimo_pro.config.capabilities = vec![crate::config::schema::ModelCapability::Text];

        let mut open = sample_model("open", "main");
        open.config.capabilities = vec![crate::config::schema::ModelCapability::Text];

        settings.models.insert("mimo25".into(), mimo_v25);
        settings.models.insert("mimopro".into(), mimo_pro);
        settings.models.insert("open".into(), open);
        settings.agents.get_mut("default").unwrap().model_ref = "mimo25".into();
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));

        let mut incoming = settings.models.clone();
        // UI payloads send the ["text"] default; closed adapters must be normalized.
        for model in incoming.values_mut() {
            model.config.capabilities = vec![crate::config::schema::ModelCapability::Text];
        }
        writer.write_models(incoming).unwrap();

        let loaded = global_db::load_global_from_path(&db).unwrap();
        use crate::config::schema::ModelCapability;
        assert_eq!(
            loaded.models["mimo25"].config.capabilities,
            vec![
                ModelCapability::Text,
                ModelCapability::Image,
                ModelCapability::Video,
                ModelCapability::Audio,
            ],
            "mimo-v2.5 must default to full modality"
        );
        assert_eq!(
            loaded.models["mimopro"].config.capabilities,
            vec![ModelCapability::Text],
            "mimo-v2.5-pro is text-only"
        );
        assert_eq!(
            loaded.models["open"].config.capabilities,
            vec![ModelCapability::Text],
            "open adapters keep the payload as-is"
        );
    }

    #[test]
    fn write_models_normalizes_ark_turbo_image_capability() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = global_db::load_global_from_path(&db).unwrap();
        let mut ark_provider = ready_provider("ark");
        ark_provider.adapter_id = crate::config::schema::ADAPTER_ARK_CODING.into();
        settings.providers.insert("ark".into(), ark_provider);

        let mut turbo = sample_model("turbo", "ark");
        turbo.adapter_id = crate::config::schema::ADAPTER_ARK_CODING.into();
        turbo.config.api_model_id = "doubao-seed-2.1-turbo".into();
        turbo.config.capabilities = vec![crate::config::schema::ModelCapability::Text];

        let mut flash = sample_model("flash", "ark");
        flash.adapter_id = crate::config::schema::ADAPTER_ARK_CODING.into();
        flash.config.api_model_id = "deepseek-v4-flash".into();
        flash.config.capabilities = vec![crate::config::schema::ModelCapability::Text];

        settings.models.insert("turbo".into(), turbo);
        settings.models.insert("flash".into(), flash);
        settings.agents.get_mut("default").unwrap().model_ref = "turbo".into();
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));

        let mut incoming = settings.models.clone();
        for model in incoming.values_mut() {
            model.config.capabilities = vec![crate::config::schema::ModelCapability::Text];
        }
        writer.write_models(incoming).unwrap();

        let loaded = global_db::load_global_from_path(&db).unwrap();
        use crate::config::schema::ModelCapability;
        assert_eq!(
            loaded.models["turbo"].config.capabilities,
            vec![ModelCapability::Text, ModelCapability::Image]
        );
        assert_eq!(
            loaded.models["flash"].config.capabilities,
            vec![ModelCapability::Text]
        );
    }

    #[test]
    fn write_models_allows_empty_when_agents_have_no_model_ref() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let settings = global_db::load_global_from_path(&db).unwrap();
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        writer.write_models(HashMap::new()).unwrap();
        let loaded = global_db::load_global_from_path(&db).unwrap();
        assert!(loaded.models.is_empty());
    }

    #[test]
    fn write_models_rejects_empty_wipe_when_agents_reference_models() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = global_db::load_global_from_path(&db).unwrap();
        settings
            .providers
            .insert("main".into(), ready_provider("main"));
        settings
            .models
            .insert("default".into(), sample_model("default", "main"));
        settings.agents.get_mut("default").unwrap().model_ref = "default".into();
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let err = writer.write_models(HashMap::new()).unwrap_err();
        assert!(matches!(
            err,
            LitecodeError::Config(msg) if msg.contains("refusing to wipe models")
        ));
    }

    #[test]
    fn commit_log_roundtrip_and_docs() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let log = LogSettings {
            level: Some("debug".into()),
        };
        let ack = writer.write_log(log.clone()).unwrap();
        assert_eq!(ack.docs, vec![DocId::Log]);
        assert!(ack.generation >= 1);
        let loaded = writer.load_settings().unwrap().log;
        assert_eq!(loaded, log);
    }

    #[test]
    fn workspace_engines_commit_file_and_event_together() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let ws = TempDir::new().unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let mut rx = writer.subscribe();
        let file = workspace::WorkspaceEnginesFile {
            version: 1,
            lsp: workspace::WorkspaceLspState {
                desired: true,
                servers: vec!["rust-analyzer".into()],
            },
            retrieval: workspace::WorkspaceRetrievalState {
                desired: false,
            },
        };
        let ack = writer.write_engines(ws.path(), file.clone()).unwrap();
        let on_disk = workspace::read_workspace_engines(ws.path()).unwrap();
        assert_eq!(on_disk.lsp.servers, file.lsp.servers);
        assert_eq!(ack.docs, vec![DocId::Engines]);
        let event = rx.try_recv().expect("settings event");
        assert_eq!(event.docs, vec![DocId::Engines]);
        assert_eq!(event.revision, ack.generation);
    }

    #[test]
    fn write_log_rejects_invalid_level() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = crate::config::global_db::load_global_from_path(&db).unwrap();
        settings
            .providers
            .insert("main".into(), ready_provider("main"));
        global_db::import_into(&db, &settings).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let err = writer
            .write_log(LogSettings {
                level: Some("verbose".into()),
            })
            .unwrap_err();
        assert!(matches!(
            err,
            LitecodeError::Config(msg) if msg.contains("log.level")
        ));
    }

    #[test]
    fn turn_blocks_write() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let guard = Arc::new(TurnGuard::new());
        let writer = SettingsWriter::with_path(&db, guard.clone());
        guard.begin_turn();
        let err = writer
            .write_provider(ProviderDefinition {
                id: "main".into(),
                adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
                label: "main".into(),
                config: ProviderConnectionConfig {
                    endpoint: "http://x".into(),
                    api_key: "k".into(),
                    auth: ProviderAuth::Bearer,
                },
            })
            .unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg == "turn_in_progress"));
        guard.end_turn();
    }

    #[test]
    fn set_key_rejects_removed_auth_token() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let err = writer.set_key("auth.token", "nope").unwrap_err();
        assert!(err.to_string().contains("LITECODE_TOKEN"), "got: {err}");
    }

    #[test]
    fn setup_guidance_covers_provider_model_agents() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let settings = global_db::load_global_from_path(&db).unwrap();
        let summary = SettingsWriter::summary_from(&settings, 1, false);
        let guidance = summary.setup_guidance.expect("fresh seed should guide");
        assert!(guidance.contains("Provider"), "{guidance}");
        assert!(guidance.contains("Model"), "{guidance}");
        assert!(guidance.contains("default"), "{guidance}");
        assert!(guidance.contains("compaction"), "{guidance}");
    }

    #[test]
    fn setup_guidance_clears_when_ready() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        let mut settings = global_db::load_global_from_path(&db).unwrap();
        settings
            .providers
            .insert("main".into(), ready_provider("main"));
        settings
            .models
            .insert("default".into(), sample_model("default", "main"));
        settings.agents.get_mut("default").unwrap().model_ref = "default".into();
        settings.agents.get_mut("compaction").unwrap().model_ref = "default".into();
        let summary = SettingsWriter::summary_from(&settings, 1, false);
        assert_eq!(summary.setup_guidance, None);
    }
}
