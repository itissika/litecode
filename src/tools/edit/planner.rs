use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::matcher::{
    self, MatchKind, PlannedReplacement, RejectReason, ResolveOutcome, SourceSpan,
};

pub(super) const SNAPSHOT_RULE: &str = "All edits in one call match the file before any of them apply. Independent edits can share a call; if a later old_string depends on an earlier new_string, submit it in a later edit call.";

#[derive(Clone, Debug)]
pub(super) struct EditBlock {
    pub index: usize,
    pub old_string: String,
    pub new_string: String,
    pub replace_all: bool,
}

#[derive(Clone, Debug)]
pub(super) struct BlockPlan {
    pub index: usize,
    pub replacements: Vec<PlannedReplacement>,
    pub notice: Option<matcher::ApplyNotice>,
}

impl BlockPlan {
    pub(super) fn match_kind(&self) -> MatchKind {
        if self
            .replacements
            .iter()
            .any(|item| item.kind == MatchKind::Fuzzy)
        {
            MatchKind::Fuzzy
        } else {
            MatchKind::Exact
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BlockFailure {
    pub index: usize,
    pub reason: RejectReason,
}

#[derive(Clone, Debug)]
pub(super) enum BlockDecision {
    Accept(BlockPlan),
    Reject(BlockFailure),
}

pub(super) struct PlannedBatch {
    pub decisions: Vec<BlockDecision>,
    pub edited: Option<String>,
}

impl PlannedBatch {
    pub(super) fn edited_content(&self) -> Option<&str> {
        self.edited.as_deref()
    }
}

#[derive(Debug)]
pub(super) enum RequestFail {
    Cancelled,
    EmptyFile,
    InvalidPlan(String),
}

pub(super) fn cancelled_message() -> &'static str {
    "edit cancelled before write; file was not modified"
}

pub(super) fn empty_file_message() -> &'static str {
    "The file is empty. old_string cannot match an empty file."
}

pub(super) fn parse_edits(input: &Value) -> Result<Vec<EditBlock>, String> {
    let Some(edits) = input.get("edits") else {
        return Err(crate::tool::missing_parameter("edits"));
    };
    let Some(items) = edits.as_array() else {
        return Err(crate::tool::expected_type("edits", "array", edits));
    };
    if items.is_empty() {
        return Err(crate::tool::must_be(
            "edits",
            "an array with at least 1 item(s)",
        ));
    }
    let mut blocks = Vec::with_capacity(items.len());
    for (i, item_value) in items.iter().enumerate() {
        let Some(item) = item_value.as_object() else {
            return Err(crate::tool::expected_type(
                &format!("edits[{i}]"),
                "object",
                item_value,
            ));
        };
        if let Some(key) = item
            .keys()
            .filter(|key| !matches!(key.as_str(), "old_string" | "new_string" | "replace_all"))
            .min()
        {
            return Err(crate::tool::schema_validate::unknown_parameter(&format!(
                "edits[{i}].{key}"
            )));
        }
        let old = required_string_field(item_value, i, "old_string")?;
        let new = required_string_field(item_value, i, "new_string")?;
        let replace_all = match item_value.get("replace_all") {
            None => false,
            Some(value) => value.as_bool().ok_or_else(|| {
                crate::tool::expected_type(&format!("edits[{i}].replace_all"), "boolean", value)
            })?,
        };
        blocks.push(EditBlock {
            index: i + 1,
            old_string: old,
            new_string: new,
            replace_all,
        });
    }
    Ok(blocks)
}

fn required_string_field(item: &Value, index: usize, field: &str) -> Result<String, String> {
    let path = format!("edits[{index}].{field}");
    match item.get(field) {
        None => Err(crate::tool::missing_parameter(&path)),
        Some(value) => crate::tool::require_string_value(value, &path).map(str::to_string),
    }
}

pub(super) fn plan_edits(
    content: &str,
    blocks: &[EditBlock],
    cancel: &CancellationToken,
) -> Result<PlannedBatch, RequestFail> {
    if content.is_empty() {
        return Err(RequestFail::EmptyFile);
    }
    let mut resolved = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err(RequestFail::Cancelled);
        }
        resolved.push(resolve_with_relations(
            content,
            block,
            &blocks[..i],
            cancel,
        )?);
    }
    if cancel.is_cancelled() {
        return Err(RequestFail::Cancelled);
    }
    let decisions = claim_ranges(resolved);
    let edited = materialize(content, &decisions)?;
    Ok(PlannedBatch { decisions, edited })
}

