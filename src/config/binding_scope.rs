//! Role/depth scope for one agent tool binding.
//!
//! This is the single source for "may this role bind this capability?".
//! Hidden agents are cleared elsewhere; they still answer `false` here.

use serde::{Deserialize, Serialize};

use crate::config::global_db::tools::MEMORY_TOOL_IDS;
use crate::config::schema::{
    AgentRole, PRIMARY_ONLY_TOOL_IDS, SUBAGENT_CAPABILITY_ID, SUBAGENT_MEMBER_TOOL_IDS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BindingScope {
    #[default]
    All,
    PrimaryRootOnly,
}

impl BindingScope {
    /// Whether `role` can bind this capability at all (depth-independent).
    pub fn allows_role(self, role: AgentRole) -> bool {
        match self {
            Self::All => role != AgentRole::Hidden,
            Self::PrimaryRootOnly => role == AgentRole::Primary,
        }
    }

    /// Whether `role` at `depth` can bind this capability.
    pub fn allows(self, role: AgentRole, depth: u32) -> bool {
        match self {
            Self::All => role != AgentRole::Hidden,
            Self::PrimaryRootOnly => role == AgentRole::Primary && depth == 0,
        }
    }
}

/// The one scope table. Exact ids only: a custom `subagent_foo` stays `All`.
pub fn scope_of(capability_id: &str) -> BindingScope {
    if capability_id == SUBAGENT_CAPABILITY_ID
        || SUBAGENT_MEMBER_TOOL_IDS.contains(&capability_id)
        || MEMORY_TOOL_IDS.contains(&capability_id)
        || PRIMARY_ONLY_TOOL_IDS.contains(&capability_id)
    {
        BindingScope::PrimaryRootOnly
    } else {
        BindingScope::All
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_root_only_is_exact_not_prefix_based() {
        assert_eq!(scope_of("subagent_tools"), BindingScope::PrimaryRootOnly);
        assert_eq!(scope_of("subagent_launch"), BindingScope::PrimaryRootOnly);
        assert_eq!(scope_of("plan"), BindingScope::PrimaryRootOnly);
        assert_eq!(scope_of("memory"), BindingScope::PrimaryRootOnly);
        assert_eq!(scope_of("memory.list"), BindingScope::PrimaryRootOnly);
        assert_eq!(scope_of("subagent_foo"), BindingScope::All);
        assert_eq!(scope_of("read"), BindingScope::All);
    }

    #[test]
    fn role_and_depth_matrix() {
        assert!(BindingScope::All.allows(AgentRole::Subagent, 1));
        assert!(!BindingScope::All.allows(AgentRole::Hidden, 0));
        assert!(BindingScope::PrimaryRootOnly.allows(AgentRole::Primary, 0));
        assert!(!BindingScope::PrimaryRootOnly.allows(AgentRole::Primary, 1));
        assert!(!BindingScope::PrimaryRootOnly.allows(AgentRole::Subagent, 0));
        assert!(!BindingScope::PrimaryRootOnly.allows(AgentRole::Hidden, 0));
    }
}
