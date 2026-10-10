pub mod bash;
pub mod bash_safety;
pub mod bash_status;
pub mod code_search;
pub mod custom;
pub mod custom_hub;
pub mod edit;
pub mod file_path;
pub mod glob;
pub mod grep;
pub mod kill_shell;
pub mod knowledge;
pub mod litecode_workspace;
pub mod lsp;
pub mod lsp_feedback;
pub mod mcp_tool;
pub mod ask_user;
pub mod plan;
pub mod read;
pub mod session_search;
pub mod subagent;
pub mod todo;
pub mod wait_shell;
pub mod webfetch;
pub mod websearch;
pub mod write;

#[cfg(test)]
mod bash_jobs_contract;

/// Model-facing description of a built-in tool.
///
/// Prose lives in `descriptions/<name>.md` and is embedded with `include_str!`.
/// Trailing whitespace, including the file's final newline, is not part of the string sent to the model.
/// `edit` replaces `{{SNAPSHOT_RULE}}` with the planner constant.
/// `subagent_launch` appends the live agent catalog when one is configured.
pub(crate) fn description_text(embedded: &str) -> String {
    embedded.trim_end().to_string()
}
