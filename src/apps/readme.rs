//! README excerpts from raw.githubusercontent.com only.
//!
//! The excerpt is shown as markdown, so raw HTML at the top of many upstream
//! READMEs (logos, badges, centered intro blocks) is reduced to its text:
//! react-markdown skips HTML nodes and would otherwise render a blank panel.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use regex::Regex;

const MAX_README_BYTES: u64 = 512 * 1024;
const MAX_EXCERPT_CHARS: usize = 800;
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

pub struct ReadmeCache {
    entries: Mutex<HashMap<String, (Instant, String)>>,
    client: reqwest::blocking::Client,
}

impl ReadmeCache {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        Self {
            entries: Mutex::new(HashMap::new()),
            client,
        }
    }

    pub fn fetch(&self, url: &str) -> Option<String> {
        if !url.starts_with("https://raw.githubusercontent.com/") {
            return None;
        }
        if let Ok(cache) = self.entries.lock() {
            if let Some((at, text)) = cache.get(url) {
                if at.elapsed() < CACHE_TTL {
                    return Some(text.clone());
                }
            }
        }
        let response = self.client.get(url).send().ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_README_BYTES)
            .read_to_end(&mut bytes)
            .ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let excerpt = excerpt(&text);
        if excerpt.is_empty() {
            return None;
        }
        if let Ok(mut cache) = self.entries.lock() {
            cache.insert(url.to_string(), (Instant::now(), excerpt.clone()));
        }
        Some(excerpt)
    }
}

fn excerpt(text: &str) -> String {
    let text = comment_re().replace_all(text, "");
    let mut lines: Vec<String> = Vec::new();
    let mut in_fence = false;
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            lines.push(raw.trim_end().to_string());
            continue;
        }
        if in_fence {
            lines.push(raw.trim_end().to_string());
            continue;
        }
        let mut line = image_link_re().replace_all(raw, "").into_owned();
        line = image_re().replace_all(&line, "").into_owned();
        line = empty_link_re().replace_all(&line, "").into_owned();
        line = br_re().replace_all(&line, "\n").into_owned();
        line = block_close_re().replace_all(&line, "\n").into_owned();
        line = autolink_re().replace_all(&line, "$1").into_owned();
        line = tag_re().replace_all(&line, "").into_owned();
        let line = decode_entities(&line);
        if line.trim().is_empty() {
            if !lines.last().is_some_and(|line| line.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        for piece in line.lines() {
            let piece = piece.trim_end();
            if piece.trim().is_empty() {
                if lines.last().is_some_and(|line| line.is_empty()) {
                    continue;
                }
                lines.push(String::new());
            } else {
                // Stripped HTML wrappers leave their indentation behind. A
                // four-space remainder would turn into a code block, so the
                // preview flattens it; fenced blocks keep their content.
                lines.push(piece.trim_start().to_string());
            }
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let joined = lines.join("\n");
    joined
        .trim()
        .chars()
        .take(MAX_EXCERPT_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn comment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("readme comment regex"))
}

fn image_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\[!\[[^\]]*\]\([^)]*\)\]\([^)]*\)").expect("readme image link regex")
    })
}

fn image_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("readme image regex"))
}

fn empty_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\]\([^)]*\)").expect("readme empty link regex"))
}

fn br_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)<br\s*/?>").expect("readme br regex"))
}

fn block_close_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)</(?:p|div|h[1-6]|li|tr|ul|ol|table|blockquote|picture|section|header|footer)>",
        )
        .expect("readme block close regex")
    })
}

fn autolink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<((?:https?|mailto):[^>\s]+)>").expect("readme autolink regex"))
}

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)</?[a-zA-Z][^>]*>").expect("readme tag regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_html_but_keeps_text() {
        let text = "<h1 align=\"center\">\n<div>\n  <a href=\"https://example.com\">\n    <img src=\"logo.png\" />\n  </a>\n</div>\n</h1>\n\n<p align=\"center\">Fast databases.</p>\n";
        assert_eq!(excerpt(text), "Fast databases.");
    }

    #[test]
    fn removes_badges_and_images() {
        let text =
            "# OpenRefine\n\n[![DOI](https://img.example/x.svg)](https://doi.example)\n![Shot](https://img.example/shot.png)\n\nA tool.";
        assert_eq!(excerpt(text), "# OpenRefine\n\nA tool.");
    }

    #[test]
    fn drops_empty_links_left_by_stripped_images() {
        let text = "Text\n\n[](docs/readme/chinese.md)\n[](docs/readme/french.md)\n";
        assert_eq!(excerpt(text), "Text");
    }

    #[test]
    fn keeps_code_fences_and_autolinks() {
        let text = "Run it.\n\n```html\n<div>keep</div>\n```\n\nSee <https://example.com/docs>.";
        let out = excerpt(text);
        assert!(out.contains("<div>keep</div>"));
        assert!(out.contains("https://example.com/docs"));
    }

    #[test]
    fn strips_indentation_left_by_html_wrappers() {
        let text = "<p align=\"center\">\n    <a href=\"x\"><b>Website</b></a> ·\n</p>\n";
        assert_eq!(excerpt(text), "Website ·");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(
            excerpt("Audio &amp; waveform&nbsp;editing"),
            "Audio & waveform editing"
        );
    }
}
