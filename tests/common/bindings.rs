//! Shared agent tool binding fixtures for integration tests.

use litecode::config::schema::{AgentToolBinding, ToolPreset};

pub fn binding_all_for(_tool_id: &str) -> AgentToolBinding {
    AgentToolBinding {
        enabled: true,
        preset: Some(ToolPreset::All),
        allowed_tools: None,
    }
}

pub fn binding_safe_for(_tool_id: &str) -> AgentToolBinding {
    AgentToolBinding {
        enabled: true,
        preset: Some(ToolPreset::Safe),
        allowed_tools: None,
    }
}

pub fn binding_none_tool() -> AgentToolBinding {
    AgentToolBinding {
        enabled: true,
        preset: None,
        allowed_tools: None,
    }
}
