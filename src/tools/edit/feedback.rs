use crate::types::ToolCallResult;

use super::matcher::{
    ActionKey, CandidateEvidence, MatchKind, NoOpKind, RejectReason, WhitespaceKind,
    format_line_list, format_line_range,
};
use super::planner::{BlockDecision, PlannedBatch};

const RETRY_PARTIAL: &str = "File was modified. Re-read it, then retry only failed edits. Do not resubmit already applied ones.";

pub(super) fn render_tool_result(
    path: &str,
    planned: &PlannedBatch,
    wrote: bool,
    output_limit: usize,
) -> ToolCallResult {
    let mut applied = 0usize;
    let mut warnings = 0usize;
    let mut failed = 0usize;
    let mut failed_indexes = Vec::new();
    let mut warning_indexes = Vec::new();
    for decision in &planned.decisions {
        match decision {
            BlockDecision::Accept(plan) => {
                applied += 1;
                if plan.notice.is_some() {
                    warnings += 1;
                    warning_indexes.push(plan.index);
                }
            }
            BlockDecision::Reject(failure) => {
                failed += 1;
                failed_indexes.push(failure.index);
            }
        }
    }
    let body = format_batch_body(
        path,
        planned,
        wrote,
        applied,
        warnings,
        failed,
        &warning_indexes,
        &failed_indexes,
        output_limit,
    );
    if applied == 0 {
        ToolCallResult::error(body)
    } else if warnings == 0 && failed == 0 {
        ToolCallResult::ok(body)
    } else {
        let status = if failed > 0 && warnings > 0 {
            "some edits were not applied; others were auto-applied via fuzzy match"
        } else if failed > 0 {
            "some edits were not applied"
        } else {
            "auto-applied via fuzzy match; re-read to confirm"
        };
        ToolCallResult::ok(body).with_warning(status)
    }
}

fn format_batch_body(
    path: &str,
    planned: &PlannedBatch,
    wrote: bool,
    applied: usize,
    warnings: usize,
    failed: usize,
    warning_indexes: &[usize],
    failed_indexes: &[usize],
    output_limit: usize,
) -> String {
    let budget = output_limit.max(512);
    let mut summary = if applied == 0 {
        format!(
            "No edits applied in {path} (0 applied / {warnings} warning / {failed} failed). File was not modified."
        )
    } else {
        let write_state = if wrote {
            "File updated."
        } else {
            "File unchanged."
        };
        format!(
            "Edited {path} ({applied} applied / {warnings} warning / {failed} failed). {write_state}"
        )
    };
    if !failed_indexes.is_empty() {
        summary.push_str(&format!(
            "\nFailed edits: {}.",
            format_index_list(failed_indexes)
        ));
    }
    if !warning_indexes.is_empty() {
        summary.push_str(&format!(
            "\nWarning edits: {}.",
            format_index_list(warning_indexes)
        ));
    }

    let mut guidance = Vec::new();
    if wrote && failed > 0 {
        guidance.push(RETRY_PARTIAL.to_string());
    }

    let mut details = planned
        .decisions
        .iter()
        .map(|decision| render_decision(decision, wrote))
        .collect::<Vec<_>>();
    let shared_actions = collect_shared_actions(&mut details);

    let guidance_len = guidance
        .iter()
        .map(|line| line.chars().count() + 2)
        .sum::<usize>();
    let actions_len: usize = shared_actions
        .iter()
        .map(|line| line.chars().count() + 2)
        .sum();
    let budget = budget.max(summary.chars().count() + guidance_len + actions_len + 80);

    let mut out = summary;
    let mut omitted = Vec::new();
    for detail in &details {
        let extra = format!(
            "\n\n{}",
            detail.for_budget(budget, out.chars().count() + guidance_len + actions_len)
        );
        if out.chars().count() + extra.chars().count() + guidance_len + actions_len + 80 > budget {
            omitted.push(detail.index);
            continue;
        }
        out.push_str(&extra);
    }
    if !omitted.is_empty() {
        out.push_str(&format!(
            "\n\n({} edit detail(s) omitted: {}.)",
            omitted.len(),
            format_index_list(&omitted)
        ));
    }
    for line in guidance {
        out.push_str(&format!("\n\n{line}"));
    }
    for line in shared_actions {
        out.push_str(&format!("\n\n{line}"));
    }
    out
}

