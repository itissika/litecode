//! Agent bind persistence: normalize profiles on write; catalog is evaluation-only.

use std::collections::HashMap;

use crate::config::schema::{AgentToolBinding, PRIMARY_ONLY_TOOL_IDS, SUBAGENT_SERIES_TOOL_IDS};

pub fn normalize_agent_profile(profile: &mut crate::config::schema::AgentProfile) {
    use crate::config::schema::AgentRole;

    if profile.role == AgentRole::Hidden {
        profile.tools.clear();
        profile.allowed_subagents.clear();
        return;
    }

    if profile.role != AgentRole::Primary {
        profile.allowed_subagents.clear();
    }

    if profile.role == AgentRole::Subagent {
        strip_primary_only_bindings(&mut profile.tools);
        strip_subagent_series_bindings(&mut profile.tools);
    }
}

pub fn strip_primary_only_bindings(tools: &mut HashMap<String, AgentToolBinding>) {
    for tool_id in PRIMARY_ONLY_TOOL_IDS {
        tools.remove(*tool_id);
    }
}

pub fn strip_subagent_series_bindings(tools: &mut HashMap<String, AgentToolBinding>) {
    for tool_id in SUBAGENT_SERIES_TOOL_IDS {
        tools.remove(*tool_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{AgentProfile, AgentRole, AgentToolBinding};

    #[test]
    fn hidden_role_clears_bindings_generically() {
        let mut profile = AgentProfile {
            role: AgentRole::Hidden,
            tools: HashMap::from([(
                "read".to_string(),
                AgentToolBinding {
                    enabled: true,
                    policy: Default::default(),
                    path_mode: Default::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            )]),
            allowed_subagents: vec!["explore".into()],
            ..AgentProfile::default()
        };

        normalize_agent_profile(&mut profile);

        assert!(profile.tools.is_empty());
        assert!(profile.allowed_subagents.is_empty());
    }
}
