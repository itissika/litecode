use std::sync::Arc;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::mcp::McpConnectionPool;
use crate::tool::Tool;
use crate::types::ToolCallResult;

/// Configuration for connecting to an MCP server.
pub struct McpServerConnection {
    pub tool_name: String,
    pub server_name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub pool: Arc<McpConnectionPool>,
}

pub struct McpTool {
    tool_name: String,
    tool_description: String,
    input_schema: Value,
    server_connection: McpServerConnection,
}

impl McpTool {
    pub fn new(
        description: String,
        input_schema: Value,
        server_connection: McpServerConnection,
    ) -> Self {
        let tool_name = server_connection.tool_name.clone();
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

    fn schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn call_inner(&self, input: Value) -> ToolCallResult {
        let conn = &self.server_connection;
        let pool = Arc::clone(&conn.pool);
        let pool_for_hub = Arc::clone(&pool);
        let mcp_tool_name = conn.tool_name.clone();
        let server_command = conn.command.clone();
        let server_args = conn.args.clone();
        let server_env = conn.env.clone();
        let server_key = conn.server_name.clone();
        let input = input.clone();

        match pool.block_on_hub(async move {
            let timeout_key = server_key.clone();
            match tokio::time::timeout(
                std::time::Duration::from_secs(60),
                pool_for_hub.call_on_hub(
                    &server_key,
                    &server_command,
                    &server_args,
                    &server_env,
                    &mcp_tool_name,
                    input,
                ),
            )
            .await
            {
                Ok(Ok(s)) => ToolCallResult::ok(s),
                Ok(Err(e)) => ToolCallResult::error(e.to_string()),
                Err(_) => {
                    pool_for_hub.stop_on_hub(&timeout_key).await;
                    ToolCallResult::error("MCP tool call timed out after 60 seconds")
                }
            }
        }) {
            Ok(output) => output,
            Err(e) => ToolCallResult::error(e.to_string()),
        }
    }

    fn timeout(&self) -> Option<u64> {
        Some(75)
    }

    fn description(&self, _ctx: &Context) -> String {
        self.tool_description.clone()
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        false
    }
}