fn resolve_with_relations(
    content: &str,
    block: &EditBlock,
    prior: &[EditBlock],
    cancel: &CancellationToken,
) -> Result<BlockDecision, RequestFail> {
    let allow_fuzzy = chained_from_prior(prior, &block.old_string).is_none();
    let outcome = matcher::resolve_block(content, block, cancel, allow_fuzzy)?;
    Ok(match outcome {
        ResolveOutcome::Apply(plan) => BlockDecision::Accept(BlockPlan {
            index: block.index,
            replacements: plan.replacements,
            notice: plan.notice,
        }),
        ResolveOutcome::Reject(reason) => {
            let reason = overlay_chain(prior, &block.old_string, reason);
            BlockDecision::Reject(BlockFailure {
                index: block.index,
                reason,
            })
        }
    })
}

fn overlay_chain(prior: &[EditBlock], old: &str, reason: RejectReason) -> RejectReason {
    if matches!(
        reason,
        RejectReason::EmptyOldString
            | RejectReason::NoOp { .. }
            | RejectReason::ReadLinePrefix { .. }
            | RejectReason::WhitespaceOnly { .. }
            | RejectReason::WhitespaceOnlyInput
            | RejectReason::MultipleExact { .. }
            | RejectReason::UnicodeConfusable
    ) {
        return reason;
    }
    if let Some(prior_index) = chained_from_prior(prior, old) {
        return RejectReason::Chained { prior_index };
    }
    reason
}

fn chained_from_prior(prior: &[EditBlock], old: &str) -> Option<usize> {
    let old_lf = old.replace("\r\n", "\n");
    prior.iter().find_map(|block| {
        let new_lf = block.new_string.replace("\r\n", "\n");
        (block.new_string.contains(old) || new_lf.contains(&old_lf)).then_some(block.index)
    })
}

fn claim_ranges(resolved: Vec<BlockDecision>) -> Vec<BlockDecision> {
    let mut claimed: Vec<(usize, SourceSpan)> = Vec::new();
    let mut out = Vec::with_capacity(resolved.len());
    for decision in resolved {
        match decision {
            BlockDecision::Reject(failure) => out.push(BlockDecision::Reject(failure)),
            BlockDecision::Accept(plan) => {
                if let Some((other_index, other_span, this_span)) =
                    first_overlap(&claimed, &plan.replacements)
                {
                    out.push(BlockDecision::Reject(BlockFailure {
                        index: plan.index,
                        reason: RejectReason::Overlap {
                            other_index,
                            this_range: (this_span.start_line, this_span.end_line),
                            other_range: (other_span.start_line, other_span.end_line),
                            this_preview: this_span.preview,
                            other_preview: other_span.preview,
                        },
                    }));
                } else {
                    for item in &plan.replacements {
                        claimed.push((plan.index, item.span.clone()));
                    }
                    out.push(BlockDecision::Accept(plan));
                }
            }
        }
    }
    out
}

fn first_overlap(
    claimed: &[(usize, SourceSpan)],
    replacements: &[PlannedReplacement],
) -> Option<(usize, SourceSpan, SourceSpan)> {
    for item in replacements {
        for (index, span) in claimed {
            if item.span.overlaps(span) {
                return Some((*index, span.clone(), item.span.clone()));
            }
        }
    }
    None
}

