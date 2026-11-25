//! Workspace citations in Markdown prose: standard CommonMark inline links
//! `[validate.rs](src/a.rs)`, `[validate.rs:4-9](src/a.rs#L4-L9)`,
//! `[Store.save](src/a.rs#Store.save)`, and
//! `[seq](knowledge/contracts/seq.md)`.
//!
//! Fenced code blocks, inline code spans, images (`![...](...)`), web links
//! (`https://...`), and in-page anchors (`#section`) are not workspace citations.

use std::sync::LazyLock;

use regex::Regex;

use crate::engines::code_search::enclosing::normalize_symbol_query;

static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$").expect("key pattern")
});

pub fn normalize_key(key: &str) -> String {
    key.trim().to_string()
}

pub fn is_knowledge_key(key: &str) -> bool {
    KEY.is_match(key)
}

/// Inclusive 1-based line span of a range citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    pub start: u32,
    pub end: u32,
}

/// Format as `12` or `12-30` (used in `ref_key` and human display).
pub fn format_line_span(span: LineSpan) -> String {
    if span.start == span.end {
        format!("{}", span.start)
    } else {
        format!("{}-{}", span.start, span.end)
    }
}

/// Format as GitHub-style fragment `#L12` or `#L12-L30` (without leading `#`).
pub fn format_line_anchor(span: LineSpan) -> String {
    if span.start == span.end {
        format!("L{}", span.start)
    } else {
        format!("L{}-L{}", span.start, span.end)
    }
}

/// Parse a `#L...` line anchor: `L12`, `L12-30`, or `L12-L30`.
/// Strict: ASCII digits only, no leading zero, no sign, no spaces, `1 <= A <= B <= u32::MAX`.
/// Bare `12` / `12-30` without `L` is NOT accepted.
pub fn parse_line_anchor(raw: &str) -> Option<LineSpan> {
    let body = raw.strip_prefix('L')?;
    let (start, end) = match body.split_once('-') {
        Some((start_s, end_s)) => {
            let end_s = end_s.strip_prefix('L').unwrap_or(end_s);
            (parse_line_number(start_s)?, parse_line_number(end_s)?)
        }
        None => {
            let line = parse_line_number(body)?;
            (line, line)
        }
    };
    if end < start {
        return None;
    }
    Some(LineSpan { start, end })
}

/// True when a `#fragment` looks like an attempted line range that failed
/// [`parse_line_anchor`] (so it reports `invalid_lines` instead of `missing_symbol`).
pub fn is_line_like_anchor(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    if let Some(rest) = raw.strip_prefix('L') {
        if let Some(first) = rest.chars().next() {
            return first.is_numeric() || first == '-' || first == '+' || first.is_whitespace();
        }
        return false;
    }
    // Bare digits/dashes/signs without `L` (e.g. `#12`, `#12-30`, `#0`, `#+3`, `#３`).
    let mut has_digit = false;
    for ch in raw.chars() {
        if ch.is_numeric() {
            has_digit = true;
        } else if matches!(ch, '-' | '+' | ' ' | '\t') {
            continue;
        } else {
            return false;
        }
    }
    has_digit
}

fn parse_line_number(text: &str) -> Option<u32> {
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

/// Format one citation as a standard CommonMark inline link.
pub fn citation_source(
    label: &str,
    path: &str,
    symbol: Option<&str>,
    lines: Option<LineSpan>,
) -> String {
    let mut dest = path.to_string();
    if let Some(symbol) = symbol.map(str::trim).filter(|text| !text.is_empty()) {
        dest.push('#');
        dest.push_str(&normalize_symbol_query(symbol));
    } else if let Some(span) = lines {
        dest.push('#');
        dest.push_str(&format_line_anchor(span));
    }
    let wrapped = format_link_dest(&dest);
    format!("[{label}]({wrapped})")
}

fn format_link_dest(dest: &str) -> String {
    if dest.chars().any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>')) {
        format!("<{dest}>")
    } else {
        dest.to_string()
    }
}

/// Words shown for a file, symbol, or range citation in plain-text summaries.
pub fn citation_fact(path: &str, symbol: Option<&str>, lines: Option<LineSpan>) -> String {
    let mut out = path.to_string();
    if let Some(symbol) = symbol.map(str::trim).filter(|text| !text.is_empty()) {
        out.push_str(" : ");
        out.push_str(symbol);
    }
    if let Some(span) = lines {
        out.push_str(" : ");
        out.push_str(&format_line_span(span));
    }
    out
}

