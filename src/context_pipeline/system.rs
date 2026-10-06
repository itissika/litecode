use crate::config::AgentConfig;
use crate::config::global_db::{
    CITATION_PROMPT, HARNESS_SYSTEM, HARNESS_TOOLS, builtin_prompt_for, is_builtin_prompt_marker,
};
use crate::context_pipeline::env::Context;

/// Unified entry: harness header + editable layers + tool rules + citations + CLAUDE.md.
/// Hidden agents get the editable body only.
pub fn build_system_prompt(
    agent_id: &str,
    agent_config: &AgentConfig,
    ctx: Option<&Context>,
) -> String {
    let editable = resolve_body(agent_id, &agent_config.system_prompt);
    if agent_config.role == "hidden" {
        return editable;
    }
    let body = splice_harness(&editable);
    let claude_md = ctx.and_then(|c| c.claude_md.as_deref()).unwrap_or("");
    splice_claude_md(&body, claude_md)
}

/// Fixed LiteCode layers around the editable block. An empty editable block still
/// gets the header, tool rules, and citations.
fn splice_harness(editable: &str) -> String {
    let mut parts = Vec::with_capacity(4);
    parts.push(HARNESS_SYSTEM.trim());
    let editable = editable.trim();
    if !editable.is_empty() {
        parts.push(editable);
    }
    parts.push(HARNESS_TOOLS.trim());
    parts.push(CITATION_PROMPT.trim());
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
    use crate::config::global_db::{
        CITATION_PROMPT, COMPACTION_PROMPT, DEFAULT_PROMPT, GENERAL_PROMPT, HARNESS_SYSTEM,
        HARNESS_TOOLS, ORCHESTRATOR_PROMPT,
    };

    fn visible(editable: &str) -> String {
        let mut parts = vec![HARNESS_SYSTEM.trim()];
        let editable = editable.trim();
        if !editable.is_empty() {
            parts.push(editable);
        }
        parts.push(HARNESS_TOOLS.trim());
        parts.push(CITATION_PROMPT.trim());
        parts.join("\n\n")
    }

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
        assert!(prompt.starts_with("# System"));
        assert!(!prompt.contains("You are litecode"));
        assert_eq!(prompt, visible(DEFAULT_PROMPT));
        let identity = prompt
            .find("You are a General Purpose Agent in LiteCode.")
            .unwrap();
        assert!(prompt.find("# System").unwrap() < identity);
        assert!(identity < prompt.find("# Using your tools").unwrap());
        assert!(prompt.find("# Using your tools").unwrap() < prompt.find("# Citations").unwrap());
        assert!(prompt.contains("[@ key=\"seq\"]"));
        assert!(prompt.contains("knowledge guide"));
        assert!(prompt.contains("[@ file=\"src/a.rs\"]"));
        assert!(!prompt.contains("## Knowledge nodes"));
        assert!(!prompt.contains("[Docs](https://example.com/docs)"));
        assert!(!prompt.contains("file:src/auth/validate.ts"));
        assert!(!prompt.contains("[@ id="));
        assert!(!prompt.contains("file_path:line_number"));
        assert!(!prompt.contains("owner/repo#123"));
        assert!(!prompt.contains("# Doing tasks"));
    }

    #[test]
    fn builtin_orchestrator_marker_loads_orchestrator_pack() {
        let prompt = build_system_prompt(
            "orchestrator",
            &cfg("primary", "builtin:orchestrator"),
            None,
        );
        assert!(prompt.starts_with("# System"));
        assert_eq!(prompt, visible(ORCHESTRATOR_PROMPT));
        let identity = prompt.find("You are LiteCode's Orchestrator.").unwrap();
        assert!(prompt.find("# System").unwrap() < identity);
        assert!(identity < prompt.find("# Citations").unwrap());
        assert!(!prompt.contains("file_path:line_number"));
    }

    #[test]
    fn user_override_replaces_editable_layers() {
        let prompt =
            build_system_prompt("default", &cfg("primary", "You are a custom agent."), None);
        assert_eq!(prompt, visible("You are a custom agent."));
        assert!(prompt.contains("# System"));
        assert!(prompt.contains("# Using your tools"));
        assert!(prompt.contains("# Citations"));
        assert!(!prompt.contains("General Purpose Agent"));
    }

    #[test]
    fn empty_override_still_splices_harness() {
        let prompt = build_system_prompt("default", &cfg("primary", "  "), None);
        assert_eq!(prompt, visible(""));
        assert!(prompt.starts_with("# System"));
        assert!(prompt.contains("# Using your tools"));
        assert!(prompt.contains("# Citations"));
        assert!(!prompt.contains("General Purpose Agent"));
    }

    #[test]
    fn primary_splices_claude_md_not_agents_md() {
        let ctx = make_ctx(Some("# contract"), Some("never splice agents md"));
        let prompt = build_system_prompt("default", &cfg("primary", "builtin:general"), Some(&ctx));
        assert!(prompt.contains("<context from=\"CLAUDE.md\">"));
        assert!(prompt.contains("# contract"));
        assert!(prompt.find("# System").unwrap() < prompt.find("General Purpose Agent").unwrap());
        assert!(
            prompt.find("# Citations").unwrap()
                < prompt.find("<context from=\"CLAUDE.md\">").unwrap()
        );
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
        assert!(!prompt.contains("# System"));
        assert!(!prompt.contains("# Citations"));
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
        assert!(prompt.starts_with("# System"));
        assert!(prompt.contains("Explore Purpose Agent"));
        assert!(prompt.contains("Don't change local or remote state."));
        assert!(prompt.contains("# Citations"));
        assert!(!prompt.contains("READ-ONLY"));
        assert!(!prompt.contains("# Core Tools"));
    }

    #[test]
    fn general_marker_loads_general_pack() {
        let prompt = build_system_prompt("general", &cfg("subagent", "builtin:general"), None);
        assert!(prompt.starts_with("# System"));
        assert!(prompt.contains("You are general,"));
        assert!(prompt.contains("You can work heads-down."));
        assert!(prompt.contains("# Citations"));
        assert!(!prompt.contains("# Collaboration"));
        let system = prompt.find("# System").unwrap();
        let identity = prompt.find("You are general,").unwrap();
        assert!(system < identity);
        assert_eq!(prompt, visible(GENERAL_PROMPT));
    }
}
