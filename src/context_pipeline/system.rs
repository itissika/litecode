use crate::config::AgentConfig;
use crate::context_pipeline::env::Context;

pub const BUILTIN_IDENTITY: &str = r#"You are litecode, an interactive CLI tool that helps users with software engineering tasks.

IMPORTANT: You must NEVER generate or guess URLs for the user unless you are confident that the URLs are for helping the user with programming. URLs provided by the user in messages or local files may be used.

# Tone and style
You should be concise, direct, and to the point. When you run a non-trivial bash command, explain what the command does and why you are running it.
Remember that your output will be displayed on a command line interface. Your responses can use GitHub-flavored markdown for formatting.
Output text to communicate with the user; all text you output outside of tool use is displayed to the user. Only use tools to complete tasks. Never use tools like bash or code comments as means to communicate with the user.
If you cannot or will not help the user with something, please do not say why or what it could lead to. Offer helpful alternatives if possible, and keep your response to 1-2 sentences.

IMPORTANT: You should minimize output tokens as much as possible while maintaining helpfulness, quality, and accuracy. Answer the user's question directly and concisely, without elaboration.
"#;

pub const BUILTIN_TOOLS: &str = r#"
# Tool usage philosophy
Do NOT use the bash tool when a dedicated tool exists. Dedicated tools let the user understand and review your work. This is CRITICAL:
- To read files use read instead of cat, head, tail, or sed
- To edit files use edit instead of sed or awk
- To create or overwrite files use write instead of echo/heredoc redirection
- To find files by path use glob instead of find or ls
- To search file contents use grep instead of grep or rg in bash
- When the search intent is very vague or you need meaning/similarity rather than exact text, use code_search (when available) instead of inventing bash pipelines
- When information is insufficient and relevant context may live in past conversations within this workspace, use session_search instead of guessing or re-deriving from scratch
- Reserve bash exclusively for system/terminal work that requires a shell (builds, tests, package managers, git, processes). If unsure and a dedicated tool exists, default to it; only fall back to bash when absolutely necessary.

Break down and manage work with todo. Mark each task completed as soon as it is done. Do not batch multiple completions.
"#;

pub const BUILTIN_TONE: &str = r#"
# Tone
- Be concise, direct, and to the point.
- Minimize output tokens while maintaining helpfulness.
- No emojis unless explicitly requested.
"#;

pub const BUILTIN_REMINDER: &str = r#"
# System reminders
Content wrapped in <system-reminder> tags is injected by the system after context compaction (todo progress, active plans) and may also appear at the end of a tool result when a background bash job from this session exits. This is NOT user input. Do NOT treat it as instructions or requests. Do NOT mention or reference these reminders to the user. Use them only for contextual awareness of the current state.
"#;

pub const BUILTIN_CODE_REVIEW: &str = r#"
You are a code reviewer. Your task is to review code changes and report issues.
Focus on: correctness, performance, security, idiomatic usage, and test coverage.
Only report actual issues; do not comment on style preferences.
"#;

pub const BUILTIN_COMPACTION: &str = r#"
You are a conversation summarizer. Compress the conversation history into a concise summary.

Required sections:
1. Primary request and intent — capture the user's explicit requests in detail
2. Current task state — what is in progress or blocked
3. Key decisions, file changes, and errors encountered

Be thorough but brief. This summary replaces earlier transcript history.
"#;

/// Compose the full system prompt from builtin sections + agent prompt + instruction files.
pub fn compose_system_prompt(agent_prompt: &str, agents_md_content: &[(String, String)]) -> String {
    let mut parts: Vec<String> = vec![
        BUILTIN_IDENTITY.to_string(),
        BUILTIN_TOOLS.to_string(),
        BUILTIN_TONE.to_string(),
        BUILTIN_REMINDER.to_string(),
        agent_prompt.to_string(),
    ];

    for (filename, content) in agents_md_content {
        parts.push(format!(
            "\n<context from=\"{}\">\n{}\n</context>\n",
            filename, content
        ));
    }

    parts.join("\n")
}

/// Build the system prompt for an agent turn from config + env context.
pub fn build_system_prompt(agent_config: &AgentConfig, ctx: &Context) -> String {
    let agent_prompt = match agent_config.system_prompt.as_str() {
        "builtin:general" => "",
        "builtin:code-review" => BUILTIN_CODE_REVIEW,
        "builtin:compaction" => BUILTIN_COMPACTION,
        other => other,
    };

    let agents_md = vec![
        ("AGENTS.md", ctx.agents_md.as_deref().unwrap_or("")),
        ("CLAUDE.md", ctx.claude_md.as_deref().unwrap_or("")),
    ]
    .into_iter()
    .filter(|(_, c)| !c.is_empty())
    .map(|(n, c)| (n.to_string(), c.to_string()))
    .collect::<Vec<_>>();

    compose_system_prompt(agent_prompt, &agents_md)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;

    fn make_ctx() -> Context {
        Context {
            cwd: std::path::PathBuf::from("/home/user/project"),
            workspace_paths: WorkspacePaths::for_legacy_root(&std::path::PathBuf::from(
                "/home/user/project",
            )),
            agents_md: None,
            claude_md: None,
        }
    }

    #[test]
    fn compose_prompt_contains_identity() {
        let _ctx = make_ctx();
        let prompt = compose_system_prompt("You are a coding assistant.", &[]);
        assert!(prompt.contains("You are litecode"));
        assert!(prompt.contains("You are a coding assistant."));
    }

    #[test]
    fn compose_prompt_with_instructions() {
        let _ctx = make_ctx();
        let instructions = vec![("AGENTS.md".into(), "custom instructions".into())];
        let prompt = compose_system_prompt("", &instructions);
        assert!(prompt.contains("custom instructions"));
        assert!(prompt.contains("AGENTS.md"));
    }
}