/// Kind of a citation as the knowledge API names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CiteKind {
    Node,
    File,
    Symbol,
    Range,
}

impl CiteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::File => "file",
            Self::Symbol => "symbol",
            Self::Range => "range",
        }
    }
}

/// One workspace citation extracted from Markdown prose, in appearance order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cite {
    pub kind: CiteKind,
    /// Link label text `[label](...)`.
    pub label: String,
    /// Workspace path as written (percent-decoded, backslashes kept; see [`Cite::norm_path`]).
    pub path: String,
    /// Normalized dot-separated symbol anchor, when `kind == CiteKind::Symbol`.
    pub symbol: Option<String>,
    /// Valid span of a range citation.
    pub lines: Option<LineSpan>,
    /// Raw `#fragment` when it looked like a line range but failed [`parse_line_anchor`].
    pub bad_lines: Option<String>,
    /// Exact source text `[label](dest)` in the markdown.
    pub raw: String,
}

impl Cite {
    /// KR-PATH-2: `\` is `/`.
    pub fn norm_path(&self) -> String {
        normalize_ref_path(&self.path)
    }

    /// Syntax-derived identity of a citation inside one node (KR-KEY-1):
    /// `path:<norm_path>`, `sym:<norm_path>\0<symbol>`, `range:<norm_path>\0<lines>`.
    /// Independent of whether the backend folds a bare path to a knowledge node.
    pub fn ref_key(&self) -> String {
        let path = self.norm_path();
        match self.kind {
            CiteKind::Node | CiteKind::File => format!("path:{path}"),
            CiteKind::Symbol => format!("sym:{path}\0{}", self.symbol.as_deref().unwrap_or("")),
            CiteKind::Range => {
                let lines = match (self.lines, self.bad_lines.as_deref()) {
                    (Some(span), _) => format_line_span(span),
                    (None, Some(raw)) => raw.to_string(),
                    (None, None) => String::new(),
                };
                format!("range:{path}\0{lines}")
            }
        }
    }
}

/// KR-PATH-2 path form used for identity and lookups.
pub fn normalize_ref_path(path: &str) -> String {
    path.trim().replace('\\', "/")
}

