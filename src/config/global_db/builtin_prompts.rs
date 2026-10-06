//! Built-in agent prompt packs. Seed stores markers; assembly resolves these.
//!
//! Editable layers (identity, work, voice) live in `prompts/<agent>/*.md`.
//! Harness layers live in `prompts/harness/` and `prompts/citations.md`.
//! `include_str!` embeds every file at compile time. Each file ends with one
//! trailing newline. `DEFAULT_PROMPT` and its siblings are the editable layers
//! only. `build_system_prompt` splices the harness around them for every
//! non-hidden agent.

pub const HARNESS_SYSTEM: &str = include_str!("prompts/harness/system.md");
pub const HARNESS_TOOLS: &str = include_str!("prompts/harness/tools.md");

pub const ORCHESTRATOR_PROMPT: &str = concat!(
    include_str!("prompts/orchestrator/identity.md"),
    "\n",
    include_str!("prompts/orchestrator/work.md"),
    "\n",
    include_str!("prompts/orchestrator/voice.md"),
);
pub const DEFAULT_PROMPT: &str = concat!(
    include_str!("prompts/default/identity.md"),
    "\n",
    include_str!("prompts/default/work.md"),
    "\n",
    include_str!("prompts/default/voice.md"),
);
pub const GENERAL_PROMPT: &str = concat!(
    include_str!("prompts/general/identity.md"),
    "\n",
    include_str!("prompts/general/work.md"),
    "\n",
    include_str!("prompts/general/voice.md"),
);
pub const EXPLORE_PROMPT: &str = concat!(
    include_str!("prompts/explore/identity.md"),
    "\n",
    include_str!("prompts/explore/work.md"),
    "\n",
    include_str!("prompts/explore/voice.md"),
);
pub const COMPACTION_PROMPT: &str = include_str!("prompts/compaction.md");

/// Shared citation rules for every non-hidden agent. Spliced by `build_system_prompt`.
pub const CITATION_PROMPT: &str = include_str!("prompts/citations.md");

pub const DEFAULT_DESCRIPTION: &str = "General-purpose coding assistant";

pub const ORCHESTRATOR_DESCRIPTION: &str =
    "Orchestrator. Manages a team of subagents and owns the user's goal.";

pub const GENERAL_DESCRIPTION: &str = "General-purpose implementer. Edits code, runs commands, and reports back. Use for bounded implementation, tests, and fixes.";

pub const EXPLORE_DESCRIPTION: &str = "A read-only Explore agent. Skilled in investigating local codebases and online sources, returning clear, well-analyzed conclusions. Use when comprehensive understanding across local and web sources is required.";

/// DB `system_prompt` values that mean "use the built-in pack for this agent id".
pub fn is_builtin_prompt_marker(stored: &str) -> bool {
    let trimmed = stored.trim();
    trimmed == "builtin" || trimmed.starts_with("builtin:")
}

/// Built-in editable pack for a seeded agent id. Lookup is by id, not by marker suffix.
/// Hidden compaction is the whole body; other ids are identity + work + voice.
pub fn builtin_prompt_for(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "default" => Some(DEFAULT_PROMPT),
        "orchestrator" => Some(ORCHESTRATOR_PROMPT),
        "general" => Some(GENERAL_PROMPT),
        "explore" => Some(EXPLORE_PROMPT),
        "compaction" => Some(COMPACTION_PROMPT),
        _ => None,
    }
}
