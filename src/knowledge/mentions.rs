//! Citations in knowledge prose: `[@ key="seq"]`, `[@ file="src/a.rs"]`.
//! Fenced blocks and inline code are not citations.

use std::sync::LazyLock;

use regex::Regex;

static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$").expect("key pattern")
});

/// Node citation. Only `key` is recognized.
static SHORTCODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[@ key="([^"]*)"\]"#).expect("node citation"));

/// File citation. `symbol` and `lines` are optional, in that order.
/// A match with neither symbol nor lines is a file. A match with `symbol` is a
/// symbol citation; one with only `lines` is a line range. A symbol citation
/// never takes `lines` (KR-SYM-6): the attribute is still matched so the
/// citation is reported as `invalid_lines` instead of turning into plain text.
static FILE_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[@ file="([^"]*)"(?: symbol="([^"]*)")?(?: lines="([^"]*)")?\]"#)
        .expect("file citation")
});

pub fn normalize_key(key: &str) -> String {
    key.trim().to_string()
}

pub fn is_knowledge_key(key: &str) -> bool {
    KEY.is_match(key)
}

pub fn mention_source(key: &str) -> String {
    format!(r#"[@ key="{key}"]"#)
}

/// Words shown for a file or symbol citation. A node shows its key.
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

/// Inclusive 1-based line span of a range citation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    pub start: u32,
    pub end: u32,
}

pub fn format_line_span(span: LineSpan) -> String {
    if span.start == span.end {
        format!("{}", span.start)
    } else {
        format!("{}-{}", span.start, span.end)
    }
}

/// `12` or `2148-2165` (KR-SYN-5). The only valid forms are `A` and `A-B`:
/// ASCII decimal digits, no sign, no spaces, no leading zero, `1 <= A <= B`,
/// each fitting `u32`. Anything else is not a span (`invalid_lines`).
pub fn parse_line_span(raw: &str) -> Option<LineSpan> {
    let (start, end) = match raw.split_once('-') {
        Some((start, end)) => (parse_line_number(start)?, parse_line_number(end)?),
        None => {
            let line = parse_line_number(raw)?;
            (line, line)
        }
    };
    if end < start {
        return None;
    }
    Some(LineSpan { start, end })
}

fn parse_line_number(text: &str) -> Option<u32> {
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok()
}

