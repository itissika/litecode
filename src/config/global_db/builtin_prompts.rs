//! Built-in agent prompt packs. Seed stores markers; assembly resolves these.
//!
//! Editable layers (identity, work, voice) live in `prompts/<agent>/*.md`.
//! Harness layers live in `prompts/harness/` and `prompts/citations.md`.
//! `include_str!` embeds every file at compile time. Each file ends with one
//! trailing newline. `DEFAULT_PROMPT` and its siblings are the editable layers
//! only. `build_system_prompt` splices the harness around them for every
//! built-in agent.

pub const HARNESS_SYSTEM: &str = include_str!("prompts/harness/system.md");
pub const HARNESS_CODE_RUNTIME: &str = include_str!("prompts/harness/code_runtime.md");
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
pub const PLAN_PROMPT: &str = concat!(
    include_str!("prompts/plan/identity.md"),
    "\n",
    include_str!("prompts/plan/work.md"),
    "\n",
    include_str!("prompts/plan/voice.md"),
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
pub const VERIFICATION_PROMPT: &str = concat!(
    include_str!("prompts/verification/identity.md"),
    "\n",
    include_str!("prompts/verification/work.md"),
    "\n",
    include_str!("prompts/verification/voice.md"),
);
/// DSH-faithful compaction constant: the trailing summarization instruction.
/// Compaction is a call mode of the main request, never a hidden agent prompt.
pub const COMPACTION_INSTRUCTION: &str = include_str!("prompts/compaction_instruction.md");

/// Framing prepended to every compacted checkpoint inside its `<system-reminder>`.
pub const CHECKPOINT_PREAMBLE: &str = include_str!("prompts/checkpoint_preamble.md");

/// Shared citation rules for primary agents. Spliced by `build_system_prompt`.
pub const CITATION_PROMPT: &str = include_str!("prompts/citations.md");

pub const DEFAULT_DESCRIPTION: &str = "General-purpose software engineering agent. Investigates, implements, verifies, and delivers end-to-end changes.";

pub const PLAN_DESCRIPTION: &str = "Read-only software architect and planner. Explores the codebase, clarifies ambiguities, and produces a concrete implementation plan for user approval.";

pub const ORCHESTRATOR_DESCRIPTION: &str = "Team coordinator and goal owner. Plans work, delegates bounded tasks to explore/general/verification subagents, enforces independent verification, and owns the final outcome.";

pub const GENERAL_DESCRIPTION: &str = "Bounded general-purpose implementer. Executes assigned code changes, fixes, and tests within its owned scope and returns a concise evidence-backed report.";

pub const EXPLORE_DESCRIPTION: &str = "Fast read-only codebase and source investigator. Searches files, traces implementations, and returns concise, evidence-backed conclusions.";

pub const VERIFICATION_DESCRIPTION: &str = "Independent adversarial verifier. Does not modify project files; runs builds, tests, linters, and adversarial probes to issue an evidence-backed PASS, FAIL, or PARTIAL verdict.";

/// DB `system_prompt` values that mean "use the built-in pack for this agent id".
pub fn is_builtin_prompt_marker(stored: &str) -> bool {
    let trimmed = stored.trim();
    trimmed == "builtin" || trimmed.starts_with("builtin:")
}

/// Built-in editable pack for a seeded agent id. Lookup is by id, not by marker suffix.
/// Every built-in agent is identity + work + voice.
pub fn builtin_prompt_for(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        "default" => Some(DEFAULT_PROMPT),
        "plan" => Some(PLAN_PROMPT),
        "orchestrator" => Some(ORCHESTRATOR_PROMPT),
        "general" => Some(GENERAL_PROMPT),
        "explore" => Some(EXPLORE_PROMPT),
        "verification" => Some(VERIFICATION_PROMPT),
        _ => None,
    }
}
