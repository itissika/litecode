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
    AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, InitScope, LogSettings,
    McpServerDefinition, McpTransport, ModelDefinition, PROTECTED_AGENT_IDS, ProviderDefinition,
    ToolCatalogEntry, ToolPreset, ToolTier, WebSearchSettings,
};
use super::turn_guard::TurnGuard;
use super::workspace;
use crate::optional::EngineManager;
use crate::tool::catalog::{normalize_agent_profile, normalize_agent_tool_bindings};
use crate::types::{LitecodeError, Result};

/// One toast-ready string covering provider → model → required agent bindings.
fn setup_guidance(settings: &GlobalSettings) -> Option<String> {
    let mut steps = Vec::new();
    if !settings
        .providers
        .values()
        .any(|p| crate::llm::provider_ready(p))
    {
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

fn ensure_core_catalog(catalog: &HashMap<String, ToolCatalogEntry>) -> Result<()> {
    for id in tools::core_tool_ids() {
        if !catalog.contains_key(&id) {
            return Err(LitecodeError::Config(format!(
                "tool catalog is missing required core entry '{id}'"
            )));
        }
    }
    Ok(())
}

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

    /// Readiness source for write-path judgements.
    ///
    /// # Lock constraint
    /// Callers MUST NOT hold `state.runtime` write lock across this call; this
    /// reads the live runtime under a short read lock and returns clones.
    fn live_readiness(
        &self,
    ) -> (
        crate::config::runtime_catalog_state::RuntimeCatalogState,
        Option<std::collections::HashMap<String, crate::config::schema::ToolReadiness>>,
    ) {
        match self.runtime.get() {
            Some(rt) => {
                let rt = rt.read().expect("runtime lock");
                (
                    rt.resolved.runtime_catalog_state().clone(),
                    Some(rt.resolved.workspace_tool_readiness().clone()),
                )
            }
            None => (
                crate::config::runtime_catalog_state::RuntimeCatalogState::default(),
                None,
            ),
        }
    }

    pub fn reconcile_engines(&self, workspace: &super::resolved::WorkspaceState) -> Result<()> {
        let Some(engine_manager) = &self.engine_manager else {
            return Ok(());
        };
        let (runtime_catalog_state, workspace_readiness_opt) = self.live_readiness();
        let settings = self.load()?;
        let workspace = workspace::workspace_with_disk_readiness(workspace);
        let mut resolved = ConfigManager::resolve(settings, workspace);
        *resolved.runtime_catalog_state_mut() = runtime_catalog_state;
        if let Some(wr) = workspace_readiness_opt {
            resolved.workspace_mut().workspace_tool_readiness = wr;
        }
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

    fn commit_partial<F>(&self, mutate: F) -> Result<(u64, bool)>
    where
        F: FnOnce(&mut GlobalSettings) -> Result<bool>,
    {
        // REV-6: process-level mutex serializes the read-modify-write so
        // concurrent commits cannot lose updates (single-process write surface;
        // CAS would be over-engineering here).
        static COMMIT_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let _guard = COMMIT_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        self.ensure_writable()
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let mut settings = self.load()?;
        let restart_required = mutate(&mut settings)?;
        ConfigManager::validate(&settings)?;
        let conn = global_db::open(&self.db_path)?;
        store::replace_all(&conn, &settings)?;
        let revision = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let summary = Self::summary_from(&settings, revision, restart_required);
        let _ = self
            .broadcast
            .send(SettingsChangedEvent { revision, summary });
        Ok((revision, restart_required))
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
            catalog_count: settings.tool_catalog.len(),
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

    pub fn write_provider(&self, provider: ProviderDefinition) -> Result<u64> {
        self.commit_partial(|settings| {
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
        .map(|(rev, _)| rev)
    }

    pub fn write_providers(&self, providers: HashMap<String, ProviderDefinition>) -> Result<u64> {
        self.commit_partial(|settings| {
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
        .map(|(rev, _)| rev)
    }

    pub fn websearch_view(&self) -> Result<WebSearchView> {
        let settings = self.load()?;
        Ok(WebSearchView {
            search_endpoint: settings.websearch.search_endpoint.clone(),
        })
    }

    pub fn write_websearch(&self, websearch: WebSearchSettings) -> Result<u64> {
        self.commit_partial(|settings| {
            settings.websearch = websearch;
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn write_models(&self, models: HashMap<String, ModelDefinition>) -> Result<u64> {
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
                if crate::platform_knobs::is_closed_adapter(&model.adapter_id) {
                    model.config.capabilities = crate::llm::adapter_default_capabilities(
                        &model.adapter_id,
                        &model.config.api_model_id,
                    );
                }
                (id, model)
            })
            .collect();
        self.commit_partial(|settings| {
            // Full replace: PUT body is the desired registry (guards above + validate block orphans).
            settings.models = normalized;
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn write_agent(
        &self,
        id: &str,
        mut profile: AgentProfile,
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<u64> {
        validate_agent_id(id)?;
        let workspace = workspace::workspace_with_disk_readiness(workspace);
        let (runtime_catalog_state, workspace_readiness_opt) = self.live_readiness();
        let settings = self.load()?;
        let mut resolved = ConfigManager::resolve(settings, workspace);
        *resolved.runtime_catalog_state_mut() = runtime_catalog_state;
        if let Some(wr) = workspace_readiness_opt {
            resolved.workspace_mut().workspace_tool_readiness = wr;
        }
        expand_binding_presets(&mut profile.tools);
        normalize_agent_tool_bindings(
            resolved.tool_catalog(),
            resolved.workspace_tool_readiness(),
            resolved.runtime_catalog_state(),
            &mut profile.tools,
        )?;
        normalize_agent_profile(id, &mut profile);
        let id = id.to_string();
        self.commit_partial(|settings| {
            settings.agents.insert(id.clone(), profile);
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn delete_agent(&self, id: &str) -> Result<u64> {
        if PROTECTED_AGENT_IDS.contains(&id) {
            return Err(LitecodeError::Config(format!(
                "agent '{id}' is protected and cannot be deleted"
            )));
        }
        self.commit_partial(|settings| {
            if settings.agents.remove(id).is_none() {
                return Err(LitecodeError::Config(format!("agent not found: {id}")));
            }
            for profile in settings.agents.values_mut() {
                profile.allowed_subagents.retain(|s| s != id);
            }
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn apply_agent_tool_preset(
        &self,
        agent_id: &str,
        preset: ToolPreset,
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<u64> {
        validate_agent_id(agent_id)?;
        let workspace = workspace::workspace_with_disk_readiness(workspace);
        let (runtime_catalog_state, workspace_readiness_opt) = self.live_readiness();
        let workspace_readiness =
            workspace_readiness_opt.unwrap_or_else(|| workspace.workspace_tool_readiness.clone());
        if !self.load()?.agents.contains_key(agent_id) {
            return Err(LitecodeError::Config(format!(
                "agent not found: {agent_id}"
            )));
        }
        let id = agent_id.to_string();
        self.commit_partial(move |settings| {
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
            normalize_agent_tool_bindings(
                &settings.tool_catalog,
                &workspace_readiness,
                &runtime_catalog_state,
                &mut profile.tools,
            )?;
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn write_tool_catalog(
        &self,
        catalog: HashMap<String, ToolCatalogEntry>,
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<u64> {
        let revision = self
            .commit_partial(|settings| {
                if catalog.is_empty() && !settings.tool_catalog.is_empty() {
                    return Err(LitecodeError::Config(
                        "refusing to wipe tool catalog; reload settings and try again".into(),
                    ));
                }
                for (id, entry) in catalog {
                    settings.tool_catalog.insert(id, entry);
                }
                ensure_core_catalog(&settings.tool_catalog)?;
                Ok(false)
            })
            .map(|(rev, _)| rev)?;
        self.reconcile_engines(workspace)?;
        Ok(revision)
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

    pub fn write_custom_tool(&self, id: &str, mut def: CustomToolDefinition) -> Result<u64> {
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

        self.commit_partial(|settings| {
            if let Some(existing) = settings.custom_tools.iter_mut().find(|t| t.name == id) {
                *existing = def.clone();
            } else {
                settings.custom_tools.push(def.clone());
            }
            settings
                .tool_catalog
                .entry(id.to_string())
                .or_insert_with(|| ToolCatalogEntry {
                    id: id.to_string(),
                    tier: ToolTier::Custom,
                    init_scope: InitScope::Global,
                    catalog_enabled: false,
                });
            if let Some(entry) = settings.tool_catalog.get_mut(id) {
                entry.tier = ToolTier::Custom;
                entry.init_scope = InitScope::Global;
            }
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn delete_custom_tool(&self, id: &str) -> Result<u64> {
        self.commit_partial(|settings| {
            let before = settings.custom_tools.len();
            settings.custom_tools.retain(|t| t.name != id);
            if settings.custom_tools.len() == before {
                return Err(LitecodeError::Config(format!(
                    "custom tool not found: {id}"
                )));
            }
            settings.tool_catalog.remove(id);
            for profile in settings.agents.values_mut() {
                profile.tools.remove(id);
            }
            Ok(false)
        })
        .map(|(rev, _)| rev)
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

    pub fn write_mcp_server(&self, id: &str, mut def: McpServerDefinition) -> Result<u64> {
        validate_tool_id(id)?;
        if tools::is_core_tool(id) || tools::is_optional_builtin(id) {
            return Err(LitecodeError::Config(format!(
                "MCP server id '{id}' conflicts with a builtin tool"
            )));
        }
        def.command = def.command.trim().to_string();
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

        let catalog_id = tools::mcp_catalog_id(id);
        self.commit_partial(move |settings| {
            settings.mcp_servers.insert(id.to_string(), def.clone());
            settings
                .tool_catalog
                .entry(catalog_id.clone())
                .or_insert_with(|| ToolCatalogEntry {
                    id: catalog_id.clone(),
                    tier: ToolTier::Mcp,
                    init_scope: InitScope::Global,
                    catalog_enabled: false,
                });
            if let Some(entry) = settings.tool_catalog.get_mut(&catalog_id) {
                entry.tier = ToolTier::Mcp;
                entry.init_scope = InitScope::Global;
            }
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    pub fn delete_mcp_server(&self, id: &str) -> Result<u64> {
        let catalog_id = tools::mcp_catalog_id(id);
        self.commit_partial(|settings| {
            if settings.mcp_servers.remove(id).is_none() {
                return Err(LitecodeError::Config(format!("MCP server not found: {id}")));
            }
            settings.tool_catalog.remove(&catalog_id);
            for profile in settings.agents.values_mut() {
                profile.tools.remove(&catalog_id);
            }
            Ok(false)
        })
        .map(|(rev, _)| rev)
    }

    /// Initialize catalog readiness for global optional tools.
    /// Workspace infrastructure engines are not initialized here — use
    /// `enable_code_search_engine` / `write_lsp_init`.
    pub fn write_log(&self, log: LogSettings) -> Result<u64> {
        let revision = self
            .commit_partial(|settings| {
                settings.log = log;
                Ok(false)
            })
            .map(|(rev, _)| rev)?;
        log_filter::reload_from_path(&self.db_path);
        Ok(revision)
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
                .map(|rev| (rev, false)),
            "websearch.search_endpoint" => self
                .write_websearch(WebSearchSettings {
                    search_endpoint: Some(value.to_string()),
                })
                .map(|rev| (rev, false)),
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
        if tools::core_none_tools().contains(&tool_id.as_str()) || tools::is_mcp_catalog_id(tool_id)
        {
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
