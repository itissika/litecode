use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::types::{LitecodeError, Result};

const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 30;

pub struct McpRemoteClient {
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
    next_id: AtomicU64,
}

impl McpRemoteClient {
    pub fn new(url: &str, headers: &HashMap<String, String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
            .build()
            .map_err(|e| {
                LitecodeError::ToolExecution(format!("failed to create HTTP client: {}", e))
            })?;

        Ok(Self {
            url: url.to_string(),
            headers: headers.clone(),
            client,
            next_id: AtomicU64::new(1),
        })
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_request_id();
        let mut body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            body["params"] = p;
        }

        let mut req = self.client.post(&self.url).json(&body);

        for (key, value) in &self.headers {
            req = req.header(key, value);
            tracing::debug!(header = %key, "MCP remote request header");
        }

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                LitecodeError::ToolExecution(format!(
                    "MCP remote server '{}' connection timed out after {} seconds",
                    self.url, DEFAULT_CONNECT_TIMEOUT_SECS
                ))
            } else {
                LitecodeError::ToolExecution(format!("MCP remote HTTP error: {}", e))
            }
        })?;

        if !resp.status().is_success() {
            return Err(LitecodeError::ToolExecution(format!(
                "MCP remote server returned HTTP {}",
                resp.status()
            )));
        }

        let response: Value = resp.json().await.map_err(|e| {
            LitecodeError::ToolExecution(format!("MCP remote JSON parse error: {}", e))
        })?;

        if let Some(error) = response.get("error") {
            let msg = error["message"].as_str().unwrap_or("unknown error");
            let code = error["code"].as_i64().unwrap_or(-1);
            return Err(LitecodeError::ToolExecution(format!(
                "MCP error (code {}): {}",
                code, msg
            )));
        }

        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn initialize(&self) -> Result<Value> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "litecode",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        self.send_request("initialize", Some(params)).await
    }

    pub async fn list_tools(&self) -> Result<Value> {
        self.send_request("tools/list", Some(serde_json::json!({})))
            .await
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });
        self.send_request("tools/call", Some(params)).await
    }

    /// Fetch tool schemas from remote MCP server.
    pub async fn tool_schemas(&self) -> Result<Vec<crate::mcp::McpToolSchema>> {
        self.initialize().await?;
        let result = self.list_tools().await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tool| {
                        let name = tool.get("name")?.as_str()?.to_string();
                        let input_schema = tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));
                        let description = tool
                            .get("description")
                            .and_then(|description| description.as_str())
                            .unwrap_or_default()
                            .to_string();
                        Some(crate::mcp::McpToolSchema {
                            name,
                            description,
                            input_schema,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(tools)
    }
}
