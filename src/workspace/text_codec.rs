//! Shared UTF-8 / BOM / EOL helpers for known-path file tools.
//!
//! Matching may treat CRLF and LF as equivalent, and may treat a narrow set of
//! Unicode typography confusables as equivalent to ASCII for search. Write-back
//! for **edit** must not rewrite unmatched bytes. **write** does not use this
//! layer for content.

/// UTF-8 file decoded for display and editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText {
    pub has_bom: bool,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8DecodeError {
    Utf16,
    Binary,
    NotUtf8,
}

pub fn bytes_contain_nul(bytes: &[u8]) -> bool {
    bytes.contains(&0)
}

pub fn binary_file_message(path: &str) -> String {
    format!("Binary file detected, cannot display: {path}")
}

pub fn decode_utf8_bytes(bytes: &[u8]) -> Result<DecodedText, Utf8DecodeError> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(Utf8DecodeError::Utf16);
    }
    if bytes_contain_nul(bytes) {
        return Err(Utf8DecodeError::Binary);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Utf8DecodeError::NotUtf8)?;
    Ok(strip_utf8_bom(text))
}

pub fn strip_utf8_bom(text: &str) -> DecodedText {
    match text.strip_prefix('\u{feff}') {
        Some(rest) => DecodedText {
            has_bom: true,
            text: rest.to_string(),
        },
        None => DecodedText {
            has_bom: false,
            text: text.to_string(),
        },
    }
}

pub fn reattach_utf8_bom(has_bom: bool, text: &str) -> String {
    if has_bom && !text.starts_with('\u{feff}') {
        let mut out = String::with_capacity(3 + text.len());
        out.push('\u{feff}');
        out.push_str(text);
        out
    } else {
        text.to_string()
    }
}

pub fn decode_error_for_path(err: Utf8DecodeError, path: &str) -> String {
    match err {
        Utf8DecodeError::Binary => binary_file_message(path),
        other => utf8_decode_error_message(other),
    }
}

pub fn utf8_decode_error_message(err: Utf8DecodeError) -> String {
    match err {
        Utf8DecodeError::Utf16 => "UTF-16 file, not UTF-8".into(),
        Utf8DecodeError::Binary => "Binary file detected, cannot display".into(),
        Utf8DecodeError::NotUtf8 => {
            "content is not valid UTF-8 (if this is a GBK/ANSI file, convert it to UTF-8 first)"
                .into()
        }
    }
}

/// Original-content byte range. `start`/`end` are UTF-8 char boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

/// Result of exact edit matching (EOL-agnostic, then typography-confusable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditReplace {
    Applied { content: String, count: usize },
    NotFound,
    Ambiguous,
}

/// All exact matches against the original snapshot, or why matching stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactSpans {
    Hits(Vec<ByteSpan>),
    NotFound,
    Ambiguous,
}

/// Narrow typography substitutions that are almost always accidental
/// (smart quotes, dashes, ellipsis, non-breaking space). Not Unicode NFC/NFKC.
const CONFUSABLE_MAP: &[(char, &str)] = &[
    ('\u{201C}', "\""),  // left double quotation mark
    ('\u{201D}', "\""),  // right double quotation mark
    ('\u{2018}', "'"),   // left single quotation mark
    ('\u{2019}', "'"),   // right single quotation mark
    ('\u{2014}', "--"),  // em-dash
    ('\u{2013}', "-"),   // en-dash
    ('\u{2026}', "..."), // horizontal ellipsis
    ('\u{00A0}', " "),   // non-breaking space
];

fn confusable_ascii(c: char) -> Option<&'static str> {
    CONFUSABLE_MAP
        .iter()
        .find(|&&(ch, _)| ch == c)
        .map(|&(_, replacement)| replacement)
}

fn has_confusables(s: &str) -> bool {
    s.chars().any(|c| confusable_ascii(c).is_some())
}

fn normalize_confusables(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match confusable_ascii(c) {
            Some(replacement) => out.push_str(replacement),
            None => out.push(c),
        }
    }
    out
}

