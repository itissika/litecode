pub mod binding_scope;
pub mod bridge;
pub mod eval_profile;
pub mod gate;
pub mod git_install;
pub mod global_db;
pub mod guides;
pub mod log_filter;
pub mod manager;
pub mod path;
pub mod resolved;
pub mod schema;
pub mod settings_writer;
pub mod suite_bindings;
pub mod turn_guard;
pub mod workspace;
pub mod workspace_identity;

pub use binding_scope::BindingScope;
pub use bridge::{agent_config_from_profile, primary_agent_infos, warn_unresolved_agent_models};
pub use gate::{ApplyPlan, CommitAck, DocId, EvalView, PersistDoc};
pub use manager::ConfigManager;
pub use path::{
    canon_abs, canon_abs_lossy, canon_join_nonexistent, is_under, os_probe_abs, strip_verbatim,
};
pub use resolved::{ResolvedConfig, WorkspacePaths, WorkspaceState, resolve};
pub use schema::GlobalSettings;
pub use settings_writer::{
    CatalogModelView, LlmProviderView, LlmSettingsView, SettingsChangedEvent, SettingsSummary,
    SettingsWriteError, SettingsWriter,
};
pub use turn_guard::{TurnGuard, cli_turn_guard};
pub use workspace::{
    ContractRead, active_paths, canonicalize_workspace_root, clear_runtime_paths, init_workspace,
    load_workspace_state, read_contract, read_contract_result, resolve_workspace_root,
    set_runtime_paths, workspace_root_from_paths, workspace_root_lap,
};
pub use workspace_identity::{
    ensure_workspace_identity, peek_workspace_id, workspace_identity_path, workspace_registry_path,
};

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub role: String,
    pub model_ref: String,
    pub system_prompt: String,
    pub description: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            role: "primary".into(),
            model_ref: "default".into(),
            system_prompt: String::new(),
            description: String::new(),
        }
    }
}
