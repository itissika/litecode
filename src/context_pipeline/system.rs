use crate::config::AgentConfig;
use crate::config::global_db::{
    CITATION_PROMPT, DEFAULT_PROMPT, HARNESS_CODE_RUNTIME, HARNESS_SYSTEM, builtin_prompt_for,
    is_builtin_prompt_marker,
};
use crate::context_pipeline::env::Context;

/// Unified entry: harness header + code runtime + editable layers
/// + primary-only citations + workspace `AGENTS.md` then `CLAUDE.md`
/// (skipped for `explore`).
pub fn build_system_prompt(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
) -> String {
    build_system_prompt_with_platform(agent_id, agent_config, ctx, None)
}

/// Same stack with an engine-owned platform segment between the editable body
/// and citations. Only [`build_system_prompt`] calls this with `None` today; bot
/// sessions use [`build_system_prompt_for_bot`], which owns the `# Bot Role`
/// segment as well.
pub fn build_system_prompt_with_platform(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
    platform: Option<&str>,
) -> String {
    let editable = resolve_body(agent_id, &agent_config.system_prompt);
    let is_primary = agent_config.role == "primary";
    let body = splice_harness(&editable, is_primary, platform);
    if agent_id == "explore" {
        return body;
    }
    splice_workspace_docs(body, ctx)
}

/// System prompt for a bot session (D2 + D8):
/// `harness/system` → `harness/code_runtime` → `DEFAULT_PROMPT` → optional
/// `# Bot Role\n<role>` → platform → citations → `AGENTS.md` → `CLAUDE.md`.
///
/// A bot sits on the locked `default` base, so the editable block is always the
/// compile-time `DEFAULT_PROMPT`, never a stored body. `bot_role` is the user's
/// role/identity text; it is skipped when absent or blank, so no empty heading
/// and no stray blank line is produced.
pub fn build_system_prompt_for_bot(
    ctx: Option<&Context>,
    bot_role: Option<&str>,
    platform: &str,
) -> String {
    let bot_role = bot_role.map(str::trim).filter(|role| !role.is_empty());
    let body = splice_harness_with_role(DEFAULT_PROMPT.trim(), bot_role, true, Some(platform));
    splice_workspace_docs(body, ctx)
}

/// Fixed LiteCode layers around the editable block. An empty editable block still
/// gets the harness header, code runtime, and (for primary agents) citations.
fn splice_harness(editable: &str, is_primary: bool, platform: Option<&str>) -> String {
    splice_harness_with_role(editable, None, is_primary, platform)
}

/// The one harness splice. `bot_role` is the only segment a non-bot session
/// never passes; everything else is shared so both paths keep `"\n\n"`
/// between layers.
fn splice_harness_with_role(
    editable: &str,
    bot_role: Option<&str>,
    is_primary: bool,
    platform: Option<&str>,
) -> String {
    let role_segment = bot_role.map(|role| format!("# Bot Role\n{role}"));
    let mut parts = Vec::with_capacity(6);
    parts.push(HARNESS_SYSTEM.trim());
    parts.push(HARNESS_CODE_RUNTIME.trim());
    let editable = editable.trim();
    if !editable.is_empty() {
        parts.push(editable);
    }
    if let Some(role_segment) = role_segment.as_deref() {
        parts.push(role_segment);
    }
    if let Some(platform) = platform
        .map(str::trim)
        .filter(|platform| !platform.is_empty())
    {
        parts.push(platform);
    }
    if is_primary {
        parts.push(CITATION_PROMPT.trim());
    }
    parts.join("\n\n")
}

/// Workspace instruction files, appended in a stable order: `AGENTS.md` then
/// `CLAUDE.md`. An absent file adds no bytes, so a CLAUDE-only workspace keeps
/// the same suffix it had before.
fn splice_workspace_docs(body: String, ctx: Option<&Context>) -> String {
    let agents_md = ctx.and_then(|c| c.agents_md.as_deref()).unwrap_or("");
    let claude_md = ctx.and_then(|c| c.claude_md.as_deref()).unwrap_or("");
    let body = splice_instruction_file(&body, "AGENTS.md", agents_md);
    splice_instruction_file(&body, "CLAUDE.md", claude_md)
}