struct DecisionRender {
    index: usize,
    head: String,
    cause: Option<String>,
    evidence: Option<String>,
    action: Option<(ActionKey, String)>,
    share_action: bool,
}

impl DecisionRender {
    fn for_budget(&self, budget: usize, used: usize) -> String {
        let mut out = self.head.clone();
        if let Some(cause) = &self.cause {
            out.push('\n');
            out.push_str(cause);
        }
        if let Some(action) = &self.action {
            if !self.share_action {
                out.push('\n');
                out.push_str(&action.1);
            }
        }
        if let Some(evidence) = &self.evidence {
            let with_evidence = format!("{out}\n{evidence}");
            if used + with_evidence.chars().count() + 80 <= budget {
                return with_evidence;
            }
        }
        out
    }
}

fn render_decision(decision: &BlockDecision, wrote: bool) -> DecisionRender {
    match decision {
        BlockDecision::Accept(plan) => {
            let kind = match plan.match_kind() {
                MatchKind::Exact => "exact",
                MatchKind::Fuzzy => "fuzzy",
            };
            let n = plan.replacements.len();
            let range = if plan.replacements.len() == 1 {
                format_line_range(
                    plan.replacements[0].span.start_line,
                    plan.replacements[0].span.end_line,
                )
            } else {
                let lines: Vec<usize> = plan
                    .replacements
                    .iter()
                    .map(|item| item.span.start_line)
                    .collect();
                format!("lines {}", format_line_list(&lines))
            };
            if let Some(notice) = &plan.notice {
                let mut cause = format!(
                    "exact match missed; auto-applied unique fuzzy match at {}.",
                    format_line_range(notice.range.0, notice.range.1)
                );
                if notice.replace_all_ignored {
                    cause.push_str(
                        " replace_all only applies to exact matches; one fuzzy candidate was applied.",
                    );
                }
                cause.push_str(" Re-read the file if this was not the intended region.");
                let evidence = if notice.preview.is_empty() {
                    None
                } else {
                    Some(notice.preview.clone())
                };
                DecisionRender {
                    index: plan.index,
                    head: format!(
                        "[{}] applied_with_warning: {kind}, {n} replacement{} ({range})",
                        plan.index,
                        if n == 1 { "" } else { "s" }
                    ),
                    cause: Some(cause),
                    evidence,
                    action: None,
                    share_action: false,
                }
            } else {
                DecisionRender {
                    index: plan.index,
                    head: format!(
                        "[{}] applied: {kind}, {n} replacement{} ({range})",
                        plan.index,
                        if n == 1 { "" } else { "s" }
                    ),
                    cause: None,
                    evidence: None,
                    action: None,
                    share_action: false,
                }
            }
        }
        BlockDecision::Reject(failure) => {
            let (cause, evidence, action) = render_reason(&failure.reason, wrote, failure.index);
            DecisionRender {
                index: failure.index,
                head: format!("[{}] failed: {}", failure.index, failure.reason.label()),
                cause: Some(cause),
                evidence,
                action: Some(action),
                share_action: false,
            }
        }
    }
}

