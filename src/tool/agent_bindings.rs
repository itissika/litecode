//! Agent bind persistence: normalize profiles on write; catalog is evaluation-only.

use std::collections::HashMap;

use crate::config::schema::{AgentToolBinding, SUBAGENT_SERIES_TOOL_IDS};

pub fn normalize_agent_profile(agent_id: &str, profile: &mut crate::config::schema::AgentProfile) {
    use crate::config::schema::AgentRole;

    if profile.role == AgentRole::Hidden || agent_id == "compaction" {
        profile.tools.clear();
        profile.allowed_subagents.clear();
        return;
    }

    if profile.role != AgentRole::Primary {
        profile.allowed_subagents.clear();
    }

    if profile.role == AgentRole::Subagent {
        strip_subagent_series_bindings(&mut profile.tools);
    }
}

pub fn strip_subagent_series_bindings(tools: &mut HashMap<String, AgentToolBinding>) {
    for tool_id in SUBAGENT_SERIES_TOOL_IDS {
        tools.remove(*tool_id);
    }
}
