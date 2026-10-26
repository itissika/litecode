//! Agent bind persistence: normalize profiles on write; catalog is evaluation-only.

use crate::config::binding_scope::scope_of;
use crate::config::schema::AgentRole;

pub fn normalize_agent_profile(profile: &mut crate::config::schema::AgentProfile) {
    if profile.role == AgentRole::Hidden {
        profile.tools.clear();
        profile.allowed_subagents.clear();
        return;
    }

    if profile.role != AgentRole::Primary {
        profile.allowed_subagents.clear();
    }

    if profile.role == AgentRole::Subagent {
        profile
            .tools
            .retain(|tool_id, _| scope_of(tool_id).allows_role(AgentRole::Subagent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{AgentProfile, AgentToolBinding};
    use std::collections::HashMap;

    fn binding() -> AgentToolBinding {
        AgentToolBinding {
            enabled: true,
            policy: Default::default(),
            path_mode: Default::default(),
            last_applied_preset: None,
            allowed_tools: None,
        }
    }

    #[test]
    fn hidden_role_clears_bindings_generically() {
        let mut profile = AgentProfile {
            role: AgentRole::Hidden,
            tools: HashMap::from([("read".to_string(), binding())]),
            allowed_subagents: vec!["explore".into()],
            ..AgentProfile::default()
        };

        normalize_agent_profile(&mut profile);

        assert!(profile.tools.is_empty());
        assert!(profile.allowed_subagents.is_empty());
    }

    #[test]
    fn subagent_drops_primary_root_only_but_keeps_all_scope() {
        let mut profile = AgentProfile {
            role: AgentRole::Subagent,
            tools: HashMap::from([
                ("read".to_string(), binding()),
                ("plan".to_string(), binding()),
                ("subagent_tools".to_string(), binding()),
                ("subagent_launch".to_string(), binding()),
                ("subagent_foo".to_string(), binding()),
            ]),
            allowed_subagents: vec!["explore".into()],
            ..AgentProfile::default()
        };

        normalize_agent_profile(&mut profile);

        assert!(profile.tools.contains_key("read"));
        assert!(profile.tools.contains_key("subagent_foo"));
        assert!(!profile.tools.contains_key("plan"));
        assert!(!profile.tools.contains_key("subagent_tools"));
        assert!(!profile.tools.contains_key("subagent_launch"));
        assert!(profile.allowed_subagents.is_empty());
    }
}