/// Workspace-relative path. `..`, `.`, an empty segment, and an absolute path are not.
/// The length cap counts Unicode scalars, matching the editor.
pub fn is_workspace_file_ref(path: &str) -> bool {
    let slash = path.replace('\\', "/");
    if slash.is_empty() || slash.chars().count() > 512 {
        return false;
    }
    if slash.starts_with('/') || slash.chars().nth(1) == Some(':') {
        return false;
    }
    slash
        .split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// True when `rel` names a file or directory inside `workspace` exactly as it
/// is on disk (KR-PATH-3/4, KR-LINK-1). Same predicate as the index.
pub fn workspace_file_exists(workspace: &std::path::Path, rel: &str) -> bool {
    let mut listings = super::fsref::Listings::default();
    matches!(
        super::fsref::lookup(workspace, rel, &mut listings).kind,
        super::fsref::Kind::File | super::fsref::Kind::Dir
    )
}

/// Every workspace Markdown link in prose, in appearance order. Repeats are kept.
/// Fenced blocks, inline code, images, web links, and `#` anchors are skipped.
pub fn extract_cites(markdown: &str) -> Vec<Cite> {
    let mut refs = Vec::new();
    for (line, prose) in scan_prose_with_source(markdown) {
        for hit in scan_links_in_line(&line, &prose) {
            if let Some(cite) = hit.cite {
                refs.push(cite);
            }
        }
    }
    refs
}

/// Rewrite workspace citations whose normalized path equals `from_path` to `to_path`.
/// Preserves any `#locator`, skips fenced blocks and inline code, and updates the
/// link label when it matched the old file stem or filename.
pub fn replace_citation_path(markdown: &str, from_path: &str, to_path: &str) -> String {
    let from_norm = normalize_ref_path(from_path);
    let to_norm = normalize_ref_path(to_path);
    if from_norm.is_empty() || from_norm == to_norm {
        return markdown.to_string();
    }
    let old_stem = super::document::file_stem(&from_norm);
    let new_stem = super::document::file_stem(&to_norm);
    let old_base = from_norm.rsplit('/').next().unwrap_or(&from_norm);
    let new_base = to_norm.rsplit('/').next().unwrap_or(&to_norm);
    rewrite_prose_links(markdown, |cite| {
        if cite.norm_path() != from_norm {
            return None;
        }
        let next_label = if cite.label == old_stem {
            new_stem.as_str()
        } else if cite.label == old_base {
            new_base
        } else {
            cite.label.as_str()
        };
        Some(rebuilt_citation(next_label, &to_norm, cite))
    })
}

/// Rewrite workspace citations whose normalized path is under `from_prefix/` to `to_prefix/`.
/// Skips fenced blocks and inline code.
pub fn replace_citation_path_prefix(markdown: &str, from_prefix: &str, to_prefix: &str) -> String {
    let from_clean = normalize_ref_path(from_prefix).trim_end_matches('/').to_string();
    let to_clean = normalize_ref_path(to_prefix).trim_end_matches('/').to_string();
    if from_clean.is_empty() || from_clean == to_clean {
        return markdown.to_string();
    }
    let prefix_slash = format!("{from_clean}/");
    rewrite_prose_links(markdown, |cite| {
        let norm = cite.norm_path();
        let rest = norm.strip_prefix(&prefix_slash)?;
        let next_path = if to_clean.is_empty() {
            rest.to_string()
        } else {
            format!("{to_clean}/{rest}")
        };
        Some(rebuilt_citation(&cite.label, &next_path, cite))
    })
}

fn rebuilt_citation(label: &str, next_path: &str, cite: &Cite) -> String {
    let mut dest = next_path.to_string();
    if let Some(sym) = cite.symbol.as_deref() {
        dest.push('#');
        dest.push_str(sym);
    } else if let Some(span) = cite.lines {
        dest.push('#');
        dest.push_str(&format_line_anchor(span));
    } else if let Some(bad) = cite.bad_lines.as_deref() {
        dest.push('#');
        dest.push_str(bad);
    }
    format!("[{label}]({})", format_link_dest(&dest))
}

fn rewrite_prose_links(markdown: &str, mut rewrite: impl FnMut(&Cite) -> Option<String>) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fence_char = '\0';
    let mut fence_len = 0usize;
    let lines: Vec<&str> = markdown.split('\n').collect();
    for (idx, raw) in lines.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let has_cr = raw.ends_with('\r');
        let line = raw.trim_end_matches('\r');
        if fence_len > 0 {
            if fence_closed(line, fence_char, fence_len) {
                fence_len = 0;
            }
            out.push_str(raw);
            continue;
        }
        if let Some((ch, len)) = fence_open(line) {
            fence_char = ch;
            fence_len = len;
            out.push_str(raw);
            continue;
        }
        let prose = mask_inline_code_same_bytes(line);
        let hits = scan_links_in_line(line, &prose);
        let mut last = 0usize;
        for hit in hits {
            if let Some(cite) = &hit.cite
                && let Some(replacement) = rewrite(cite)
            {
                out.push_str(&line[last..hit.start]);
                out.push_str(&replacement);
                last = hit.end;
            }
        }
        out.push_str(&line[last..]);
        if has_cr {
            out.push('\r');
        }
    }
    out
}

// ------------------------------------------------------------ link scanner

struct LinkHit {
    start: usize,
    end: usize,
    cite: Option<Cite>,
}

fn scan_links_in_line(line: &str, prose: &str) -> Vec<LinkHit> {
    let mut hits = Vec::new();
    let bytes = prose.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        // Skip Markdown images `![alt](...)` and escaped `\[`.
        if (i > 0 && bytes[i - 1] == b'!') || is_escaped(bytes, i) {
            i += 1;
            continue;
        }
        let Some(close_bracket) = find_closing_bracket(bytes, i + 1) else {
            i += 1;
            continue;
        };
        if close_bracket + 1 >= bytes.len() || bytes[close_bracket + 1] != b'(' {
            i += 1;
            continue;
        }
        let Some((dest_start, dest_end, close_paren)) =
            parse_link_destination(bytes, close_bracket + 2)
        else {
            i += 1;
            continue;
        };
        let label = &line[i + 1..close_bracket];
        let dest_raw = &line[dest_start..dest_end];
        let raw = &line[i..=close_paren];
        let cite = parse_workspace_dest(label, dest_raw, raw);
        hits.push(LinkHit {
            start: i,
            end: close_paren + 1,
            cite,
        });
        i = close_paren + 1;
    }
    hits
}

