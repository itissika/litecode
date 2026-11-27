//! Static specs for the six built-in agents.
//!
//! This table is the one place that states who the built-in agents are: role,
//! description, prompt pack, allowed subagents, and which tool bindings the
//! system owns (`core`, locked later) versus which the user may still toggle
//! (`default_optional`).
//!
//! W1 (pure refactor) uses it to generate exactly the rows the old per-agent
//! seed functions wrote, so the resulting DB is byte-identical. Later work
//! reads runtime profiles from here as well.

use crate::config::schema::{AgentRole, AgentToolBinding, ToolPreset};

use super::builtin_prompts::{
    DEFAULT_DESCRIPTION, DEFAULT_PROMPT, EXPLORE_DESCRIPTION, EXPLORE_PROMPT, GENERAL_DESCRIPTION,
    GENERAL_PROMPT, ORCHESTRATOR_DESCRIPTION, ORCHESTRATOR_PROMPT, PLAN_DESCRIPTION, PLAN_PROMPT,
    VERIFICATION_DESCRIPTION, VERIFICATION_PROMPT,
};

/// One built-in agent definition.
pub struct BuiltinAgentSpec {
    pub id: &'static str,
    pub role: AgentRole,
    pub description: &'static str,
    /// Editable prompt pack for this agent (`prompts/<id>/*.md`). Seeded as the
    /// `builtin:<id>` marker, never as text.
    pub prompt: &'static str,
    pub allowed_subagents: &'static [&'static str],
    /// Core tool bindings (system-locked in W3). `(tool_id, preset)`.
    pub core: &'static [(&'static str, Option<ToolPreset>)],
    /// Default optional builtin bindings (user-overridable in W3). `(tool_id, preset)`.
    pub default_optional: &'static [(&'static str, Option<ToolPreset>)],
}

impl BuiltinAgentSpec {
    /// DB `agents.system_prompt` value: `builtin:<id>`.
    pub fn prompt_marker(&self) -> String {
        format!("builtin:{}", self.id)
    }

