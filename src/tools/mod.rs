pub mod ask_user;
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

/// First line of `code_search` / `session_search` results (WP-057 sync% contract).
pub(crate) fn format_sync_pct_line(sync: &crate::engines::code_search::SyncPct) -> String {
    match sync.sync_label {
        Some(label) => format!("sync_pct: {}% ({})", sync.sync_pct, label),
        None => format!("sync_pct: {}%", sync.sync_pct),
    }
}

/// Resolve sync% from IDE index view; Tools only consume, never invent a second progress scale.
pub(crate) fn sync_pct_for_workspace(
    engines: &crate::engines::WorkspaceEngines,
    workspace_root: Option<&std::path::Path>,
) -> crate::engines::code_search::SyncPct {
    use crate::engines::code_search::{read_pending_hint, resolve_index_view, sync_pct_from_view};
    let root = workspace_root
        .map(|p| p.to_path_buf())
        .or_else(|| engines.code_search().workspace_root());
    match root {
        Some(root) => {
            let view = resolve_index_view(&root, engines.state("code_search"));
            sync_pct_from_view(&view, read_pending_hint(&root))
        }
        None => crate::engines::code_search::SyncPct {
            sync_pct: 0,
            sync_label: None,
        },
    }
}

pub(crate) fn prepend_sync_pct(sync: &crate::engines::code_search::SyncPct, body: &str) -> String {
    format!(
        "{}
{}",
        format_sync_pct_line(sync),
        body
    )
}
