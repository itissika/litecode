//! Exa hosted MCP web search (`https://mcp.exa.ai/mcp`).

use crate::config::schema::DEFAULT_WEBSEARCH_MCP_URL;
use serde_json::Value;

pub const DEFAULT_MCP_URL: &str = DEFAULT_WEBSEARCH_MCP_URL;

/// MCP URL with optional `EXA_API_KEY` query param (OpenCode-compatible).
pub fn mcp_url(endpoint: &str) -> String {
    let base = endpoint.trim().trim_end_matches('/');
    if let Ok(key) = std::env::var("EXA_API_KEY") {
        let key = key.trim();
        if !key.is_empty() && !base.contains("exaApiKey=") {
            return format!("{base}?exaApiKey={}", percent_encode(key));
        }
    }
    base.to_string()
}

pub fn search(
    client: &reqwest::blocking::Client,
    mcp_url: &str,
    query: &str,
    num_results: u32,
) -> Result<String, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "web_search_exa",
            "arguments": {
                "query": query,
                "type": "auto",
                "numResults": num_results,
                "livecrawl": "fallback"
            }
        }
    });

    let response = client
        .post(mcp_url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .map_err(|e| format!("exa search request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("exa search http error: {e}"))?;

    let text = response
        .text()
        .map_err(|e| format!("exa search read body: {e}"))?;

    parse_mcp_response(&text).ok_or_else(|| {
        if text.len() > 200 {
            format!("exa search: unparseable response: {}…", &text[..200])
        } else {
            format!("exa search: unparseable response: {text}")
        }
    })
}

/// Parse MCP JSON or SSE (`event: message` / `data: {...}`) body.
pub fn parse_mcp_response(body: &str) -> Option<String> {
    let normalized = body.replace('\r', "");
    if let Some(text) = extract_result_text(normalized.trim()) {
        return Some(text);
    }
    for line in normalized.lines() {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        if let Some(text) = extract_result_text(payload.trim()) {
            return Some(text);
        }
    }
    None
}

fn extract_result_text(payload: &str) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(payload).ok()?;
    let content = value.pointer("/result/content")?.as_array()?;
    content
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", byte));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_exa_response() {
        let body = "event: message\r\ndata: {\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"1. Title | https://example.com | snippet\"}]}}\r\n\r\n";
        let text = parse_mcp_response(body).expect("parse");
        assert!(text.contains("Title"));
        assert!(text.contains("https://example.com"));
    }

    #[test]
    fn parse_json_exa_response() {
        let body = r#"{"result":{"content":[{"type":"text","text":"ok"}]}}"#;
        assert_eq!(parse_mcp_response(body).as_deref(), Some("ok"));
    }
}