    /// Core bindings as persisted rows. Always enabled; the preset is the one
    /// written in the spec (only tools with a permission dial may carry one).
    pub fn core_bindings(&self) -> impl Iterator<Item = (&'static str, AgentToolBinding)> + '_ {
        self.core
            .iter()
            .map(|(tool, preset)| (*tool, spec_binding(*preset)))
    }

    /// Default optional bindings as persisted rows. Same encoding as core; the
    /// difference is semantic (users may override these).
    pub fn default_optional_bindings(
        &self,
    ) -> impl Iterator<Item = (&'static str, AgentToolBinding)> + '_ {
        self.default_optional
            .iter()
            .map(|(tool, preset)| (*tool, spec_binding(*preset)))
    }
}

fn spec_binding(preset: Option<ToolPreset>) -> AgentToolBinding {
    AgentToolBinding {
        enabled: true,
        preset,
        allowed_tools: None,
    }
}

// ---- shared binding lists ----

const PRIMARY_CORE: &[(&str, Option<ToolPreset>)] = &[
    ("read", Some(ToolPreset::All)),
    ("grep", Some(ToolPreset::All)),
    ("glob", Some(ToolPreset::All)),
    ("write", Some(ToolPreset::All)),
    ("edit", Some(ToolPreset::All)),
    ("exec", Some(ToolPreset::All)),
    ("session_search", None),
    ("litecode_workspace", Some(ToolPreset::All)),
    ("plan", None),
    ("todo", None),
    ("memory", None),
    ("ask_user", None),
    ("knowledge", None),
    ("subagent_tools", None),
];

const PLAN_CORE: &[(&str, Option<ToolPreset>)] = &[
    ("read", Some(ToolPreset::All)),
    ("grep", Some(ToolPreset::All)),
    ("glob", Some(ToolPreset::All)),
    ("write", Some(ToolPreset::All)),
    ("edit", Some(ToolPreset::All)),
    ("exec", Some(ToolPreset::Safe)),
    ("session_search", None),
    ("litecode_workspace", Some(ToolPreset::Safe)),
    // Plan is a planning agent: no todo board.
    ("plan", None),
    ("memory", None),
    ("ask_user", None),
    ("knowledge", None),
    ("subagent_tools", None),
];

const GENERAL_CORE: &[(&str, Option<ToolPreset>)] = &[
    ("read", Some(ToolPreset::All)),
    ("grep", Some(ToolPreset::All)),
    ("glob", Some(ToolPreset::All)),
    ("write", Some(ToolPreset::All)),
    ("edit", Some(ToolPreset::All)),
    ("exec", Some(ToolPreset::All)),
    ("session_search", None),
    ("litecode_workspace", Some(ToolPreset::All)),
    ("knowledge", None),
];

const EXPLORE_CORE: &[(&str, Option<ToolPreset>)] = &[
    ("read", Some(ToolPreset::Safe)),
    ("grep", Some(ToolPreset::Safe)),
    ("glob", Some(ToolPreset::Safe)),
    ("session_search", None),
    ("exec", Some(ToolPreset::Safe)),
    ("litecode_workspace", Some(ToolPreset::Safe)),
    ("knowledge", None),
];

const VERIFICATION_CORE: &[(&str, Option<ToolPreset>)] = &[
    ("read", Some(ToolPreset::All)),
    ("grep", Some(ToolPreset::All)),
    ("glob", Some(ToolPreset::All)),
    ("exec", Some(ToolPreset::All)),
    ("knowledge", None),
];

/// Optional defaults for the five agents that bind `code_search` plus the two
/// web engines. `webfetch` / `websearch` are optional builtins (D3): on by
/// default, user-toggleable, and locked out of custom/MCP name reuse by
/// `is_reserved_builtin`.
const CODE_SEARCH_AND_WEB_OPTIONAL: &[(&str, Option<ToolPreset>)] = &[
    ("code_search", None),
    ("webfetch", None),
    ("websearch", None),
];

/// `verification` never binds `code_search`, but it does get the web engines.
const WEB_ONLY_OPTIONAL: &[(&str, Option<ToolPreset>)] = &[("webfetch", None), ("websearch", None)];

/// The built-in agent set. Order is stable: it is the seed order.
static BUILTIN_AGENTS: [BuiltinAgentSpec; 6] = [
    BuiltinAgentSpec {
        id: "default",
        role: AgentRole::Primary,
        description: DEFAULT_DESCRIPTION,
        prompt: DEFAULT_PROMPT,
        allowed_subagents: &["explore"],
        core: PRIMARY_CORE,
        default_optional: CODE_SEARCH_AND_WEB_OPTIONAL,
    },
    BuiltinAgentSpec {
        id: "plan",
        role: AgentRole::Primary,
        description: PLAN_DESCRIPTION,
        prompt: PLAN_PROMPT,
        allowed_subagents: &["explore"],
        core: PLAN_CORE,
        default_optional: CODE_SEARCH_AND_WEB_OPTIONAL,
    },
    BuiltinAgentSpec {
        id: "orchestrator",
        role: AgentRole::Primary,
        description: ORCHESTRATOR_DESCRIPTION,
        prompt: ORCHESTRATOR_PROMPT,
        allowed_subagents: &["explore", "general", "verification"],
        core: PRIMARY_CORE,
        default_optional: CODE_SEARCH_AND_WEB_OPTIONAL,
    },
    BuiltinAgentSpec {
        id: "explore",
        role: AgentRole::Subagent,
        description: EXPLORE_DESCRIPTION,
        prompt: EXPLORE_PROMPT,
        allowed_subagents: &[],
        core: EXPLORE_CORE,
        default_optional: CODE_SEARCH_AND_WEB_OPTIONAL,
    },
    BuiltinAgentSpec {
        id: "general",
        role: AgentRole::Subagent,
        description: GENERAL_DESCRIPTION,
        prompt: GENERAL_PROMPT,
        allowed_subagents: &[],
        core: GENERAL_CORE,
        default_optional: CODE_SEARCH_AND_WEB_OPTIONAL,
    },
    BuiltinAgentSpec {
        id: "verification",
        role: AgentRole::Subagent,
        description: VERIFICATION_DESCRIPTION,
        prompt: VERIFICATION_PROMPT,
        allowed_subagents: &[],
        core: VERIFICATION_CORE,
        default_optional: WEB_ONLY_OPTIONAL,
    },
];

/// Every built-in agent, in stable order.
pub fn builtin_agents() -> &'static [BuiltinAgentSpec] {
    &BUILTIN_AGENTS
}

/// Spec for one built-in agent id.
pub fn builtin_agent(id: &str) -> Option<&'static BuiltinAgentSpec> {
    builtin_agents().iter().find(|spec| spec.id == id)
}

