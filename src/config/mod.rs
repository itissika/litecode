pub mod bridge;
pub mod git_install;
pub mod global_db;
pub mod log_filter;
pub mod manager;
pub mod path;
pub mod resolved;
pub mod schema;
pub mod settings_writer;
pub mod turn_guard;
pub mod workspace;
pub mod workspace_identity;

pub use bridge::{
    DEFAULT_CONTEXT_WINDOW, agent_config_for, agent_config_from_profile, api_model_id_for_agent,
    context_window_for_agent, json_output_for_agent, max_tokens_for_agent,
    reasoning_effort_for_agent, thinking_mode_for_agent, warn_bridge_fallbacks,
};
pub use manager::ConfigManager;
pub use path::{
    canon_abs, canon_abs_lossy, canon_join_nonexistent, is_under, os_probe_abs, strip_verbatim,
};
pub use resolved::{ResolvedConfig, WorkspacePaths, WorkspaceState, resolve};
pub use schema::GlobalSettings;
pub use settings_writer::{
    ProviderView, SettingsChangedEvent, SettingsSummary, SettingsWriteError, SettingsWriter,
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

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default = "default_model_ref")]
    pub model_ref: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            role: default_role(),
            model_ref: default_model_ref(),
            system_prompt: String::new(),
            description: String::new(),
            temperature: default_temperature(),
            max_steps: default_max_steps(),
        }
    }
}

fn default_max_steps() -> u32 {
    50
}

fn default_role() -> String {
    "primary".into()
}

fn default_model_ref() -> String {
    "default".into()
}

fn default_temperature() -> f64 {
    0.7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config_defaults() {
        let config = AgentConfig::default();
        assert_eq!(config.role, "primary");
        assert_eq!(config.model_ref, "default");
        assert_eq!(config.temperature, default_temperature());
    }
}
