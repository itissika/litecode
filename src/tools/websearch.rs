use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::optional::exa_mcp;
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

/// Exa's documented `numResults` (numResults: default 10, range 1-100).
/// The default follows the provider default; the ceiling is the public limit.
const DEFAULT_TOP_K: u32 = 10;
const MAX_TOP_K: u32 = 100;

pub struct WebSearchTool {
    client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
    endpoint: Arc<RwLock<Option<String>>>,
    /// WP-167: builds the client on first use (startup no longer does).
    warm: Option<crate::optional::WarmHook>,
}

impl WebSearchTool {
    pub fn new(
        client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
        endpoint: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self {
            client,
            endpoint,
            warm: None,
        }
    }

    pub fn with_warm_hook(mut self, warm: crate::optional::WarmHook) -> Self {
        self.warm = Some(warm);
        self
    }

    fn search_outcome(&self, query: &str, top_k: u32) -> SearchOutcome {
        // WP-167: one ensure_warm per call (builds on first use, reuses after,
        // retries after a failed build). Runs on the blocking pool (see
        // `call_async`). A failed build is returned as is.
        if let Some(warm) = &self.warm
            && let Err(e) = warm()
        {
            return SearchOutcome::Hard {
                code: "unavailable",
                message: format!(
                    "websearch engine failed to start: {e}. Set an Exa API key in Settings → Advanced, or EXA_API_KEY in the environment"
                ),
            };
        }
        let client = match self.client.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                return SearchOutcome::Hard {
                    code: "unavailable",
                    message: format!("websearch engine lock: {e}"),
                };
            }
        };
        let client = match client {
            Some(c) => c,
            None => {
                return SearchOutcome::Hard {
                    code: "unavailable",
                    message: "websearch engine not warmed. Set an Exa API key in Settings → Advanced, or EXA_API_KEY in the environment"
                        .to_string(),
                };
            }
        };
        let endpoint = match self.endpoint.read() {
            Ok(guard) => guard.clone(),
            Err(e) => {
                return SearchOutcome::Hard {
                    code: "unavailable",
                    message: format!("websearch endpoint lock: {e}"),
                };
            }
        };
        let endpoint = match endpoint.filter(|s| !s.is_empty()) {
            Some(ep) => ep,
            None => {
                return SearchOutcome::Hard {
                    code: "unavailable",
                    message: "websearch is not configured. Set an Exa API key in Settings → Advanced, or EXA_API_KEY in the environment"
                        .to_string(),
                };
            }
        };
        let query = query.to_string();

        match std::thread::spawn(move || exa_mcp::search(&client, &endpoint, &query, top_k)).join()
        {
            Ok(Ok(text)) => SearchOutcome::Ok { text },
            Ok(Err(message)) => SearchOutcome::Soft { message },
            Err(_) => SearchOutcome::Hard {
                code: "internal",
                message: "websearch thread panicked".to_string(),
            },
        }
    }
}

#[derive(Debug)]
enum SearchOutcome {
    Ok { text: String },
    Soft { message: String },
    Hard { code: &'static str, message: String },
}

#[derive(Debug, Clone, Serialize)]
struct WebSearchData {
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    results: Vec<SearchHit>,
    message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

/// Split the provider's text block (`Title:` / `URL:` / `Text:` records) into
/// at most `top_k` hits. Unknown shapes keep one hit with the leading text.
fn parse_hits(text: &str, top_k: usize) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut current: Option<SearchHit> = None;
    let mut in_body = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("Title:") {
            if let Some(hit) = current.take() {
                hits.push(hit);
            }
            current = Some(SearchHit {
                title: title.trim().to_string(),
                url: String::new(),
                snippet: String::new(),
            });
            in_body = false;
            continue;
        }
        let Some(hit) = current.as_mut() else {
            continue;
        };
        if let Some(url) = line.strip_prefix("URL:") {
            hit.url = url.trim().to_string();
        } else if let Some(body) = line
            .strip_prefix("Text:")
            .or_else(|| line.strip_prefix("Highlights:"))
            .or_else(|| line.strip_prefix("Summary:"))
        {
            in_body = true;
            push_snippet(&mut hit.snippet, body.trim());
        } else if in_body && !line.is_empty() {
            push_snippet(&mut hit.snippet, line);
        }
    }
    if let Some(hit) = current.take() {
        hits.push(hit);
    }
    if hits.is_empty() && !text.trim().is_empty() {
        let mut snippet = String::new();
        push_snippet(&mut snippet, text.trim());
        hits.push(SearchHit {
            title: String::new(),
            url: String::new(),
            snippet,
        });
    }
    hits.truncate(top_k);
    hits
}

/// Append one body line to the hit. Exa sizes highlights to the query's
/// relevance and documents that there is no character budget to tune, so keep
/// the provider text as returned instead of re-truncating it.
fn push_snippet(snippet: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !snippet.is_empty() {
        snippet.push(' ');
    }
    snippet.push_str(text);
}

