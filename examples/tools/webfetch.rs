use regex::Regex;
use serde_json::Value;

use litecode::context_pipeline::Context;
use litecode::tool::Tool;
use litecode::types::ToolCallResult;

pub struct WebFetchTool;

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "description": "Desired output format (default: markdown)"
                }
            },
            "required": ["url"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let url_str = match litecode::tool::require_nonempty_string_trimmed(&input, "url") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };

        let _url: url::Url = match url_str.parse() {
            Ok(u) => u,
            Err(e) => return ToolCallResult::error(format!("invalid url: {}", e)),
        };

        let format = input["format"].as_str().unwrap_or("markdown");

        let mut curl = std::process::Command::new("curl");
        curl.arg("-sL").arg("--max-time").arg("30");
        if url_str.contains("127.0.0.1") || url_str.contains("localhost") {
            curl.arg("--noproxy").arg("*");
        }
        let result = match curl.arg(url_str).output()
        {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                return ToolCallResult::error(format!("curl failed: {}", stderr.trim()));
            }
            Err(e) => return ToolCallResult::error(format!("curl error: {}", e)),
        };

        let formatted = match format {
            "html" => result,
            "text" => strip_html_tags(&result),
            _ => html_to_markdown(&result),
        };

        let truncated = if formatted.len() > 50000 {
            format!(
                "{}... [truncated {} bytes]",
                &formatted[..50000],
                formatted.len() - 50000
            )
        } else {
            formatted
        };

        ToolCallResult::ok(truncated)
    }

    fn max_result_size(&self) -> usize {
        50000
    }

    fn description(&self, _ctx: &Context) -> String {
        "Fetch content from a URL. Supports html, text, and markdown output formats.".into()
    }
}

/// Strip all HTML tags, returning plain text.
fn strip_html_tags(html: &str) -> String {
    let re = Regex::new("<[^>]*>").expect("valid regex for HTML tag stripping");
    let text = re.replace_all(html, "").to_string();
    // Collapse multiple whitespace/newlines
    collapse_whitespace(&text)
}

/// Simple HTML-to-markdown conversion: strip tags, preserve some structure.
fn html_to_markdown(html: &str) -> String {
    let mut text = html.to_string();

    // Preserve headings: <h1>...</h1> → # ...
    // regex crate doesn't support backreferences, match each level separately
    for level in 1..=6 {
        let tag = format!("h{}", level);
        let re = Regex::new(&format!(r"(?i)<{tag}[^>]*>(.*?)</{tag}>")).expect("valid regex");
        let hashes = "#".repeat(level);
        text = re
            .replace_all(&text, |caps: &regex::Captures| {
                format!("{} {}", hashes, caps[1].trim())
            })
            .to_string();
    }

    // <li> → - item
    let li_re = Regex::new(r"(?i)<li[^>]*>").expect("valid regex");
    text = li_re.replace_all(&text, "- ").to_string();

    // <br> and <br/> → newline
    let br_re = Regex::new(r"(?i)<br\s*/?>").expect("valid regex");
    text = br_re.replace_all(&text, "\n").to_string();

    // <p> → double newline
    let p_re = Regex::new(r"(?i)</p>").expect("valid regex");
    text = p_re.replace_all(&text, "\n\n").to_string();

    // Strip all remaining tags
    let tag_re = Regex::new("<[^>]*>").expect("valid regex");
    text = tag_re.replace_all(&text, "").to_string();

    // Decode common HTML entities
    text = text.replace("&amp;", "&");
    text = text.replace("&lt;", "<");
    text = text.replace("&gt;", ">");
    text = text.replace("&quot;", "\"");
    text = text.replace("&#39;", "'");
    text = text.replace("&nbsp;", " ");

    collapse_whitespace(&text)
}

/// Collapse multiple blank lines into at most two newlines,
/// and trim trailing whitespace from lines.
fn collapse_whitespace(text: &str) -> String {
    let lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
    let mut result = String::new();
    let mut blank_count = 0usize;
    for line in &lines {
        if line.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }
    result.trim_end().to_string()
}
