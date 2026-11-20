use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::mcp::McpConnectionPool;
use crate::tool::Tool;
use crate::types::{MediaSource, ToolCallResult, ToolOutputPart};

/// Configuration for connecting to an MCP server.
pub struct McpServerConnection {
    pub tool_name: String,
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub cwd: Option<std::path::PathBuf>,
    pub pool: Arc<McpConnectionPool>,
    pub timeout_secs: u64,
}

pub struct McpTool {
    tool_name: String,
    tool_description: String,
    input_schema: Value,
    server_connection: McpServerConnection,
}

impl McpTool {
    pub fn new(
        server_id: &str,
        description: String,
        input_schema: Value,
        server_connection: McpServerConnection,
    ) -> Self {
        // Agent-facing dotted code-mode name: `tools.mcp_<server>.<inner>`.
        let tool_name = format!("mcp_{server_id}.{}", server_connection.tool_name);
        Self {
            tool_name,
            tool_description: description,
            input_schema,
            server_connection,
        }
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn dynamic_code_spec(&self) -> Option<crate::tool::DynamicCodeSpec> {
        Some(crate::tool::DynamicCodeSpec {
            description: self.tool_description.clone(),
            input_schema: self.input_schema.clone(),
            output_schema: None,
        })
    }

    fn execute(
        &self,
        input: Value,
        _execution: crate::tool::trait_::ToolExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ToolCallResult> + Send + '_>> {
        // Await the hub. `block_on_hub` would stall this thread before the
        // executor's timeout can wrap the future.
        let pool = Arc::clone(&self.server_connection.pool);
        let call = hub_call(&self.server_connection, input);
        Box::pin(async move {
            match pool.on_hub(call).await {
                Ok(output) => output,
                Err(e) => ToolCallResult::error(e.to_string()),
            }
        })
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let pool = Arc::clone(&self.server_connection.pool);
        match pool.block_on_hub(hub_call(&self.server_connection, input)) {
            Ok(output) => output,
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(
            self.server_connection.timeout_secs.saturating_add(15),
        ))
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
}

fn hub_call(
    conn: &McpServerConnection,
    input: Value,
) -> impl Future<Output = ToolCallResult> + Send + 'static {
    let pool = Arc::clone(&conn.pool);
    let mcp_tool_name = conn.tool_name.clone();
    let server_command = conn.command.clone();
    let server_args = conn.args.clone();
    let server_env = conn.env.clone();
    let server_cwd = conn.cwd.clone();
    let server_key = conn.server_name.clone();
    let timeout_secs = if conn.timeout_secs == 0 {
        crate::config::schema::DEFAULT_MCP_TOOL_TIMEOUT_SECS
    } else {
        conn.timeout_secs
    };
    async move {
        let timeout_key = server_key.clone();
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            pool.call_on_hub(
                &server_key,
                &server_command,
                &server_args,
                &server_env,
                server_cwd,
                &mcp_tool_name,
                input,
                timeout_secs,
            ),
        )
        .await
        {
            Ok(Ok(value)) => convert_mcp_result(value),
            Ok(Err(e)) => ToolCallResult::error(e.to_string()),
            Err(_) => {
                pool.stop_on_hub(&timeout_key).await;
                ToolCallResult::error(format!(
                    "MCP tool call timed out after {timeout_secs} seconds"
                ))
            }
        }
    }
}

fn convert_mcp_result(value: Value) -> ToolCallResult {
    let is_error = value
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let structured = value
        .get("structuredContent")
        .filter(|v| !v.is_null())
        .cloned();

    let mut texts: Vec<String> = Vec::new();
    let mut parts: Vec<ToolOutputPart> = Vec::new();
    let mut non_text_items: Vec<Value> = Vec::new();

    if let Some(items) = value.get("content").and_then(Value::as_array) {
        for item in items {
            let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
            match kind {
                "text" => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        texts.push(text.to_string());
                    }
                }
                "image" | "audio" => {
                    if let Some(part) = parse_mcp_media_item(kind, item) {
                        parts.push(part);
                    } else {
                        non_text_items.push(item.clone());
                    }
                }
                _ => non_text_items.push(item.clone()),
            }
        }
    } else if let Some(arr) = value.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                texts.push(s.to_string());
            } else if item.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    texts.push(text.to_string());
                }
            } else {
                non_text_items.push(item.clone());
            }
        }
    } else if let Some(s) = value.as_str() {
        texts.push(s.to_string());
    }

    let joined_text = texts.join("\n");
    if is_error {
        let msg = if !joined_text.is_empty() {
            joined_text
        } else {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "MCP tool returned isError".into())
        };
        return ToolCallResult::error(msg);
    }

    let data = if let Some(structured) = structured {
        structured
    } else if non_text_items.is_empty()
        && texts.len() == 1
        && let Ok(parsed @ (Value::Object(_) | Value::Array(_))) =
            serde_json::from_str::<Value>(&texts[0])
    {
        parsed
    } else if non_text_items.is_empty() {
        serde_json::json!({ "text": joined_text })
    } else {
        serde_json::json!({
            "text": joined_text,
            "items": non_text_items,
        })
    };

    let content = if !joined_text.is_empty() {
        joined_text
    } else {
        serde_json::to_string_pretty(&data).unwrap_or_default()
    };

    let mut result = if parts.is_empty() {
        ToolCallResult::ok(content)
    } else {
        ToolCallResult::ok_with_parts(content, parts)
    };
    result.data = Some(data);
    result
}

fn parse_mcp_media_item(kind: &str, item: &Value) -> Option<ToolOutputPart> {
    let default_mime = if kind == "audio" { "audio/wav" } else { "image/png" };
    let mime = item
        .get("mimeType")
        .or_else(|| item.get("mime_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_mime)
        .to_string();
    let source = if let Some(data) = item
        .get("data")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        MediaSource::Url {
            url: format!("data:{mime};base64,{data}"),
        }
    } else if let Some(url) = item
        .get("url")
        .or_else(|| item.get("uri"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        MediaSource::Url {
            url: url.to_string(),
        }
    } else {
        return None;
    };
    match kind {
        "image" => Some(ToolOutputPart::image(source, mime)),
        "audio" => Some(ToolOutputPart::audio(source, mime)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MediaKind;

    #[test]
    fn mcp_tool_extracts_structured_text_and_multimodal_parts() {
        let raw = serde_json::json!({
            "content": [
                { "type": "text", "text": "screenshot captured" },
                { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
                { "type": "audio", "data": "UklGRg==", "mimeType": "audio/wav" }
            ],
            "isError": false
        });
        let result = convert_mcp_result(raw);
        assert!(result.error.is_none());
        assert_eq!(result.data, Some(serde_json::json!({ "text": "screenshot captured" })));
        assert_eq!(result.parts.len(), 2);
        match &result.parts[0] {
            ToolOutputPart::Media { artifact } => {
                assert_eq!(artifact.kind, MediaKind::Image);
                assert_eq!(artifact.mime_type, "image/png");
            }
            other => panic!("expected image media part, got {other:?}"),
        }
        match &result.parts[1] {
            ToolOutputPart::Media { artifact } => {
                assert_eq!(artifact.kind, MediaKind::Audio);
                assert_eq!(artifact.mime_type, "audio/wav");
            }
            other => panic!("expected audio media part, got {other:?}"),
        }
    }

    #[test]
    fn mcp_is_error_produces_tool_error() {
        let raw = serde_json::json!({
            "content": [{ "type": "text", "text": "bad query" }],
            "isError": true
        });
        let result = convert_mcp_result(raw);
        assert!(result.error.is_some());
        assert_eq!(result.content, "bad query");
    }
}