fn parse_top_k(input: &Value) -> Result<(u32, Option<String>), String> {
    let clamped =
        crate::tool::clamp::clamp_int(input, "top_k", 1, u64::from(MAX_TOP_K), "results")?;
    Ok(match clamped {
        None => (DEFAULT_TOP_K, None),
        Some(clamped) => (clamped.value as u32, clamped.note),
    })
}

fn present_websearch(data: WebSearchData) -> ToolCallResult {
    ToolOutcome::ok(data).into_call_result("")
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
    }

    fn surface(&self) -> crate::tool::ToolSurface {
        crate::tool::ToolSurface::Code
    }

    fn call_async(
        &self,
        input: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // WP-167: the client build (first call) and the search are blocking;
        // run them in `spawn_blocking`, as webfetch does. `call` keeps the
        // default path's `finalize_signals`.
        let tool = WebSearchTool {
            client: Arc::clone(&self.client),
            endpoint: Arc::clone(&self.endpoint),
            warm: self.warm.clone(),
        };
        Box::pin(async move {
            let join = tokio::task::spawn_blocking(move || tool.call(input));
            match join.await {
                Ok(result) => result,
                Err(e) => coded_error("internal", format!("websearch task join failed: {e}")),
            }
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(s) => s.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };
        let (top_k, top_k_note) = match parse_top_k(&input) {
            Ok(parsed) => parsed,
            Err(e) => return coded_error("invalid_input", e),
        };

        match self.search_outcome(&query, top_k) {
            SearchOutcome::Ok { text } => {
                let results = parse_hits(&text, top_k as usize);
                let mut message = if results.is_empty() {
                    "No results.".to_string()
                } else {
                    format!("{} results.", results.len())
                };
                if let Some(note) = top_k_note {
                    message = format!("{message} {note}");
                }
                present_websearch(WebSearchData {
                    query: Some(query),
                    results,
                    message,
                })
            }
            SearchOutcome::Soft { message } => coded_error("io", message),
            SearchOutcome::Hard { code, message } => coded_error(code, message),
        }
    }

    /// Aligned with the provider MCP server's own 60s search budget.
    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSignalLevel;

    fn cold_tool() -> WebSearchTool {
        WebSearchTool::new(Arc::new(RwLock::new(None)), Arc::new(RwLock::new(None)))
    }

    #[test]
    fn not_warmed_surfaces_configuration_hint() {
        let tool = cold_tool();
        let result = tool.call_inner(serde_json::json!({ "query": "rust async" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert_eq!(result.error_code.as_deref(), Some("unavailable"));
        assert!(
            result.content.contains("not warmed") || result.content.contains("not configured"),
            "cold engine must say not warmed/configured: {}",
            result.content
        );
        assert!(
            result.content.contains("EXA_API_KEY") || result.content.contains("Exa"),
            "must hint how to configure: {}",
            result.content
        );
    }

    #[test]
    fn soft_search_error_is_io_failure() {
        let result = coded_error("io", "exa http 500");
        assert_eq!(result.error_code.as_deref(), Some("io"));
        assert!(result.data.is_none(), "{result:?}");
    }

    #[test]
    fn websearch_top_k_yields_structured_results() {
        let text = "Title: One\nURL: https://a.example\nText: first body line\nmore body\n\n\
                    Title: Two\nURL: https://b.example\nHighlights: second\n\n\
                    Title: Three\nURL: https://c.example\nText: third";
        let hits = parse_hits(text, 2);
        assert_eq!(
            hits,
            vec![
                SearchHit {
                    title: "One".into(),
                    url: "https://a.example".into(),
                    snippet: "first body line more body".into()
                },
                SearchHit {
                    title: "Two".into(),
                    url: "https://b.example".into(),
                    snippet: "second".into()
                },
            ]
        );
        assert_eq!(
            parse_top_k(&serde_json::json!({})).unwrap(),
            (DEFAULT_TOP_K, None)
        );
        assert_eq!(
            parse_top_k(&serde_json::json!({"top_k": 50})).unwrap(),
            (50, None)
        );
        let (top_k, note) = parse_top_k(&serde_json::json!({"top_k": 500})).unwrap();
        assert_eq!(top_k, MAX_TOP_K);
        assert_eq!(
            note.as_deref(),
            Some("`top_k` was reduced from 500 to 100 results.")
        );
        let (top_k, note) = parse_top_k(&serde_json::json!({"top_k": 0})).unwrap();
        assert_eq!(top_k, 1);
        assert_eq!(
            note.as_deref(),
            Some("`top_k` was raised from 0 to 1 results.")
        );
        let tool: std::sync::Arc<dyn crate::tool::Tool> = Arc::new(WebSearchTool::new(
            Arc::new(RwLock::new(None)),
            Arc::new(RwLock::new(None)),
        ));
        let surface = crate::tool::code_contract::build_surface(&[tool])
            .expect("websearch JS contract surface");
        assert!(
            surface.tools["websearch"].output_schema["properties"]
                .get("text")
                .is_none()
        );
        assert!(
            surface.tools["websearch"].input_schema["properties"]
                .get("top_k")
                .is_some()
        );
    }
}
