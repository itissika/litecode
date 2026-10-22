pub mod apply_edits;
pub mod ask_user;
pub mod bash_safety;
pub mod code;
pub mod code_search;
pub mod command_job;
pub mod custom;
pub mod custom_hub;
pub mod edit;
pub mod file_path;
pub mod glob;
pub mod grep;
pub mod knowledge;
pub mod litecode_workspace;
pub mod lsp;
pub mod lsp_feedback;
pub mod mcp_tool;
pub mod plan;
pub mod read;
pub mod run;
pub mod wait;
pub mod kill;
pub mod session_search;
pub mod subagent;
pub mod todo;
pub mod webfetch;
pub mod websearch;
pub mod write;

#[cfg(test)]
mod baseline;
#[cfg(test)]
mod run_lock_gate;

/// Model-facing description of a built-in tool.
///
/// Prose lives in `descriptions/<name>.md` and is embedded with `include_str!`.
/// Trailing whitespace, including the file's final newline, is not part of the string sent to the model.
/// `subagent_launch` appends the live agent catalog when one is configured.
pub(crate) fn description_text(embedded: &str) -> String {
    embedded.trim_end().to_string()
}

/// snake_case wire name of a progress enum (same as the engines detail JSON).
fn wire_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// First line of `code_search` / `session_search` results — the only formatter
/// (WP-057 sync% contract v2, `.litecode/ops/artifacts/WP-057-sync-pct-contract.md`):
///
/// `sync_pct: <value>[ (<label>)][ chunks <done>/<total>]`, value ∈ `<n>%` | `scanning` |
/// `failed` | `unknown`. `n` comes only from [`CorpusProgress::percent`]; the chunks
/// segment is emitted only with `<n>%`.
/// Fixed-label first lines for code_search non-ready returns (WP-148, contract v1 §C).
#[derive(Debug, Clone, Copy)]
pub(crate) enum SyncLineLabel<'a> {
    /// `sync_pct: unknown (engine off)`
    EngineOff,
    /// `sync_pct: N% (warming) chunks d/t` or `sync_pct: scanning (warming)`
    Warming,
    /// `sync_pct: failed (<sanitized reason>)`
    Failed(&'a str),
}

const FAILED_REASON_MAX_CHARS: usize = 60;

/// Strip `()` and line breaks, collapse whitespace, cap length: keeps the label regex-safe.
pub(crate) fn sanitize_sync_label(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '(' | ')' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let truncated: String = collapsed.chars().take(FAILED_REASON_MAX_CHARS).collect();
    let out = truncated.trim().to_string();
    if out.is_empty() {
        "engine failed".into()
    } else {
        out
    }
}

pub(crate) fn format_sync_pct_line(
    sync: Option<&crate::engines::progress::CorpusProgress>,
) -> String {
    format_sync_pct_line_labeled(sync, None)
}

/// The single sync_pct first-line builder (contract v2 grammar).
pub(crate) fn format_sync_pct_line_labeled(
    sync: Option<&crate::engines::progress::CorpusProgress>,
    label: Option<SyncLineLabel<'_>>,
) -> String {
    use crate::engines::progress::{CorpusPhase, CorpusState};
    match label {
        None => {}
        Some(SyncLineLabel::EngineOff) => return "sync_pct: unknown (engine off)".into(),
        Some(SyncLineLabel::Failed(reason)) => {
            // Engine detail first (sanitized); fall back to the progress reason code.
            let reason = if reason.trim().is_empty() {
                sync.and_then(|p| p.error.as_ref())
                    .map(|err| wire_name(&err.reason))
                    .unwrap_or_default()
            } else {
                reason.to_string()
            };
            return format!("sync_pct: failed ({})", sanitize_sync_label(&reason));
        }
        Some(SyncLineLabel::Warming) => {
            if let Some(p) = sync {
                if p.phase != Some(CorpusPhase::Scanning) {
                    if let (Some(pct), Some(total)) = (p.percent(), p.chunks_total) {
                        return format!(
                            "sync_pct: {}% (warming) chunks {}/{total}",
                            pct.min(100),
                            p.chunks_done.min(total)
                        );
                    }
                }
            }
            return "sync_pct: scanning (warming)".into();
        }
    }
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
                // Defensive: never show done > total even on malformed input.
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

pub(crate) fn prepend_sync_pct(
    sync: Option<&crate::engines::progress::CorpusProgress>,
    body: &str,
) -> String {
    format!("{}\n{}", format_sync_pct_line(sync), body)
}

#[cfg(test)]
mod sync_line_tests {
    
    

}
