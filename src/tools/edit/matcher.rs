use similar::TextDiff;
use tokio_util::sync::CancellationToken;

use crate::workspace::text_codec::{self, ByteSpan, ExactSpans};

use super::planner::{EditBlock, RequestFail};

pub(super) const SUGGEST_FUZZY_MIN_SCORE: u8 = 80;
pub(super) const AUTO_FUZZY_MIN_SCORE: u8 = 90;
pub(super) const AUTO_FUZZY_MIN_MARGIN: u8 = 5;
const MIN_AUTO_FUZZY_NON_WHITESPACE: usize = 8;
const MAX_FUZZY_PREVIEWS: usize = 3;
pub(super) const PREVIEW_CHARS: usize = 160;
const CANCEL_CHECK_INTERVAL: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MatchKind {
    Exact,
    Fuzzy,
}

#[derive(Clone, Debug)]
pub(super) struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub preview: String,
}

impl SourceSpan {
    pub(super) fn from_byte(content: &str, span: ByteSpan) -> Self {
        let (start_line, end_line) = text_codec::byte_span_lines(content, span);
        let slice = &content[span.start..span.end];
        let preview = truncate_hint(slice.lines().next().unwrap_or(slice), PREVIEW_CHARS);
        Self {
            start: span.start,
            end: span.end,
            start_line,
            end_line,
            preview,
        }
    }

