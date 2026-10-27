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
    AgentProfile, AgentToolBinding, CustomSuiteDefinition, CustomToolDefinition, GlobalSettings,
    LogSettings, McpServerDefinition, McpTransport, PROTECTED_AGENT_IDS, ToolPreset,
    WebSearchSettings,
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
    let default_ready = settings.agents.get("default").is_some_and(|profile| {
        !profile.model_ref.trim().is_empty()
            && catalog
                .model(&profile.model_ref)
                .is_some_and(|model| keyed(&model.provider_id))
    });
    if !default_ready {
        steps
            .push("assign a model to the default (primary) agent in Settings and Save".to_string());
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
                            CatalogModelView::from_model(model, enabled_for(&model.reference))
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
        let mut settings = self.load()?;
        let defs = settings.custom_tools.clone();
        for profile in settings.agents.values_mut() {
            let notes = super::suite_bindings::collapse_custom_suite_bindings(
                &mut profile.tools,
                &defs,
            );
            for note in notes {
                tracing::warn!("{note}");
            }
        }
        Ok(settings)
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

    /// Workspace file + DB under one lock.
    ///
    /// Snapshots `file_path` before `write`. If mutate / validate / DB fails
    /// after a successful write, restores the snapshot best-effort and returns
    /// the error. Cross-medium is still not a true DB transaction (restore can
    /// fail); generation advances only after both sides succeed.
    fn commit_mixed<W, G>(
        &self,
        docs: &[DocId],
        file_path: &Path,
        write: W,
        mutate: G,
    ) -> Result<CommitAck>
    where
        W: FnOnce() -> Result<()>,
        G: FnOnce(&mut GlobalSettings) -> Result<bool>,
    {
        let _guard = Self::commit_lock();
        self.ensure_writable()
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
        let previous = snapshot_workspace_file(file_path)?;
        write()?;
        let after_write = (|| -> Result<(GlobalSettings, bool)> {
            let mut settings = self.load()?;
            let restart_required = mutate(&mut settings)?;
            ConfigManager::validate(&settings)?;
            let conn = global_db::open(&self.db_path)?;
            store::replace_all(&conn, &settings)?;
            Ok((settings, restart_required))
        })();
        let (settings, restart_required) = match after_write {
            Ok(ok) => ok,
            Err(e) => {
                if let Err(restore_err) = restore_workspace_file(file_path, previous.as_deref()) {
                    tracing::error!(
                        error = %restore_err,
                        path = %file_path.display(),
                        "commit_mixed: failed to restore workspace file after DB-side error"
                    );
                }
                return Err(e);
            }
        };
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
                "provider '{provider_id}' is not declared in the provider catalog"
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
                "provider '{provider_id}' is not declared in the provider catalog"
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
                "model '{model_ref}' is not declared in the provider catalog"
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

    pub fn write_agent(&self, id: &str, mut profile: AgentProfile) -> Result<CommitAck> {
        validate_agent_id(id)?;
        let loaded = self.load()?;
        let defs = loaded.custom_tools.clone();
        for note in super::suite_bindings::collapse_custom_suite_bindings(&mut profile.tools, &defs)
        {
            tracing::warn!("{note}");
        }
        normalize_agent_profile(&mut profile);
        let id = id.to_string();
        self.commit_partial(&[DocId::Agents], |settings| {
            // WP-149: a newly created primary agent binds code_search by default
            // unless the caller sent the key (true or false). Existing agents
            // switched to primary are not auto-bound.
            if !settings.agents.contains_key(&id)
                && profile.role == crate::config::schema::AgentRole::Primary
            {
                for tool in super::global_db::tools::default_bound_optional_builtins() {
                    if !profile.tools.contains_key(*tool) {
                        profile.tools.insert(
                            (*tool).to_string(),
                            super::global_db::default_optional_binding(tool),
                        );
                    }
                }
            }
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
        workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        validate_agent_id(agent_id)?;
        let loaded = self.load()?;
        if !loaded.agents.contains_key(agent_id) {
            return Err(LitecodeError::Config(format!(
                "agent not found: {agent_id}"
            )));
        }
        let safe = merged_custom_rules(
            &loaded.custom_tools,
            &workspace.workspace_custom_tools,
            &loaded.custom_suites,
            &workspace.workspace_custom_suites,
        );
        let mut defs = loaded.custom_tools.clone();
        for (name, definition) in &workspace.workspace_custom_tools {
            defs.retain(|tool| &tool.name != name);
            defs.push(definition.clone());
        }
        let id = agent_id.to_string();
        self.commit_partial(&[DocId::Agents], move |settings| {
            let profile = settings
                .agents
                .get_mut(&id)
                .expect("agent exists after load check");
            for (tool_id, binding) in profile.tools.iter_mut() {
                apply_forced_preset(tool_id, binding, preset, &safe);
            }
            for note in super::suite_bindings::collapse_custom_suite_bindings(
                &mut profile.tools,
                &defs,
            ) {
                tracing::warn!("{note}");
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
        let suites = self.load()?.custom_suites;
        validate_custom_definition_with_suites(id, &mut def, &suites)?;

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
            let removed_suite = settings
                .custom_tools
                .iter()
                .find(|tool| tool.name == id)
                .and_then(|tool| tool.suite.clone());
            let before = settings.custom_tools.len();
            settings.custom_tools.retain(|t| t.name != id);
            if settings.custom_tools.len() == before {
                return Err(LitecodeError::Config(format!(
                    "custom tool not found: {id}"
                )));
            }
            if !keep_binding {
                for profile in settings.agents.values_mut() {
                    match removed_suite.as_deref() {
                        Some(suite) => {
                            let still_declared = settings
                                .custom_tools
                                .iter()
                                .any(|tool| tool.suite.as_deref() == Some(suite))
                                || workspace
                                    .workspace_custom_tools
                                    .values()
                                    .any(|tool| tool.suite.as_deref() == Some(suite));
                            if !still_declared {
                                profile
                                    .tools
                                    .remove(&super::suite_bindings::suite_id(suite));
                            }
                        }
                        None => {
                            profile.tools.remove(id);
                        }
                    }
                }
            }
            Ok(false)
        })
    }

    pub fn list_custom_suites(&self) -> Result<Vec<(String, CustomSuiteDefinition)>> {
        let settings = self.load()?;
        let mut suites: Vec<_> = settings.custom_suites.into_iter().collect();
        suites.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(suites)
    }

    pub fn get_custom_suite(&self, id: &str) -> Result<Option<CustomSuiteDefinition>> {
        Ok(self.load()?.custom_suites.get(id).cloned())
    }

    pub fn write_custom_suite(
        &self,
        id: &str,
        mut def: CustomSuiteDefinition,
    ) -> Result<CommitAck> {
        let settings = self.load()?;
        validate_custom_suite(id, &mut def)?;
        if settings.custom_tools.iter().any(|tool| tool.name == id) {
            return Err(LitecodeError::Config(format!(
                "custom suite id '{id}' collides with a custom tool name"
            )));
        }
        self.commit_partial(&[DocId::CustomToolsGlobal], move |settings| {
            settings.custom_suites.insert(id.to_string(), def.clone());
            Ok(false)
        })
    }

    pub fn delete_custom_suite(
        &self,
        id: &str,
        _workspace: &super::resolved::WorkspaceState,
    ) -> Result<CommitAck> {
        let members = self
            .load()?
            .custom_tools
            .iter()
            .filter(|tool| tool.suite.as_deref() == Some(id))
            .count();
        if members > 0 {
            return Err(LitecodeError::Config(format!(
                "custom suite '{id}' still has {members} member(s); move or delete them first"
            )));
        }
        self.commit_partial(&[DocId::CustomToolsGlobal, DocId::Agents], |settings| {
            if settings.custom_suites.remove(id).is_none() {
                return Err(LitecodeError::Config(format!(
                    "custom suite not found: {id}"
                )));
            }
            for profile in settings.agents.values_mut() {
                profile
                    .tools
                    .remove(&super::suite_bindings::suite_id(id));
            }
            Ok(false)
        })
    }

    pub fn list_workspace_custom_suites(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<(String, CustomSuiteDefinition)>> {
        let mut suites: Vec<_> = workspace::read_workspace_custom_tools(workspace_root)?
            .suites
            .into_iter()
            .collect();
        suites.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(suites)
    }

    pub fn get_workspace_custom_suite(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<Option<CustomSuiteDefinition>> {
        Ok(workspace::read_workspace_custom_tools(workspace_root)?
            .suites
            .get(id)
            .cloned())
    }

    pub fn write_workspace_custom_suite(
        &self,
        workspace_root: &Path,
        id: &str,
        mut def: CustomSuiteDefinition,
    ) -> Result<CommitAck> {
        let file = workspace::read_workspace_custom_tools(workspace_root)?;
        validate_custom_suite(id, &mut def)?;
        if file.tools.values().any(|tool| tool.name == id) {
            return Err(LitecodeError::Config(format!(
                "custom suite id '{id}' collides with a custom tool name"
            )));
        }
        let root = workspace_root.to_path_buf();
        self.commit_workspace_file(DocId::CustomToolsWorkspace, move || {
            workspace::upsert_workspace_custom_suite(&root, id, def)
        })
    }

    pub fn delete_workspace_custom_suite(
        &self,
        workspace_root: &Path,
        id: &str,
    ) -> Result<CommitAck> {
        let file = workspace::read_workspace_custom_tools(workspace_root)?;
        let members = file
            .tools
            .values()
            .filter(|tool| tool.suite.as_deref() == Some(id))
            .count();
        if members > 0 {
            return Err(LitecodeError::Config(format!(
                "custom suite '{id}' still has {members} member(s); move or delete them first"
            )));
        }
        let root = workspace_root.to_path_buf();
        self.commit_workspace_file(DocId::CustomToolsWorkspace, move || {
            workspace::delete_workspace_custom_suite(&root, id).map(|_| ())
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
        let suites = workspace::read_workspace_custom_tools(workspace_root)?.suites;
        validate_custom_definition_with_suites(id, &mut def, &suites)?;
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
        let loaded = self.load()?;
        let keep_binding = loaded.custom_tools.iter().any(|t| t.name == id);
        let removed_suite = file.tools.get(id).and_then(|tool| tool.suite.clone());
        let root = workspace_root.to_path_buf();
        let id_owned = id.to_string();
        if keep_binding {
            return self.commit_workspace_file(DocId::CustomToolsWorkspace, move || {
                workspace::delete_workspace_custom_tool(&root, &id_owned).map(|_| ())
            });
        }
        let strip_id = id.to_string();
        let suite_still_declared = removed_suite.as_deref().is_some_and(|suite| {
            let in_workspace = file
                .tools
                .values()
                .any(|tool| tool.name != id && tool.suite.as_deref() == Some(suite));
            let in_global = loaded
                .custom_tools
                .iter()
                .any(|tool| tool.suite.as_deref() == Some(suite));
            in_workspace || in_global
        });
        let file_path = workspace::workspace_custom_tools_path(&root);
        self.commit_mixed(
            &[DocId::CustomToolsWorkspace, DocId::Agents],
            &file_path,
            move || workspace::delete_workspace_custom_tool(&root, &id_owned).map(|_| ()),
            move |settings| {
                for profile in settings.agents.values_mut() {
                    match removed_suite.as_deref() {
                        Some(suite) if !suite_still_declared => {
                            profile
                                .tools
                                .remove(&super::suite_bindings::suite_id(suite));
                        }
                        Some(_) => {}
                        None => {
                            profile.tools.remove(&strip_id);
                        }
                    }
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
        let file_path = workspace::workspace_mcp_path(&root);
        self.commit_mixed(
            &[DocId::McpWorkspace, DocId::Agents],
            &file_path,
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
                    "provider and model facts are part of this build; set a key in Settings → Providers or PUT /api/settings/providers/{provider_id}/key"
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

/// Non-empty rule lists keyed by the binding that owns them. Workspace custom
/// tools and suites replace global entries. Suite members route to one
/// `suite:<name>` binding, so only the suite's own rules make that binding
/// dialable.
fn merged_custom_rules(
    global: &[CustomToolDefinition],
    workspace: &HashMap<String, CustomToolDefinition>,
    global_suites: &HashMap<String, CustomSuiteDefinition>,
    workspace_suites: &HashMap<String, CustomSuiteDefinition>,
) -> HashMap<String, Vec<crate::permission::PolicyRule>> {
    let mut effective: HashMap<String, (Option<String>, Vec<crate::permission::PolicyRule>)> =
        HashMap::new();
    for tool in global {
        effective.insert(
            tool.name.clone(),
            (tool.suite.clone(), tool.rules.clone()),
        );
    }
    for (name, tool) in workspace {
        effective.insert(name.clone(), (tool.suite.clone(), tool.rules.clone()));
    }

    let mut suites: HashMap<String, Vec<crate::permission::PolicyRule>> = HashMap::new();
    for (name, suite) in global_suites {
        suites.insert(name.clone(), suite.rules.clone());
    }
    for (name, suite) in workspace_suites {
        suites.insert(name.clone(), suite.rules.clone());
    }

    let mut rules = HashMap::new();
    for (name, (suite, declared)) in effective {
        if let Some(suite) = suite.filter(|suite| !suite.trim().is_empty()) {
            if let Some(suite_rules) = suites.get(&suite)
                && !suite_rules.is_empty()
            {
                rules.insert(
                    crate::config::suite_bindings::suite_id(&suite),
                    suite_rules.clone(),
                );
            }
        } else if !declared.is_empty() {
            rules.insert(name, declared);
        }
    }
    rules
}

/// Bulk dial: write only the intent. Fixed rows keep `preset = None`; custom
/// rows get a dial only when their current definition declares rules.
fn apply_forced_preset(
    tool_id: &str,
    binding: &mut AgentToolBinding,
    preset: ToolPreset,
    custom_rules: &HashMap<String, Vec<crate::permission::PolicyRule>>,
) {
    let mcp = tools::is_mcp_catalog_id(tool_id);
    if !mcp {
        binding.allowed_tools = None;
    }
    let dialable = crate::permission::has_permission_preset(tool_id)
        || custom_rules
            .get(tool_id)
            .is_some_and(|rules| !rules.is_empty());
    binding.preset = dialable.then_some(preset);
}

pub(crate) fn validate_mcp_definition(id: &str, def: &mut McpServerDefinition) -> Result<()> {
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

/// Capture current bytes of a workspace settings file (or `None` if missing).
fn snapshot_workspace_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(LitecodeError::Config(format!(
            "snapshot {}: {e}",
            path.display()
        ))),
    }
}

/// Restore a workspace settings file from [`snapshot_workspace_file`].
fn restore_workspace_file(path: &Path, previous: Option<&[u8]>) -> Result<()> {
    match previous {
        Some(bytes) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| LitecodeError::Config(e.to_string()))?;
            }
            std::fs::write(path, bytes).map_err(|e| LitecodeError::Config(e.to_string()))
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(LitecodeError::Config(format!(
                "restore remove {}: {e}",
                path.display()
            ))),
        },
    }
}

/// Custom tool rules shared by the settings page and `litecode_workspace refresh`.
///
/// Normalizes the body in place (empty name → id, empty schema type → object,
/// `timeout: 0` → 120) and rejects ids the runtime could not bind.
pub(crate) fn validate_custom_definition(id: &str, def: &mut CustomToolDefinition) -> Result<()> {
    validate_custom_definition_with_suites(id, def, &HashMap::new())
}

pub(crate) fn normalize_custom_definition(id: &str, def: &mut CustomToolDefinition) {
    if def.name.is_empty() {
        def.name = id.to_string();
    }
    if def.schema.schema_type.trim().is_empty() {
        def.schema.schema_type = "object".into();
    }
    if def.timeout == 0 {
        def.timeout = 120;
    }
    for rule in &mut def.rules {
        rule.id = rule.id.trim().to_string();
    }
}

pub(crate) fn validate_custom_definition_with_suites(
    id: &str,
    def: &mut CustomToolDefinition,
    suites: &HashMap<String, CustomSuiteDefinition>,
) -> Result<()> {
    normalize_custom_definition(id, def);
    if let Some(diagnostic) = custom_definition_diagnostic(id, def, suites, "settings", None) {
        return Err(LitecodeError::Config(diagnostic.message));
    }
    Ok(())
}

pub(crate) fn validate_custom_suite(id: &str, def: &mut CustomSuiteDefinition) -> Result<()> {
    validate_tool_id(id)?;
    if let Some(reason) = tools::non_transport_name_rejection(id)
        .or_else(|| tools::reserved_command_name_rejection(id))
    {
        return Err(LitecodeError::Config(reason.to_string()));
    }
    if tools::is_core_tool(id)
        || tools::is_optional_builtin(id)
        || crate::tools::lsp::LSP_CODE_TOOL_NAMES.contains(&id)
    {
        return Err(LitecodeError::Config(format!(
            "custom suite id '{id}' conflicts with a builtin tool"
        )));
    }
    if let Some(error) = crate::permission::policy::custom_rule_ids_error(&def.rules) {
        return Err(LitecodeError::Config(format!(
            "custom suite '{id}' has invalid rules: {error}"
        )));
    }
    if def.label.trim().is_empty() {
        def.label = id.to_string();
    }
    Ok(())
}

/// One first diagnostic for a custom definition. Validation, settings projection
/// and refresh all consume this so the field and fix stay identical.
pub(crate) fn custom_definition_diagnostic(
    id: &str,
    def: &CustomToolDefinition,
    suites: &HashMap<String, CustomSuiteDefinition>,
    source: &str,
    file: Option<String>,
) -> Option<crate::config::schema::CustomToolDiagnostic> {
    use crate::config::schema::{CallSurface, CustomToolDiagnostic};

    let tool = if def.name.trim().is_empty() {
        id.to_string()
    } else {
        def.name.clone()
    };
    let suite = def
        .suite
        .as_deref()
        .map(str::trim)
        .filter(|suite| !suite.is_empty())
        .map(str::to_string);
    let diag = |field: &str,
                actual: Option<String>,
                expected: &str,
                fix: &str| {
        let mut message = format!(
            "{source}: custom tool `{tool}` field `{field}`: expected {expected}"
        );
        if let Some(actual) = actual.as_deref() {
            message.push_str(&format!("; actual `{actual}`"));
        }
        message.push_str(&format!("; fix: {fix}"));
        CustomToolDiagnostic {
            source: source.to_string(),
            file: file.clone(),
            tool: Some(tool.clone()),
            suite: suite.clone(),
            field: field.to_string(),
            actual,
            expected: expected.to_string(),
            fix: fix.to_string(),
            message,
        }
    };

    if validate_tool_id(id).is_err() {
        return Some(diag(
            "name",
            Some(id.to_string()),
            "[a-z][a-z0-9_]*",
            "rename the tool id; this is the binding key",
        ));
    }
    if let Some(reason) = tools::non_transport_name_rejection(id)
        .or_else(|| tools::reserved_command_name_rejection(id))
    {
        return Some(diag(
            "name",
            Some(id.to_string()),
            "a non-reserved tool name",
            &reason,
        ));
    }
    if let Some(prefix) = ["mcp_", "lsp_", "subagent_"]
        .into_iter()
        .find(|prefix| id.starts_with(prefix))
    {
        return Some(diag(
            "name",
            Some(id.to_string()),
            "a name outside the mcp_ / lsp_ / subagent_ namespaces",
            &format!("rename the tool without the '{prefix}' prefix"),
        ));
    }
    if tools::is_core_tool(id)
        || tools::is_optional_builtin(id)
        || crate::tools::lsp::LSP_CODE_TOOL_NAMES.contains(&id)
    {
        return Some(diag(
            "name",
            Some(id.to_string()),
            "a name that is not a builtin tool",
            "choose a different custom tool id",
        ));
    }
    if def.name != id {
        return Some(diag(
            "name",
            Some(def.name.clone()),
            &format!("`{id}` (the map/path key)"),
            "set name to the same value as the key",
        ));
    }
    if suites.contains_key(id) {
        return Some(diag(
            "name",
            Some(id.to_string()),
            "a name not used by a custom suite",
            "rename the tool; suite names and tool names share one namespace",
        ));
    }
    if def.command.trim().is_empty() {
        return Some(diag(
            "command",
            Some(def.command.clone()),
            "a non-empty executable command",
            "set command to the program to run",
        ));
    }
    if def.schema.schema_type != "object" {
        return Some(diag(
            "schema.type",
            Some(def.schema.schema_type.clone()),
            "object",
            "make the input schema an object schema",
        ));
    }

    let surface = match suite.as_deref() {
        Some(suite_name) => {
            if def.surface.is_some() {
                return Some(diag(
                    "surface",
                    serde_json::to_string(&def.surface).ok(),
                    "omitted on a suite member",
                    "remove surface; the suite owns it",
                ));
            }
            if !def.rules.is_empty() {
                return Some(diag(
                    "rules",
                    Some(format!("{} rule(s)", def.rules.len())),
                    "empty on a suite member",
                    &format!("move rules to suites.{suite_name}.rules"),
                ));
            }
            if validate_tool_id(suite_name).is_err() {
                return Some(diag(
                    "suite",
                    Some(suite_name.to_string()),
                    "[a-z][a-z0-9_]*",
                    "rename the suite",
                ));
            }
            let Some(suite_def) = suites.get(suite_name) else {
                return Some(diag(
                    "suite",
                    Some(suite_name.to_string()),
                    "an existing suites entry",
                    &format!(
                        "add suites.{suite_name} with surface, label and rules, or remove suite from the tool"
                    ),
                ));
            };
            if let Some(error) = crate::permission::policy::custom_rule_ids_error(&suite_def.rules) {
                return Some(diag(
                    &format!("suites.{suite_name}.rules"),
                    Some(error),
                    "rules with unique non-empty ids",
                    "fix the suite rule ids",
                ));
            }
            suite_def.surface
        }
        None => {
            let Some(surface) = def.surface else {
                return Some(diag(
                    "surface",
                    None,
                    "`direct` or `code` (standalone), or a suite reference",
                    "add surface: \"direct\" for model-facing tools, or suite: \"<name>\" for members",
                ));
            };
            if let Some(error) = crate::permission::policy::custom_rule_ids_error(&def.rules) {
                return Some(diag(
                    "rules",
                    Some(error),
                    "rules with unique non-empty ids",
                    "fix the rule ids",
                ));
            }
            surface
        }
    };

    match surface {
        CallSurface::Direct => {
            if def.output_schema.is_some() {
                return Some(diag(
                    "output_schema",
                    Some("present".into()),
                    "omitted on a direct tool",
                    "remove output_schema; direct tools return text or an envelope",
                ));
            }
        }
        CallSurface::Code => {
            let Some(schema) = def.output_schema.as_ref() else {
                return Some(diag(
                    "output_schema",
                    None,
                    "an object schema",
                    "add output_schema: { \"type\": \"object\", \"properties\": {}, \"required\": [] }",
                ));
            };
            if schema.get("type").and_then(serde_json::Value::as_str) != Some("object") {
                return Some(diag(
                    "output_schema",
                    Some("not an object schema".into()),
                    "type: object",
                    "make output_schema an object schema",
                ));
            }
        }
    }

    if def
        .schema
        .properties
        .as_object()
        .is_some_and(|properties| properties.contains_key("run_in_background"))
    {
        return Some(diag(
            "schema.properties.run_in_background",
            Some("present".into()),
            "absent",
            "remove run_in_background; Litecode adds it for direct tools",
        ));
    }

    let mut effective = def.clone();
    effective.surface = Some(surface);
    if let Some(suite_name) = suite.as_deref() {
        if let Some(suite_def) = suites.get(suite_name) {
            effective.rules = suite_def.rules.clone();
        }
    }
    if let Err(error) = crate::tools::custom::validate_custom_code_contract(&effective, surface) {
        return Some(diag(
            "schema",
            Some(error.clone()),
            "a code-admissible object schema",
            "use a simple input/output object schema with no unsupported keywords (no $ref), or declare surface \"direct\"",
        ));
    }
    None
}

pub(crate) fn validate_tool_id(id: &str) -> Result<()> {
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
        let catalog = std::sync::Arc::new(
            crate::provider_catalog::ProviderCatalog::parse(
                TEST_CATALOG,
                std::path::Path::new("provider-catalog.toml"),
            )
            .expect("test catalog"),
        );
        crate::provider_catalog::pin(&db, catalog);
        crate::config::global_db::open(&db).unwrap();
        let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        (dir, db, writer)
    }

    #[test]
    fn model_switch_rejects_a_reference_the_catalog_does_not_declare() {
        let (_dir, _db, writer) = writer_with_catalog();
        let unknown = writer.set_model_enabled("main/ghost", false).unwrap_err();
        assert!(unknown.to_string().contains("main/ghost"), "{unknown}");
    }

    fn declared_rules() -> Vec<crate::permission::PolicyRule> {
        use crate::permission::{ArgMatcher, PermissionAction, PolicyRule};
        vec![PolicyRule {
            id: "outside_workspace".into(),
            when: ArgMatcher::PathOutsideWorkspace {
                name: "path".into(),
            },
            action: PermissionAction::Deny,
        }]
    }

    #[test]
    fn code_surface_custom_tool_rejects_unrenderable_schema_with_field_reason() {
        let mut def = custom_def("code_probe", Vec::new());
        def.surface = Some(crate::config::schema::CallSurface::Code);
        def.output_schema = Some(serde_json::json!({
            "type": "object", "properties": {}, "required": []
        }));
        validate_custom_definition("code_probe", &mut def).expect("simple schema is admissible");

        def.schema.properties = serde_json::json!({
            "item": { "$ref": "#/Item" }
        });
        let error = validate_custom_definition("code_probe", &mut def)
            .expect_err("unresolved ref must fail code admission");
        let message = error.to_string();
        assert!(message.contains("surface 'code'"), "{message}");
        assert!(message.contains("direct"), "{message}");
    }

    fn custom_def(name: &str, rules: Vec<crate::permission::PolicyRule>) -> CustomToolDefinition {
        use crate::config::schema::ToolSchema;
        CustomToolDefinition {
            name: name.into(),
            output_schema: None,
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "demo".into(),
            args: Vec::new(),
            timeout: 120,
            rules,
            surface: Some(crate::config::schema::CallSurface::Direct),
            suite: None,
        }
    }

    fn dial_binding(preset: Option<super::ToolPreset>) -> AgentToolBinding {
        AgentToolBinding {
            enabled: true,
            preset: preset,
            allowed_tools: None,
        }
    }

    fn diagnostic(
        surface: Option<crate::config::schema::CallSurface>,
        suite: Option<&str>,
        rules: Vec<crate::permission::PolicyRule>,
        output_schema: Option<serde_json::Value>,
    ) -> crate::config::schema::CustomToolDiagnostic {
        let def = CustomToolDefinition {
            name: "probe".into(),
            description: String::new(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 120,
            rules,
            surface,
            suite: suite.map(str::to_string),
            output_schema,
        };
        custom_definition_diagnostic("probe", &def, &HashMap::new(), "test", None)
            .expect("probe must be invalid")
    }

    #[test]
    fn custom_definition_diagnostics_name_the_field_and_fix() {
        let missing = diagnostic(None, None, Vec::new(), None);
        assert_eq!(missing.field, "surface");
        assert!(missing.fix.contains("direct"), "{}", missing.fix);

        let direct_output = diagnostic(
            Some(crate::config::schema::CallSurface::Direct),
            None,
            Vec::new(),
            Some(serde_json::json!({"type": "object"})),
        );
        assert_eq!(direct_output.field, "output_schema");

        let code_missing = diagnostic(
            Some(crate::config::schema::CallSurface::Code),
            None,
            Vec::new(),
            None,
        );
        assert_eq!(code_missing.field, "output_schema");
        assert!(code_missing.fix.contains("output_schema"));

        let member_surface = diagnostic(
            Some(crate::config::schema::CallSurface::Code),
            Some("deck"),
            Vec::new(),
            Some(serde_json::json!({"type": "object"})),
        );
        assert_eq!(member_surface.field, "surface");

        let member_rules = diagnostic(
            None,
            Some("deck"),
            declared_rules(),
            Some(serde_json::json!({"type": "object"})),
        );
        assert_eq!(member_rules.field, "rules");
    }

    #[test]
    fn one_bad_definition_keeps_the_same_field_and_fix_across_sources() {
        let mut def = custom_def("probe", Vec::new());
        def.surface = None;
        let suites = HashMap::new();
        let settings = crate::config::settings_writer::custom_definition_diagnostic(
            "probe", &def, &suites, "settings", None,
        )
        .expect("missing surface");
        let refresh = crate::config::settings_writer::custom_definition_diagnostic(
            "probe",
            &def,
            &suites,
            "refresh",
            Some(".litecode/custom_tools.json".into()),
        )
        .expect("missing surface");
        let validate = crate::config::settings_writer::custom_definition_diagnostic(
            "probe",
            &def,
            &suites,
            "validate_custom",
            None,
        )
        .expect("missing surface");
        for other in [&refresh, &validate] {
            assert_eq!(settings.field, other.field);
            assert_eq!(settings.expected, other.expected);
            assert_eq!(settings.fix, other.fix);
            assert_eq!(settings.actual, other.actual);
        }
    }

    #[test]
    fn suite_member_inherits_surface_and_output_contract() {
        let mut suites = HashMap::new();
        suites.insert(
            "deck".into(),
            CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Code,
                label: "Deck".into(),
                rules: Vec::new(),
            },
        );
        let mut def = CustomToolDefinition {
            name: "probe".into(),
            description: String::new(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 120,
            rules: Vec::new(),
            surface: None,
            suite: Some("deck".into()),
            output_schema: Some(serde_json::json!({
                "type": "object", "properties": {}, "required": []
            })),
        };
        validate_custom_definition_with_suites("probe", &mut def, &suites)
            .expect("suite member inherits a code surface");

        def.output_schema = None;
        let error = validate_custom_definition_with_suites("probe", &mut def, &suites)
            .unwrap_err();
        assert!(error.to_string().contains("output_schema"), "{error}");
    }

    #[test]
    fn custom_tool_name_cannot_equal_a_suite_name() {
        let mut suites = HashMap::new();
        suites.insert(
            "deck".into(),
            CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Direct,
                label: "Deck".into(),
                rules: Vec::new(),
            },
        );
        let mut def = custom_def("deck", Vec::new());
        let error = validate_custom_definition_with_suites("deck", &mut def, &suites)
            .expect_err("suite and tool names share one namespace");
        assert!(error.to_string().contains("custom suite"), "{error}");
    }

    #[test]
    fn custom_tool_cannot_claim_a_reserved_namespace() {
        for name in ["mcp_probe", "lsp_probe", "subagent_probe"] {
            let mut def = custom_def(name, Vec::new());
            let error = validate_custom_definition(name, &mut def)
                .expect_err("reserved namespace must be rejected");
            assert!(error.to_string().contains("namespaces"), "{error}");
        }
    }

    #[test]
    fn custom_tool_cannot_shadow_an_lsp_member() {
        let mut def = custom_def("hover", Vec::new());
        let error = validate_custom_definition("hover", &mut def)
            .expect_err("lsp member names are reserved");
        assert!(error.to_string().contains("builtin tool"), "{error}");
    }

    #[test]
    fn workspace_custom_tool_surface_roundtrips_through_json() {
        let ws = TempDir::new().unwrap();
        let root = ws.path().to_path_buf();
        let mut def = custom_def("code_probe", Vec::new());
        def.surface = Some(crate::config::schema::CallSurface::Code);
        def.output_schema = Some(serde_json::json!({
            "type": "object", "properties": {}, "required": []
        }));
        validate_custom_definition("code_probe", &mut def).expect("valid code contract");
        workspace::upsert_workspace_custom_tool(&root, def).unwrap();

        let loaded = workspace::read_workspace_custom_tools(&root).unwrap();
        assert_eq!(
            loaded.tools["code_probe"].surface,
            Some(crate::config::schema::CallSurface::Code)
        );
    }

    #[test]
    fn merged_rules_come_from_the_suite_definition() {
        let mut members = custom_def("suite_a", Vec::new());
        members.suite = Some("deck".into());
        let mut suites = HashMap::new();
        suites.insert(
            "deck".into(),
            CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Direct,
                label: "Deck".into(),
                rules: declared_rules(),
            },
        );
        let safe = super::merged_custom_rules(
            &[members],
            &std::collections::HashMap::new(),
            &suites,
            &std::collections::HashMap::new(),
        );
        let suite = safe
            .get(&super::super::suite_bindings::suite_id("deck"))
            .expect("suite rules");
        assert_eq!(suite.len(), 1);

        let mut binding = AgentToolBinding {
            enabled: true,
            preset: None,
            allowed_tools: None,
        };
        super::apply_forced_preset("suite:deck", &mut binding, ToolPreset::Safe, &safe);
        assert_eq!(binding.preset, Some(ToolPreset::Safe));
    }

    #[test]
    fn workspace_suite_rules_replace_global_suite_rules() {
        let global = [(
            "deck".to_string(),
            CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Direct,
                label: "Deck".into(),
                rules: declared_rules(),
            },
        )];
        let workspace = [(
            "deck".to_string(),
            CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Direct,
                label: "Deck".into(),
                rules: Vec::new(),
            },
        )];
        let mut members = custom_def("suite_a", Vec::new());
        members.suite = Some("deck".into());
        let safe = super::merged_custom_rules(
            &[members],
            &std::collections::HashMap::new(),
            &global.into_iter().collect(),
            &workspace.into_iter().collect(),
        );
        assert!(safe.is_empty(), "workspace suite rules replace global rules");
    }

    #[test]
    fn workspace_custom_without_rules_hides_global_rules() {
        let declared = declared_rules();
        let global = vec![custom_def("demo", declared)];
        let mut workspace = std::collections::HashMap::new();
        workspace.insert("demo".into(), custom_def("demo", Vec::new()));
        let safe = super::merged_custom_rules(
            &global,
            &workspace,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        );
        assert!(safe.is_empty());
    }

    /// ISSUE-017 / WP-008: after a successful file write, a DB-side failure
    /// must restore the prior file bytes (no durable half-write).
    #[test]
    fn commit_mixed_restores_file_when_db_step_fails() {
        let (_dir, _db, writer) = writer_with_catalog();
        let ws = TempDir::new().unwrap();
        let root = ws.path().to_path_buf();
        let path = workspace::workspace_custom_tools_path(&root);
        workspace::upsert_workspace_custom_tool(&root, custom_def("demo", Vec::new())).unwrap();
        let before = std::fs::read(&path).expect("snapshot before write");

        let err = writer
            .commit_mixed(
                &[DocId::CustomToolsWorkspace, DocId::Agents],
                &path,
                {
                    let root = root.clone();
                    move || workspace::delete_workspace_custom_tool(&root, "demo").map(|_| ())
                },
                |_settings| Err(LitecodeError::Config("injected replace_all failure".into())),
            )
            .expect_err("DB-side failure must surface");
        assert!(
            err.to_string().contains("injected replace_all failure"),
            "got: {err}"
        );
        let after = std::fs::read(&path).expect("file must still exist after restore");
        assert_eq!(
            after, before,
            "workspace file must be restored to pre-write bytes"
        );
        let restored = workspace::read_workspace_custom_tools(&root).unwrap();
        assert!(
            restored.tools.contains_key("demo"),
            "demo tool must be present again after restore"
        );
    }

    /// Same restore path when validate rejects the mutated settings.
    #[test]
    fn commit_mixed_restores_file_when_validate_fails() {
        let (_dir, _db, writer) = writer_with_catalog();
        let ws = TempDir::new().unwrap();
        let root = ws.path().to_path_buf();
        let path = workspace::workspace_mcp_path(&root);
        let def = McpServerDefinition {
            command: "echo".into(),
            ..Default::default()
        };
        workspace::upsert_workspace_mcp(&root, "demo_mcp", def).unwrap();
        let before = std::fs::read(&path).unwrap();

        let err = writer
            .commit_mixed(
                &[DocId::McpWorkspace, DocId::Agents],
                &path,
                {
                    let root = root.clone();
                    move || workspace::delete_workspace_mcp(&root, "demo_mcp").map(|_| ())
                },
                |settings| {
                    // Empty tool id fails ConfigManager::validate.
                    if let Some(profile) = settings.agents.values_mut().next() {
                        profile.tools.insert(String::new(), dial_binding(None));
                    }
                    Ok(false)
                },
            )
            .expect_err("validate failure must surface");
        assert!(err.to_string().contains("empty tool binding"), "got: {err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(
            workspace::read_workspace_mcp(&root)
                .unwrap()
                .servers
                .contains_key("demo_mcp")
        );
    }

    // ---- WP-149: new primary agents bind code_search by default ----

    fn cs_binding(writer: &SettingsWriter, id: &str) -> Option<bool> {
        writer
            .load_settings()
            .unwrap()
            .agents
            .get(id)
            .and_then(|p| p.tools.get("code_search"))
            .map(|b| b.enabled)
    }

    #[test]
    fn write_agent_roundtrips_intent_without_a_workspace() {
        let (_dir, db, writer) = writer_with_catalog();
        let mut profile = AgentProfile {
            role: crate::config::schema::AgentRole::Subagent,
            ..Default::default()
        };
        profile.tools.insert(
            "read".into(),
            AgentToolBinding {
                enabled: true,
                preset: Some(super::ToolPreset::Safe),
                allowed_tools: None,
            },
        );
        profile.tools.insert(
            "mcp_probe".into(),
            AgentToolBinding {
                enabled: true,
                preset: Some(super::ToolPreset::All),
                allowed_tools: Some(vec!["search".into()]),
            },
        );
        writer.write_agent("ab_agent", profile).unwrap();

        // A second writer has no workspace input; the stored intent is complete.
        let reopened = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
        let loaded = reopened.load_settings().unwrap();
        let saved = &loaded.agents["ab_agent"].tools;
        assert_eq!(saved["read"].preset, Some(super::ToolPreset::Safe));
        assert_eq!(saved["mcp_probe"].preset, Some(super::ToolPreset::All));
        assert_eq!(
            saved["mcp_probe"].allowed_tools.as_deref(),
            Some(&["search".to_string()][..])
        );
    }

    #[test]
    fn write_agent_new_primary_missing_code_search_binds_true() {
        let (_dir, _db, writer) = writer_with_catalog();
        let profile = AgentProfile {
            role: crate::config::schema::AgentRole::Primary,
            ..Default::default()
        };
        writer.write_agent("wp149_new", profile).unwrap();
        assert_eq!(cs_binding(&writer, "wp149_new"), Some(true));
    }

    #[test]
    fn write_agent_new_primary_explicit_false_stays_false() {
        let (_dir, _db, writer) = writer_with_catalog();
        let mut off = crate::config::global_db::default_optional_binding("code_search");
        off.enabled = false;
        let mut profile = AgentProfile {
            role: crate::config::schema::AgentRole::Primary,
            ..Default::default()
        };
        profile.tools.insert("code_search".into(), off);
        writer.write_agent("wp149_off", profile).unwrap();
        assert_eq!(cs_binding(&writer, "wp149_off"), Some(false));
    }

    #[test]
    fn write_agent_existing_agent_switched_to_primary_not_auto_bound() {
        let (_dir, _db, writer) = writer_with_catalog();
        let sub = AgentProfile {
            role: crate::config::schema::AgentRole::Subagent,
            ..Default::default()
        };
        writer.write_agent("wp149_sw", sub).unwrap();
        assert_eq!(cs_binding(&writer, "wp149_sw"), None);
        let prim = AgentProfile {
            role: crate::config::schema::AgentRole::Primary,
            ..Default::default()
        };
        writer.write_agent("wp149_sw", prim).unwrap();
        assert_eq!(cs_binding(&writer, "wp149_sw"), None);
    }
}
