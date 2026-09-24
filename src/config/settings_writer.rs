//! Shared settings writer for REST and CLI (`litecode config set`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::gate::{CommitAck, DocId};
use super::global_db::{self, store, tools};
use super::log_filter;
use super::manager::ConfigManager;
use super::schema::{
    AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, LogSettings,
    McpServerDefinition, McpTransport, PROTECTED_AGENT_IDS, ToolPreset, WebSearchSettings,
};
use super::turn_guard::TurnGuard;
use super::workspace;
use crate::optional::EngineManager;
use crate::provider_catalog::ProviderCatalog;
use crate::tool::agent_bindings::normalize_agent_profile;
use crate::types::{LitecodeError, Result};

/// One toast-ready string covering provider key → model → required agent bindings.
fn setup_guidance(settings: &GlobalSettings, catalog: &ProviderCatalog) -> Option<String> {
    let mut steps = Vec::new();
    let keyed = |provider_id: &str| {
        settings
            .provider_credentials
            .get(provider_id)
            .is_some_and(|key| !key.trim().is_empty())
    };
    if !catalog.providers().iter().any(|p| keyed(&p.id)) {
        steps.push("add a Provider API key in Settings → Providers".to_string());
    }
    let mut missing = Vec::new();
    for id in ["default", "compaction"] {
        let ready = settings.agents.get(id).is_some_and(|profile| {
            !profile.model_ref.trim().is_empty()
                && catalog
                    .model(&profile.model_ref)
                    .is_some_and(|model| keyed(&model.provider_id))
        });
        if !ready {
            missing.push(id);
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

/// Sanitized settings view (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsSummary {
    pub revision: u64,
    /// Catalog providers that have a credential.
    pub configured_provider_count: usize,
    /// Catalog models that are selectable right now (provider has a credential).
    pub active_model_count: usize,
    pub agent_count: usize,
    pub log_level: Option<String>,
    /// Most settings apply on the next agent turn without restarting serve.
    pub effective_next_turn: bool,
    /// True when the server process must be restarted for a setting to take effect.
    pub restart_required: bool,
    /// When set, AI setup is incomplete — FE toasts this guidance (no hard gate).
    pub setup_guidance: Option<String>,
}

/// One catalog model as the UI sees it.
///
/// The same DTO feeds the nested provider lists and `active_models`: one
/// projection function, one shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModelView {
    /// Stable reference `{provider_id}/{model_id}`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub id: String,
    pub label: String,
    pub provider_id: String,
    pub provider_name: String,
    pub context_window: usize,
    pub context_window_max: usize,
    pub max_output: u32,
    pub modalities: Vec<String>,
    /// `false` models are marked as unusable for agents in the picker.
    pub tool_call: bool,
    pub json_output: bool,
    /// `false` keeps the model out of every picker; the catalog still lists it.
    pub enabled: bool,
}

impl CatalogModelView {
    pub fn from_model(model: &crate::provider_catalog::ResolvedModel, enabled: bool) -> Self {
        Self {
            reference: model.reference.clone(),
            id: model.id.clone(),
            label: model.display_label().to_string(),
            provider_id: model.provider_id.clone(),
            provider_name: model.provider_name.clone(),
            context_window: model.context_window,
            context_window_max: model.context_window_max,
            max_output: model.max_output,
            modalities: model
                .modalities
                .iter()
                .map(|modality| modality.as_str().to_string())
                .collect(),
            tool_call: model.tool_call,
            json_output: model.json_output,
            enabled,
        }
    }
}

/// One catalog provider with its credential state and models.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmProviderView {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub configured: bool,
    pub masked_api_key: Option<String>,
    pub endpoint: String,
    pub endpoint_type: String,
    pub models: Vec<CatalogModelView>,
}

