use crate::workspace::text_codec::{self, ByteSpan, ExactSpans};

pub(super) const PREVIEW_CHARS: usize = 160;

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
        }
    }

    /// Guidance for the model. Line hints stay in the text — not a details tree.
    pub(super) fn message(&self) -> String {
        match self {
            Self::EmptyOldString => "old_string is empty".into(),
            Self::NoOp {
                kind: NoOpKind::IdenticalStrings,
            } => "old_string and new_string are the same".into(),
            Self::NoOp {
                kind: NoOpKind::ReplacementUnchanged,
            } => "replacement does not change the matched text".into(),
            Self::ReadLinePrefix { ranges } => {
                if ranges.is_empty() {
                    "old_string includes a read line-number prefix; strip the prefixes and retry".into()
                } else {
                    let lines = format_lines(ranges);
                    format!(
                        "old_string includes a read line-number prefix (near lines {lines}); strip the prefixes and retry"
                    )
                }
            }
            Self::WhitespaceOnly {
                start_line,
                end_line,
                kind,
                file_preview,
            } => {
                let kind = match kind {
                    WhitespaceKind::LeadingTrailing => "leading/trailing whitespace",
                    WhitespaceKind::InternalSpacing => "internal spacing",
                };
                format!(
                    "old_string differs from the file only by {kind} (lines {start_line}-{end_line}); file has: {file_preview}"
                )
            }
            Self::WhitespaceOnlyInput => "old_string is only whitespace".into(),
            Self::MultipleExact { ranges } => {
                let lines = format_lines(ranges);
                format!(
                    "old_string matched more than once (lines {lines}); add surrounding lines to make the match unique"
                )
            }
            Self::NoMatch => "old_string was not found".into(),
        }
    }
}

fn format_lines(ranges: &[(usize, usize)]) -> String {
    ranges
        .iter()
        .map(|(start, _)| start.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug)]
pub(super) enum ResolveOutcome {
    /// Full file contents after the unique replacement.
    Apply(String),
    Reject(RejectReason),
}

pub(super) fn resolve_one(
    content: &str,
    old_string: &str,
    new_string: &str,
) -> ResolveOutcome {
    if old_string.is_empty() {
        return ResolveOutcome::Reject(RejectReason::EmptyOldString);
    }
    if old_string == new_string {
        return ResolveOutcome::Reject(RejectReason::NoOp {
            kind: NoOpKind::IdenticalStrings,
        });
    }
    match text_codec::find_exact_spans(content, old_string) {
        ExactSpans::Hits(spans) => resolve_exact(content, old_string, new_string, &spans),
        ExactSpans::NotFound => resolve_exact_miss(content, old_string),
    }
}

fn resolve_exact(
    content: &str,
    _old_string: &str,
    new_string: &str,
    spans: &[ByteSpan],
) -> ResolveOutcome {
    if spans.is_empty() {
        return ResolveOutcome::Reject(RejectReason::NoMatch);
    }
    if spans.len() > 1 {
        let ranges = spans
            .iter()
            .map(|span| text_codec::byte_span_lines(content, *span))
            .collect();
        return ResolveOutcome::Reject(RejectReason::MultipleExact { ranges });
    }
    let span = spans[0];
    let replacement = text_codec::render_eol_replacement(content, span.start, new_string);
    if replacement == content[span.start..span.end] {
        return ResolveOutcome::Reject(RejectReason::NoOp {
            kind: NoOpKind::ReplacementUnchanged,
        });
    }
    match text_codec::apply_byte_spans(content, &[(span, replacement)]) {
        Ok(edited) => {
            if edited == content {
                ResolveOutcome::Reject(RejectReason::NoOp {
                    kind: NoOpKind::ReplacementUnchanged,
                })
            } else {
                ResolveOutcome::Apply(edited)
            }
        }
        Err(_) => ResolveOutcome::Reject(RejectReason::NoMatch),
    }
}

fn resolve_exact_miss(content: &str, old_string: &str) -> ResolveOutcome {
    if let Some(reason) = diagnose_structural_miss(content, old_string) {
        return ResolveOutcome::Reject(reason);
    }
    if non_ws_len(old_string) == 0 {
        return ResolveOutcome::Reject(RejectReason::WhitespaceOnlyInput);
    }
    ResolveOutcome::Reject(RejectReason::NoMatch)
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
        let content = "fn greet_user_alpha() {\n    println!(\"hi\");\n}\n";
        let outcome = resolve_one(
            content,
            "fn greet_user_alpa() {\n    println!(\"hi\");\n}",
            "fn greet_user_beta() {}",
        );
        match outcome {
            ResolveOutcome::Reject(reason) => assert_eq!(reason.code(), "no_match"),
            ResolveOutcome::Apply(edited) => panic!("near typo must not apply: {edited}"),
        }
    }

    #[test]
    fn multiple_match_message_lists_lines() {
        let outcome = resolve_one("foo\nbar\nfoo\n", "foo", "FOO");
        match outcome {
            ResolveOutcome::Reject(RejectReason::MultipleExact { ranges }) => {
                assert_eq!(ranges, vec![(1, 1), (3, 3)]);
                let msg = RejectReason::MultipleExact { ranges }.message();
                assert!(msg.contains("lines 1, 3"), "{msg}");
                assert!(!msg.contains("replace_all"), "{msg}");
            }
            other => panic!("expected multiple: {other:?}"),
        }
    }
}