/// Locate every EOL-equivalent (then typography-confusable) match in `content`.
/// Spans refer to the original snapshot; nothing is rewritten.
pub fn find_exact_spans(content: &str, old: &str) -> ExactSpans {
    let eol = find_eol_spans(content, old);
    if !eol.is_empty() {
        return ExactSpans::Hits(eol);
    }
    if !has_confusables(content) && !has_confusables(old) {
        return ExactSpans::NotFound;
    }
    match find_confusable_spans(content, old) {
        ConfusableMatch::NoMatch => ExactSpans::NotFound,
        ConfusableMatch::Ambiguous => ExactSpans::Ambiguous,
        ConfusableMatch::Matches(spans) => ExactSpans::Hits(
            spans
                .into_iter()
                .map(|(start, end)| ByteSpan { start, end })
                .collect(),
        ),
    }
}

fn find_eol_spans(content: &str, old: &str) -> Vec<ByteSpan> {
    let (content_lf, map) = to_lf_with_index_map(content);
    let old_lf = old.replace("\r\n", "\n");
    if old_lf.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut lf_pos = 0usize;
    while let Some(rel) = content_lf[lf_pos..].find(&old_lf) {
        let start = lf_pos + rel;
        let end = start + old_lf.len();
        let orig_start = map[start];
        let orig_end = map[end];
        if orig_end > orig_start
            && content.is_char_boundary(orig_start)
            && content.is_char_boundary(orig_end)
        {
            spans.push(ByteSpan {
                start: orig_start,
                end: orig_end,
            });
        }
        lf_pos = end;
    }
    spans
}

/// Render `new` with the newline style inferred at `orig_start` in `content`.
pub fn render_eol_replacement(content: &str, orig_start: usize, new: &str) -> String {
    let new_lf = new.replace("\r\n", "\n");
    let mut out = String::new();
    push_replacement(&mut out, content, orig_start, &new_lf);
    out
}

/// 1-based inclusive line range covered by `span`.
pub fn byte_span_lines(content: &str, span: ByteSpan) -> (usize, usize) {
    let start = span.start.min(content.len());
    let start_line = content[..start].matches('\n').count() + 1;
    if span.end <= span.start {
        return (start_line, start_line);
    }
    let mut end = span.end.min(content.len());
    if end > 0 && content.as_bytes()[end - 1] == b'\n' {
        end -= 1;
    }
    let end_line = content[..end].matches('\n').count() + 1;
    (start_line, end_line.max(start_line))
}

/// Reject overlapping, unordered, out-of-range, or non-UTF-8-boundary spans.
pub fn validate_byte_spans(content: &str, spans: &[ByteSpan]) -> Result<(), String> {
    let mut prev_end = 0usize;
    for span in spans {
        if span.end < span.start || span.end > content.len() {
            return Err("edit plan span is out of range".into());
        }
        if !content.is_char_boundary(span.start) || !content.is_char_boundary(span.end) {
            return Err("edit plan span is not on a UTF-8 boundary".into());
        }
        if span.start < prev_end {
            return Err("edit plan spans overlap or are unordered".into());
        }
        prev_end = span.end;
    }
    Ok(())
}

/// Splice already-rendered replacements into `content`. Spans must be valid
/// original-snapshot ranges; they are sorted by start before applying.
pub fn apply_byte_spans(
    content: &str,
    replacements: &[(ByteSpan, String)],
) -> Result<String, String> {
    let mut ordered = replacements.to_vec();
    ordered.sort_by_key(|(span, _)| span.start);
    let spans: Vec<ByteSpan> = ordered.iter().map(|(span, _)| *span).collect();
    validate_byte_spans(content, &spans)?;
    let mut out = String::new();
    let mut last = 0usize;
    for (span, replacement) in &ordered {
        out.push_str(&content[last..span.start]);
        out.push_str(replacement);
        last = span.end;
    }
    out.push_str(&content[last..]);
    Ok(out)
}

