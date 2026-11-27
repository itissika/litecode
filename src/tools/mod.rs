pub mod apply_edits;
pub mod ask_user;
pub mod bash_safety;
pub mod bot;
pub mod browser;
pub mod code;
pub mod code_search;
pub mod command_job;
pub mod custom;
pub mod edit;
pub mod exec_command;
pub mod file_path;
pub mod glob;
pub mod grep;
pub mod knowledge;
pub mod litecode_workspace;
pub mod lsp;
pub mod lsp_feedback;
pub mod mcp_tool;
pub mod memory;
pub mod plan;
pub mod read;
pub mod session_search;
pub mod subagent;
pub mod todo;
pub mod webfetch;
pub mod websearch;
pub mod write;

#[cfg(test)]
mod baseline;

/// snake_case wire name of a progress enum (same as the engines detail JSON).
fn wire_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Single formatter for the `sync_pct:` line in `code_search` / `session_search` results.
pub(crate) fn format_sync_pct_line(
    sync: Option<&crate::engines::progress::CorpusProgress>,
) -> String {
    use crate::engines::progress::{CorpusPhase, CorpusState};

    let Some(p) = sync else {
        return "sync_pct: unknown".into();
    };
    let label = || match p.phase {
        Some(phase) => format!("{}, {}", wire_name(&p.state), wire_name(&phase)),
        None => wire_name(&p.state),
    };
    match p.state {
        CorpusState::Failed => match &p.error {
            Some(err) => format!("sync_pct: failed ({})", wire_name(&err.reason)),
            None => "sync_pct: failed".into(),
        },
        CorpusState::NeedsRebuild => "sync_pct: unknown (needs_rebuild)".into(),
        _ if p.phase == Some(CorpusPhase::Scanning) || p.chunks_total.is_none() => {
            format!("sync_pct: scanning ({})", label())
        }
        _ => match (p.percent(), p.chunks_total) {
            (Some(pct), Some(total)) => format!(
                "sync_pct: {}% ({}) chunks {}/{total}",
                pct.min(100),
                label(),
                p.chunks_done.min(total)
            ),
            _ => format!("sync_pct: unknown ({})", label()),
        },
    }
}

/// Merged retrieval progress (`.total`) from IDE; Tools never estimate a second scale.
pub(crate) fn sync_pct_for_workspace(
    engines: &crate::engines::WorkspaceEngines,
    workspace_root: Option<&std::path::Path>,
) -> Option<crate::engines::progress::CorpusProgress> {
    let root = workspace_root
        .map(|p| p.to_path_buf())
        .or_else(|| engines.code_search().workspace_root())?;
    Some(crate::engines::retrieval_progress_for_workspace(engines, &root).total)
}
