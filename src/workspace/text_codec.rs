//! Shared UTF-8 / BOM / EOL helpers for known-path file tools.
//!
//! Matching may treat CRLF and LF as equivalent. Write-back for **edit** must
//! not rewrite unmatched bytes. **write** does not use this layer for content.

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

/// Result of an exact, EOL-agnostic replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditReplace {
    Applied { content: String, count: usize },
    NotFound,
}

/// All exact matches against the original snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExactSpans {
    Hits(Vec<ByteSpan>),
    NotFound,
}

/// Locate every EOL-equivalent match in `content`.
/// Spans refer to the original snapshot; nothing is rewritten.
pub fn find_exact_spans(content: &str, old: &str) -> ExactSpans {
    let eol = find_eol_spans(content, old);
    if eol.is_empty() {
        ExactSpans::NotFound
    } else {
        ExactSpans::Hits(eol)
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

/// Exact replace used by the edit tool. CRLF and LF compare equal.
pub fn edit_preserving_replace(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> EditReplace {
    match find_exact_spans(content, old) {
        ExactSpans::NotFound => EditReplace::NotFound,
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

/// 1-based start line of each EOL-normalized match.
pub fn edit_match_line_numbers(content: &str, old: &str) -> Vec<usize> {
    match find_exact_spans(content, old) {
        ExactSpans::Hits(spans) => spans
            .into_iter()
            .map(|span| byte_span_lines(content, span).0)
            .collect(),
        _ => Vec::new(),
    }
}

fn push_replacement(out: &mut String, content: &str, orig_start: usize, new_lf: &str) {
    if inferred_eol(content, orig_start) == "\r\n" {
        out.push_str(&new_lf.replace('\n', "\r\n"));
    } else {
        out.push_str(new_lf);
    }
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
    fn quoted_punctuation_is_not_an_ascii_match() {
        assert!(matches!(
            find_exact_spans("say \u{201C}hello\u{201D} world", "\"hello\""),
            ExactSpans::NotFound
        ));
        assert!(matches!(
            edit_preserving_replace("a\u{2014}b", "--", "-", false),
            EditReplace::NotFound
        ));
    }

    #[test]
    fn exact_spans_respect_utf8_boundaries() {
        let content = "say \"hello\" world";
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