/// Replace `old` with `new` after treating CRLF and LF as equivalent in the
/// search. Unmatched regions keep their original line endings. The replacement
/// text uses the newline style inferred at the match site.
pub fn eol_preserving_replace(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> (String, usize) {
    let spans = find_eol_spans(content, old);
    let count = spans.len();
    if count == 0 {
        return (content.to_string(), 0);
    }
    if !replace_all && count > 1 {
        return (content.to_string(), count);
    }
    let apply: &[ByteSpan] = if replace_all { &spans } else { &spans[..1] };
    match apply_rendered_spans(content, apply, new) {
        Ok(edited) => (edited, apply.len()),
        Err(_) => (content.to_string(), 0),
    }
}

fn apply_rendered_spans(content: &str, spans: &[ByteSpan], new: &str) -> Result<String, String> {
    let replacements: Vec<(ByteSpan, String)> = spans
        .iter()
        .map(|span| (*span, render_eol_replacement(content, span.start, new)))
        .collect();
    apply_byte_spans(content, &replacements)
}

/// Exact replace used by the edit tool: EOL-agnostic first, then a confusable
/// fallback that remaps matches back to original bytes.
pub fn edit_preserving_replace(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> EditReplace {
    match find_exact_spans(content, old) {
        ExactSpans::NotFound => EditReplace::NotFound,
        ExactSpans::Ambiguous => EditReplace::Ambiguous,
        ExactSpans::Hits(spans) => {
            let count = spans.len();
            if count == 0 {
                return EditReplace::NotFound;
            }
            if !replace_all && count > 1 {
                return EditReplace::Applied {
                    content: content.to_string(),
                    count,
                };
            }
            let apply: &[ByteSpan] = if replace_all { &spans } else { &spans[..1] };
            match apply_rendered_spans(content, apply, new) {
                Ok(edited) => EditReplace::Applied {
                    content: edited,
                    count: apply.len(),
                },
                Err(_) => EditReplace::NotFound,
            }
        }
    }
}

/// 1-based start line of each EOL- or confusable-normalized match.
pub fn edit_match_line_numbers(content: &str, old: &str) -> Vec<usize> {
    match find_exact_spans(content, old) {
        ExactSpans::Hits(spans) => spans
            .into_iter()
            .map(|span| byte_span_lines(content, span).0)
            .collect(),
        _ => Vec::new(),
    }
}

/// Hint when an exact miss is explained by typography confusables.
pub fn confusable_miss_hint(content: &str, old: &str) -> Option<String> {
    if !has_confusables(content) && !has_confusables(old) {
        return None;
    }
    let (content_lf, eol_map) = to_lf_with_index_map(content);
    let old_lf = old.replace("\r\n", "\n");
    if old_lf.is_empty() {
        return None;
    }
    let (norm_file, conf_map) = build_confusable_offset_map(&content_lf);
    let norm_old = normalize_confusables(&old_lf);
    if norm_old.is_empty() {
        return None;
    }
    let norm_start = norm_file.find(&norm_old)?;
    let lf_start = conf_map[norm_start];
    let lf_end = conf_map[norm_start + norm_old.len()];
    if lf_end < lf_start || lf_end > content_lf.len() {
        return None;
    }
    let orig_start = eol_map[lf_start];
    let orig_end = eol_map[lf_end];
    let match_start_line = content[..orig_start].matches('\n').count() + 1;
    let match_end_line = content[..orig_end].matches('\n').count() + 1;
    let mut affected_lines: Vec<usize> = detect_confusable_lines(content)
        .into_iter()
        .filter(|line| *line >= match_start_line && *line <= match_end_line)
        .collect();
    affected_lines.dedup();
    const MAX_LISTED_LINES: usize = 8;
    if affected_lines.is_empty() {
        return Some(
            "old_string contains Unicode typography characters (smart quotes, em-dashes, etc.) \
             that look identical to ASCII but differ at the byte level. Re-read the file and \
             use a shorter old_string anchored on nearby ASCII-only context."
                .into(),
        );
    }
    let line_summary = if affected_lines.len() <= MAX_LISTED_LINES {
        affected_lines
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        let shown: Vec<String> = affected_lines[..MAX_LISTED_LINES]
            .iter()
            .map(|n| n.to_string())
            .collect();
        format!(
            "{} (and {} more)",
            shown.join(", "),
            affected_lines.len() - MAX_LISTED_LINES
        )
    };
    Some(format!(
        "The nearest matching region contains Unicode typography characters \
         (smart quotes, em-dashes, etc.) on lines {line_summary} that look identical to \
         ASCII but differ at the byte level. Re-read the file and use a shorter \
         old_string anchored on nearby ASCII-only context."
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfusableMatch {
    NoMatch,
    Matches(Vec<(usize, usize)>),
    Ambiguous,
}

fn find_confusable_spans(content: &str, old: &str) -> ConfusableMatch {
    let (content_lf, eol_map) = to_lf_with_index_map(content);
    let old_lf = old.replace("\r\n", "\n");
    if old_lf.is_empty() {
        return ConfusableMatch::NoMatch;
    }
    let (norm_text, conf_map) = build_confusable_offset_map(&content_lf);
    let norm_old = normalize_confusables(&old_lf);
    if norm_old.is_empty() {
        return ConfusableMatch::NoMatch;
    }

    let mut validated = Vec::new();
    let mut had_rejected = false;
    for (norm_start, _) in norm_text.match_indices(&norm_old) {
        let norm_end = norm_start + norm_old.len();
        let lf_start = conf_map[norm_start];
        let lf_end = conf_map[norm_end];
        if lf_end <= lf_start || lf_end > content_lf.len() {
            had_rejected = true;
            continue;
        }
        let lf_slice = &content_lf[lf_start..lf_end];
        if normalize_confusables(lf_slice) != norm_old {
            had_rejected = true;
            continue;
        }
        let orig_start = eol_map[lf_start];
        let orig_end = eol_map[lf_end];
        if orig_end <= orig_start {
            had_rejected = true;
            continue;
        }
        validated.push((orig_start, orig_end));
    }

    if had_rejected {
        return ConfusableMatch::Ambiguous;
    }
    if validated.is_empty() {
        return ConfusableMatch::NoMatch;
    }
    for window in validated.windows(2) {
        if window[0].1 > window[1].0 {
            return ConfusableMatch::Ambiguous;
        }
    }
    ConfusableMatch::Matches(validated)
}

fn push_replacement(out: &mut String, content: &str, orig_start: usize, new_lf: &str) {
    if inferred_eol(content, orig_start) == "\r\n" {
        out.push_str(&new_lf.replace('\n', "\r\n"));
    } else {
        out.push_str(new_lf);
    }
}

fn detect_confusable_lines(s: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut line: usize = 1;
    for c in s.chars() {
        if confusable_ascii(c).is_some() && lines.last() != Some(&line) {
            lines.push(line);
        }
        if c == '\n' {
            line += 1;
        }
    }
    lines
}

fn build_confusable_offset_map(s: &str) -> (String, Vec<usize>) {
    let mut normalized = String::with_capacity(s.len());
    let mut offset_map: Vec<usize> = Vec::with_capacity(s.len() + 1);
    for (orig_byte_offset, c) in s.char_indices() {
        match confusable_ascii(c) {
            Some(replacement) => {
                for _ in 0..replacement.len() {
                    offset_map.push(orig_byte_offset);
                }
                normalized.push_str(replacement);
            }
            None => {
                let char_len = c.len_utf8();
                for i in 0..char_len {
                    offset_map.push(orig_byte_offset + i);
                }
                normalized.push(c);
            }
        }
    }
    offset_map.push(s.len());
    debug_assert_eq!(offset_map.len(), normalized.len() + 1);
    (normalized, offset_map)
}

/// Split on `\n` / `\r\n` only, keeping each terminator.
pub fn split_keep_eol(s: &str) -> (Vec<&str>, Vec<&str>) {
    let mut bodies = Vec::new();
    let mut eols = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            bodies.push(&s[start..i]);
            eols.push(&s[i..i + 2]);
            i += 2;
            start = i;
        } else if bytes[i] == b'\n' {
            bodies.push(&s[start..i]);
            eols.push(&s[i..i + 1]);
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if start < s.len() || s.is_empty() {
        bodies.push(&s[start..]);
        eols.push("");
    }
    (bodies, eols)
}

pub fn join_keep_eol(bodies: &[&str], eols: &[&str]) -> String {
    let mut out = String::new();
    for (i, body) in bodies.iter().enumerate() {
        out.push_str(body);
        if let Some(eol) = eols.get(i) {
            out.push_str(eol);
        }
    }
    out
}

fn to_lf_with_index_map(s: &str) -> (String, Vec<usize>) {
    let bytes = s.as_bytes();
    let mut lf = String::new();
    let mut map = Vec::with_capacity(bytes.len() + 1);
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            map.push(i);
            lf.push('\n');
            i += 2;
            continue;
        }
        let ch = s[i..].chars().next().expect("valid utf-8");
        let len = ch.len_utf8();
        for k in 0..len {
            map.push(i + k);
        }
        lf.push(ch);
        i += len;
    }
    map.push(s.len());
    (lf, map)
}

fn inferred_eol(s: &str, at: usize) -> &'static str {
    let bytes = s.as_bytes();
    if let Some(rel) = s[at..].find('\n') {
        let abs = at + rel;
        if abs > 0 && bytes[abs - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else if let Some(i) = s[..at].rfind('\n') {
        if i > 0 && bytes[i - 1] == b'\r' {
            "\r\n"
        } else {
            "\n"
        }
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_and_reattaches_bom() {
        let decoded = decode_utf8_bytes("\u{feff}hello".as_bytes()).unwrap();
        assert!(decoded.has_bom);
        assert_eq!(decoded.text, "hello");
        assert_eq!(reattach_utf8_bom(true, "hello"), "\u{feff}hello");
    }

    #[test]
    fn utf16_le_is_distinct_error() {
        assert_eq!(
            decode_utf8_bytes(&[0xFF, 0xFE, b'A', 0x00]),
            Err(Utf8DecodeError::Utf16)
        );
    }

    #[test]
    fn mixed_eol_replace_keeps_unmatched() {
        let content = "a\r\nb\nc\n";
        let (result, count) = eol_preserving_replace(content, "b", "B", false);
        assert_eq!(count, 1);
        assert_eq!(result, "a\r\nB\nc\n");
    }

    #[test]
    fn mixed_eol_multiline_replacement_uses_matched_line_ending() {
        let content = "a\r\nb\nc\n";
        let (result, count) = eol_preserving_replace(content, "b", "B\nB2", false);
        assert_eq!(count, 1);
        assert_eq!(result, "a\r\nB\nB2\nc\n");
    }

    #[test]
    fn eol_replace_without_replace_all_does_not_apply_when_multiple() {
        let content = "foo\nbar\nfoo\n";
        let (result, count) = eol_preserving_replace(content, "foo", "FOO", false);
        assert_eq!(count, 2);
        assert_eq!(result, content);
    }

    #[test]
    fn lf_old_matches_crlf_file() {
        let content = "a\r\nb\r\n";
        let (result, count) = eol_preserving_replace(content, "a\nb", "X", false);
        assert_eq!(count, 1);
        assert_eq!(result, "X\r\n");
    }

    fn applied(content: &str, old: &str, new: &str, replace_all: bool) -> (String, usize) {
        match edit_preserving_replace(content, old, new, replace_all) {
            EditReplace::Applied { content, count } => (content, count),
            other => panic!("expected Applied, got {other:?}"),
        }
    }

    #[test]
    fn confusable_smart_quotes_replace() {
        let content = "say \u{201C}hello\u{201D} world";
        let (result, count) = applied(content, "\"hello\"", "\"goodbye\"", false);
        assert_eq!(count, 1);
        assert_eq!(result, "say \"goodbye\" world");
    }

    #[test]
    fn confusable_em_dash_and_nbsp() {
        let content = "foo\u{2014}bar hello\u{00A0}world";
        let (result, count) = applied(content, "foo--bar hello world", "X", false);
        assert_eq!(count, 1);
        assert_eq!(result, "X");
    }

    #[test]
    fn confusable_preserves_crlf_and_unmatched() {
        let content = "say \u{201C}hi\u{201D}\r\nnext\r\n";
        let (result, count) = applied(content, "say \"hi\"", "say \"ok\"", false);
        assert_eq!(count, 1);
        assert_eq!(result, "say \"ok\"\r\nnext\r\n");
    }

    #[test]
    fn confusable_partial_dash_is_ambiguous() {
        assert_eq!(
            edit_preserving_replace("\u{2014}", "-", "x", false),
            EditReplace::Ambiguous
        );
        assert_eq!(
            edit_preserving_replace("a\u{2026}b", ".", "x", false),
            EditReplace::Ambiguous
        );
    }

    #[test]
    fn confusable_valid_and_partial_matches_are_ambiguous_together() {
        assert_eq!(
            edit_preserving_replace("a\u{2013}b\u{2014}c", "-", "x", true),
            EditReplace::Ambiguous
        );
    }

    #[test]
    fn confusable_full_em_dash_ok() {
        let (result, count) = applied("a\u{2014}b", "--", "-", false);
        assert_eq!(count, 1);
        assert_eq!(result, "a-b");
    }

    #[test]
    fn confusable_ascii_old_against_ascii_file_still_exact() {
        let (result, count) = applied("hello world", "hello", "hi", false);
        assert_eq!(count, 1);
        assert_eq!(result, "hi world");
    }

    #[test]
    fn confusable_hint_lists_affected_line() {
        let hint = confusable_miss_hint("plain\n\u{201C}quoted\u{201D}\n", "\"quoted\"");
        let hint = hint.expect("hint");
        assert!(hint.contains("lines 2"), "{hint}");
        assert!(hint.contains("typography"), "{hint}");
    }

    #[test]
    fn confusable_hint_skips_unrelated_typography() {
        assert!(
            confusable_miss_hint("\u{201C}quoted\u{201D}\nplain\n", "totally_unrelated").is_none()
        );
        assert!(confusable_miss_hint("hello world\n", "xyz").is_none());
    }

    #[test]
    fn confusable_replace_all() {
        let content = "\u{201C}a\u{201D} and \u{201C}a\u{201D}";
        let (result, count) = applied(content, "\"a\"", "'a'", true);
        assert_eq!(count, 2);
        assert_eq!(result, "'a' and 'a'");
    }

    #[test]
    fn confusable_multiple_without_replace_all() {
        let content = "\u{201C}a\u{201D} and \u{201C}a\u{201D}";
        let (_, count) = applied(content, "\"a\"", "x", false);
        assert_eq!(count, 2);
    }

    #[test]
    fn offset_map_roundtrip_smart_quotes() {
        let original = "\u{201C}hi\u{201D}";
        let (normalized, map) = build_confusable_offset_map(original);
        assert_eq!(normalized, "\"hi\"");
        let start = normalized.find("\"hi\"").unwrap();
        let end = start + 4;
        let orig = &original[map[start]..map[end]];
        assert_eq!(normalize_confusables(orig), "\"hi\"");
    }

    #[test]
    fn exact_spans_map_lf_old_to_crlf_original() {
        let content = "a\r\nb\r\nc\r\n";
        match find_exact_spans(content, "a\nb") {
            ExactSpans::Hits(spans) => {
                assert_eq!(spans, vec![ByteSpan { start: 0, end: 4 }]);
                assert_eq!(&content[0..4], "a\r\nb");
                assert_eq!(byte_span_lines(content, spans[0]), (1, 2));
                let rendered = render_eol_replacement(content, spans[0].start, "X");
                let out = apply_byte_spans(content, &[(spans[0], rendered)]).unwrap();
                assert_eq!(out, "X\r\nc\r\n");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn exact_spans_confusable_and_utf8_boundaries() {
        let content = "say \u{201C}hello\u{201D} world";
        match find_exact_spans(content, "\"hello\"") {
            ExactSpans::Hits(spans) => {
                assert_eq!(spans.len(), 1);
                assert!(content.is_char_boundary(spans[0].start));
                assert!(content.is_char_boundary(spans[0].end));
                let rendered = render_eol_replacement(content, spans[0].start, "\"hi\"");
                let out = apply_byte_spans(content, &[(spans[0], rendered)]).unwrap();
                assert_eq!(out, "say \"hi\" world");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn apply_byte_spans_rejects_overlap() {
        let content = "abcdef";
        let err = apply_byte_spans(
            content,
            &[
                (ByteSpan { start: 0, end: 3 }, "X".into()),
                (ByteSpan { start: 2, end: 4 }, "Y".into()),
            ],
        )
        .unwrap_err();
        assert!(err.contains("overlap"), "{err}");
    }
}
