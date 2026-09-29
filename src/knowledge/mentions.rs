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

static FILE_SHORTCODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[@ file="([^"]*)" label="([^"]*)"\]"#).expect("file shortcode pattern")
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub id: String,
    pub label: String,
}

/// Workspace-relative path. `..`, `.`, an empty segment, and an absolute path are not.
pub fn is_workspace_file_ref(path: &str) -> bool {
    let slash = path.replace('\\', "/");
    if slash.is_empty() || slash.len() > 512 {
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

/// File paths cited in prose. Fenced blocks and inline code are not citations.
pub fn extract_file_refs(markdown: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in scan_prose(markdown) {
        for caps in FILE_SHORTCODE.captures_iter(&line) {
            let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if path.is_empty() || !seen.insert(path.to_string()) {
                continue;
            }
            paths.push(path.to_string());
        }
    }
    paths
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
            let next_id = if id_key == source { target.as_str() } else { id };
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
            if label.is_empty() { id } else { label.to_string() }
        })
        .into_owned();
    FILE_SHORTCODE
        .replace_all(&nodes, |caps: &regex::Captures| {
            let full = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let path = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
            if path.is_empty() {
                return full.to_string();
            }
            let label = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            if label.is_empty() {
                path.rsplit('/').next().unwrap_or(path).to_string()
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
        assert_eq!(knowledge_preview(&value, 3), "see `node : seq` and 序号\nbeta");
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
