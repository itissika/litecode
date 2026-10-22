use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::context_pipeline::Context;
use crate::optional::exa_mcp;
use crate::tool::Tool;
use crate::tool::outcome::ToolOutcome;
use crate::tools::file_path::coded_error;
use crate::types::ToolCallResult;

const MAX_RESULTS: u32 = 5;

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

    fn search_outcome(&self, query: &str) -> SearchOutcome {
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

        match std::thread::spawn(move || exa_mcp::search(&client, &endpoint, &query, MAX_RESULTS))
            .join()
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
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    message: String,
}

fn websearch_data_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "description": "websearch outcome. Soft failures (provider HTTP/business/parse/timeout) resolve with ok=false; hard failures reject ToolError.",
        "properties": {
            "ok": { "type": "boolean" },
            "query": { "type": "string" },
            "text": { "type": "string" },
            "message": { "type": "string" }
        },
        "required": ["ok", "message"]
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

    fn data_schema(&self) -> Option<Value> {
        Some(websearch_data_schema())
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(s) => s.to_string(),
            Err(e) => return coded_error("invalid_input", e),
        };

        match self.search_outcome(&query) {
            SearchOutcome::Ok { text } => present_websearch(WebSearchData {
                ok: true,
                query: Some(query),
                text: Some(text),
                message: "Search completed.".to_string(),
            }),
            SearchOutcome::Soft { message } => present_websearch(WebSearchData {
                ok: false,
                query: Some(query),
                text: None,
                message,
            }),
            SearchOutcome::Hard { code, message } => coded_error(code, message),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/websearch.md"))
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(30))
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
    fn soft_fail_shape_resolves_ok_false() {
        let result = present_websearch(WebSearchData {
            ok: false,
            query: Some("rust".into()),
            text: None,
            message: "exa http 500".into(),
        });
        assert_ne!(result.level, ToolSignalLevel::Error);
        assert!(result.error.is_none());
        let data = result.data.expect("data");
        assert_eq!(data["ok"], false);
        assert_eq!(data["query"], "rust");
        assert_eq!(data["message"], "exa http 500");
    }

}