fn materialize(content: &str, decisions: &[BlockDecision]) -> Result<Option<String>, RequestFail> {
    let mut replacements: Vec<(crate::workspace::text_codec::ByteSpan, String)> = Vec::new();
    for decision in decisions {
        if let BlockDecision::Accept(plan) = decision {
            for item in &plan.replacements {
                replacements.push((item.span.as_byte(), item.replacement.clone()));
            }
        }
    }
    if replacements.is_empty() {
        return Ok(None);
    }
    replacements.sort_by_key(|(span, _)| span.start);
    crate::workspace::text_codec::validate_byte_spans(
        content,
        &replacements
            .iter()
            .map(|(span, _)| *span)
            .collect::<Vec<_>>(),
    )
    .map_err(RequestFail::InvalidPlan)?;
    let edited = crate::workspace::text_codec::apply_byte_spans(content, &replacements)
        .map_err(RequestFail::InvalidPlan)?;
    if edited == content {
        Ok(None)
    } else {
        Ok(Some(edited))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn plan_of(content: &str, edits: &[(&str, &str, bool)]) -> PlannedBatch {
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
        plan_edits(content, &blocks, &CancellationToken::new()).unwrap()
    }

    fn accepted<'a>(batch: &'a PlannedBatch, index: usize) -> &'a BlockPlan {
        match &batch.decisions[index] {
            BlockDecision::Accept(plan) => plan,
            BlockDecision::Reject(failure) => panic!("expected accept, got {failure:?}"),
        }
    }

    fn rejected<'a>(batch: &'a PlannedBatch, index: usize) -> &'a BlockFailure {
        match &batch.decisions[index] {
            BlockDecision::Reject(failure) => failure,
            BlockDecision::Accept(plan) => panic!("expected reject, got {plan:?}"),
        }
    }

    #[test]
    fn exact_replace_matches_lf_old_in_crlf_file() {
        let content = "a\r\nb\r\n";
        let (result, count) =
            crate::workspace::text_codec::eol_preserving_replace(content, "a\nb", "X", false);
        assert_eq!(count, 1);
        assert_eq!(result, "X\r\n");
    }

    #[test]
    fn mixed_eol_exact_replace_keeps_unmatched() {
        let content = "a\r\nb\nc\n";
        let (result, count) =
            crate::workspace::text_codec::eol_preserving_replace(content, "b", "B", false);
        assert_eq!(count, 1);
        assert_eq!(result, "a\r\nB\nc\n");
    }

    #[test]
    fn plan_single_exact_and_replace_all() {
        let one = plan_of(
            "fn start() {}\n",
            &[("fn start() {}", "fn main() {}", false)],
        );
        assert_eq!(accepted(&one, 0).replacements.len(), 1);
        assert_eq!(one.edited.as_deref(), Some("fn main() {}\n"));

        let many = plan_of("foo\nbar\nfoo\n", &[("foo", "FOO", true)]);
        assert_eq!(accepted(&many, 0).replacements.len(), 2);
        assert_eq!(many.edited.as_deref(), Some("FOO\nbar\nFOO\n"));

        let multi = plan_of("foo\nbar\nfoo\n", &[("foo", "FOO", false)]);
        assert!(matches!(
            rejected(&multi, 0).reason,
            RejectReason::MultipleExact { .. }
        ));
        assert!(multi.edited.is_none());
    }

    #[test]
    fn plan_noop_and_empty_old() {
        let noop = plan_of("fn main() {}\n", &[("fn main() {}", "fn main() {}", false)]);
        assert!(matches!(
            rejected(&noop, 0).reason,
            RejectReason::NoOp { .. }
        ));
        let empty = plan_of("fn main() {}\n", &[("", "x", false)]);
        assert!(matches!(
            rejected(&empty, 0).reason,
            RejectReason::EmptyOldString
        ));
    }

    #[test]
    fn plan_empty_file_is_request_level() {
        let blocks = [EditBlock {
            index: 1,
            old_string: "hello".into(),
            new_string: "world".into(),
            replace_all: false,
        }];
        assert!(matches!(
            plan_edits("", &blocks, &CancellationToken::new()),
            Err(RequestFail::EmptyFile)
        ));
    }

    #[test]
    fn plan_exact_blocks_fuzzy() {
        let batch = plan_of(
            "fn unique_name() { alpha }\nfn unique_name() { beta }\n",
            &[(
                "fn unique_name() { alpha }",
                "fn unique_name() { gamma }",
                false,
            )],
        );
        assert_eq!(accepted(&batch, 0).match_kind(), MatchKind::Exact);
        assert!(accepted(&batch, 0).notice.is_none());
    }

    #[test]
    fn plan_fuzzy_too_short_does_not_apply() {
        let batch = plan_of("hello world\n", &[("zz", "yy", false)]);
        assert!(matches!(
            rejected(&batch, 0).reason,
            RejectReason::FuzzyTooShort
        ));
        assert!(batch.edited.is_none());
    }

    #[test]
    fn plan_unique_fuzzy_applies_with_warning_same_window() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let batch = plan_of(
            content,
            &[(
                old,
                "fn greet_user_beta() {\n    println!(\"hi\");\n}",
                false,
            )],
        );
        let plan = accepted(&batch, 0);
        assert_eq!(plan.match_kind(), MatchKind::Fuzzy);
        assert!(plan.notice.is_some());
        assert_eq!(plan.replacements[0].span.start_line, 1);
        assert_eq!(
            plan.replacements[0].span.end_line,
            plan.replacements[0].span.start_line + old.lines().count().saturating_sub(1)
        );
        assert!(batch.edited.unwrap().contains("greet_user_beta"));
    }

    #[test]
    fn plan_long_unique_fuzzy_is_not_shadowed_by_expanded_windows() {
        let content = (0..40)
            .map(|i| format!("unique_line_{i:02}();"))
            .collect::<Vec<_>>()
            .join("\n");
        let old = content.replace("unique_line_20", "unique_lime_20");
        let batch = plan_of(&content, &[(&old, "REPLACED", false)]);
        assert_eq!(accepted(&batch, 0).match_kind(), MatchKind::Fuzzy);
        assert_eq!(batch.edited.as_deref(), Some("REPLACED"));
    }

    #[test]
    fn plan_replace_all_unique_fuzzy_applies_once() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let batch = plan_of(content, &[(old, "CHANGED", true)]);
        let plan = accepted(&batch, 0);
        assert_eq!(plan.replacements.len(), 1);
        assert_eq!(plan.match_kind(), MatchKind::Fuzzy);
        assert!(plan.notice.as_ref().is_some_and(|n| n.replace_all_ignored));
    }

    #[test]
    fn plan_replace_all_exact_three_hits() {
        let batch = plan_of("foo\nbar\nfoo\nbaz\nfoo\n", &[("foo", "FOO", true)]);
        assert_eq!(accepted(&batch, 0).replacements.len(), 3);
        assert_eq!(batch.edited.as_deref(), Some("FOO\nbar\nFOO\nbaz\nFOO\n"));
    }

    #[test]
    fn plan_partial_success_overlap_and_prior_failure_does_not_claim() {
        let content = "alpha\nbeta\ngamma\n";
        let batch = plan_of(
            content,
            &[
                ("zz", "x", false),
                ("alpha\nbeta", "AB", false),
                ("beta\ngamma", "BG", false),
            ],
        );
        assert!(matches!(
            rejected(&batch, 0).reason,
            RejectReason::FuzzyTooShort
        ));
        assert!(matches!(&batch.decisions[1], BlockDecision::Accept(_)));
        assert!(matches!(
            rejected(&batch, 2).reason,
            RejectReason::Overlap { .. }
        ));
        assert_eq!(batch.edited.as_deref(), Some("AB\ngamma\n"));
    }

    #[test]
    fn plan_adjacent_ranges_allowed() {
        let batch = plan_of("abcde", &[("ab", "AB", false), ("cd", "CD", false)]);
        assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
        assert!(matches!(&batch.decisions[1], BlockDecision::Accept(_)));
        assert_eq!(batch.edited.as_deref(), Some("ABCDe"));
    }

    #[test]
    fn plan_replace_all_conflict_fails_whole_block() {
        let content = "foo\nbar\nfoo\n";
        let batch = plan_of(content, &[("foo\nbar", "X", false), ("foo", "FOO", true)]);
        assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
        assert!(matches!(
            rejected(&batch, 1).reason,
            RejectReason::Overlap { .. }
        ));
        assert_eq!(batch.edited.as_deref(), Some("X\nfoo\n"));
    }

    #[test]
    fn plan_chained_edit_is_called_out() {
        let batch = plan_of(
            "alpha\n",
            &[("alpha", "beta", false), ("beta", "gamma", false)],
        );
        assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
        assert!(matches!(
            rejected(&batch, 1).reason,
            RejectReason::Chained { prior_index: 1 }
        ));
    }

    #[test]
    fn plan_fuzzy_overlap_does_not_apply_second_candidate() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\nfn greet_user_gamma() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let batch = plan_of(
            content,
            &[
                (
                    "fn greet_user_alpha() {\n    println!(\"hi\");\n}",
                    "CHANGED",
                    false,
                ),
                (old, "OTHER", false),
            ],
        );
        assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
        match &batch.decisions[1] {
            BlockDecision::Reject(failure) => {
                assert!(
                    matches!(
                        failure.reason,
                        RejectReason::Overlap { .. } | RejectReason::FuzzySuggestedAmbiguous { .. }
                    ),
                    "{failure:?}"
                );
            }
            BlockDecision::Accept(plan) => panic!("must not apply runner-up, got {plan:?}"),
        }
        let edited = batch.edited.as_deref().unwrap();
        assert!(edited.contains("CHANGED"));
        assert!(!edited.contains("OTHER"));
    }

    #[test]
    fn plan_unicode_confusable_does_not_fuzzy() {
        let batch = plan_of("foo\u{2014}bar\n", &[("-", "=", false)]);
        assert!(matches!(
            rejected(&batch, 0).reason,
            RejectReason::UnicodeConfusable
        ));
        assert!(batch.edited.is_none());
    }

    #[test]
    fn plan_reverse_splice_keeps_offsets() {
        let batch = plan_of(
            "one\ntwo\nthree\n",
            &[("one", "ONE", false), ("three", "THREE", false)],
        );
        assert_eq!(batch.edited.as_deref(), Some("ONE\ntwo\nTHREE\n"));
    }

    #[test]
    fn fuzzy_window_matches_replacement_range() {
        let content = "keep\nfn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let old = "fn greet_user_alpa() {\n    println!(\"hi\");\n}";
        let batch = plan_of(content, &[(old, "X", false)]);
        let plan = accepted(&batch, 0);
        let span = &plan.replacements[0].span;
        assert_eq!(span.end_line - span.start_line + 1, old.lines().count());
    }

    #[test]
    fn planner_never_auto_applies_known_prefix_or_indent_mistakes() {
        let line = "fn exceptionally_long_unique_function_name_for_edit_safety() {}";
        let prefixed = format!("     1: {line}");
        let prefix = plan_of(&format!("{line}\n"), &[(&prefixed, "changed", false)]);
        assert!(matches!(
            rejected(&prefix, 0).reason,
            RejectReason::ReadLinePrefix { .. }
        ));
        assert!(prefix.edited.is_none());

        let content = format!("{line}\n");
        let indented_old = format!("    {line}");
        let indent = plan_of(&content, &[(&indented_old, "changed", false)]);
        assert!(matches!(
            rejected(&indent, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ));
        assert!(indent.edited.is_none());
    }

    #[test]
    fn whitespace_only_old_never_auto_applies() {
        let batch = plan_of("keep\n", &[("   \n\t", "x", false)]);
        assert!(matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnlyInput
        ));
        assert!(batch.edited.is_none());
    }

    #[test]
    fn chain_blocks_fuzzy_despite_high_score() {
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let batch = plan_of(
            content,
            &[
                (
                    "fn greet_user_alpha() {\n    println!(\"hi\");\n}",
                    "fn greet_user_alpa() {\n    println!(\"hi\");\n}",
                    false,
                ),
                (
                    "fn greet_user_alpa() {\n    println!(\"hi\");\n}",
                    "CHANGED",
                    false,
                ),
            ],
        );
        assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
        assert!(matches!(
            rejected(&batch, 1).reason,
            RejectReason::Chained { prior_index: 1 }
        ));
        assert!(batch.edited.unwrap().contains("greet_user_alpa"));
    }

    #[test]
    fn prefix_blocks_fuzzy_despite_high_score() {
        let line = "fn greet_user_alpha() { println!(\"hi\"); }";
        let batch = plan_of(
            &format!("{line}\n"),
            &[(&format!("     1: {line}"), "CHANGED", false)],
        );
        assert!(matches!(
            rejected(&batch, 0).reason,
            RejectReason::ReadLinePrefix { .. }
        ));
        assert!(batch.edited.is_none());
    }
}
