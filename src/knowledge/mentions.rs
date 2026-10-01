//! Mention shortcodes in knowledge prose: `[@ id="seq" label="seq"]`.
//! Fenced blocks and inline code are not citations.

use std::sync::LazyLock;

use regex::Regex;

static KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$").expect("key pattern")
});

static SHORTCODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[@ id="([^"]*)" label="([^"]*)"\]"#).expect("shortcode pattern")
});

/// File attribute shortcode. `symbol` and `lines` are optional and, when present,
/// sit between `file` and `label` in that order. A match with neither is a file
/// citation; a match with either is a symbol or a line-range citation.
static FILE_ATTR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[@ file="([^"]*)"(?: symbol="([^"]*)")?(?: lines="([^"]*)")? label="([^"]*)"\]"#)
        .expect("file attr pattern")
});

pub fn normalize_key(key: &str) -> String {
    key.trim().to_string()
}

pub fn is_knowledge_key(key: &str) -> bool {
    KEY.is_match(key)
}

pub fn mention_source(id: &str, label: &str) -> String {
    format!(r#"[@ id="{id}" label="{label}"]"#)
}

/// Inclusive 1-based line span stored on a symbol citation.
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

/// `12` or `2148-2165`. Zero and an inverted range are not spans.
pub fn parse_line_span(raw: &str) -> Option<LineSpan> {
    let raw = raw.trim();
    let (start, end) = if let Some((start, end)) = raw.split_once('-') {
        (start.parse::<u32>().ok()?, end.parse::<u32>().ok()?)
    } else {
        let line = raw.parse::<u32>().ok()?;
        (line, line)
    };
    if start == 0 || end == 0 || end < start {
        return None;
    }
    Some(LineSpan { start, end })
}

/// One symbol or range citation as TipTap writes it.
///
/// Attribute order is `file`, `symbol`, `lines`, `label`. Absent `symbol` is a
/// range-only citation. Absent `lines` is a knowledge-body citation.
pub fn symbol_mention_source(
    file: &str,
    symbol: Option<&str>,
    lines: Option<LineSpan>,
    label: &str,
) -> String {
    let mut out = format!(r#"[@ file="{file}""#);
    if let Some(symbol) = symbol.map(str::trim).filter(|text| !text.is_empty()) {
        out.push_str(&format!(r#" symbol="{symbol}""#));
    }
    if let Some(span) = lines {
        out.push_str(&format!(r#" lines="{}""#, format_line_span(span)));
    }
    out.push_str(&format!(r#" label="{label}"]"#));
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
        lines: Option<LineSpan>,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub id: String,
    pub label: String,
}

fn file_label(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
}

fn capsule_label(path: &str, symbol: &str) -> String {
    let file = file_label(path);
    let name = symbol
        .rsplit(" › ")
        .next()
        .filter(|text| !text.is_empty())
        .unwrap_or(symbol);
    if file.is_empty() {
        return name.to_string();
    }
    if name.is_empty() {
        return file.to_string();
    }
    format!("{file} {name}")
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

/// True when `rel` is a file or directory inside `workspace`.
pub fn workspace_file_exists(workspace: &std::path::Path, rel: &str) -> bool {
    if !is_workspace_file_ref(rel) {
        return false;
    }
    workspace.join(rel.replace('\\', "/")).exists()
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
    let mut refs = Vec::new();
    for line in scan_prose(markdown) {
        let mut hits: Vec<(usize, OrderedRef)> = Vec::new();
        for caps in SHORTCODE.captures_iter(&line) {
            let id = normalize_key(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            if id.is_empty() || !is_knowledge_key(&id) {
                continue;
            }
            let label_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let label = if label_raw.is_empty() {
                id.clone()
            } else {
                label_raw.to_string()
            };
            let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            hits.push((start, OrderedRef::Node { id, label }));
        }
        for caps in FILE_ATTR.captures_iter(&line) {
            let Some(cite) = file_attr_ref(&caps) else {
                continue;
            };
            let start = caps.get(0).map(|m| m.start()).unwrap_or(0);
            hits.push((start, cite));
        }
        hits.sort_by_key(|(start, _)| *start);
        for (_, cite) in hits {
            refs.push(cite);
        }
    }
    refs
}

fn file_attr_ref(caps: &regex::Captures<'_>) -> Option<OrderedRef> {
    let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
    if path.is_empty() {
        return None;
    }
    let path = path.to_string();
    let symbol = caps
        .get(2)
        .map(|m| m.as_str().trim().to_string())
        .filter(|text| !text.is_empty());
    let lines = caps.get(3).and_then(|m| parse_line_span(m.as_str()));
    let label_raw = caps.get(4).map(|m| m.as_str()).unwrap_or("");
    let label = if label_raw.is_empty() {
        match symbol.as_deref().filter(|text| !text.is_empty()) {
            Some(chain) => capsule_label(&path, chain),
            None => file_label(&path).to_string(),
        }
    } else {
        label_raw.to_string()
    };
    if symbol.is_none() && lines.is_none() && caps.get(3).is_some() {
        // `lines` was written but does not parse. Keep the file so the path
        // is still a citation, and drop the broken range.
        return Some(OrderedRef::File { path, label });
    }
    if symbol.is_some() || lines.is_some() {
        return Some(OrderedRef::Symbol {
            path,
            symbol,
            lines,
            label,
        });
    }
    Some(OrderedRef::File { path, label })
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
        let id = normalize_key(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
        if id.is_empty() || !is_knowledge_key(&id) {
            continue;
        }
        let start = mat.start();
        if start > last {
            out.push(Segment::Text(text[last..start].to_string()));
        }
        let label_raw = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        let label = if label_raw.is_empty() {
            id.clone()
        } else {
            label_raw.to_string()
        };
        out.push(Segment::Ref { id, label });
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

/// Rewrite mention ids equal to `from`. A label equal to the old id is rewritten too.
pub fn replace_mention_key(text: &str, from: &str, to: &str) -> String {
    let source = normalize_key(from);
    let target = normalize_key(to);
    if source.is_empty() || source == target {
        return text.to_string();
    }
    SHORTCODE
        .replace_all(text, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let id = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let label = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let id_key = normalize_key(id);
            let label_key = normalize_key(label);
            if id_key != source && label_key != source {
                return full.to_string();
            }
            let next_id = if id_key == source {
                target.as_str()
            } else {
                id
            };
            let next_label = if label_key == source {
                target.as_str()
            } else {
                label
            };
            mention_source(next_id, next_label)
        })
        .into_owned()
}

const PREVIEW_CODE_CHARS: usize = 24;

/// First lines of a value, with mentions reduced to their label.
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
    out.push_str(&replace_shortcodes_with_labels(text));
    text.clear();
}

fn replace_shortcodes_with_labels(text: &str) -> String {
    let nodes = SHORTCODE
        .replace_all(text, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let id = normalize_key(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
            if !is_knowledge_key(&id) {
                return full.to_string();
            }
            let label = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if label.is_empty() {
                id
            } else {
                label.to_string()
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
            let label = caps.get(4).map(|m| m.as_str()).unwrap_or("");
            if label.is_empty() {
                let symbol = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
                if symbol.is_empty() {
                    file_label(path).to_string()
                } else {
                    capsule_label(path, symbol)
                }
            } else {
                label.to_string()
            }
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
    use super::*;

    #[test]
    fn preview_keeps_inline_code_and_hides_fences() {
        let hidden = mention_source("hidden", "hidden");
        let shown = mention_source("seq", "序号");
        let value = format!("see `node : seq` and {shown}\n```\n{hidden}\n```\nbeta");
        assert_eq!(
            knowledge_preview(&value, 3),
            "see `node : seq` and 序号\nbeta"
        );
        assert_eq!(extract_markers(&value), vec!["seq".to_string()]);
        let inside = format!("`{hidden}`");
        assert!(knowledge_preview(&inside, 1).contains("id=\"hidden\""));
        assert!(extract_markers(&inside).is_empty());
    }

    #[test]
    fn file_refs_skip_code_and_reject_escapes() {
        let value = "\
see [@ file=\"src/a.rs\" label=\"a.rs\"]
```
[@ file=\"skip.rs\" label=\"skip.rs\"]
```
`[@ file=\"nope.rs\" label=\"nope.rs\"]`
[@ file=\"../secret\" label=\"secret\"]
[@ file=\"/etc/passwd\" label=\"passwd\"]
";
        assert_eq!(
            extract_file_refs(value),
            vec![
                "src/a.rs".to_string(),
                "../secret".to_string(),
                "/etc/passwd".to_string(),
            ]
        );
        assert!(extract_markers(value).is_empty());
        assert!(is_workspace_file_ref("src/a.rs"));
        assert!(is_workspace_file_ref("src"));
        assert!(!is_workspace_file_ref("../secret"));
        assert!(!is_workspace_file_ref("/etc/passwd"));
        assert!(!is_workspace_file_ref("C:/abs"));
        assert!(!is_workspace_file_ref("src/../a.rs"));
        let within = "文".repeat(512);
        let over = "文".repeat(513);
        assert!(is_workspace_file_ref(&within));
        assert!(!is_workspace_file_ref(&over));
    }

    #[test]
    fn an_empty_symbol_label_uses_the_file_name_and_the_last_hop() {
        let source = r#"[@ file="src/a.rs" symbol="impl Store › fn save" label=""]"#;
        let refs = extract_refs_in_order(source);
        assert_eq!(
            refs,
            vec![OrderedRef::Symbol {
                path: "src/a.rs".into(),
                symbol: Some("impl Store › fn save".into()),
                lines: None,
                label: "a.rs fn save".into(),
            }]
        );
    }

    #[test]
    fn symbol_refs_keep_order_and_skip_code() {
        let symbol = symbol_mention_source(
            "src/session/manager.rs",
            Some("impl SessionManager › fn append_reminder"),
            Some(LineSpan {
                start: 2148,
                end: 2165,
            }),
            "fn append_reminder",
        );
        let range = symbol_mention_source(
            "src/a.rs",
            None,
            Some(LineSpan { start: 4, end: 4 }),
            "a.rs",
        );
        let knowledge = symbol_mention_source("src/a.rs", Some("fn alpha"), None, "fn alpha");
        let value = format!(
            "see {symbol} then {} and {knowledge}\n```\n{symbol}\n```\n`{range}`",
            mention_source("seq", "序号")
        );
        assert_eq!(
            extract_refs_in_order(&value),
            vec![
                OrderedRef::Symbol {
                    path: "src/session/manager.rs".into(),
                    symbol: Some("impl SessionManager › fn append_reminder".into()),
                    lines: Some(LineSpan {
                        start: 2148,
                        end: 2165,
                    }),
                    label: "fn append_reminder".into(),
                },
                OrderedRef::Node {
                    id: "seq".into(),
                    label: "序号".into(),
                },
                OrderedRef::Symbol {
                    path: "src/a.rs".into(),
                    symbol: Some("fn alpha".into()),
                    lines: None,
                    label: "fn alpha".into(),
                },
            ]
        );
        assert_eq!(
            extract_file_refs(&value),
            vec!["src/session/manager.rs".to_string(), "src/a.rs".to_string(),]
        );
        assert_eq!(knowledge_preview(&symbol, 1), "fn append_reminder");
        assert_eq!(
            symbol_mention_source(
                "src/a.rs",
                None,
                Some(LineSpan { start: 4, end: 9 }),
                "a.rs"
            ),
            r#"[@ file="src/a.rs" lines="4-9" label="a.rs"]"#
        );
        assert!(parse_line_span("0-1").is_none());
        assert!(parse_line_span("9-4").is_none());
        assert_eq!(parse_line_span("12"), Some(LineSpan { start: 12, end: 12 }));
    }

    #[test]
    fn preview_clips_long_inline_code() {
        let long = "a".repeat(30);
        let value = format!("`{long}`");
        assert_eq!(
            knowledge_preview(&value, 1),
            format!("`{}…`", "a".repeat(24))
        );
    }
}
