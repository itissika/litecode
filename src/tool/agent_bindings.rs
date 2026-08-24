//! Agent bind persistence: merge visible cards, keep dormant rows.

use std::collections::{HashMap, HashSet};

use crate::config::resolved::ResolvedConfig;
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

/// Merge PUT tools with stored bindings.
///
/// - ids in `available`: incoming wins; omitted means unbind for this workspace
/// - ids not in `available`: keep existing (dormant)
pub fn merge_agent_tool_bindings(
    existing: &HashMap<String, AgentToolBinding>,
    incoming: HashMap<String, AgentToolBinding>,
    available: &HashSet<String>,
) -> HashMap<String, AgentToolBinding> {
    let mut result = existing.clone();
    for id in available {
        match incoming.get(id) {
            Some(binding) => {
                result.insert(id.clone(), binding.clone());
            }
            None => {
                result.remove(id);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::AgentToolBinding;

    fn bind(enabled: bool) -> AgentToolBinding {
        AgentToolBinding {
            enabled,
            policy: crate::permission::ToolPolicy::allow_all(),
            path_mode: crate::permission::BindingPathMode::default(),
            last_applied_preset: None,
            allowed_tools: None,
        }
    }

    #[test]
    fn merge_keeps_dormant_and_unbinds_visible() {
        let mut existing = HashMap::new();
        existing.insert("read".into(), bind(true));
        existing.insert("mcp_gone".into(), bind(true));
        let mut incoming = HashMap::new();
        incoming.insert("read".into(), bind(false));
        let available = HashSet::from(["read".into(), "bash".into()]);
        let merged = merge_agent_tool_bindings(&existing, incoming, &available);
        assert_eq!(merged.get("read").unwrap().enabled, false);
        assert!(merged.contains_key("mcp_gone"));
        assert!(!merged.contains_key("bash"));
    }
}

pub fn available_id_set(resolved: &ResolvedConfig) -> HashSet<String> {
    super::availability::available_tools(resolved)
        .into_iter()
        .map(|t| t.id)
        .collect()
}
