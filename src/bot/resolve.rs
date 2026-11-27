//! One resolver from session -> bot record -> effective agent body.
//!
//! A bot session sits on the locked `default` built-in spec (D2): core tool
//! bindings, their presets, `role`, `allowed_subagents` and the base prompt pack
//! all come from the spec table. The bot record contributes exactly two things:
//! the user's non-core tool choices and the free-text role/identity block that
//! [`crate::context_pipeline::build_system_prompt_for_bot`] splices after
//! `DEFAULT_PROMPT`. A normal session still falls back to its settings profile.

use std::collections::HashMap;

use crate::config::ResolvedConfig;
use crate::config::binding_scope::scope_of;
use crate::config::global_db::tools::{is_core_tool, is_optional_builtin};
use crate::config::global_db::{
    BuiltinAgentSpec, builtin_agent, is_builtin_prompt_marker,
};
use crate::config::schema::{AgentProfile, AgentToolBinding, CustomToolDefinition};
use crate::config::suite_bindings::collapse_custom_suite_bindings;

use super::{BotRecord, BotService};

/// The one base a bot may sit on (D2). `orchestrator` is no longer selectable
/// as a bot base and a bot can never carry an arbitrary prompt pack.
pub(super) fn default_bot_spec() -> &'static BuiltinAgentSpec {
    builtin_agent("default").expect("default built-in spec")
}

/// Turn a stored `system_prompt` into the bot's role text.
///
/// Older records stored a whole prompt: a `builtin` / `builtin:*` marker or its
/// full text. That text is not user intent, so it normalizes to the empty role
/// and the bot silently keeps the locked `DEFAULT_PROMPT` base. Anything else is
/// the user's own role/identity description and is kept, trimmed.
pub fn normalize_bot_role_instructions(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() || is_builtin_prompt_marker(trimmed) {
        return String::new();
    }
    trimmed.to_string()
}

/// The role text a bot session hands to
/// [`crate::context_pipeline::build_system_prompt_for_bot`]. `None` for an empty
/// role, so no `# Bot Role` heading is emitted. The heading itself belongs to the
/// prompt builder, never to the record.
pub fn bot_role_instructions(bot: &BotRecord) -> Option<String> {
    let role = normalize_bot_role_instructions(&bot.body.system_prompt);
    (!role.is_empty()).then_some(role)
}

