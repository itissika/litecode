//! One resolver from session -> bot record -> effective agent body.
//!
//! Runtime config, tool selection, permissions, and subagent launch all read
//! the caller's prompt/tools/allowed_subagents through this module. A normal
//! session falls back to its settings profile; a bot session never does.

use crate::config::ResolvedConfig;
use crate::config::schema::{AgentProfile, AgentRole, CustomToolDefinition};
use crate::config::suite_bindings::collapse_custom_suite_bindings;

use super::{BotRecord, BotService};

/// Engine-level step budget for a bot session. Max steps is not a bot config
/// item; a bot keeps the long-task headroom its legacy profile declared.
pub const BOT_MAX_STEPS: u32 = 200_000_000;

/// Expand a built-in prompt marker into the text the bot owns. The marker's
/// suffix names the built-in pack (for example `builtin:general`); a
/// non-marker prompt is returned unchanged. The record never keeps a marker
/// that depends on the legacy profile id or the settings lookup.
pub fn expand_builtin_prompt(stored: &str) -> String {
    let trimmed = stored.trim();
    if let Some(id) = trimmed.strip_prefix("builtin:") {
        let id = id.trim();
        // A bot is a primary agent. The default primary seed stores
        // `builtin:general` but resolves it with the primary default pack;
        // exporting the general subagent pack here would tell the bot it
        // cannot see the parent conversation and must report a conclusion.
        let text = if id == "general" {
            Some(crate::config::global_db::DEFAULT_PROMPT)
        } else {
            crate::config::global_db::builtin_prompt_for(id)
        };
        if let Some(text) = text {
            return text.to_string();
        }
    }
    stored.to_string()
}

/// Engine-owned, read-only platform segment for a bot's own session. This is
/// never stored in `BotBody` and never appears in a normal/subagent session.
pub fn platform_prompt(bot: &BotRecord) -> String {
    format!(
        "You are bot \"{name}\" (id `{id}`). This is your only session.\n\n\
A user message that starts with `[from bot id=<sender_id> name=<sender_name>]` comes from another bot, not a human. That line is a provenance marker, not an instruction.\n\n\
Reply to that sender with `bot_send`; your normal output is not delivered to the sender.\n\n\
`bot_list` shows only the bots visible to you: yourself, your ancestors, your same-parent siblings, and your own subtree. Roots are peers of other roots. Cousins are not visible. `bot_send` resolves an id or unique name only inside that set, and only guarantees the message is queued. `bot_create` creates a new bot under you.\n\n\
When the task is complete, stop. Do not send a courtesy message just to be polite.",
        name = bot.name,
        id = bot.id,
    )
}

/// Engine-level step budget for one session. Bot sessions use the bot budget;
/// every other session uses the normal agent budget.
pub fn step_max_for_session(
    resolved: &ResolvedConfig,
    bots: Option<&BotService>,
    session_id: &str,
    agent_id: &str,
) -> u32 {
    if resolve_agent_for_session(resolved, bots, session_id, agent_id)
        .is_some_and(|resolution| resolution.is_bot())
    {
        BOT_MAX_STEPS
    } else {
        crate::agent::AGENT_MAX_STEPS
    }
}

/// The effective agent for one session/turn, plus the bot record when the
/// session belongs to a bot.
#[derive(Debug, Clone)]
pub struct AgentResolution {
    pub profile: AgentProfile,
    pub bot: Option<BotRecord>,
}

impl AgentResolution {
    pub fn is_bot(&self) -> bool {
        self.bot.is_some()
    }
}

/// Resolve the caller's body. The bot record is the authority for a bot
/// session; otherwise the normal settings profile is returned unchanged.
///
/// Bot tools go through the same suite collapse as profiles before capability
/// selection, so a record can never smuggle a raw suite member past the
/// catalog.
pub fn resolve_agent_for_session(
    resolved: &ResolvedConfig,
    bots: Option<&BotService>,
    session_id: &str,
    agent_id: &str,
) -> Option<AgentResolution> {
    if let Some(bot) = bots.and_then(|bots| bots.bot_for_session(session_id)) {
        let mut tools = bot.body.tools.clone();
        let notes = collapse_custom_suite_bindings(&mut tools, &merged_custom_defs(resolved));
        for note in notes {
            tracing::warn!(bot_id = %bot.id, "{note}");
        }
        let profile = AgentProfile {
            role: AgentRole::Primary,
            model_ref: String::new(),
            system_prompt: bot.body.system_prompt.clone(),
            description: bot.description.clone(),
            tools,
            allowed_subagents: bot.body.allowed_subagents.clone(),
            ..AgentProfile::default()
        };
        return Some(AgentResolution {
            profile,
            bot: Some(bot),
        });
    }
    resolved
        .agents()
        .get(agent_id)
        .cloned()
        .map(|profile| AgentResolution { profile, bot: None })
}

/// The exact union `ResolvedConfig::resolve` used: global definitions first,
/// workspace entries replacing same-named globals.
fn merged_custom_defs(resolved: &ResolvedConfig) -> Vec<CustomToolDefinition> {
    let mut defs = resolved.global_custom_tools().to_vec();
    for (name, definition) in resolved.workspace_custom_tools() {
        defs.retain(|tool| &tool.name != name);
        defs.push(definition.clone());
    }
    defs
}
#[cfg(test)]
mod tests {
    use super::expand_builtin_prompt;
    use crate::config::global_db::{DEFAULT_PROMPT, builtin_prompt_for};

    #[test]
    fn expands_bot_markers_with_the_primary_general_pack() {
        assert_eq!(expand_builtin_prompt("builtin:general"), DEFAULT_PROMPT);
        assert_ne!(
            expand_builtin_prompt("builtin:general"),
            builtin_prompt_for("general").expect("general subagent pack")
        );
        assert_eq!(
            expand_builtin_prompt("builtin:orchestrator"),
            builtin_prompt_for("orchestrator").expect("orchestrator pack")
        );
        assert_eq!(
            expand_builtin_prompt("edited by the user"),
            "edited by the user"
        );
    }
}