/// Read-only LLM projection for the Settings provider page and every model
/// picker. Provider/model facts are catalog data; only `configured` and
/// `masked_api_key` come from the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmSettingsView {
    pub catalog_path: String,
    pub revision: u64,
    pub providers: Vec<LlmProviderView>,
    pub active_models: Vec<CatalogModelView>,
}

impl LlmSettingsView {
    pub fn project(
        catalog: &ProviderCatalog,
        credentials: &HashMap<String, String>,
        disabled: &HashSet<String>,
        revision: u64,
    ) -> Self {
        let key_for = |provider_id: &str| {
            credentials
                .get(provider_id)
                .map(String::as_str)
                .filter(|key| !key.trim().is_empty())
        };
        // Enablement is the exception: a ref that is not named is on.
        let enabled_for = |reference: &str| !disabled.contains(reference);
        let providers: Vec<LlmProviderView> = catalog
            .providers()
            .iter()
            .map(|provider| {
                let key = key_for(&provider.id);
                LlmProviderView {
                    id: provider.id.clone(),
                    name: provider.name.clone(),
                    visible: provider.visible,
                    configured: key.is_some(),
                    masked_api_key: SettingsWriter::mask_api_key(key),
                    endpoint: provider.endpoint.clone(),
                    endpoint_type: provider.endpoint_type.as_str().to_string(),
                    models: catalog
                        .models_of(&provider.id)
                        .iter()
                        .map(|model| {
                            CatalogModelView::from_model(
                                model,
                                enabled_for(&model.reference),
                            )
                        })
                        .collect(),
                }
            })
            .collect();
        let active_models = catalog
            .models()
            .iter()
            .filter(|model| key_for(&model.provider_id).is_some())
            .filter(|model| enabled_for(&model.reference))
            .map(|model| CatalogModelView::from_model(model, true))
            .collect();
        Self {
            catalog_path: catalog.path().display().to_string(),
            revision,
            providers,
            active_models,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchView {
    /// Masked Exa API key (same shape as provider keys).
    #[serde(default)]
    pub api_key: Option<String>,
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
        let catalog = self.catalog()?;
        let workspace = workspace::workspace_with_disk_readiness(workspace);
        let resolved = ConfigManager::resolve(settings, workspace, catalog);
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

    /// Provider catalog for this database (loaded once per process).
    pub fn catalog(&self) -> Result<Arc<ProviderCatalog>> {
        crate::provider_catalog::shared_for_db(&self.db_path)
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
        // Auto-heal agent model refs in the same commit: adding or removing a
        // provider key, and switching a model off, can strand an agent on a ref
        // that can no longer run. Repaired in place, no shadow field — the
        // catalog (plus the credential map) is the source of truth and the
        // stored ref is derived state.
        let catalog = self.catalog()?;
        let mut docs = docs.to_vec();
        let repaired = crate::config::bridge::repair_agent_models(&mut settings, &catalog);
        if !repaired.is_empty() {
            tracing::info!(
                agents = ?repaired,
                "auto-assigned a runnable model to agents whose model_ref could not run"
            );
            if !docs.contains(&DocId::Agents) {
                docs.push(DocId::Agents);
            }
        }
        ConfigManager::validate(&settings)?;
        let conn = global_db::open(&self.db_path)?;
        store::replace_all(&conn, &settings)?;
        let generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        let summary = Self::summary_from(&settings, &catalog, generation, restart_required);
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
        let catalog = self.catalog()?;
        let summary = Self::summary_from(&settings, &catalog, generation, false);
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
        let catalog = self.catalog()?;
        let summary = Self::summary_from(&settings, &catalog, generation, restart_required);
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
        let catalog = self.catalog()?;
        Ok(Self::summary_from(
            &settings,
            &catalog,
            self.current_revision(),
            false,
        ))
    }

    pub fn summary_from(
        settings: &GlobalSettings,
        catalog: &ProviderCatalog,
        revision: u64,
        restart_required: bool,
    ) -> SettingsSummary {
        let keyed = |provider_id: &str| {
            settings
                .provider_credentials
                .get(provider_id)
                .is_some_and(|key| !key.trim().is_empty())
        };
        let configured_provider_count = catalog
            .providers()
            .iter()
            .filter(|provider| keyed(&provider.id))
            .count();
        let active_model_count = catalog
            .models()
            .iter()
            .filter(|model| keyed(&model.provider_id))
            .filter(|model| !settings.disabled_models.contains(&model.reference))
            .count();
        SettingsSummary {
            revision,
            configured_provider_count,
            active_model_count,
            agent_count: settings.agents.len(),
            log_level: settings.log.level.clone(),
            effective_next_turn: !restart_required,
            restart_required,
            setup_guidance: setup_guidance(settings, catalog),
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

    /// The single LLM projection: catalog facts + credential state.
    pub fn llm_view(&self) -> Result<LlmSettingsView> {
        let settings = self.load()?;
        let catalog = self.catalog()?;
        Ok(LlmSettingsView::project(
            &catalog,
            &settings.provider_credentials,
            &settings.disabled_models,
            self.current_revision(),
        ))
    }

    /// Store or replace the API key of a catalog provider.
    pub fn write_provider_key(&self, provider_id: &str, api_key: &str) -> Result<CommitAck> {
        let catalog = self.catalog()?;
        if catalog.provider(provider_id).is_none() {
            return Err(LitecodeError::Config(format!(
                "provider '{provider_id}' is not declared in the provider catalog ({})",
                catalog.path().display()
            )));
        }
        let key = api_key.trim();
        if key.is_empty() {
            return Err(LitecodeError::Config(
                "api_key must not be empty (use DELETE to remove a credential)".into(),
            ));
        }
        let provider_id = provider_id.to_string();
        let key = key.to_string();
        self.commit_partial(&[DocId::Llm], move |settings| {
            settings
                .provider_credentials
                .insert(provider_id.clone(), key.clone());
            Ok(false)
        })
    }

    /// Remove a provider credential. The catalog provider itself never goes away.
    pub fn delete_provider_key(&self, provider_id: &str) -> Result<CommitAck> {
        let catalog = self.catalog()?;
        if catalog.provider(provider_id).is_none() {
            return Err(LitecodeError::Config(format!(
                "provider '{provider_id}' is not declared in the provider catalog ({})",
                catalog.path().display()
            )));
        }
        let provider_id = provider_id.to_string();
        self.commit_partial(&[DocId::Llm], move |settings| {
            settings.provider_credentials.remove(&provider_id);
            Ok(false)
        })
    }

    /// Switch one catalog model on or off for every picker.
    ///
    /// Enablement belongs to the user, not to the catalog: a switched-off model
    /// keeps existing and keeps its row under its provider, it only leaves
    /// `active_models`.
    pub fn set_model_enabled(&self, model_ref: &str, enabled: bool) -> Result<CommitAck> {
        let catalog = self.catalog()?;
        if catalog.model(model_ref).is_none() {
            return Err(LitecodeError::Config(format!(
                "model '{model_ref}' is not declared in the provider catalog ({})",
                catalog.path().display()
            )));
        }
        let model_ref = model_ref.to_string();
        self.commit_partial(&[DocId::Llm], move |settings| {
            if enabled {
                settings.disabled_models.remove(&model_ref);
            } else {
                settings.disabled_models.insert(model_ref.clone());
            }
            Ok(false)
        })
    }

    /// PUT patch for `api_key`: omit keeps current; empty clears; masked echo keeps.
    pub fn apply_api_key_patch(current: Option<String>, incoming: Option<&str>) -> Option<String> {
        let Some(raw) = incoming else {
            return current;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        if Self::mask_api_key(current.as_deref()).as_deref() == Some(trimmed) {
            return current;
        }
        Some(trimmed.to_string())
    }

    /// Masked websearch key view.
    pub fn websearch_view(&self) -> Result<WebSearchView> {
        let settings = self.load()?;
        Ok(WebSearchView {
            api_key: Self::mask_api_key(settings.websearch.api_key.as_deref()),
        })
    }

    pub fn write_websearch(&self, websearch: WebSearchSettings) -> Result<CommitAck> {
        self.commit_partial(&[DocId::Websearch], |settings| {
            settings.websearch = websearch;
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
            "provider.endpoint" | "provider.api_key" | "providers" | "models" => {
                Err(LitecodeError::Config(
                    "provider and model facts live in provider-catalog.toml; set a key in the Web                      Settings → Providers page or PUT /api/settings/providers/{provider_id}/key"
                        .into(),
                ))
            }
            "log.level" => self
                .write_log(LogSettings {
                    level: Some(value.to_string()),
                })
                .map(|ack| (ack.generation, false)),
            "websearch.search_endpoint" => Err(LitecodeError::Config(
                "websearch.search_endpoint removed; set the Exa API key via Settings → Advanced or websearch.api_key".into(),
            )),
            "websearch.api_key" => {
                let mut websearch = self.load()?.websearch;
                websearch.api_key = Self::apply_api_key_patch(websearch.api_key, Some(value));
                self.write_websearch(websearch)
                    .map(|ack| (ack.generation, false))
            }
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
/// NONE tools (`plan` / `todo` / `subagent_launch` / `subagent_wait` / `subagent_stop` /
/// `subagent_list` / `subagent_send`) are left untouched.
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
    use crate::config::schema::LogSettings;
    use tempfile::TempDir;

    const TEST_CATALOG: &str = r#"version = 1

[[providers]]
id = "main"
name = "Main"
endpoint = "https://api.example.com/v1"
endpoint_type = "responses"
tiers = { low = "low", medium = "medium", high = "high" }

[[providers]]
id = "other"
name = "Other"
endpoint = "https://other.example.com/v1"
endpoint_type = "chat_completions"

[[models]]
id = "default"
provider_id = "main"
context_window = 128000
max_output = 4096
modalities = ["text", "image"]

[[models]]
id = "compact"
provider_id = "main"
context_window = 200000
max_output = 4096

[[models]]
id = "small"
provider_id = "other"
context_window = 32000
max_output = 1024
"#;

    /// Settings writer bound to a temp DB that already owns the test catalog.
    fn writer_with_catalog() -> (TempDir, std::path::PathBuf, SettingsWriter) {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("litecode.db");
        crate::provider_catalog::store::forget(&db);
        std::fs::write(
            crate::provider_catalog::catalog_path_for_db(&db),
            TEST_CATALOG,
        )
        .unwrap();
        crate::config::global_db::open(&db).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        (dir, db, writer)
    }

    fn project(writer: &SettingsWriter) -> crate::config::LlmSettingsView {
        writer.llm_view().unwrap()
    }

    #[test]
    fn mask_api_key_hides_middle() {
        assert_eq!(
            SettingsWriter::mask_api_key(Some("sk-abcdefghij")),
            Some("sk-***ghij".into())
        );
    }

    #[test]
    fn api_key_patch_keeps_masked_echo_and_clears_empty() {
        let stored = Some("sk-abcdefghij".to_string());
        assert_eq!(
            SettingsWriter::apply_api_key_patch(stored.clone(), Some("sk-***ghij")),
            stored
        );
        assert_eq!(
            SettingsWriter::apply_api_key_patch(stored.clone(), Some("")),
            None
        );
        assert_eq!(
            SettingsWriter::apply_api_key_patch(stored, Some("new-secret-key")),
            Some("new-secret-key".into())
        );
    }

    #[test]
    fn provider_key_write_stores_the_credential_and_masks_it() {
        let (_dir, _db, writer) = writer_with_catalog();
        let ack = writer.write_provider_key("main", "sk-secret-value").unwrap();
        // The same commit heals the seeded agents' empty model_ref.
        assert_eq!(ack.docs, vec![DocId::Llm, DocId::Agents]);

        let view = project(&writer);
        let main = view.providers.iter().find(|p| p.id == "main").unwrap();
        assert!(main.configured);
        assert_eq!(main.masked_api_key.as_deref(), Some("sk-***alue"));
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(
            !serialized.contains("sk-secret-value"),
            "the raw credential must never be projected: {serialized}"
        );
        assert_eq!(view.active_models.len(), 2, "main owns two models");
        assert!(view.active_models.iter().all(|m| m.provider_id == "main"));
    }

    #[test]
    fn provider_key_write_rejects_an_unknown_provider_and_an_empty_key() {
        let (_dir, _db, writer) = writer_with_catalog();
        let unknown = writer.write_provider_key("ghost", "sk").unwrap_err();
        assert!(unknown.to_string().contains("ghost"), "{unknown}");
        let empty = writer.write_provider_key("main", "   ").unwrap_err();
        assert!(empty.to_string().contains("api_key"), "{empty}");
        assert!(project(&writer).active_models.is_empty());
    }

    #[test]
    fn switching_a_model_off_drops_it_from_the_pickers_only() {
        let (_dir, db, writer) = writer_with_catalog();
        writer.write_provider_key("main", "sk-one").unwrap();
        let view = project(&writer);
        assert_eq!(view.active_models.len(), 2);
        assert!(view.active_models.iter().all(|m| m.enabled));

        let reference = view.active_models[0].reference.clone();
        let ack = writer.set_model_enabled(&reference, false).unwrap();
        assert_eq!(ack.docs, vec![DocId::Llm]);

        let view = project(&writer);
        assert_eq!(view.active_models.len(), 1, "the off model leaves the pickers");
        assert!(!view.active_models.iter().any(|m| m.reference == reference));
        // It keeps existing under its provider: switched off, not gone.
        let model = view
            .providers
            .iter()
            .find(|p| p.id == "main")
            .unwrap()
            .models
            .iter()
            .find(|m| m.reference == reference)
            .unwrap();
        assert!(!model.enabled);

        // The exception is persisted as a row, so it survives a reopen.
        let reopened = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        assert!(!project(&reopened)
            .active_models
            .iter()
            .any(|m| m.reference == reference));

        // On again removes the row rather than flipping a flag.
        writer.set_model_enabled(&reference, true).unwrap();
        let view = project(&writer);
        assert_eq!(view.active_models.len(), 2);
        assert!(view.active_models.iter().all(|m| m.enabled));
    }

    #[test]
    fn model_switch_rejects_a_reference_the_catalog_does_not_declare() {
        let (_dir, _db, writer) = writer_with_catalog();
        let unknown = writer.set_model_enabled("main/ghost", false).unwrap_err();
        assert!(unknown.to_string().contains("main/ghost"), "{unknown}");
    }

    #[test]
    fn deleting_a_credential_returns_the_provider_to_the_picker() {
        let (_dir, db, writer) = writer_with_catalog();
        writer.write_provider_key("main", "sk-one").unwrap();
        writer.write_provider_key("other", "sk-two").unwrap();
        assert_eq!(project(&writer).active_models.len(), 3);

        let ack = writer.delete_provider_key("main").unwrap();
        // The agents that pointed at `main` are handed to `other` in the same commit.
        assert_eq!(ack.docs, vec![DocId::Llm, DocId::Agents]);
        let view = project(&writer);
        let main = view.providers.iter().find(|p| p.id == "main").unwrap();
        assert!(!main.configured);
        assert_eq!(main.masked_api_key, None);
        assert_eq!(view.active_models.len(), 1);
        assert_eq!(view.active_models[0].provider_id, "other");

        // The credential table is the only place the key ever lived.
        let loaded = global_db::load_global_from_path(&db).unwrap();
        assert!(!loaded.provider_credentials.contains_key("main"));
        assert!(loaded.provider_credentials.contains_key("other"));
    }

    #[test]
    fn replacing_a_key_keeps_one_row_per_provider() {
        let (_dir, _db, writer) = writer_with_catalog();
        writer.write_provider_key("main", "sk-first").unwrap();
        writer.write_provider_key("main", "sk-second").unwrap();
        let view = project(&writer);
        let main = view.providers.iter().find(|p| p.id == "main").unwrap();
        assert_eq!(main.masked_api_key.as_deref(), Some("sk-***cond"));
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("sk-first"), "{serialized}");
    }

    #[test]
    fn settings_writes_hand_agents_a_runnable_model() {
        let (_dir, db, writer) = writer_with_catalog();
        // Seeded agents start with an empty model_ref and stay that way while no
        // provider is keyed: there is nothing to hand them.
        let seeded = global_db::load_global_from_path(&db).unwrap();
        assert!(!seeded.agents.is_empty());
        assert!(seeded.agents.values().all(|a| a.model_ref.is_empty()));

        writer.write_provider_key("main", "sk-one").unwrap();
        let healed = global_db::load_global_from_path(&db).unwrap();
        for (id, profile) in &healed.agents {
            assert_eq!(profile.model_ref, "main/default", "agent {id}");
        }

        // A second provider only matters once the first one loses its credential.
        writer.write_provider_key("other", "sk-two").unwrap();
        let ack = writer.delete_provider_key("main").unwrap();
        assert!(ack.docs.contains(&DocId::Agents), "{:?}", ack.docs);
        let moved = global_db::load_global_from_path(&db).unwrap();
        for (id, profile) in &moved.agents {
            assert_eq!(profile.model_ref, "other/small", "agent {id}");
        }
    }

    #[test]
    fn summary_counts_configured_providers_and_active_models() {
        let (_dir, _db, writer) = writer_with_catalog();
        let empty = writer.summary().unwrap();
        assert_eq!(empty.configured_provider_count, 0);
        assert_eq!(empty.active_model_count, 0);
        assert!(empty.setup_guidance.is_some());

        writer.write_provider_key("main", "sk").unwrap();
        let one = writer.summary().unwrap();
        assert_eq!(one.configured_provider_count, 1);
        assert_eq!(one.active_model_count, 2);
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
            retrieval: workspace::WorkspaceRetrievalState { desired: false },
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
        let (_dir, _db, writer) = writer_with_catalog();
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
        let (_dir, _db, writer) = writer_with_catalog();
        let guard = writer.turn_guard().clone();
        guard.begin_turn();
        let err = writer.write_provider_key("main", "sk").unwrap_err();
        assert!(matches!(err, LitecodeError::Config(msg) if msg == "turn_in_progress"));
        guard.end_turn();
        writer.write_provider_key("main", "sk").unwrap();
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
    fn setup_guidance_covers_provider_key_and_agents() {
        let (_dir, _db, writer) = writer_with_catalog();
        let summary = writer.summary().unwrap();
        let guidance = summary.setup_guidance.expect("fresh seed should guide");
        assert!(guidance.contains("Providers"), "{guidance}");
        assert!(guidance.contains("default"), "{guidance}");
        assert!(guidance.contains("compaction"), "{guidance}");
    }

    #[test]
    fn setup_guidance_clears_when_ready() {
        let (_dir, _db, writer) = writer_with_catalog();
        writer.write_provider_key("main", "sk").unwrap();
        let mut settings = writer.load_settings().unwrap();
        settings.agents.get_mut("default").unwrap().model_ref = "main/default".into();
        settings.agents.get_mut("compaction").unwrap().model_ref = "main/compact".into();
        global_db::import_into(writer.db_path(), &settings).unwrap();

        let summary = writer.summary().unwrap();
        assert_eq!(summary.setup_guidance, None);
    }
}