    pub(super) fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub(super) fn as_byte(&self) -> ByteSpan {
        ByteSpan {
            start: self.start,
            end: self.end,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PlannedReplacement {
    pub span: SourceSpan,
    pub replacement: String,
    pub kind: MatchKind,
}

#[derive(Clone, Debug)]
pub(super) struct ApplyNotice {
    pub range: (usize, usize),
    pub preview: String,
    pub replace_all_ignored: bool,
}

#[derive(Clone, Debug)]
pub(super) struct ApplyPlan {
    pub replacements: Vec<PlannedReplacement>,
    pub notice: Option<ApplyNotice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NoOpKind {
    IdenticalStrings,
    ReplacementUnchanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WhitespaceKind {
    LeadingTrailing,
    InternalSpacing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CandidateEvidence {
    pub start_line: usize,
    pub end_line: usize,
    pub diff_line: Option<usize>,
    pub preview: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FuzzyBand {
    AutoApplied,
    SuggestedUnique,
    SuggestedAmbiguous,
    NoUsefulMatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RejectReason {
    EmptyOldString,
    NoOp {
        kind: NoOpKind,
    },
    ReadLinePrefix {
        stripped_lines: Vec<usize>,
    },
    WhitespaceOnly {
        start_line: usize,
        end_line: usize,
        kind: WhitespaceKind,
        file_preview: String,
    },
    WhitespaceOnlyInput,
    MultipleExact {
        lines: Vec<usize>,
    },
    UnicodeConfusable,
    FuzzyTooShort,
    FuzzySuggestedUnique {
        candidate: CandidateEvidence,
    },
    FuzzySuggestedAmbiguous {
        candidates: Vec<CandidateEvidence>,
    },
    NoUsefulMatch,
    Chained {
        prior_index: usize,
    },
    Overlap {
        other_index: usize,
        this_range: (usize, usize),
        other_range: (usize, usize),
        this_preview: String,
        other_preview: String,
    },
}

impl RejectReason {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::EmptyOldString => "empty_old_string",
            Self::NoOp { .. } => "no_op",
            Self::ReadLinePrefix { .. } => "read_line_prefix",
            Self::WhitespaceOnly { .. } | Self::WhitespaceOnlyInput => "whitespace_only",
            Self::MultipleExact { .. } => "multiple_exact",
            Self::UnicodeConfusable => "unicode_confusable",
            Self::FuzzyTooShort => "fuzzy_input_too_short",
            Self::FuzzySuggestedUnique { .. } => "fuzzy_suggested",
            Self::FuzzySuggestedAmbiguous { .. } => "ambiguous_fuzzy",
            Self::NoUsefulMatch => "no_useful_match",
            Self::Chained { .. } => "chained",
            Self::Overlap { .. } => "overlap",
        }
    }

    pub(super) fn action_key(&self) -> ActionKey {
        match self {
            Self::EmptyOldString => ActionKey::ProvideOldString,
            Self::NoOp { .. } => ActionKey::SkipNoOp,
            Self::ReadLinePrefix { .. } => ActionKey::DropPrefixes,
            Self::WhitespaceOnly { .. } | Self::WhitespaceOnlyInput => ActionKey::CopyWhitespace,
            Self::MultipleExact { .. } => ActionKey::DisambiguateExact,
            Self::UnicodeConfusable => ActionKey::AnchorAscii,
            Self::FuzzyTooShort
            | Self::FuzzySuggestedUnique { .. }
            | Self::FuzzySuggestedAmbiguous { .. }
            | Self::NoUsefulMatch => ActionKey::AddUniqueContext,
            Self::Chained { .. } => ActionKey::SplitChain,
            Self::Overlap { .. } => ActionKey::ResolveOverlap,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ActionKey {
    ProvideOldString,
    SkipNoOp,
    DropPrefixes,
    CopyWhitespace,
    DisambiguateExact,
    AnchorAscii,
    AddUniqueContext,
    SplitChain,
    ResolveOverlap,
}

#[derive(Debug)]
pub(super) enum ResolveOutcome {
    Apply(ApplyPlan),
    Reject(RejectReason),
}

pub(super) fn resolve_block(
    content: &str,
    block: &EditBlock,
    cancel: &CancellationToken,
    allow_fuzzy: bool,
) -> Result<ResolveOutcome, RequestFail> {
    if block.old_string.is_empty() {
        return Ok(ResolveOutcome::Reject(RejectReason::EmptyOldString));
    }
    if block.old_string == block.new_string {
        return Ok(ResolveOutcome::Reject(RejectReason::NoOp {
            kind: NoOpKind::IdenticalStrings,
        }));
    }
    Ok(
        match text_codec::find_exact_spans(content, &block.old_string) {
            ExactSpans::Ambiguous => ResolveOutcome::Reject(RejectReason::UnicodeConfusable),
            ExactSpans::Hits(spans) => resolve_exact(content, block, &spans),
            ExactSpans::NotFound => resolve_after_exact_miss(content, block, cancel, allow_fuzzy)?,
        },
    )
}

fn resolve_exact(content: &str, block: &EditBlock, spans: &[ByteSpan]) -> ResolveOutcome {
    if spans.is_empty() {
        return ResolveOutcome::Reject(RejectReason::NoUsefulMatch);
    }
    if !block.replace_all && spans.len() > 1 {
        let lines: Vec<usize> = spans
            .iter()
            .map(|span| text_codec::byte_span_lines(content, *span).0)
            .collect();
        return ResolveOutcome::Reject(RejectReason::MultipleExact { lines });
    }
    let apply = if block.replace_all {
        spans.to_vec()
    } else {
        vec![spans[0]]
    };
    match replacements_from_spans(content, &apply, &block.new_string, MatchKind::Exact) {
        Ok(replacements) => ResolveOutcome::Apply(ApplyPlan {
            replacements,
            notice: None,
        }),
        Err(reason) => ResolveOutcome::Reject(reason),
    }
}

fn resolve_after_exact_miss(
    content: &str,
    block: &EditBlock,
    cancel: &CancellationToken,
    allow_fuzzy: bool,
) -> Result<ResolveOutcome, RequestFail> {
    if let Some(reason) = diagnose_structural_miss(content, &block.old_string) {
        return Ok(ResolveOutcome::Reject(reason));
    }
    if non_ws_len(&block.old_string) == 0 {
        return Ok(ResolveOutcome::Reject(RejectReason::WhitespaceOnlyInput));
    }
    if non_ws_len(&block.old_string) < MIN_AUTO_FUZZY_NON_WHITESPACE {
        return Ok(ResolveOutcome::Reject(RejectReason::FuzzyTooShort));
    }
    if !allow_fuzzy {
        return Ok(ResolveOutcome::Reject(RejectReason::NoUsefulMatch));
    }
    resolve_fuzzy(content, block, cancel)
}

fn replacements_from_spans(
    content: &str,
    spans: &[ByteSpan],
    new: &str,
    kind: MatchKind,
) -> Result<Vec<PlannedReplacement>, RejectReason> {
    let mut out = Vec::new();
    for span in spans {
        let replacement = text_codec::render_eol_replacement(content, span.start, new);
        if replacement == content[span.start..span.end] {
            continue;
        }
        out.push(PlannedReplacement {
            span: SourceSpan::from_byte(content, *span),
            replacement,
            kind,
        });
    }
    if out.is_empty() {
        return Err(RejectReason::NoOp {
            kind: NoOpKind::ReplacementUnchanged,
        });
    }
    Ok(out)
}

fn diagnose_structural_miss(content: &str, old_string: &str) -> Option<RejectReason> {
    if looks_like_read_line_prefixes(old_string) {
        let stripped = strip_read_line_prefixes(old_string);
        let stripped_lines = text_codec::edit_match_line_numbers(content, &stripped);
        return Some(RejectReason::ReadLinePrefix { stripped_lines });
    }
    if let Some((start_line, end_line, kind, preview)) = unique_relaxed_match(content, old_string) {
        return Some(RejectReason::WhitespaceOnly {
            start_line,
            end_line,
            kind,
            file_preview: preview,
        });
    }
    None
}

fn resolve_fuzzy(
    content: &str,
    block: &EditBlock,
    cancel: &CancellationToken,
) -> Result<ResolveOutcome, RequestFail> {
    let search_lines: Vec<&str> = block.old_string.lines().collect();
    if search_lines.is_empty() {
        return Ok(ResolveOutcome::Reject(RejectReason::NoUsefulMatch));
    }
    let (bodies, _eols) = text_codec::split_keep_eol(content);
    if bodies.len() < search_lines.len() {
        return Ok(ResolveOutcome::Reject(RejectReason::NoUsefulMatch));
    }
    let mut candidates = enumerate_fuzzy_candidates(&bodies, &search_lines, cancel)?;
    if candidates.is_empty() {
        return Ok(ResolveOutcome::Reject(RejectReason::NoUsefulMatch));
    }
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.start.cmp(&b.start)));
    let best = candidates[0];
    let second = candidates.get(1).map(|item| item.score);
    match classify_fuzzy(best.score, second) {
        FuzzyBand::NoUsefulMatch => Ok(ResolveOutcome::Reject(RejectReason::NoUsefulMatch)),
        FuzzyBand::SuggestedUnique => {
            Ok(ResolveOutcome::Reject(RejectReason::FuzzySuggestedUnique {
                candidate: candidate_evidence(content, &search_lines, &best),
            }))
        }
        FuzzyBand::SuggestedAmbiguous => {
            let shown: Vec<CandidateEvidence> = candidates
                .iter()
                .filter(|item| item.score >= SUGGEST_FUZZY_MIN_SCORE)
                .take(MAX_FUZZY_PREVIEWS)
                .map(|item| candidate_evidence(content, &search_lines, item))
                .collect();
            Ok(ResolveOutcome::Reject(
                RejectReason::FuzzySuggestedAmbiguous { candidates: shown },
            ))
        }
        FuzzyBand::AutoApplied => {
            let span = line_window_span(content, best.start, best.end);
            let replacement =
                render_line_window_replacement(content, best.start, best.end, &block.new_string);
            if replacement == content[span.start..span.end] {
                return Ok(ResolveOutcome::Reject(RejectReason::NoOp {
                    kind: NoOpKind::ReplacementUnchanged,
                }));
            }
            let evidence = candidate_evidence(content, &search_lines, &best);
            Ok(ResolveOutcome::Apply(ApplyPlan {
                replacements: vec![PlannedReplacement {
                    span,
                    replacement,
                    kind: MatchKind::Fuzzy,
                }],
                notice: Some(ApplyNotice {
                    range: (evidence.start_line, evidence.end_line),
                    preview: evidence.preview.unwrap_or_default(),
                    replace_all_ignored: block.replace_all,
                }),
            }))
        }
    }
}

pub(super) fn classify_fuzzy(best: u8, second: Option<u8>) -> FuzzyBand {
    if best < SUGGEST_FUZZY_MIN_SCORE {
        return FuzzyBand::NoUsefulMatch;
    }
    let unique = second
        .map(|other| best > other && best.saturating_sub(other) >= AUTO_FUZZY_MIN_MARGIN)
        .unwrap_or(true);
    if best >= AUTO_FUZZY_MIN_SCORE && unique {
        FuzzyBand::AutoApplied
    } else if unique {
        FuzzyBand::SuggestedUnique
    } else {
        FuzzyBand::SuggestedAmbiguous
    }
}

#[derive(Clone, Copy, Debug)]
struct FuzzyCandidate {
    start: usize,
    end: usize,
    score: u8,
}

fn enumerate_fuzzy_candidates(
    content_lines: &[&str],
    search_lines: &[&str],
    cancel: &CancellationToken,
) -> Result<Vec<FuzzyCandidate>, RequestFail> {
    let search_len = search_lines.len();
    if search_len == 0 || content_lines.len() < search_len {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for start in 0..=(content_lines.len() - search_len) {
        if start % CANCEL_CHECK_INTERVAL == 0 && cancel.is_cancelled() {
            return Err(RequestFail::Cancelled);
        }
        let end = start + search_len;
        let score = compute_line_similarity(&content_lines[start..end], search_lines);
        out.push(FuzzyCandidate { start, end, score });
    }
    Ok(out)
}

fn candidate_evidence(
    content: &str,
    search_lines: &[&str],
    candidate: &FuzzyCandidate,
) -> CandidateEvidence {
    let span = line_window_span(content, candidate.start, candidate.end);
    let (bodies, _eols) = text_codec::split_keep_eol(content);
    let window = &bodies[candidate.start..candidate.end];
    let (diff_line, preview) = first_diff_preview(search_lines, window, span.start_line);
    CandidateEvidence {
        start_line: span.start_line,
        end_line: span.end_line,
        diff_line,
        preview,
    }
}

fn first_diff_preview(
    search_lines: &[&str],
    window: &[&str],
    start_line: usize,
) -> (Option<usize>, Option<String>) {
    for (i, (old, file)) in search_lines.iter().zip(window.iter()).enumerate() {
        if old != file {
            return (
                Some(start_line + i),
                Some(truncate_hint(file, PREVIEW_CHARS)),
            );
        }
    }
    (None, None)
}

fn line_window_span(content: &str, start: usize, end: usize) -> SourceSpan {
    let (bodies, eols) = text_codec::split_keep_eol(content);
    let mut byte = 0usize;
    for i in 0..start {
        byte +=
            bodies.get(i).map(|s| s.len()).unwrap_or(0) + eols.get(i).map(|s| s.len()).unwrap_or(0);
    }
    let span_start = byte;
    for i in start..end {
        byte +=
            bodies.get(i).map(|s| s.len()).unwrap_or(0) + eols.get(i).map(|s| s.len()).unwrap_or(0);
    }
    SourceSpan::from_byte(
        content,
        ByteSpan {
            start: span_start,
            end: byte.min(content.len()),
        },
    )
}

fn render_line_window_replacement(content: &str, start: usize, end: usize, new: &str) -> String {
    let (_bodies, eols) = text_codec::split_keep_eol(content);
    let new_lines: Vec<&str> = new.lines().collect();
    let region_eol = eols
        .get(start)
        .copied()
        .filter(|eol| !eol.is_empty())
        .or_else(|| eols.iter().copied().find(|eol| !eol.is_empty()))
        .unwrap_or("\n");
    let last_replaced_eol = eols.get(end.saturating_sub(1)).copied().unwrap_or("");
    let mut out = String::new();
    for (j, line) in new_lines.iter().enumerate() {
        out.push_str(line);
        if j + 1 < new_lines.len() {
            out.push_str(region_eol);
        } else {
            out.push_str(last_replaced_eol);
        }
    }
    out
}

fn looks_like_read_line_prefixes(old: &str) -> bool {
    let lines: Vec<&str> = old.lines().collect();
    if lines.is_empty() {
        return false;
    }
    let prefixed = lines
        .iter()
        .filter(|line| strip_one_read_prefix(line).is_some())
        .count();
    prefixed * 2 >= lines.len()
}

fn strip_read_line_prefixes(old: &str) -> String {
    old.lines()
        .map(|line| strip_one_read_prefix(line).unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_one_read_prefix(line: &str) -> Option<&str> {
    let rest = line.trim_start_matches(' ');
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after = &rest[digits..];
    if let Some(stripped) = after.strip_prefix(": ") {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix(':') {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix("| ") {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix('|') {
        return Some(stripped);
    }
    if let Some(stripped) = after.strip_prefix('→') {
        return Some(stripped.strip_prefix(' ').unwrap_or(stripped));
    }
    None
}

fn unique_relaxed_match(
    content: &str,
    old: &str,
) -> Option<(usize, usize, WhitespaceKind, String)> {
    let search: Vec<&str> = old.lines().collect();
    if search.is_empty() || search.iter().all(|line| line.trim().is_empty()) {
        return None;
    }
    let (bodies, _eols) = text_codec::split_keep_eol(content);
    if bodies.len() < search.len() {
        return None;
    }
    let mut trim_hits = Vec::new();
    let mut space_hits = Vec::new();
    for start in 0..=(bodies.len() - search.len()) {
        let window = &bodies[start..start + search.len()];
        if window.iter().zip(search.iter()).all(|(a, b)| *a == *b) {
            continue;
        }
        if window
            .iter()
            .zip(search.iter())
            .all(|(a, b)| a.trim() == b.trim())
        {
            trim_hits.push(start);
        } else if window
            .iter()
            .zip(search.iter())
            .all(|(a, b)| collapse_ws(a) == collapse_ws(b))
        {
            space_hits.push(start);
        }
    }
    let (starts, kind) = if trim_hits.len() == 1 {
        (trim_hits, WhitespaceKind::LeadingTrailing)
    } else if space_hits.len() == 1 {
        (space_hits, WhitespaceKind::InternalSpacing)
    } else {
        return None;
    };
    let start = starts[0];
    let end = start + search.len();
    let preview = truncate_hint(bodies[start], PREVIEW_CHARS);
    Some((start + 1, end, kind, preview))
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compute_similarity(a: &str, b: &str) -> u8 {
    if a == b {
        return 100;
    }
    let diff = TextDiff::from_chars(a, b);
    (diff.ratio() * 100.0) as u8
}

fn compute_line_similarity(a_lines: &[&str], b_lines: &[&str]) -> u8 {
    if a_lines.is_empty() && b_lines.is_empty() {
        return 100;
    }
    if a_lines.is_empty() || b_lines.is_empty() {
        return 0;
    }
    let max_len = a_lines.len().max(b_lines.len());
    let mut total_similarity: f64 = 0.0;
    for i in 0..max_len {
        let line_sim = match (a_lines.get(i), b_lines.get(i)) {
            (Some(a), Some(b)) => {
                if a == b {
                    100.0
                } else {
                    compute_similarity(a, b) as f64
                }
            }
            (None, _) | (_, None) => 0.0,
        };
        total_similarity += line_sim;
    }
    (total_similarity / max_len as f64) as u8
}

fn non_ws_len(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

pub(super) fn format_line_range(start: usize, end: usize) -> String {
    crate::tool::format_line_label(start as u32, end as u32)
}

pub(super) fn format_line_list(lines: &[usize]) -> String {
    let nums: Vec<u32> = lines.iter().map(|n| *n as u32).collect();
    crate::tool::format_line_list(&nums)
}

pub(super) fn truncate_hint(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let kept: String = s.chars().take(max_chars).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn classify_fuzzy_three_bands_four_states() {
        assert_eq!(classify_fuzzy(79, None), FuzzyBand::NoUsefulMatch);
        assert_eq!(classify_fuzzy(80, Some(40)), FuzzyBand::SuggestedUnique);
        assert_eq!(classify_fuzzy(89, Some(88)), FuzzyBand::SuggestedAmbiguous);
        assert_eq!(classify_fuzzy(90, Some(86)), FuzzyBand::SuggestedAmbiguous);
        assert_eq!(classify_fuzzy(90, Some(85)), FuzzyBand::AutoApplied);
        assert_eq!(classify_fuzzy(95, Some(94)), FuzzyBand::SuggestedAmbiguous);
        assert_eq!(classify_fuzzy(92, None), FuzzyBand::AutoApplied);
    }

    #[test]
    fn structural_prefix_and_indent_block_fuzzy() {
        let line = "fn exceptionally_long_unique_function_name_for_edit_safety() {}";
        let prefix = diagnose_structural_miss(
            &format!("{line}\n"),
            crate::tool::format_file_line(1, line).trim_end(),
        );
        assert!(matches!(prefix, Some(RejectReason::ReadLinePrefix { .. })));
        let indent = diagnose_structural_miss(&format!("{line}\n"), &format!("    {line}"));
        assert!(matches!(indent, Some(RejectReason::WhitespaceOnly { .. })));
    }

    #[test]
    fn multiline_indent_only_is_structural() {
        let content = "fn alpha() {\n    keep();\n}\n";
        let old = "    fn alpha() {\n        keep();\n    }";
        let miss = diagnose_structural_miss(content, old);
        assert!(
            matches!(
                miss,
                Some(RejectReason::WhitespaceOnly {
                    start_line: 1,
                    end_line: 3,
                    ..
                })
            ),
            "{miss:?}"
        );
    }

    #[test]
    fn first_diff_preview_skips_matching_prefix_lines() {
        let (line, preview) = first_diff_preview(
            &["fn greet_user_alpa() {", "    println!(\"hi\");", "}"],
            &["fn greet_user_alpha() {", "    println!(\"hi\");", "}"],
            10,
        );
        assert_eq!(line, Some(10));
        assert!(preview.unwrap().contains("greet_user_alpha"));
    }

    #[test]
    fn fuzzy_candidate_scan_honors_cancellation() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let block = super::super::planner::EditBlock {
            index: 1,
            old_string: "candidate_search_token".into(),
            new_string: "x".into(),
            replace_all: false,
        };
        let content = vec!["candidate"; 1_000].join("\n");
        let result = resolve_block(&content, &block, &cancel, true);
        assert!(matches!(
            result,
            Err(super::super::planner::RequestFail::Cancelled)
        ));
    }
}
