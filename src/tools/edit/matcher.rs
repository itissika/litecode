use serde_json::{Value, json};

use crate::workspace::text_codec::{self, ByteSpan, ExactSpans};

use super::planner::{EditBlock, RequestFail};

pub(super) const PREVIEW_CHARS: usize = 160;

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
}

#[derive(Clone, Debug)]
pub(super) struct ApplyPlan {
    pub replacements: Vec<PlannedReplacement>,
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
pub(super) enum RejectReason {
    EmptyOldString,
    NoOp {
        kind: NoOpKind,
    },
    ReadLinePrefix {
        ranges: Vec<(usize, usize)>,
    },
    WhitespaceOnly {
        start_line: usize,
        end_line: usize,
        kind: WhitespaceKind,
        file_preview: String,
    },
    WhitespaceOnlyInput,
    MultipleExact {
        ranges: Vec<(usize, usize)>,
    },
    NoMatch,
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
    pub(super) fn code(&self) -> &'static str {
        match self {
            Self::EmptyOldString => "empty_old_string",
            Self::NoOp { .. } => "no_op",
            Self::ReadLinePrefix { .. } => "read_line_prefix",
            Self::WhitespaceOnly { .. } | Self::WhitespaceOnlyInput => "whitespace_only",
            Self::MultipleExact { .. } => "multiple_matches",
            Self::NoMatch => "no_match",
            Self::Chained { .. } => "chained",
            Self::Overlap { .. } => "overlap",
        }
    }

    pub(super) fn message(&self) -> String {
        match self {
            Self::EmptyOldString => "old_string is empty".into(),
            Self::NoOp {
                kind: NoOpKind::IdenticalStrings,
            } => "old_string and new_string are the same".into(),
            Self::NoOp {
                kind: NoOpKind::ReplacementUnchanged,
            } => "replacement does not change the matched text".into(),
            Self::ReadLinePrefix { .. } => {
                "old_string includes a read line-number prefix".into()
            }
            Self::WhitespaceOnly { .. } => {
                "old_string differs from the file only by whitespace".into()
            }
            Self::WhitespaceOnlyInput => "old_string is only whitespace".into(),
            Self::MultipleExact { .. } => {
                "old_string matched more than once; set replace_all or add surrounding lines"
                    .into()
            }
            Self::NoMatch => "old_string was not found".into(),
            Self::Chained { prior_index } => format!(
                "old_string depends on edit {prior_index} in this call; submit it in a later edit call"
            ),
            Self::Overlap { other_index, .. } => {
                format!("this edit overlaps edit {other_index} in this call")
            }
        }
    }

    pub(super) fn details(&self) -> Option<Value> {
        match self {
            Self::NoOp {
                kind: NoOpKind::IdenticalStrings,
            } => Some(json!({ "kind": "identical_strings" })),
            Self::NoOp {
                kind: NoOpKind::ReplacementUnchanged,
            } => Some(json!({ "kind": "replacement_unchanged" })),
            Self::ReadLinePrefix { ranges } if !ranges.is_empty() => {
                Some(json!({ "lines": ranges.iter().map(|(start, _)| start).collect::<Vec<_>>() }))
            }
            Self::WhitespaceOnly { kind, file_preview, .. } => Some(json!({
                "kind": match kind {
                    WhitespaceKind::LeadingTrailing => "leading_trailing",
                    WhitespaceKind::InternalSpacing => "internal_spacing",
                },
                "excerpt": file_preview,
            })),
            Self::WhitespaceOnlyInput => Some(json!({ "kind": "input" })),
            Self::MultipleExact { ranges } => Some(json!({
                "lines": ranges.iter().map(|(start, _)| start).collect::<Vec<_>>(),
            })),
            Self::Chained { prior_index } => Some(json!({ "prior_index": prior_index })),
            Self::Overlap {
                other_index,
                other_range,
                ..
            } => Some(json!({
                "other_index": other_index,
                "other_range": {
                    "start_line": other_range.0,
                    "end_line": other_range.1,
                },
            })),
            Self::EmptyOldString | Self::NoMatch | Self::ReadLinePrefix { .. } => None,
        }
    }