/// Whether `id` names a built-in agent.
pub fn is_builtin_agent(id: &str) -> bool {
    builtin_agent(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::binding_scope::scope_of;
    use crate::config::global_db::builtin_prompts::builtin_prompt_for;
    use crate::config::global_db::tools::{is_core_tool, is_optional_builtin};
    use crate::permission::presets::has_permission_preset;
    use std::collections::HashSet;

    fn assert_binding_surface(
        agent: &str,
        kind: &str,
        bindings: &[(&'static str, Option<ToolPreset>)],
        is_member: fn(&str) -> bool,
    ) {
        let mut seen = HashSet::new();
        for (tool, preset) in bindings {
            assert!(
                seen.insert(*tool),
                "{agent}: duplicate {kind} binding for {tool}"
            );
            assert!(is_member(tool), "{agent}: {tool} is not a {kind} tool");
            assert_eq!(
                preset.is_some(),
                has_permission_preset(tool),
                "{agent}: {tool} preset disagrees with has_permission_preset"
            );
        }
    }

    #[test]
    fn builtin_specs_are_structurally_consistent() {
        let specs = builtin_agents();
        assert_eq!(specs.len(), 6);

        let ids: Vec<&str> = specs.iter().map(|spec| spec.id).collect();
        assert_eq!(
            ids,
            [
                "default",
                "plan",
                "orchestrator",
                "explore",
                "general",
                "verification"
            ],
            "the built-in id set and its order are part of the contract"
        );
        assert_eq!(
            ids.iter().copied().collect::<HashSet<_>>().len(),
            ids.len(),
            "built-in ids must be unique"
        );

        for spec in specs {
            assert_eq!(
                spec.prompt,
                builtin_prompt_for(spec.id)
                    .unwrap_or_else(|| panic!("{} has no prompt pack", spec.id)),
                "{} prompt must be the built-in pack",
                spec.id
            );
            assert_eq!(spec.prompt_marker(), format!("builtin:{}", spec.id));
            assert_eq!(builtin_agent(spec.id).map(|found| found.id), Some(spec.id));
            assert!(is_builtin_agent(spec.id));
            assert!(!spec.description.is_empty());

            assert_binding_surface(spec.id, "core", spec.core, is_core_tool);
            assert_binding_surface(
                spec.id,
                "optional",
                spec.default_optional,
                is_optional_builtin,
            );

            for subagent in spec.allowed_subagents {
                let target = builtin_agent(subagent)
                    .unwrap_or_else(|| panic!("{} allows unknown subagent {subagent}", spec.id));
                assert_eq!(
                    target.role,
                    AgentRole::Subagent,
                    "{} allows {subagent}, which is not a subagent",
                    spec.id
                );
            }
            if spec.role != AgentRole::Primary {
                assert!(
                    spec.allowed_subagents.is_empty(),
                    "{} is not primary and must not allow subagents",
                    spec.id
                );
            }

            if spec.role == AgentRole::Subagent {
                for (tool, _) in spec.core.iter().chain(spec.default_optional.iter()) {
                    assert!(
                        scope_of(tool).allows_role(AgentRole::Subagent),
                        "{} is a subagent and cannot bind {tool}",
                        spec.id
                    );
                }
            }
        }

        assert!(!is_builtin_agent("my_agent"));
        assert!(builtin_agent("my_agent").is_none());
    }

    /// D3: the web engines left `core` and joined every agent's
    /// `default_optional` — on by default, user-toggleable, no permission dial.
    #[test]
    fn all_builtin_agents_default_to_the_web_engines() {
        for spec in builtin_agents() {
            for tool in ["webfetch", "websearch"] {
                assert!(
                    !spec.core.iter().any(|(core, _)| *core == tool),
                    "{}: {tool} must not be core",
                    spec.id
                );
                let binding = spec
                    .default_optional_bindings()
                    .find(|(optional, _)| *optional == tool)
                    .unwrap_or_else(|| panic!("{}: {tool} must be a default optional", spec.id));
                assert!(binding.1.enabled, "{}: {tool} must default on", spec.id);
                assert_eq!(binding.1.preset, None, "{}: {tool} has no dial", spec.id);
            }
        }
    }

    /// `code_search` stays on the five agents the W1 baseline gave it to;
    /// `verification` still must not bind it.
    #[test]
    fn code_search_defaults_are_unchanged() {
        for spec in builtin_agents() {
            let has = spec
                .default_optional
                .iter()
                .any(|(tool, _)| *tool == "code_search");
            assert_eq!(
                has,
                spec.id != "verification",
                "{}: code_search default drifted",
                spec.id
            );
        }
    }
}
