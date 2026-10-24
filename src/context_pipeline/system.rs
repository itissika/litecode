use crate::config::AgentConfig;
use crate::config::global_db::{
    CITATION_PROMPT, HARNESS_SYSTEM, HARNESS_TOOLS, builtin_prompt_for, is_builtin_prompt_marker,
};
use crate::context_pipeline::env::Context;

/// Unified entry: harness header + editable layers + tool rules + citations
/// + process-snapshotted `AGENTS.md` then `CLAUDE.md`.
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
    let agents_md = ctx.and_then(|c| c.agents_md.as_deref()).unwrap_or("");
    let claude_md = ctx.and_then(|c| c.claude_md.as_deref()).unwrap_or("");
    // Stable order: AGENTS.md then CLAUDE.md. An absent file adds no bytes,
    // so a CLAUDE-only workspace keeps the same suffix it had before.
    let body = splice_instruction_file(&body, "AGENTS.md", agents_md);
    splice_instruction_file(&body, "CLAUDE.md", claude_md)
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
        let agents = prompt.find("<context from=\"AGENTS.md\">").unwrap();
        let claude = prompt.find("<context from=\"CLAUDE.md\">").unwrap();
        assert!(prompt.find("# System").unwrap() < prompt.find("General Purpose Agent").unwrap());
        assert!(prompt.find("# Citations").unwrap() < agents);
        assert!(agents < claude);
        assert!(prompt.contains("follow agents"));
        assert!(prompt.contains("# contract"));
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