/// One symbol or range citation. With a `symbol` it is a symbol citation and
/// `lines` is never written (KR-SYM-6). Without one, `lines` makes a line range.
pub fn symbol_mention_source(file: &str, symbol: Option<&str>, lines: Option<LineSpan>) -> String {
    let mut out = format!(r#"[@ file="{file}""#);
    if let Some(symbol) = symbol.map(str::trim).filter(|text| !text.is_empty()) {
        out.push_str(&format!(r#" symbol="{symbol}""#));
    } else if let Some(span) = lines {
        out.push_str(&format!(r#" lines="{}""#, format_line_span(span)));
    }
    out.push(']');
    out
}

/// A citation in the order it appears in prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderedRef {
    Node {
        id: String,
        label: String,
    },
    File {
        path: String,
        label: String,
    },
    Symbol {
        path: String,
        symbol: Option<String>,
        /// Span of a range citation. Always `None` with a `symbol` (KR-SYM-6).
        lines: Option<LineSpan>,
        /// Raw `lines` text when the attribute was written but is not allowed:
        /// not a valid span on a range (KR-SYN-5), or any `lines` at all on a
        /// symbol citation (KR-SYM-6). The citation stays a citation and
        /// reports `invalid_lines`.
        bad_lines: Option<String>,
        label: String,
    },
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

/// One citation with its source text, in appearance order (KR-SYN-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cite {
    pub kind: CiteKind,
    /// Node citation key, trimmed.
    pub key: Option<String>,
    /// File path as written, trimmed (backslashes kept; see [`Cite::norm_path`]).
    pub path: Option<String>,
    pub symbol: Option<String>,
    /// Valid span of a range citation. Always `None` on a symbol citation.
    pub lines: Option<LineSpan>,
    /// Raw `lines` text that is not allowed: an invalid span on a range
    /// (KR-SYN-5) or any `lines` on a symbol citation (KR-SYM-6).
    pub bad_lines: Option<String>,
    /// The citation text exactly as it appears in the markdown.
    pub raw: String,
}

impl Cite {
    /// KR-PATH-2: `\` is `/`.
    pub fn norm_path(&self) -> Option<String> {
        self.path.as_deref().map(normalize_ref_path)
    }

    /// Shared identity of a citation inside one node (KR-KEY-1):
    /// `node:<key>`, `file:<path>`, `sym:<path>\0<chain>`, `range:<path>\0<lines>`.
    pub fn ref_key(&self) -> String {
        let path = self.norm_path().unwrap_or_default();
        match self.kind {
            CiteKind::Node => format!("node:{}", self.key.as_deref().unwrap_or("")),
            CiteKind::File => format!("file:{path}"),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub id: String,
    pub label: String,
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

/// File paths cited in prose, including the file of a symbol citation.
/// Fenced blocks and inline code are not citations.
pub fn extract_file_refs(markdown: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in scan_prose(markdown) {
        for caps in FILE_ATTR.captures_iter(&line) {
            let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if path.is_empty() || !seen.insert(path.to_string()) {
                continue;
            }
            paths.push(path.to_string());
        }
    }
    paths
}

/// Citations in prose, in appearance order. Repeats are kept.
/// Fenced blocks and inline code are not citations.
pub fn extract_refs_in_order(markdown: &str) -> Vec<OrderedRef> {
    extract_cites(markdown)
        .into_iter()
        .map(|cite| match cite.kind {
            CiteKind::Node => {
                let id = cite.key.unwrap_or_default();
                OrderedRef::Node {
                    label: id.clone(),
                    id,
                }
            }
            CiteKind::File => {
                let path = cite.path.unwrap_or_default();
                OrderedRef::File {
                    label: path.clone(),
                    path,
                }
            }
            CiteKind::Symbol | CiteKind::Range => {
                let path = cite.path.unwrap_or_default();
                let label = citation_fact(&path, cite.symbol.as_deref(), cite.lines);
                OrderedRef::Symbol {
                    path,
                    symbol: cite.symbol,
                    lines: cite.lines,
                    bad_lines: cite.bad_lines,
                    label,
                }
            }
        })
        .collect()
}

/// Citations with their source text, in appearance order. Repeats are kept.
pub fn extract_cites(markdown: &str) -> Vec<Cite> {
    let mut refs = Vec::new();
    for (line, prose) in scan_prose_with_source(markdown) {
        let mut hits: Vec<(usize, Cite)> = Vec::new();
        for caps in SHORTCODE.captures_iter(&prose) {
            let Some(id) = node_caps(&caps) else {
                continue;
            };
            let whole = caps.get(0).expect("full match");
            hits.push((
                whole.start(),
                Cite {
                    kind: CiteKind::Node,
                    key: Some(id),
                    path: None,
                    symbol: None,
                    lines: None,
                    bad_lines: None,
                    raw: line[whole.range()].to_string(),
                },
            ));
        }
        for caps in FILE_ATTR.captures_iter(&prose) {
            let whole = caps.get(0).expect("full match");
            let Some(cite) = file_attr_cite(&caps, &line[whole.range()]) else {
                continue;
            };
            hits.push((whole.start(), cite));
        }
        hits.sort_by_key(|(start, _)| *start);
        refs.extend(hits.into_iter().map(|(_, cite)| cite));
    }
    refs
}

fn file_attr_cite(caps: &regex::Captures<'_>, raw: &str) -> Option<Cite> {
    let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
    if path.is_empty() {
        return None;
    }
    let symbol = caps
        .get(2)
        .map(|m| m.as_str().trim().to_string())
        .filter(|text| !text.is_empty());
    let lines_raw = caps.get(3).map(|m| m.as_str());
    // KR-SYM-6: a symbol citation locates by chain only; any `lines` on it is
    // a format error, never a span.
    let lines = if symbol.is_some() {
        None
    } else {
        lines_raw.and_then(parse_line_span)
    };
    let bad_lines = match (lines_raw, lines) {
        (Some(raw), None) => Some(raw.to_string()),
        _ => None,
    };
    let kind = if symbol.is_some() {
        CiteKind::Symbol
    } else if lines_raw.is_some() {
        CiteKind::Range
    } else {
        CiteKind::File
    };
    Some(Cite {
        kind,
        key: None,
        path: Some(path.to_string()),
        symbol,
        lines,
        bad_lines,
        raw: raw.to_string(),
    })
}

pub fn extract_markers(markdown: &str) -> Vec<String> {
    extract_mentions(markdown)
        .into_iter()
        .map(|mention| mention.id)
        .collect()
}

pub fn extract_mentions(markdown: &str) -> Vec<Mention> {
    let mut mentions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in scan_prose(markdown) {
        for segment in split_refs(&line) {
            if let Segment::Ref { id, label } = segment {
                if seen.insert(id.clone()) {
                    mentions.push(Mention { id, label });
                }
            }
        }
    }
    mentions
}

#[derive(Debug, PartialEq, Eq)]
enum Segment {
    Text(String),
    Ref { id: String, label: String },
}

fn split_refs(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut last = 0;
    for caps in SHORTCODE.captures_iter(text) {
        let mat = caps.get(0).expect("full match");
        let Some(id) = node_caps(&caps) else {
            continue;
        };
        let start = mat.start();
        if start > last {
            out.push(Segment::Text(text[last..start].to_string()));
        }
        out.push(Segment::Ref {
            id: id.clone(),
            label: id,
        });
        last = mat.end();
    }
    if last < text.len() {
        out.push(Segment::Text(text[last..].to_string()));
    }
    if out.is_empty() {
        out.push(Segment::Text(text.to_string()));
    }
    out
}

/// Rewrite a node citation whose key is `from` into `[@ key="to"]`.
pub fn replace_mention_key(text: &str, from: &str, to: &str) -> String {
    let source = normalize_key(from);
    let target = normalize_key(to);
    if source.is_empty() || source == target {
        return text.to_string();
    }
    SHORTCODE
        .replace_all(text, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let Some(id) = node_caps(caps) else {
                return full.to_string();
            };
            if id != source {
                return full.to_string();
            }
            mention_source(&target)
        })
        .into_owned()
}

fn node_caps(caps: &regex::Captures<'_>) -> Option<String> {
    let key = normalize_key(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
    if key.is_empty() || !is_knowledge_key(&key) {
        return None;
    }
    Some(key)
}

const PREVIEW_CODE_CHARS: usize = 24;

/// First lines of a value, with mentions reduced to their label.
/// Inline code stays visible, clipped, and is not scanned for citations.
/// Fenced blocks stay out.
/// Replace citations in one line with the words a person sees.
pub fn show_facts(text: &str) -> String {
    replace_shortcodes_with_labels(text)
}

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
    out.push_str(&replace_shortcodes_with_labels(text));
    text.clear();
}

fn replace_shortcodes_with_labels(text: &str) -> String {
    let nodes = SHORTCODE
        .replace_all(text, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            match node_caps(caps) {
                Some(id) => id,
                None => full.to_string(),
            }
        })
        .into_owned();
    FILE_ATTR
        .replace_all(&nodes, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if path.is_empty() {
                return full.to_string();
            }
            let symbol = caps
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .filter(|text| !text.is_empty());
            // KR-SYM-6: a symbol citation shows no line span.
            let lines = if symbol.is_some() {
                None
            } else {
                caps.get(3).and_then(|m| parse_line_span(m.as_str()))
            };
            citation_fact(path, symbol.as_deref(), lines)
        })
        .into_owned()
}

fn mask_inline_code(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
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
        let close_at = find_closer(&chars, i + ticks, ticks);
        if close_at.is_none() {
            for ch in &chars[i..] {
                out.push(*ch);
            }
            break;
        }
        let close_at = close_at.expect("checked");
        let span = close_at + ticks - i;
        for _ in 0..span {
            out.push(' ');
        }
        i = close_at + ticks;
    }
    out
}

/// [`mask_inline_code`] that keeps every byte offset: a masked character
/// becomes as many spaces as it has UTF-8 bytes.
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

/// Prose lines paired with their source text. Masking keeps byte offsets,
/// so a match range in the masked line slices the source line.
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

fn scan_prose(markdown: &str) -> Vec<String> {
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
        prose.push(mask_inline_code(line));
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
    

}