/// The editable block of the system prompt.
///
/// I1: a built-in agent id (one `builtin_prompt_for` knows) always resolves to
/// its compile-time pack, `trim()`ed. The stored text is ignored outright, so
/// no settings write, DB row or hand-built `AgentConfig` can replace a locked
/// prompt with something else.
///
/// Non-built-in ids keep the legacy rule: a `builtin`/`builtin:*` marker still
/// asks for the pack for that id (none exists for a non-built-in id, so the
/// block comes out empty), and any other stored text is used verbatim. Bot
/// sessions never come through here: they use
/// [`build_system_prompt_for_bot`], whose editable block is always `DEFAULT_PROMPT`.
fn resolve_body(agent_id: &str, stored: &str) -> String {
    if let Some(pack) = builtin_prompt_for(agent_id) {
        return pack.trim().to_string();
    }
    if is_builtin_prompt_marker(stored) {
        builtin_prompt_for(agent_id)
            .unwrap_or("")
            .trim()
            .to_string()
    } else {
        stored.to_string()
    }
}

fn splice_instruction_file(body: &str, filename: &str, content: &str) -> String {
    if content.is_empty() {
        return body.to_string();
    }
    format!("{body}\n\n<context from=\"{filename}\">\n{content}\n</context>\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;

    fn make_ctx(claude_md: Option<&str>, agents_md: Option<&str>) -> Context {
        Context {
            cwd: std::path::PathBuf::from("/home/user/project"),
            workspace_paths: WorkspacePaths::for_legacy_root(&std::path::PathBuf::from(
                "/home/user/project",
            )),
            agents_md: agents_md.map(str::to_string),
            claude_md: claude_md.map(str::to_string),
        }
    }

    fn cfg(role: &str, system_prompt: &str) -> AgentConfig {
        AgentConfig {
            role: role.into(),
            system_prompt: system_prompt.into(),
            ..AgentConfig::default()
        }
    }

    #[test]
    fn primary_splices_agents_md_then_claude_md() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let prompt = build_system_prompt("default", &cfg("primary", "builtin:general"), Some(&ctx));
        let harness = prompt.find("# System").unwrap();
        let code_rt = prompt.find("# Tool Runtime (`run_code`)").unwrap();
        let body = prompt.find("LiteCode's default coding agent").unwrap();
        let agents = prompt.find("<context from=\"AGENTS.md\">").unwrap();
        let claude = prompt.find("<context from=\"CLAUDE.md\">").unwrap();
        assert!(harness < code_rt);
        assert!(code_rt < body);
        assert!(prompt.find("# Citations").unwrap() < agents);
        assert!(agents < claude);
        assert!(prompt.contains("follow agents"));
        assert!(prompt.contains("# contract"));
    }

    #[test]
    fn subagents_omit_citations_and_explore_skips_workspace_docs() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));

        let general = build_system_prompt("general", &cfg("subagent", "builtin:general"), Some(&ctx));
        assert!(general.contains("# System"));
        assert!(general.contains("# Tool Runtime (`run_code`)"));
        assert!(!general.contains("# Citations"));
        assert!(general.contains("<context from=\"AGENTS.md\">"));
        assert!(general.contains("<context from=\"CLAUDE.md\">"));

        let verification = build_system_prompt(
            "verification",
            &cfg("subagent", "builtin:verification"),
            Some(&ctx),
        );
        assert!(verification.contains("# System"));
        assert!(verification.contains("# Tool Runtime (`run_code`)"));
        assert!(!verification.contains("# Citations"));
        assert!(verification.contains("<context from=\"AGENTS.md\">"));
        assert!(verification.contains("<context from=\"CLAUDE.md\">"));

        let explore = build_system_prompt("explore", &cfg("subagent", "builtin:explore"), Some(&ctx));
        assert!(explore.contains("# System"));
        assert!(explore.contains("# Tool Runtime (`run_code`)"));
        assert!(!explore.contains("# Citations"));
        assert!(!explore.contains("<context from=\"AGENTS.md\">"));
        assert!(!explore.contains("<context from=\"CLAUDE.md\">"));
    }

    #[test]
    fn plan_primary_keeps_citations_and_workspace_docs() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let plan = build_system_prompt("plan", &cfg("primary", "builtin:plan"), Some(&ctx));
        assert!(plan.contains("# System"));
        assert!(plan.contains("# Tool Runtime (`run_code`)"));
        assert!(plan.contains("# Citations"));
        assert!(plan.contains("<context from=\"AGENTS.md\">"));
        assert!(plan.contains("<context from=\"CLAUDE.md\">"));
    }

    #[test]
    fn bot_prompt_splices_default_role_and_platform_in_order() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let prompt = build_system_prompt_for_bot(
            Some(&ctx),
            Some("You are the release captain."),
            "BOT PLATFORM",
        );
        let harness = prompt.find("# System").expect("harness");
        let code_rt = prompt
            .find("# Tool Runtime (`run_code`)")
            .expect("code runtime");
        let default = prompt
            .find("LiteCode's default coding agent")
            .expect("default pack");
        let role = prompt
            .find("# Bot Role\nYou are the release captain.")
            .expect("bot role segment");
        let platform = prompt.find("BOT PLATFORM").expect("platform");
        let citations = prompt.find("# Citations").expect("citations");
        let agents = prompt.find("<context from=\"AGENTS.md\">").expect("agents");
        let claude = prompt.find("<context from=\"CLAUDE.md\">").expect("claude");
        assert!(harness < code_rt, "{prompt}");
        assert!(code_rt < default);
        assert!(default < role);
        assert!(role < platform);
        assert!(platform < citations);
        assert!(citations < agents);
        assert!(agents < claude);
        // D8: exactly one heading, and the role sits between the pack and the
        // platform segment on `\n\n` boundaries.
        assert_eq!(prompt.matches("# Bot Role").count(), 1);
        assert!(prompt.contains("captain.\n\nBOT PLATFORM"));
    }

    #[test]
    fn bot_prompt_omits_role_heading_when_empty() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let baseline = build_system_prompt_for_bot(Some(&ctx), None, "BOT PLATFORM");
        for role in [None, Some(""), Some("   \n\t ")] {
            let prompt = build_system_prompt_for_bot(Some(&ctx), role, "BOT PLATFORM");
            assert_eq!(prompt, baseline, "{role:?} must add no bytes");
            assert!(
                !prompt.contains("# Bot Role"),
                "{role:?} must not emit a heading"
            );
            let anchors = [
                "# System",
                "# Tool Runtime (`run_code`)",
                "LiteCode's default coding agent",
                "BOT PLATFORM",
                "# Citations",
                "<context from=\"AGENTS.md\">",
                "<context from=\"CLAUDE.md\">",
            ];
            for anchor in anchors {
                assert!(prompt.contains(anchor), "{role:?} missing {anchor}");
            }
            // No empty segment: every layer boundary is exactly one `\n\n`.
            assert!(prompt.contains(&format!("{}\n\nBOT PLATFORM", DEFAULT_PROMPT.trim())));
        }
    }

    #[test]
    fn platform_segment_sits_between_body_and_citations() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let prompt = build_system_prompt_with_platform(
            "default",
            &cfg("primary", "builtin:default"),
            Some(&ctx),
            Some("BOT PLATFORM"),
        );
        // `default` is locked, so the body anchor is the built-in pack, not
        // any text the caller stored.
        let body = prompt.find("LiteCode's default coding agent").expect("body");
        let platform = prompt.find("BOT PLATFORM").expect("platform");
        let citations = prompt.find("# Citations").expect("citations");
        let agents = prompt.find("<context from=\"AGENTS.md\">").expect("agents");
        assert!(body < platform);
        assert!(platform < citations);
        assert!(citations < agents);
    }

    #[test]
    fn builtin_agent_ignores_stored_custom_prompt() {
        // I1: the pack is locked. Any text a caller stores — a user prompt, a
        // retired marker, empty — must produce exactly the same bytes as the
        // seeded marker.
        for (id, role) in BUILTIN_ROLES {
            let expected = build_system_prompt(id, &cfg(role, &format!("builtin:{id}")), None);
            for stored in ["CUSTOM OVERRIDE", "", "builtin", "edited by the user"] {
                let prompt = build_system_prompt(id, &cfg(role, stored), None);
                assert_eq!(prompt, expected, "{id} accepted stored {stored:?}");
                assert!(
                    !prompt.contains("CUSTOM OVERRIDE"),
                    "{id} leaked the stored prompt"
                );
                assert!(
                    !prompt.contains("edited by the user"),
                    "{id} leaked the stored prompt"
                );
            }
        }
    }

    /// Absolute guard the I5 red line asks for: exact byte length and FNV-1a
    /// digest for every built-in prompt, with and without workspace context.
    /// Self-contained on purpose (no `.tmp/` read), so CI proves it too.
    #[test]
    fn builtin_prompts_match_locked_golden_digests() {
        let ctx = fixture_ctx();
        for (id, role) in BUILTIN_ROLES {
            let config = cfg(role, &format!("builtin:{id}"));
            let nocontext = build_system_prompt(id, &config, None);
            let withcontext = build_system_prompt(id, &config, Some(&ctx));
            let (want_len, want_fnv, want_len_ctx, want_fnv_ctx) = LOCKED_GOLDEN
                .iter()
                .find(|row| row.0 == id)
                .map(|row| (row.1, row.2, row.3, row.4))
                .unwrap_or_else(|| panic!("{id} missing from LOCKED_GOLDEN"));

            assert_eq!(nocontext.len(), want_len, "{id} nocontext length");
            assert_eq!(
                fnv1a64(nocontext.as_bytes()),
                want_fnv,
                "{id} nocontext digest"
            );
            assert_eq!(withcontext.len(), want_len_ctx, "{id} withcontext length");
            assert_eq!(
                fnv1a64(withcontext.as_bytes()),
                want_fnv_ctx,
                "{id} withcontext digest"
            );
        }
    }

    /// `(id, nocontext len, nocontext fnv, withcontext len, withcontext fnv)`
    /// — the baseline `.tmp/agent-lockdown/w0-report.md` §12.3 recorded from the
    /// pre-lockdown code. Both halves must stay byte-identical through the lock.
    const LOCKED_GOLDEN: [(&str, usize, u64, usize, u64); 6] = [
        (
            "default",
            7740,
            0xfce4_7d37_5948_e171,
            7850,
            0xc173_cf79_c753_84b7,
        ),
        (
            "plan",
            6497,
            0x8a19_d7c3_6d60_4cf3,
            6607,
            0x64be_72b6_91a0_9da9,
        ),
        (
            "orchestrator",
            6720,
            0xab16_13bc_7c05_ae60,
            6830,
            0x34cb_19ed_ebbb_620e,
        ),
        (
            "explore",
            4434,
            0x36e6_4abd_feef_5687,
            4434,
            0x36e6_4abd_feef_5687,
        ),
        (
            "general",
            4567,
            0x07bf_f66f_9ace_178b,
            4677,
            0x4be6_4f08_42dd_5b41,
        ),
        (
            "verification",
            6758,
            0x5ab8_a8e9_cacc_a1ea,
            6868,
            0xea2e_69df_2bd0_6e48,
        ),
    ];

    /// Every built-in id with the role its spec gives it.
    const BUILTIN_ROLES: [(&str, &str); 6] = [
        ("default", "primary"),
        ("plan", "primary"),
        ("orchestrator", "primary"),
        ("explore", "subagent"),
        ("general", "subagent"),
        ("verification", "subagent"),
    ];

    /// The exact fixture W0 used to render the `withcontext` golden.
    fn fixture_ctx() -> Context {
        let cwd = std::path::PathBuf::from("/home/litecode/fixture-workspace");
        Context {
            workspace_paths: WorkspacePaths::for_legacy_root(&cwd),
            cwd,
            agents_md: Some("AGENTS_FIXTURE".to_string()),
            claude_md: Some("CLAUDE_FIXTURE".to_string()),
        }
    }

    /// FNV-1a 64-bit, the digest W0 printed next to each golden file.
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}
