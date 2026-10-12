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
}

impl WebSearchTool {
    pub fn new(
        client: Arc<RwLock<Option<reqwest::blocking::Client>>>,
        endpoint: Arc<RwLock<Option<String>>>,
    ) -> Self {
        Self { client, endpoint }
    }

    fn search(&self, query: &str) -> Result<String, String> {
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
