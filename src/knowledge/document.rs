//! Read and write one knowledge node file.
//!
//! The declaration is a `node` fence at the start of the file. Field lines this
//! parser does not own are kept and written back after the known fields.

use std::sync::LazyLock;

use regex::Regex;

use super::mentions;

static OPEN_FENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[ \t]*\r?\n)*```node[ \t]*\r?\n").expect("open fence")
});
static CLOSE_FENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\r?\n```[ \t]*(?:\r?\n|$)").expect("close fence"));
static FIELD_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*?)\s*$").expect("field line")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Enabled,
    Disabled,
    Pending,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::Pending => "pending",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            "pending" => Some(Self::Pending),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFile {
    pub path: String,
    pub key: String,
    pub status: Status,
    /// Raw `status` text when it is not `enabled`, `disabled`, or `pending`.
    pub invalid_status: Option<String>,
    pub summary: String,
    pub refs: Vec<String>,
    pub body: String,
    pub folder_id: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub extras: Vec<String>,
}

pub fn parse_knowledge_markdown(path: &str, markdown: &str) -> ParsedFile {
    let path = normalize_rel(path);
    let source = markdown.trim_start_matches('\u{feff}');
    let mut key = String::new();
    let mut status = Status::Enabled;
    let mut invalid_status = None;
    let mut summary = String::new();
    let mut body = source.to_string();
    let mut x = None;
    let mut y = None;
    let mut w = None;
    let mut h = None;
    let mut extras = Vec::new();

    if let Some(open) = OPEN_FENCE.find(source) {
        let after_open = &source[open.end()..];
        if let Some(close) = CLOSE_FENCE.find(after_open) {
            let block = &after_open[..close.start()];
            body = strip_one_newline(&after_open[close.end()..]).to_string();
            let mut saw_node = false;
            for line in block.split('\n') {
                let trimmed = line.trim().trim_end_matches('\r').trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Some(field) = FIELD_LINE.captures(trimmed) else {
                    extras.push(trimmed.to_string());
                    continue;
                };
                let name = field.get(1).map(|m| m.as_str()).unwrap_or("");
                let value = field.get(2).map(|m| m.as_str()).unwrap_or("").trim();
                if name == "node" && !saw_node {
                    saw_node = true;
                    key = value.to_string();
                } else if name == "status" {
                    if let Some(parsed) = Status::parse(value) {
                        status = parsed;
                        invalid_status = None;
                    } else {
                        invalid_status = Some(value.to_string());
                    }
                } else if name == "summary" {
                    summary = value.to_string();
                } else if name == "ref" {
                    // Ignored. Not a citation and not written back.
                } else if name == "x" {
                    x = parse_coord(value);
                } else if name == "y" {
                    y = parse_coord(value);
                } else if name == "w" {
                    w = parse_coord(value);
                } else if name == "h" {
                    h = parse_coord(value);
                } else {
                    extras.push(trimmed.to_string());
                }
            }
        }
    }

    let refs = mentions::extract_markers(&body);
    ParsedFile {
        path: path.clone(),
        folder_id: folder_id_of(&path),
        key,
        status,
        invalid_status,
        summary,
        refs,
        body,
        x,
        y,
        w,
        h,
        extras,
    }
}

fn strip_one_newline(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("\r\n") {
        rest
    } else if let Some(rest) = text.strip_prefix('\n') {
        rest
    } else if let Some(rest) = text.strip_prefix('\r') {
        rest
    } else {
        text
    }
}

pub struct RenderDoc<'a> {
    pub key: &'a str,
    pub status: Status,
    pub invalid_status: Option<&'a str>,
    pub body: &'a str,
    pub summary: &'a str,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub extras: &'a [String],
}

pub fn render_knowledge_markdown(doc: RenderDoc<'_>) -> String {
    let body = doc.body.trim_end();
    let mut lines = vec![
        "```node".to_string(),
        format!("node : {}", doc.key),
        format!(
            "status : {}",
            doc.invalid_status.unwrap_or_else(|| doc.status.as_str())
        ),
        format!("summary : {}", doc.summary),
    ];
    for (name, value) in [("x", doc.x), ("y", doc.y), ("w", doc.w), ("h", doc.h)] {
        if let Some(line) = coord_line(name, value) {
            lines.push(line);
        }
    }
    for extra in doc.extras {
        let trimmed = extra.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    lines.push("```".to_string());
    lines.push(String::new());
    lines.push(body.to_string());
    lines.push(String::new());
    lines.join("\n")
}

fn coord_line(name: &str, value: Option<f64>) -> Option<String> {
    let value = value.filter(|n| n.is_finite())?;
    Some(format!("{name} : {}", value.round() as i64))
}

fn parse_coord(value: &str) -> Option<f64> {
    if value.is_empty() {
        return None;
    }
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

pub fn file_stem(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    if base.len() >= 3 && base[base.len() - 3..].eq_ignore_ascii_case(".md") {
        base[..base.len() - 3].to_string()
    } else {
        base.to_string()
    }
}

fn folder_id_of(file_path: &str) -> Option<String> {
    let slash = file_path.rfind('/')?;
    if slash == 0 {
        return None;
    }
    Some(file_path[..slash].to_string())
}
