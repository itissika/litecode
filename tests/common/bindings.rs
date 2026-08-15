//! Shared agent tool binding fixtures for integration tests.

use litecode::config::schema::{AgentToolBinding, ToolPreset};
use litecode::permission::{BindingPathMode, ToolPolicy};

pub fn binding_all_for(tool_id: &str) -> AgentToolBinding {
    let (policy, path_mode) =
        litecode::permission::presets::binding_for_tool(tool_id, ToolPreset::All);
    AgentToolBinding {
        enabled: true,
        policy,
        path_mode,
        last_applied_preset: Some(ToolPreset::All),
    }
}

pub fn binding_safe_for(tool_id: &str) -> AgentToolBinding {
    let (policy, path_mode) =
        litecode::permission::presets::binding_for_tool(tool_id, ToolPreset::Safe);
    AgentToolBinding {
        enabled: true,
        policy,
        path_mode,
        last_applied_preset: Some(ToolPreset::Safe),
    }
}

pub fn binding_none_tool() -> AgentToolBinding {
    AgentToolBinding {
        enabled: true,
        policy: ToolPolicy::allow_all(),
        path_mode: BindingPathMode::default(),
        last_applied_preset: None,
    }
}
