use crate::config::AgentConfig;
use crate::config::global_db::{
    CITATION_PROMPT, HARNESS_CODE_RUNTIME, HARNESS_SYSTEM, builtin_prompt_for,
    is_builtin_prompt_marker,
};
use crate::context_pipeline::env::Context;

/// Unified entry: harness header + code runtime + editable layers
/// + primary-only citations + workspace `AGENTS.md` then `CLAUDE.md`
/// (skipped for `explore`). Hidden agents get the editable body only.
pub fn build_system_prompt(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
) -> String {
    build_system_prompt_with_platform(agent_id, agent_config, ctx, None)
}

/// Same stack with an engine-owned platform segment between the editable body
/// and citations. Bot sessions pass their segment here; it is never part of the
/// bot body and never user-editable.
pub fn build_system_prompt_with_platform(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
    platform: Option<&str>,
) -> String {
    let editable = resolve_body(agent_id, &agent_config.system_prompt);
    if agent_config.role == "hidden" {
        return editable;
    }
    let is_primary = agent_config.role == "primary";
    let body = splice_harness(&editable, is_primary, platform);
    if agent_id == "explore" {
        return body;
    }
    let agents_md = ctx.and_then(|c| c.agents_md.as_deref()).unwrap_or("");
    let claude_md = ctx.and_then(|c| c.claude_md.as_deref()).unwrap_or("");
    // Stable order: AGENTS.md then CLAUDE.md. An absent file adds no bytes,
    // so a CLAUDE-only workspace keeps the same suffix it had before.
    let body = splice_instruction_file(&body, "AGENTS.md", agents_md);
    splice_instruction_file(&body, "CLAUDE.md", claude_md)
}

/// Fixed LiteCode layers around the editable block. An empty editable block still
/// gets the harness header, code runtime, and (for primary agents) citations.
fn splice_harness(editable: &str, is_primary: bool, platform: Option<&str>) -> String {
    let mut parts = Vec::with_capacity(5);
    parts.push(HARNESS_SYSTEM.trim());
    parts.push(HARNESS_CODE_RUNTIME.trim());
    let editable = editable.trim();
    if !editable.is_empty() {
        parts.push(editable);
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

fn resolve_body(agent_id: &str, stored: &str) -> String {
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
    fn platform_segment_sits_between_body_and_citations() {
        let ctx = make_ctx(Some("# contract"), Some("follow agents"));
        let prompt = build_system_prompt_with_platform(
            "default",
            &cfg("primary", "editable body"),
            Some(&ctx),
            Some("BOT PLATFORM"),
        );
        let body = prompt.find("editable body").expect("body");
        let platform = prompt.find("BOT PLATFORM").expect("platform");
        let citations = prompt.find("# Citations").expect("citations");
        let agents = prompt.find("<context from=\"AGENTS.md\">").expect("agents");
        assert!(body < platform);
        assert!(platform < citations);
        assert!(citations < agents);
    }

    #[test]
    fn hidden_agent_never_splices_harness_or_instruction_files() {
        let ctx = make_ctx(Some("# contract"), Some("agents"));
        let prompt = build_system_prompt(
            "legacy_hidden",
            &cfg("hidden", "custom hidden body"),
            Some(&ctx),
        );
        assert_eq!(prompt, "custom hidden body");
        assert!(!prompt.contains("# System"));
        assert!(!prompt.contains("# Citations"));
        assert!(!prompt.contains("CLAUDE.md"));
        assert!(!prompt.contains("# contract"));
    }
}