fn render_reason(
    reason: &RejectReason,
    wrote: bool,
    this_index: usize,
) -> (String, Option<String>, (ActionKey, String)) {
    let key = reason.action_key();
    match reason {
        RejectReason::EmptyOldString => (
            "old_string is empty.".into(),
            None,
            (key, action_text(key)),
        ),
        RejectReason::NoOp { kind } => {
            let cause = match kind {
                NoOpKind::IdenticalStrings => {
                    "old_string and new_string are identical. This is a no-op."
                }
                NoOpKind::ReplacementUnchanged => {
                    "replacement equals the matched bytes. This is a no-op."
                }
            };
            (cause.into(), None, (key, action_text(key)))
        }
        RejectReason::ReadLinePrefix { stripped_lines } => {
            let cause = if stripped_lines.len() == 1 {
                format!(
                    "old_string includes read line-number prefixes (`    N: `), which are not in the file. Without those prefixes it matches once at line {}.",
                    stripped_lines[0]
                )
            } else if stripped_lines.len() > 1 {
                format!(
                    "old_string includes read line-number prefixes (`    N: `), which are not in the file. Without those prefixes it matches {} times (lines {}).",
                    stripped_lines.len(),
                    format_line_list(stripped_lines)
                )
            } else {
                "old_string includes read line-number prefixes (`    N: `), which are not in the file.".into()
            };
            (cause, None, (key, action_text(key)))
        }
        RejectReason::WhitespaceOnly {
            start_line,
            end_line,
            kind,
            file_preview,
        } => {
            let kind_text = match kind {
                WhitespaceKind::LeadingTrailing => "leading/trailing whitespace",
                WhitespaceKind::InternalSpacing => "internal spacing (tabs vs spaces)",
            };
            let range = format_line_range(*start_line, *end_line);
            (
                format!("{range} matches old_string except for {kind_text}."),
                Some(file_preview.clone()),
                (key, action_text(key)),
            )
        }
        RejectReason::WhitespaceOnlyInput => (
            "old_string is whitespace-only and cannot uniquely match file text.".into(),
            None,
            (key, action_text(key)),
        ),
        RejectReason::MultipleExact { lines } => (
            format!(
                "Found {} exact matches (lines {}).",
                lines.len(),
                format_line_list(lines)
            ),
            None,
            (key, action_text(key)),
        ),
        RejectReason::UnicodeConfusable => (
            "old_string matched via Unicode typography normalization, but the match is ambiguous (partial or overlapping).".into(),
            None,
            (key, action_text(key)),
        ),
        RejectReason::FuzzyTooShort => (
            "exact match missed, and old_string is too short for a safe fuzzy match.".into(),
            None,
            (key, action_text(key)),
        ),
        RejectReason::FuzzySuggestedUnique { candidate } => (
            format!(
                "exact match missed. Closest unique region is {}.",
                format_line_range(candidate.start_line, candidate.end_line)
            ),
            candidate_preview(candidate),
            (key, action_text(key)),
        ),
        RejectReason::FuzzySuggestedAmbiguous { candidates } => (
            "exact match missed, and several similar regions exist.".into(),
            Some(format_ambiguous_candidates(candidates)),
            (key, action_text(key)),
        ),
        RejectReason::NoUsefulMatch => {
            let cause = if wrote {
                "No sufficiently similar region was found. This edit was not applied."
            } else {
                "No sufficiently similar region was found."
            };
            (cause.into(), None, (key, action_text(key)))
        }
        RejectReason::Chained { prior_index } => (
            format!(
                "This old_string only appears in edit {prior_index}'s new_string. Same-call edits cannot chain."
            ),
            None,
            (key, action_text(key)),
        ),
        RejectReason::Overlap {
            other_index,
            this_range,
            other_range,
            this_preview,
            other_preview,
        } => (
            format!(
                "overlaps edit {other_index} ({} vs {}). Later edits cannot claim a range already taken by an earlier successful edit.",
                format_line_range(other_range.0, other_range.1),
                format_line_range(this_range.0, this_range.1)
            ),
            Some(format!(
                "edit {other_index}: {other_preview}\nedit {this_index}: {this_preview}"
            )),
            (key, action_text(key)),
        ),
    }
}

fn candidate_preview(candidate: &CandidateEvidence) -> Option<String> {
    let preview = candidate.preview.as_deref()?.trim();
    if preview.is_empty() {
        return None;
    }
    match candidate.diff_line {
        Some(line) => Some(format!("line {line}: {preview}")),
        None => Some(preview.to_string()),
    }
}

