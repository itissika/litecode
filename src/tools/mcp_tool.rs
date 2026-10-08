use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

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
        // Agent-facing name. `server_connection.tool_name` stays the raw
        // `tools/call` name.
        let tool_name = format!("mcp_{server_id}_{}", server_connection.tool_name);
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

    fn timeout(&self) -> Option<u64> {
        Some(self.server_connection.timeout_secs + 15)
    }

    fn description(&self, _ctx: &Context) -> String {
        self.tool_description.clone()
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
            ),
        )
        .await
        {
            Ok(Ok(s)) => ToolCallResult::ok(s),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn agent_name_prefixes_server_id() {
        let tool = McpTool::new(
            "github",
            "read a file".into(),
            serde_json::json!({"type": "object"}),
            McpServerConnection {
                tool_name: "read".into(),
                server_name: "global:github".into(),
                command: "unused".into(),
                args: Vec::new(),
                env: HashMap::new(),
                cwd: None,
                pool: Arc::new(McpConnectionPool::new()),
                timeout_secs: 60,
            },
        );
        assert_eq!(tool.name(), "mcp_github_read");
    }
}