fn is_escaped(bytes: &[u8], pos: usize) -> bool {
    let mut backslashes = 0usize;
    let mut k = pos;
    while k > 0 && bytes[k - 1] == b'\\' {
        backslashes += 1;
        k -= 1;
    }
    backslashes % 2 == 1
}

fn find_closing_bracket(bytes: &[u8], from: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = from;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if b == b'[' {
            depth += 1;
        } else if b == b']' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Parse CommonMark link destination inside `(...)`. Returns `(dest_start, dest_end, close_paren)`.
fn parse_link_destination(bytes: &[u8], from: usize) -> Option<(usize, usize, usize)> {
    let mut i = from;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] == b')' {
        return None;
    }
    let (dest_start, dest_end, mut after_dest) = if bytes[i] == b'<' {
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() {
            if bytes[j] == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if bytes[j] == b'<' {
                return None;
            }
            if bytes[j] == b'>' {
                break;
            }
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'>' {
            return None;
        }
        (start, j, j + 1)
    } else {
        let start = i;
        let mut j = start;
        let mut paren_depth = 0usize;
        while j < bytes.len() {
            let b = bytes[j];
            if b == b'\\' && j + 1 < bytes.len() {
                j += 2;
                continue;
            }
            if b == b' ' || b == b'\t' {
                break;
            }
            if b == b'(' {
                paren_depth += 1;
            } else if b == b')' {
                if paren_depth == 0 {
                    break;
                }
                paren_depth -= 1;
            }
            j += 1;
        }
        (start, j, j)
    };
    if dest_end <= dest_start {
        return None;
    }
    while after_dest < bytes.len() && matches!(bytes[after_dest], b' ' | b'\t') {
        after_dest += 1;
    }
    // Optional CommonMark link title (`"..."` or `'...'`).
    if after_dest < bytes.len() && matches!(bytes[after_dest], b'"' | b'\'') {
        let quote = bytes[after_dest];
        after_dest += 1;
        while after_dest < bytes.len() {
            if bytes[after_dest] == b'\\' && after_dest + 1 < bytes.len() {
                after_dest += 2;
                continue;
            }
            if bytes[after_dest] == quote {
                after_dest += 1;
                break;
            }
            after_dest += 1;
        }
        while after_dest < bytes.len() && matches!(bytes[after_dest], b' ' | b'\t') {
            after_dest += 1;
        }
    }
    if after_dest < bytes.len() && bytes[after_dest] == b')' {
        Some((dest_start, dest_end, after_dest))
    } else {
        None
    }
}

fn parse_workspace_dest(label: &str, dest_raw: &str, raw: &str) -> Option<Cite> {
    let mut dest = dest_raw.trim();
    if dest.is_empty() || dest.starts_with("//") || dest.starts_with('#') {
        return None;
    }
    // Check URI scheme: 2+ char scheme before `/`, `?`, `#`.
    if let Some(scheme_end) = scheme_colon_index(dest) {
        let scheme = &dest[..scheme_end];
        if scheme.eq_ignore_ascii_case("file") {
            dest = &dest[scheme_end + 1..];
        } else if scheme.len() >= 2 {
            // External scheme (https:, http:, mailto:, tel:, data:, ...).
            return None;
        }
        // Single-letter scheme like `C:/foo` is a Windows drive path, NOT a web URL:
        // keep it so the validator reports `missing_file` (KR-PATH-1).
    }
    let (before_hash, frag_opt) = match dest.split_once('#') {
        Some((before, frag)) => (before, Some(frag)),
        None => (dest, None),
    };
    let path_raw = match before_hash.split_once('?') {
        Some((path, _)) => path,
        None => before_hash,
    };
    let path = percent_decode(path_raw).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let frag = frag_opt
        .map(percent_decode)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match frag {
        None => Some(Cite {
            kind: CiteKind::File,
            label: label.to_string(),
            path,
            symbol: None,
            lines: None,
            bad_lines: None,
            raw: raw.to_string(),
        }),
        Some(f) => {
            if let Some(span) = parse_line_anchor(&f) {
                Some(Cite {
                    kind: CiteKind::Range,
                    label: label.to_string(),
                    path,
                    symbol: None,
                    lines: Some(span),
                    bad_lines: None,
                    raw: raw.to_string(),
                })
            } else if is_line_like_anchor(&f) {
                Some(Cite {
                    kind: CiteKind::Range,
                    label: label.to_string(),
                    path,
                    symbol: None,
                    lines: None,
                    bad_lines: Some(f),
                    raw: raw.to_string(),
                })
            } else {
                Some(Cite {
                    kind: CiteKind::Symbol,
                    label: label.to_string(),
                    path,
                    symbol: Some(normalize_symbol_query(&f)),
                    lines: None,
                    bad_lines: None,
                    raw: raw.to_string(),
                })
            }
        }
    }
}

/// Index of `:` if `s` starts with an ASCII URI scheme (`[A-Za-z][A-Za-z0-9+.-]*:`).
fn scheme_colon_index(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    for (idx, &b) in bytes.iter().enumerate().skip(1) {
        if b == b':' {
            return Some(idx);
        }
        if !(b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-')) {
            return None;
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ------------------------------------------------------------ preview / facts

const PREVIEW_CODE_CHARS: usize = 24;

/// Replace workspace Markdown links in one line with readable text.
pub fn show_facts(text: &str) -> String {
    replace_links_with_labels(text)
}

/// First lines of a value, with workspace links reduced to their label.
/// Inline code stays visible, clipped, and is not scanned for citations.
/// Fenced blocks stay out.
pub fn knowledge_preview(value: &str, lines: usize) -> String {
    preview_lines(value)
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn preview_lines(markdown: &str) -> Vec<String> {
    let mut prose = Vec::new();
    let mut fence_char = '\0';
    let mut fence_len = 0usize;
    for raw in markdown.split('\n') {
        let line = raw.trim_end_matches('\r');
        if fence_len > 0 {
            if fence_closed(line, fence_char, fence_len) {
                fence_len = 0;
            }
            continue;
        }
        if let Some((ch, len)) = fence_open(line) {
            fence_char = ch;
            fence_len = len;
            continue;
        }
        prose.push(show_inline_code(line));
    }
    prose
}

fn show_inline_code(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            text.push(chars[i]);
            i += 1;
            continue;
        }
        let mut ticks = 0;
        while i + ticks < chars.len() && chars[i + ticks] == '`' {
            ticks += 1;
        }
        let Some(close_at) = find_closer(&chars, i + ticks, ticks) else {
            text.extend(chars[i..].iter().copied());
            break;
        };
        flush_preview_text(&mut text, &mut out);
        let content: String = chars[i + ticks..close_at].iter().collect();
        out.push('`');
        let count = content.chars().count();
        if count > PREVIEW_CODE_CHARS {
            out.extend(content.chars().take(PREVIEW_CODE_CHARS));
            out.push('…');
        } else {
            out.push_str(&content);
        }
        out.push('`');
        i = close_at + ticks;
    }
    flush_preview_text(&mut text, &mut out);
    out
}

fn flush_preview_text(text: &mut String, out: &mut String) {
    if text.is_empty() {
        return;
    }
    out.push_str(&replace_links_with_labels(text));
    text.clear();
}

fn replace_links_with_labels(text: &str) -> String {
    let prose = mask_inline_code_same_bytes(text);
    let hits = scan_links_in_line(text, &prose);
    if hits.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for hit in hits {
        let Some(cite) = hit.cite else {
            continue;
        };
        out.push_str(&text[last..hit.start]);
        let label = cite.label.trim();
        if !label.is_empty() {
            out.push_str(label);
        } else {
            out.push_str(&citation_fact(
                &cite.norm_path(),
                cite.symbol.as_deref(),
                cite.lines,
            ));
        }
        last = hit.end;
    }
    out.push_str(&text[last..]);
    out
}

// ------------------------------------------------------------ fence / inline code masking

/// Mask inline code spans with spaces while preserving exact UTF-8 byte offsets.
fn mask_inline_code_same_bytes(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let mut ticks = 0;
        while i + ticks < chars.len() && chars[i + ticks] == '`' {
            ticks += 1;
        }
        let Some(close_at) = find_closer(&chars, i + ticks, ticks) else {
            out.extend(chars[i..].iter());
            break;
        };
        for ch in &chars[i..close_at + ticks] {
            for _ in 0..ch.len_utf8() {
                out.push(' ');
            }
        }
        i = close_at + ticks;
    }
    out
}

fn find_closer(chars: &[char], from: usize, ticks: usize) -> Option<usize> {
    let mut i = from;
    while i + ticks <= chars.len() {
        if chars[i..i + ticks].iter().all(|ch| *ch == '`') {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Prose lines paired with their masked text (exact byte offsets kept).
fn scan_prose_with_source(markdown: &str) -> Vec<(String, String)> {
    let mut prose = Vec::new();
    let mut fence_char = '\0';
    let mut fence_len = 0usize;
    for raw in markdown.split('\n') {
        let line = raw.trim_end_matches('\r');
        if fence_len > 0 {
            if fence_closed(line, fence_char, fence_len) {
                fence_len = 0;
            }
            continue;
        }
        if let Some((ch, len)) = fence_open(line) {
            fence_char = ch;
            fence_len = len;
            continue;
        }
        prose.push((line.to_string(), mask_inline_code_same_bytes(line)));
    }
    prose
}

fn fence_open(line: &str) -> Option<(char, usize)> {
    let rest = line.trim_start_matches(' ');
    let indent = line.len() - rest.len();
    if indent > 3 {
        return None;
    }
    let bytes = rest.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let ch = bytes[0] as char;
    if ch != '`' && ch != '~' {
        return None;
    }
    let mut len = 0;
    while len < bytes.len() && bytes[len] as char == ch {
        len += 1;
    }
    if len >= 3 { Some((ch, len)) } else { None }
}

fn fence_closed(line: &str, fence_char: char, fence_len: usize) -> bool {
    let rest = line.trim_start_matches(' ');
    let indent = line.len() - rest.len();
    if indent > 3 {
        return false;
    }
    let mut len = 0;
    let bytes = rest.as_bytes();
    while len < bytes.len() && bytes[len] as char == fence_char {
        len += 1;
    }
    if len < fence_len {
        return false;
    }
    rest[len..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_markdown_links_and_skips_web_images_and_code() {
        let md = "See [seq](knowledge/contracts/seq.md), [a.rs](src/a.rs#L12-L30), \
                  [save](src/a.rs#Store::save), [sp](<文档/设计 说明.md>), \
                  [web](https://example.com), [sec](#intro), ![pic](assets/a.png), \
                  `[code](src/ignored.rs)`, and [@ key=\"legacy\"].";
        let cites = extract_cites(md);
        let keys: Vec<String> = cites.iter().map(|c| c.ref_key()).collect();
        assert_eq!(
            keys,
            vec![
                "path:knowledge/contracts/seq.md",
                "range:src/a.rs\012-30",
                "sym:src/a.rs\0Store.save",
                "path:文档/设计 说明.md",
            ]
        );
    }

    #[test]
    fn rewrites_paths_and_prefixes_outside_code() {
        let md = "See [old](knowledge/arch/old.md) and `[old](knowledge/arch/old.md)`.\n\
                  ```\n[old](knowledge/arch/old.md)\n```\n\
                  Also [custom](knowledge/arch/old.md#L5).";
        let renamed = replace_citation_path(md, "knowledge/arch/old.md", "knowledge/arch/new.md");
        assert!(renamed.contains("See [new](knowledge/arch/new.md)"));
        assert!(renamed.contains("`[old](knowledge/arch/old.md)`"));
        assert!(renamed.contains("```\n[old](knowledge/arch/old.md)\n```"));
        assert!(renamed.contains("[custom](knowledge/arch/new.md#L5)"));

        let moved = replace_citation_path_prefix(&renamed, "knowledge/arch", "knowledge/core/arch");
        assert!(moved.contains("See [new](knowledge/core/arch/new.md)"));
        assert!(moved.contains("[custom](knowledge/core/arch/new.md#L5)"));
    }
}