fn format_ambiguous_candidates(candidates: &[CandidateEvidence]) -> String {
    if candidates.is_empty() {
        return "no single high-confidence window stood out.".into();
    }
    candidates
        .iter()
        .enumerate()
        .map(|(i, candidate)| {
            let range = format_line_range(candidate.start_line, candidate.end_line);
            match candidate_preview(candidate) {
                Some(preview) => format!("match {}: {range}\n{preview}", i + 1),
                None => format!("match {}: {range}", i + 1),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn action_text(key: ActionKey) -> String {
    match key {
        ActionKey::ProvideOldString => "Provide the file text to replace.".into(),
        ActionKey::SkipNoOp => "Change new_string or skip this edit.".into(),
        ActionKey::DropPrefixes => {
            "Drop the prefixes and copy only the text after the colon.".into()
        }
        ActionKey::CopyWhitespace => "Copy the file's actual whitespace.".into(),
        ActionKey::DisambiguateExact => {
            "Use replace_all to change every occurrence, or add surrounding unique lines so only one place matches.".into()
        }
        ActionKey::AnchorAscii => {
            "Use a more specific old_string anchored on nearby ASCII (do not match a single '-' or '.' inside a dash or ellipsis).".into()
        }
        ActionKey::AddUniqueContext => {
            "Re-read the file and include enough unique context.".into()
        }
        ActionKey::SplitChain => "Split this into a later edit call.".into(),
        ActionKey::ResolveOverlap => {
            "Retry only this edit after the earlier change, or target a non-overlapping range.".into()
        }
    }
}

fn collect_shared_actions(details: &mut [DecisionRender]) -> Vec<String> {
    let mut counts: Vec<(ActionKey, Vec<usize>, String)> = Vec::new();
    for detail in details.iter() {
        if let Some((key, text)) = &detail.action {
            if let Some(existing) = counts.iter_mut().find(|(k, _, _)| *k == *key) {
                existing.1.push(detail.index);
            } else {
                counts.push((*key, vec![detail.index], text.clone()));
            }
        }
    }
    let mut shared = Vec::new();
    for (key, indexes, text) in counts {
        if indexes.len() > 1 {
            for detail in details.iter_mut() {
                if detail.action.as_ref().is_some_and(|(k, _)| *k == key) {
                    detail.share_action = true;
                }
            }
            shared.push(format!("{text} (edits {}.)", format_index_list(&indexes)));
        }
    }
    shared
}

fn format_index_list(indexes: &[usize]) -> String {
    indexes
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    use crate::tools::edit::planner::{self, EditBlock};

    fn plan_of(content: &str, edits: &[(&str, &str, bool)]) -> planner::PlannedBatch {
        let blocks: Vec<EditBlock> = edits
            .iter()
            .enumerate()
            .map(|(i, (old, new, replace_all))| EditBlock {
                index: i + 1,
                old_string: (*old).into(),
                new_string: (*new).into(),
                replace_all: *replace_all,
            })
            .collect();
        planner::plan_edits(content, &blocks, &CancellationToken::new()).unwrap()
    }

    #[test]
    fn no_useful_match_omits_preview_and_score() {
        let planned = plan_of("alpha\nbeta\n", &[("zzzzzzzzzzzz", "nope", false)]);
        let body = render_tool_result("t.rs", &planned, false, 8_000).content;
        assert!(body.contains("no_useful_match"), "{body}");
        assert!(
            body.contains("No sufficiently similar region was found"),
            "{body}"
        );
        assert!(body.contains("unique context"), "{body}");
        assert!(!body.contains("score"), "{body}");
        assert!(!body.contains("zzzzzzzzzzzz"), "{body}");
        assert!(!body.contains("All edits in one call"), "{body}");
        assert!(!body.contains("Fewer edits per call"), "{body}");
    }

    #[test]
    fn snapshot_rule_stays_off_single_failure_and_multi_success() {
        let single = plan_of("fn main() {}\n", &[("missing_token_xx", "x", false)]);
        let body = render_tool_result("t.rs", &single, false, 8_000).content;
        assert!(!body.contains("All edits in one call"), "{body}");

        let multi = plan_of(
            "one\ntwo\n",
            &[("one", "ONE", false), ("two", "TWO", false)],
        );
        let body = render_tool_result("t.rs", &multi, true, 8_000).content;
        assert!(!body.contains("All edits in one call"), "{body}");
        assert!(body.contains("[1] applied:"), "{body}");
        assert!(body.contains("[2] applied:"), "{body}");
    }

    #[test]
    fn partial_success_does_not_claim_file_unmodified() {
        let planned = plan_of(
            "alpha\nbeta\n",
            &[
                ("alpha", "ALPHA", false),
                ("missing_token_zz", "nope", false),
            ],
        );
        let body = render_tool_result("t.rs", &planned, true, 8_000).content;
        assert!(body.contains("File updated."), "{body}");
        assert!(body.contains(RETRY_PARTIAL), "{body}");
        assert!(
            !body.contains("No sufficiently similar region was found. The file was not modified."),
            "{body}"
        );
        assert!(body.contains("This edit was not applied"), "{body}");
    }

    #[test]
    fn shared_actions_are_deduped_for_same_reason() {
        let planned = plan_of(
            "keep\n",
            &[
                ("missing_token_aa", "x", false),
                ("missing_token_bb", "y", false),
            ],
        );
        let body = render_tool_result("t.rs", &planned, false, 8_000).content;
        let count = body
            .matches("Re-read the file and include enough unique context")
            .count();
        assert_eq!(count, 1, "{body}");
        assert!(body.contains("edits 1, 2"), "{body}");
    }

    #[test]
    fn format_batch_omits_details_under_budget() {
        let edits: Vec<(String, String, bool)> = (0..20)
            .map(|i| (format!("missing_{i}_tokenxx"), "x".to_string(), false))
            .collect();
        let refs: Vec<(&str, &str, bool)> = edits
            .iter()
            .map(|(old, new, flag)| (old.as_str(), new.as_str(), *flag))
            .collect();
        let planned = plan_of("keep\n", &refs);
        let body = render_tool_result("t.rs", &planned, false, 400).content;
        assert!(body.contains("Failed edits:"), "{body}");
        assert!(
            body.contains("omitted") || body.chars().count() <= 600,
            "{body}"
        );
        assert!(!body.contains("Fewer edits per call"), "{body}");
        assert!(!body.contains("Do not retry the same old_string"), "{body}");
    }

    #[test]
    fn fuzzy_auto_hides_numeric_score() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let planned = plan_of(
            content,
            &[(
                old,
                "fn greet_user_beta() {\n    println!(\"hi\");\n}",
                false,
            )],
        );
        let body = render_tool_result("t.rs", &planned, true, 8_000).content;
        assert!(body.contains("applied_with_warning"), "{body}");
        assert!(!body.contains("score"), "{body}");
        assert!(
            body.contains("greet_user_alpha") || body.contains("line 1"),
            "{body}"
        );
    }

    #[test]
    fn replace_all_fuzzy_explains_single_apply() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let planned = plan_of(content, &[(old, "CHANGED", true)]);
        let body = render_tool_result("t.rs", &planned, true, 8_000).content;
        assert!(
            body.contains("replace_all only applies to exact matches"),
            "{body}"
        );
        assert!(body.contains("one fuzzy candidate was applied"), "{body}");
    }

    #[test]
    fn suggested_or_ambiguous_fuzzy_does_not_echo_old_string() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\nfn greet_user_gamma() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let planned = plan_of(content, &[(old, "CHANGED", false)]);
        let body = render_tool_result("t.rs", &planned, false, 8_000).content;
        assert!(
            body.contains("ambiguous_fuzzy") || body.contains("fuzzy_suggested"),
            "{body}"
        );
        assert!(!body.contains("this is the old_string"), "{body}");
        assert!(!body.contains("score"), "{body}");
        assert!(!body.contains(old), "{body}");
    }
}
