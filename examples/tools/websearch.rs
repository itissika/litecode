use serde_json::Value;

use litecode::context_pipeline::Context;
use litecode::tool::Tool;
use litecode::types::{Result, ToolCallResult};

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match litecode::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };

        let searxng_url = std::env::var("LITECODE_SEARXNG_URL")
            .unwrap_or_else(|_| "http://localhost:8080".to_string());

        // If no search API key and SearXNG is at default (likely not running), bail
        let _api_key = std::env::var("LITECODE_SEARCH_API_KEY").ok();

        match search_searxng(&searxng_url, query) {
            Ok(o) => ToolCallResult::ok(o),
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        "Search the web for information using SearXNG.".into()
    }
}

fn search_searxng(endpoint: &str, query: &str) -> Result<String> {
    let url = format!(
        "{}/search?q={}&format=json&categories=general",
        endpoint,
        urlencoding(query)
    );

    let output = std::process::Command::new("curl")
        .arg("-sL")
        .arg("--max-time")
        .arg("10")
        .arg(&url)
        .output()
        .map_err(|e| litecode::types::LitecodeError::ToolExecution(format!("curl error: {}", e)))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(litecode::types::LitecodeError::ToolExecution(format!(
            "search failed: {}",
            err.trim()
        )));
    }

    let body: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
        litecode::types::LitecodeError::ToolExecution(format!("json parse error: {}", e))
    })?;

    let results = body["results"].as_array().ok_or_else(|| {
        litecode::types::LitecodeError::ToolExecution(
            "SearXNG response missing 'results' array".into(),
        )
    })?;

    if results.is_empty() {
        return Ok(format!("No results found for: {}", query));
    }

    let mut out = String::new();
    for (i, result) in results.iter().take(5).enumerate() {
        let title = result["title"].as_str().unwrap_or("No title");
        let url = result["url"].as_str().unwrap_or("No URL");
        let content = result["content"].as_str().unwrap_or("");
        let snippet = truncate_str(content, 200);

        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}. {}\n   {}\n   {}\n",
            i + 1,
            title,
            url,
            snippet
        ));
    }

    Ok(out)
}

/// URL-encode special characters in a query string.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", byte));
            }
        }
    }
    out
}

/// Truncate a string to at most `max_len` characters, appending "…" if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Find a valid char boundary near max_len
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("rust&cargo"), "rust%26cargo");
        assert_eq!(urlencoding("abc123"), "abc123");
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world", 5), "hello…");
    }
}
