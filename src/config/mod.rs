pub mod bridge;
pub mod git_install;
pub mod global_db;
pub mod log_filter;
pub mod manager;
pub mod path;
pub mod resolved;
pub mod runtime_catalog_state;
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

#[derive(Debug, Clone, Deserialize)]
pub struct HookCommand {
    #[serde(rename = "type")]
    pub hook_type: String,
    pub command: String,
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
}

fn default_hook_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub pre_tool_use: Vec<HookCommand>,
    #[serde(default)]
    pub post_tool_use: Vec<HookCommand>,
    #[serde(default)]
    pub session_start: Vec<HookCommand>,
    #[serde(default)]
    pub session_end: Vec<HookCommand>,
    #[serde(default)]
    pub user_prompt_submit: Vec<HookCommand>,
    #[serde(default)]
    pub pre_compact: Vec<HookCommand>,
    #[serde(default)]
    pub stop: Vec<HookCommand>,
    #[serde(default)]
    pub permission_request: Vec<HookCommand>,
}

impl HookConfig {
    pub fn get(&self, point: &str) -> &[HookCommand] {
        match point {
            "PreToolUse" => &self.pre_tool_use,
            "PostToolUse" => &self.post_tool_use,
            "SessionStart" => &self.session_start,
            "SessionEnd" => &self.session_end,
            "UserPromptSubmit" => &self.user_prompt_submit,
            "PreCompact" => &self.pre_compact,
            "Stop" => &self.stop,
            "PermissionRequest" => &self.permission_request,
            _ => &[],
        }
    }
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

    #[test]
    fn hook_config_get() {
        let mut config = HookConfig::default();
        let cmd = HookCommand {
            hook_type: "command".into(),
            command: "echo hi".into(),
            timeout: 30,
        };
        config.pre_tool_use.push(cmd);
        assert_eq!(config.get("PreToolUse").len(), 1);
        assert!(config.get("PreToolUse")[0].command.contains("echo hi"));
        assert!(config.get("PostToolUse").is_empty());
        assert!(config.get("Unknown").is_empty());
    }
}