    pub(super) fn ranges(&self) -> Vec<(usize, usize)> {
        match self {
            Self::ReadLinePrefix { ranges } | Self::MultipleExact { ranges } => ranges.clone(),
            Self::WhitespaceOnly {
                start_line,
                end_line,
                ..
            } => vec![(*start_line, *end_line)],
            Self::Overlap { this_range, .. } => vec![*this_range],
            _ => Vec::new(),
        }
    }
}

#[derive(Debug)]
pub(super) enum ResolveOutcome {
    Apply(ApplyPlan),
    Reject(RejectReason),
}

pub(super) fn resolve_block(content: &str, block: &EditBlock) -> Result<ResolveOutcome, RequestFail> {
    if block.old_string.is_empty() {
        return Ok(ResolveOutcome::Reject(RejectReason::EmptyOldString));
    }
    if block.old_string == block.new_string {
        return Ok(ResolveOutcome::Reject(RejectReason::NoOp {
            kind: NoOpKind::IdenticalStrings,
        }));
    }
    Ok(match text_codec::find_exact_spans(content, &block.old_string) {
        ExactSpans::Hits(spans) => resolve_exact(content, block, &spans),
        ExactSpans::NotFound => resolve_exact_miss(content, block),
    })
}

fn resolve_exact(content: &str, block: &EditBlock, spans: &[ByteSpan]) -> ResolveOutcome {
    if spans.is_empty() {
        return ResolveOutcome::Reject(RejectReason::NoMatch);
    }
    if !block.replace_all && spans.len() > 1 {
        let ranges = spans
            .iter()
            .map(|span| text_codec::byte_span_lines(content, *span))
            .collect();
        return ResolveOutcome::Reject(RejectReason::MultipleExact { ranges });
    }
    let apply = if block.replace_all {
        spans.to_vec()
    } else {
        vec![spans[0]]
    };
    match replacements_from_spans(content, &apply, &block.new_string) {
        Ok(replacements) => ResolveOutcome::Apply(ApplyPlan { replacements }),
        Err(reason) => ResolveOutcome::Reject(reason),
    }
}

fn resolve_exact_miss(content: &str, block: &EditBlock) -> ResolveOutcome {
    if let Some(reason) = diagnose_structural_miss(content, &block.old_string) {
        return ResolveOutcome::Reject(reason);
    }
    if non_ws_len(&block.old_string) == 0 {
        return ResolveOutcome::Reject(RejectReason::WhitespaceOnlyInput);
    }
    ResolveOutcome::Reject(RejectReason::NoMatch)
}

fn replacements_from_spans(
    content: &str,
    spans: &[ByteSpan],
    new: &str,
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
        let ranges = match text_codec::find_exact_spans(content, &stripped) {
            ExactSpans::Hits(spans) => spans
                .into_iter()
                .map(|span| text_codec::byte_span_lines(content, span))
                .collect(),
            ExactSpans::NotFound => Vec::new(),
        };
        return Some(RejectReason::ReadLinePrefix { ranges });
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

fn non_ws_len(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
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

    #[test]
    fn structural_prefix_and_indent_block_before_apply() {
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
    fn near_typo_is_no_match() {
        let block = EditBlock {
            index: 1,
            old_string: "fn greet_user_alpa() {\n    println!(\"hi\");\n}".into(),
            new_string: "fn greet_user_beta() {}".into(),
            replace_all: false,
        };
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let outcome = resolve_block(content, &block).unwrap();
        match outcome {
            ResolveOutcome::Reject(reason) => assert_eq!(reason.code(), "no_match"),
            ResolveOutcome::Apply(plan) => panic!("near typo must not apply: {plan:?}"),
        }
    }
}
