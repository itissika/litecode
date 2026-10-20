pub mod ask_user;
pub mod bash;
pub mod bash_safety;
pub mod bash_status;
pub mod code;
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
mod baseline;
#[cfg(test)]
mod phase2_gate;
#[cfg(test)]
mod phase3_gate;
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
    use super::{format_sync_pct_line, prepend_sync_pct};
    use crate::engines::progress::{CorpusError, CorpusProgress, FailReason, ProgressEngine};

    /// Exact grammar of the frontend parser (WP-138, contract v2).
    const GRAMMAR: &str = r"^sync_pct: (?:(\d{1,3})%|(scanning|failed|unknown))(?: \(([^()\n]+)\))?(?: chunks (\d+)\/(\d+))?$";

    fn assert_grammar(line: &str) {
        let re = regex::Regex::new(GRAMMAR).unwrap();
        let caps = re
            .captures(line)
            .unwrap_or_else(|| panic!("grammar: {line:?}"));
        match caps.get(1) {
            Some(pct) => {
                let n: u32 = pct.as_str().parse().unwrap();
                assert!(n <= 100, "percent over 100: {line}");
                if let (Some(d), Some(t)) = (caps.get(4), caps.get(5)) {
                    let (d, t): (u64, u64) =
                        (d.as_str().parse().unwrap(), t.as_str().parse().unwrap());
                    assert!(d <= t, "done > total: {line}");
                }
            }
            None => assert!(
                caps.get(4).is_none(),
                "no-percent line carries chunks: {line}"
            ),
        }
    }

    fn raw(
        state: crate::engines::progress::CorpusState,
        phase: Option<crate::engines::progress::CorpusPhase>,
        done: u64,
        total: Option<u64>,
        pending: u64,
    ) -> CorpusProgress {
        CorpusProgress {
            state,
            phase,
            chunks_done: done,
            chunks_total: total,
            chunks_pending: pending,
            files_pending: 0,
            error: None,
        }
    }

    #[test]
    fn malformed_done_over_total_never_exceeds_100_or_total() {
        use crate::engines::progress::{CorpusPhase as P, CorpusState as S};
        for p in [
            raw(S::Refreshing, Some(P::Embedding), 150, Some(100), 0),
            raw(S::Ready, None, 150, Some(100), 0),
            raw(S::Ready, None, 150, Some(100), 7),
            raw(S::Refreshing, None, 9, Some(3), 0),
        ] {
            let line = format_sync_pct_line(Some(&p));
            assert!(!line.contains("150%"), "{line}");
            assert_grammar(&line);
        }
    }

    #[test]
    fn malformed_scanning_or_odd_state_never_carries_chunks() {
        use crate::engines::progress::{CorpusPhase as P, CorpusState as S};
        for p in [
            raw(S::Refreshing, Some(P::Scanning), 3, Some(9), 0),
            raw(S::Ready, Some(P::Scanning), 3, Some(9), 0),
            raw(S::Ready, None, 3, None, 0),
            raw(S::Failed, Some(P::Embedding), 3, Some(9), 0),
            raw(S::NeedsRebuild, Some(P::Scanning), 3, None, 0),
        ] {
            let line = format_sync_pct_line(Some(&p));
            assert!(!line.contains("scanning chunks"), "{line}");
            if !line.contains('%') {
                assert!(!line.contains(" chunks "), "{line}");
            }
            assert_grammar(&line);
        }
    }

    #[test]
    fn sync_line_percent_from_chunks() {
        let p = CorpusProgress::embedding(9800, 10000, 200);
        let line = format_sync_pct_line(Some(&p));
        let pct = p.percent().unwrap();
        assert!(pct <= 99, "in flight never 100");
        assert_eq!(
            line,
            format!("sync_pct: {pct}% (refreshing, embedding) chunks 9800/10000")
        );
        assert_grammar(&line);
        let ready = format_sync_pct_line(Some(&CorpusProgress::ready(5, 0)));
        assert_eq!(ready, "sync_pct: 100% (ready) chunks 5/5");
        assert_grammar(&ready);
    }

    #[test]
    fn sync_line_scanning_has_no_fake_percent() {
        let line = format_sync_pct_line(Some(&CorpusProgress::scanning(5, 3)));
        assert_eq!(line, "sync_pct: scanning (refreshing, scanning)");
        assert_grammar(&line);
    }

    #[test]
    fn sync_line_failed_has_no_number() {
        let failed = CorpusProgress::failed(
            CorpusError {
                engine: ProgressEngine::Model,
                reason: FailReason::ModelMissing,
                message: "missing".into(),
            },
            7,
        );
        let line = format_sync_pct_line(Some(&failed));
        assert_eq!(line, "sync_pct: failed (model_missing)");
        assert_grammar(&line);
    }

    #[test]
    fn sync_line_unknown_for_missing_progress_and_needs_rebuild() {
        let none = format_sync_pct_line(None);
        assert_eq!(none, "sync_pct: unknown");
        assert_grammar(&none);
        let rebuild = format_sync_pct_line(Some(&CorpusProgress::needs_rebuild(3, 10)));
        assert_eq!(rebuild, "sync_pct: unknown (needs_rebuild)");
        assert_grammar(&rebuild);
    }

    #[test]
    fn result_leads_with_sync_line() {
        let p = CorpusProgress::scanning(1, 0);
        let body = prepend_sync_pct(Some(&p), "hits");
        assert!(body.lines().next().unwrap().starts_with("sync_pct: "));
        assert_eq!(body.lines().nth(1), Some("hits"));
    }
}
