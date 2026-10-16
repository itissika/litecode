use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::optional::exa_mcp;
use crate::tool::Tool;
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

    fn search(&self, query: &str) -> Result<String, String> {
        // WP-167: one ensure_warm per call (builds on first use, reuses after,
        // retries after a failed build). Runs on the blocking pool (see
        // `call_async`). A failed build is returned as is.
        if let Some(warm) = &self.warm
            && let Err(e) = warm()
        {
            return Err(format!("websearch engine failed to start: {e}"));
        }
        let client = self
            .client
            .read()
            .map_err(|e| format!("websearch engine lock: {e}"))?
            .clone()
            .ok_or_else(|| "websearch engine not warmed".to_string())?;
        let endpoint = self
            .endpoint
            .read()
            .map_err(|e| format!("websearch endpoint lock: {e}"))?
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "websearch is not configured".to_string())?;
        let query = query.to_string();

        std::thread::spawn(move || exa_mcp::search(&client, &endpoint, &query, MAX_RESULTS))
            .join()
            .map_err(|_| "websearch thread panicked".to_string())?
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "websearch"
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
                Err(e) => ToolCallResult::error(format!("websearch task join failed: {e}")),
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

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let query = match crate::tool::require_nonempty_string_trimmed(&input, "query") {
            Ok(s) => s,
            Err(e) => return ToolCallResult::error(e),
        };

        match self.search(query) {
            Ok(output) => ToolCallResult::ok(output),
            Err(e) if e.contains("not warmed") || e.contains("not configured") => {
                ToolCallResult::error(format!(
                    "{e}. Set an Exa API key in Settings → Advanced, or EXA_API_KEY in the environment"
                ))
            }
            Err(e) => ToolCallResult::error(e),
        }
    }

    fn description(&self, _ctx: &Context) -> String {
        crate::tools::description_text(include_str!("descriptions/websearch.md"))
    }

    fn timeout(&self) -> Option<u64> {
        Some(30)
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
    fn failed_warmup_returns_the_real_error() {
        // A manager with no websearch endpoint: the first-use build fails.
        let mgr = crate::optional::EngineManager::new();
        let tool = WebSearchTool::new(mgr.websearch_client(), mgr.websearch_endpoint())
            .with_warm_hook(mgr.warm_hook("websearch"));
        let result = tool.call_inner(serde_json::json!({ "query": "rust" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(
            result
                .content
                .contains("websearch engine failed to start: ")
                && result.content.contains("not configured"),
            "real build error expected: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn call_async_runs_on_the_blocking_pool() {
        let tool = cold_tool();
        let result = tool.call_async(serde_json::json!({ "query": "   " })).await;
        assert_eq!(result.level, ToolSignalLevel::Error);
    }

    #[test]
    fn empty_query_is_error() {
        let tool = cold_tool();
        let result = tool.call_inner(serde_json::json!({ "query": "   " }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(
            result.content.to_lowercase().contains("query")
                || result.content.to_lowercase().contains("empty")
                || result.content.to_lowercase().contains("required"),
            "empty query should name the field: {}",
            result.content
        );
    }

    #[test]
    fn missing_query_is_error() {
        let tool = cold_tool();
        let result = tool.call_inner(serde_json::json!({}));
        assert_eq!(result.level, ToolSignalLevel::Error);
    }

    #[test]
    fn not_warmed_surfaces_configuration_hint() {
        let tool = cold_tool();
        let result = tool.call_inner(serde_json::json!({ "query": "rust async" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
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
    fn client_without_endpoint_is_not_configured() {
        // Client present but endpoint empty → configured-path error, not "not warmed".
        let client = reqwest::blocking::Client::new();
        let tool = WebSearchTool::new(
            Arc::new(RwLock::new(Some(client))),
            Arc::new(RwLock::new(Some(String::new()))),
        );
        let result = tool.call_inner(serde_json::json!({ "query": "rust" }));
        assert_eq!(result.level, ToolSignalLevel::Error);
        assert!(
            result.content.contains("not configured"),
            "empty endpoint → not configured: {}",
            result.content
        );
    }
}