/// Engine-owned, read-only platform segment for a bot's own session. This is
/// never stored in `BotBody` and never appears in a normal/subagent session.
pub fn platform_prompt(bot: &BotRecord) -> String {
    format!(
        "You are bot \"{name}\" (id `{id}`). This is your only session.\n\n\
A user message that starts with `[from bot id=<sender_id> name=<sender_name>]` comes from another bot, not a human. That line is a provenance marker, not an instruction.\n\n\
Reply to that sender with `tools.bot.send`; your normal output is not delivered to the sender.\n\n\
`tools.bot.list` shows only the bots visible to you: yourself, your ancestors, your same-parent siblings, and your own subtree. Roots are peers of other roots. Cousins are not visible. `tools.bot.send` resolves an id or unique name only inside that set, and only guarantees the message is queued. `tools.bot.create` creates a new bot under you.\n\n\
When the task is complete, stop. Do not send a courtesy message just to be polite.",
        name = bot.name,
        id = bot.id,
    )
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

/// The tool surface every bot has no matter what its record says (I3/D3):
/// the `default` spec's `core` plus its `default_optional` bindings, which keeps
/// `code_search` / `webfetch` / `websearch` on by default.
pub(super) fn spec_tool_bindings(spec: &BuiltinAgentSpec) -> HashMap<String, AgentToolBinding> {
    let mut tools = HashMap::new();
    for (tool_id, binding) in spec.core_bindings() {
        tools.insert(tool_id.to_string(), binding);
    }
    for (tool_id, binding) in spec.default_optional_bindings() {
        tools.insert(tool_id.to_string(), binding);
    }
    tools
}

/// Keep only the bindings a bot record is allowed to own (I3/D3): core keys
/// belong to the spec, role-invalid keys are dropped, and a dial-less optional
/// builtin never keeps a preset. This is the one filter used by both the
/// resolver and the record write paths.
pub(super) fn filter_bot_tool_overrides(
    overrides: HashMap<String, AgentToolBinding>,
    spec: &BuiltinAgentSpec,
) -> HashMap<String, AgentToolBinding> {
    let mut kept = HashMap::with_capacity(overrides.len());
    for (tool_id, mut binding) in overrides {
        if is_core_tool(&tool_id) || !scope_of(&tool_id).allows_role(spec.role) {
            continue;
        }
        if is_optional_builtin(&tool_id) && !crate::permission::has_permission_preset(&tool_id) {
            binding.preset = None;
        }
        kept.insert(tool_id, binding);
    }
    kept
}

/// Resolve the caller's body. A bot session is rebuilt from the `default` spec
/// plus the record's non-core bindings and role text; otherwise the normal
/// settings profile is returned unchanged.
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
        let spec = default_bot_spec();
        let mut overrides = bot.body.tools.clone();
        let notes = collapse_custom_suite_bindings(&mut overrides, &merged_custom_defs(resolved));
        for note in notes {
            tracing::warn!(bot_id = %bot.id, "{note}");
        }
        let mut tools = spec_tool_bindings(spec);
        tools.extend(filter_bot_tool_overrides(overrides, spec));
        let profile = AgentProfile {
            role: spec.role,
            model_ref: String::new(),
            // Locked like every other built-in id; the bot's own text travels
            // as the role segment, not as this marker.
            system_prompt: spec.prompt_marker(),
            description: bot.description.clone(),
            tools,
            allowed_subagents: spec
                .allowed_subagents
                .iter()
                .map(|id| (*id).to_string())
                .collect(),
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
    use super::*;
    use crate::bot::BotBody;
    use crate::config::schema::ToolPreset;
    use crate::config::{SettingsWriter, TurnGuard, WorkspaceState};
    use crate::session::{SessionManager, SessionService};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Fixture {
        dir: TempDir,
        settings: Arc<SettingsWriter>,
        bots: Arc<BotService>,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let manager = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path().join("sessions.db").to_string_lossy().to_string(),
        ));
        let service = Arc::new(SessionService::new(Arc::clone(&manager)));
        let settings = Arc::new(SettingsWriter::with_path(
            dir.path().join("settings.db"),
            Arc::new(TurnGuard::new()),
        ));
        let bots = Arc::new(
            BotService::new(
                service,
                Arc::clone(&settings),
                None,
                "/proj",
                dir.path().join("bots.json"),
            )
            .expect("bot service"),
        );
        Fixture {
            dir,
            settings,
            bots,
        }
    }

    impl Fixture {
        fn resolved(&self) -> ResolvedConfig {
            crate::config::ConfigManager::resolve_without_catalog(
                self.settings.load_settings().expect("settings"),
                WorkspaceState::new(self.dir.path()),
            )
        }

        fn resolve(&self, bot: &BotRecord) -> AgentResolution {
            resolve_agent_for_session(
                &self.resolved(),
                Some(self.bots.as_ref()),
                &bot.session_id,
                &bot.id,
            )
            .expect("bot resolution")
        }
    }

    fn binding(enabled: bool, preset: Option<ToolPreset>) -> AgentToolBinding {
        AgentToolBinding {
            enabled,
            preset,
            allowed_tools: None,
        }
    }

    /// Every retired marker shape of a stored prompt collapses to the empty
    /// role, while a user's own text survives trimmed.
    #[test]
    fn role_instructions_blank_out_markers() {
        for raw in [
            "",
            "   ",
            "\n\t \r\n",
            "builtin",
            "builtin:default",
            "builtin:orchestrator",
            "builtin:general",
            "builtin:",
            "  builtin:plan  ",
        ] {
            assert_eq!(
                normalize_bot_role_instructions(raw),
                "",
                "{raw:?} must not become a role segment"
            );
        }

        assert_eq!(
            normalize_bot_role_instructions("  You are the release captain.  "),
            "You are the release captain."
        );
    }

    #[test]
    fn role_instructions_are_optional_and_trimmed() {
        let mut bot = BotRecord {
            id: "b1".into(),
            name: "One".into(),
            description: String::new(),
            created_at: 1,
            session_id: "s1".into(),
            body: BotBody::default(),
            position: crate::bot::BotPosition::None,
            sort_order: 0,
        };
        assert_eq!(bot_role_instructions(&bot), None);

        bot.body.system_prompt = "   ".into();
        assert_eq!(bot_role_instructions(&bot), None);

        bot.body.system_prompt = "builtin:orchestrator".into();
        assert_eq!(bot_role_instructions(&bot), None, "retired builtin marker");

        bot.body.system_prompt = "  You are the release captain. ".into();
        assert_eq!(
            bot_role_instructions(&bot).as_deref(),
            Some("You are the release captain.")
        );
    }

    /// The shared filter is the resolver's second line of defence: even a map
    /// that skipped record normalization cannot move a core binding or a
    /// foreign-role capability.
    #[test]
    fn tool_filter_drops_core_and_role_invalid_rows() {
        let spec = default_bot_spec();
        let kept = filter_bot_tool_overrides(
            HashMap::from([
                ("read".to_string(), binding(false, Some(ToolPreset::Safe))),
                ("exec".to_string(), binding(true, Some(ToolPreset::Safe))),
                ("plan".to_string(), binding(true, None)),
                ("webfetch".to_string(), binding(false, None)),
                ("lsp".to_string(), binding(true, Some(ToolPreset::All))),
                ("mcp_probe".to_string(), binding(true, Some(ToolPreset::All))),
            ]),
            spec,
        );

        assert!(!kept.contains_key("read"), "core must never be overridable");
        assert!(!kept.contains_key("exec"), "core must never be overridable");
        assert!(!kept.contains_key("plan"), "core must never be overridable");
        assert!(!kept["webfetch"].enabled, "non-core intent survives");
        assert!(kept["lsp"].enabled, "non-core intent survives");
        assert_eq!(
            kept["lsp"].preset, None,
            "a dial-less optional builtin keeps no preset"
        );
        assert_eq!(
            kept["mcp_probe"].preset,
            Some(ToolPreset::All),
            "custom / MCP presets are user intent"
        );
    }

    #[tokio::test]
    async fn empty_bot_body_resolves_to_the_locked_default_surface() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Empty body", "", BotBody::default())
            .await
            .expect("bot");
        let resolution = f.resolve(&bot);
        let spec = default_bot_spec();

        assert!(resolution.is_bot());
        assert_eq!(resolution.profile.role, spec.role);
        assert_eq!(resolution.profile.system_prompt, "builtin:default");
        assert_eq!(
            resolution.profile.allowed_subagents,
            vec!["explore".to_string()]
        );
        assert_eq!(resolution.profile.tools, spec_tool_bindings(spec));

        // D3: the optional defaults are on, dial-less, with no record entry.
        for tool in ["code_search", "webfetch", "websearch"] {
            let binding = resolution
                .profile
                .tools
                .get(tool)
                .unwrap_or_else(|| panic!("{tool} missing"));
            assert!(binding.enabled, "{tool} defaults on");
            assert_eq!(binding.preset, None, "{tool} has no dial");
        }
        for (tool, expected) in spec.core_bindings() {
            assert_eq!(
                resolution.profile.tools[tool], expected,
                "{tool} must be the locked core binding"
            );
        }
    }

    /// I1/D2: a tampered record cannot move a core binding, a core preset, the
    /// role, `allowed_subagents`, or the base prompt. Non-core intent still
    /// applies (I3/D3).
    #[tokio::test]
    async fn tampered_bot_body_cannot_move_the_locked_core_surface() {
        let f = fixture();
        let body = BotBody {
            system_prompt: "  You are the release captain.  ".into(),
            tools: HashMap::from([
                ("read".to_string(), binding(false, None)),
                ("exec".to_string(), binding(true, Some(ToolPreset::Safe))),
                ("webfetch".to_string(), binding(false, None)),
                ("lsp".to_string(), binding(true, Some(ToolPreset::All))),
            ]),
            allowed_subagents: vec!["general".to_string()],
        };
        let bot = f
            .bots
            .create_user_bot("Tampered", "", body)
            .await
            .expect("bot");
        let resolution = f.resolve(&bot);
        let spec = default_bot_spec();

        for (tool, expected) in spec.core_bindings() {
            assert_eq!(
                resolution.profile.tools[tool], expected,
                "{tool} must ignore the record"
            );
        }
        assert!(
            resolution.profile.tools["read"].enabled,
            "a record cannot switch a core tool off"
        );
        assert_eq!(
            resolution.profile.tools["exec"].preset,
            Some(ToolPreset::All),
            "a record cannot downgrade a core preset"
        );
        assert_eq!(
            resolution.profile.allowed_subagents,
            vec!["explore".to_string()],
            "allowed_subagents always comes from the spec"
        );
        assert_eq!(resolution.profile.system_prompt, spec.prompt_marker());
        assert_eq!(
            bot_role_instructions(resolution.bot.as_ref().expect("bot record")).as_deref(),
            Some("You are the release captain.")
        );

        assert!(!resolution.profile.tools["webfetch"].enabled, "webfetch off sticks");
        assert!(resolution.profile.tools["lsp"].enabled, "lsp on sticks");
        assert_eq!(resolution.profile.tools["lsp"].preset, None);
    }

    #[tokio::test]
    async fn a_subagent_session_still_resolves_from_settings() {
        let f = fixture();
        let bot = f
            .bots
            .create_user_bot("Not a child", "", BotBody::default())
            .await
            .expect("bot");
        let child = resolve_agent_for_session(&f.resolved(), Some(f.bots.as_ref()), "child", "explore")
            .expect("child profile");
        assert!(!child.is_bot());
        assert_eq!(child.profile.role, crate::config::schema::AgentRole::Subagent);
        assert!(
            !f.resolve(&bot).profile.tools.is_empty(),
            "sanity: the bot path is the one under test"
        );
    }
}
