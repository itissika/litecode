use crate::config::AgentConfig;
use crate::config::global_db::{builtin_prompt_for, is_builtin_prompt_marker};
use crate::context_pipeline::env::Context;

/// Unified entry: body + CLAUDE.md. Hidden agents get body only.
pub fn build_system_prompt(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
) -> String {
    let body = resolve_body(agent_id, &agent_config.system_prompt);
    if agent_config.role == "hidden" {
        return body;
    }
    let claude_md = ctx.and_then(|c| c.claude_md.as_deref()).unwrap_or("");
    splice_claude_md(&body, claude_md)
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

fn splice_claude_md(body: &str, claude_md: &str) -> String {
    if claude_md.is_empty() {
        return body.to_string();
    }
    format!("{body}\n\n<context from=\"CLAUDE.md\">\n{claude_md}\n</context>\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::global_db::{COMPACTION_PROMPT, DEFAULT_PROMPT};

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
    fn builtin_general_marker_loads_default_pack() {
        let prompt = build_system_prompt("default", &cfg("primary", "builtin:general"), None);
        assert!(prompt.starts_with("You are a General Purpose Agent in LiteCode."));
        assert!(!prompt.contains("You are litecode"));
        assert_eq!(prompt, DEFAULT_PROMPT.trim());
    }

    #[test]
    fn user_override_replaces_builtin() {
        let prompt =
            build_system_prompt("default", &cfg("primary", "You are a custom agent."), None);
        assert_eq!(prompt, "You are a custom agent.");
        assert!(!prompt.contains("General Purpose Agent"));
    }

    #[test]
    fn primary_splices_claude_md_not_agents_md() {
        let ctx = make_ctx(Some("# contract"), Some("never splice agents md"));
        let prompt = build_system_prompt("default", &cfg("primary", "builtin:general"), Some(&ctx));
        assert!(prompt.contains("<context from=\"CLAUDE.md\">"));
        assert!(prompt.contains("# contract"));
        assert!(!prompt.contains("never splice agents md"));
        assert!(!prompt.contains("AGENTS.md"));
    }

    #[test]
    fn hidden_never_splices_md() {
        let ctx = make_ctx(Some("# contract"), Some("agents"));
        let prompt = build_system_prompt(
            "compaction",
            &cfg("hidden", "builtin:compaction"),
            Some(&ctx),
        );
        assert_eq!(prompt, COMPACTION_PROMPT.trim());
        assert!(!prompt.contains("CLAUDE.md"));
        assert!(!prompt.contains("# contract"));
    }

    #[test]
    fn code_review_marker_is_just_builtin_for_that_agent() {
        let prompt = build_system_prompt("default", &cfg("primary", "builtin:code-review"), None);
        assert!(prompt.contains("General Purpose Agent"));
        assert!(!prompt.contains("code reviewer"));
    }

    #[test]
    fn explore_marker_loads_explore_pack() {
        let prompt = build_system_prompt("explore", &cfg("subagent", "builtin:explore"), None);
        assert!(prompt.contains("Explore Purpose Agent"));
        assert!(prompt.contains("READ-ONLY"));
    }
}
