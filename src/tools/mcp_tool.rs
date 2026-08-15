use std::sync::Arc;

use serde_json::Value;

use crate::context_pipeline::Context;
use crate::mcp::McpConnectionPool;
use crate::tool::Tool;
use crate::types::{Result, ToolCallResult};

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
        let mcp_tool_name = conn.tool_name.clone();
        let pool = Arc::clone(&conn.pool);
        let server_command = conn.command.clone();
        let server_args = conn.args.clone();
        let server_env = conn.env.clone();
        let server_key = conn.server_name.clone();

        let (tx, rx) = std::sync::mpsc::channel::<ToolCallResult>();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(ToolCallResult::error(format!(
                        "failed to create runtime: {}",
                        e
                    )));
                    return;
                }
            };

            // 2.10: run initialize + call under an internal timeout so that a hung
            // server is killed (releasing the pool guard) instead of leaking.
            let output = rt.block_on(async {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(60),
                    run_mcp_call(
                        &pool,
                        &server_key,
                        &server_command,
                        &server_args,
                        &server_env,
                        &mcp_tool_name,
                        input.clone(),
                    ),
                )
                .await
                {
                    Ok(Ok(s)) => ToolCallResult::ok(s),
                    Ok(Err(e)) => ToolCallResult::error(e.to_string()),
                    Err(_) => {
                        // Kill the child process (releasing the guard lock) so the
                        // timed-out server cannot linger.
                        if let Ok(client) = pool
                            .get_or_create(&server_key, &server_command, &server_args, &server_env)
                            .await
                        {
                            client.lock().await.kill().await;
                        }
                        ToolCallResult::error("MCP tool call timed out after 60 seconds")
                    }
                }
            });

            let _ = tx.send(output);
        });

        match rx.recv_timeout(std::time::Duration::from_secs(70)) {
            Ok(result) => result,
            Err(_) => ToolCallResult::error("MCP tool call timed out after 70 seconds"),
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

#[allow(clippy::too_many_arguments)]
async fn run_mcp_call(
    pool: &Arc<McpConnectionPool>,
    server_key: &str,
    server_command: &str,
    server_args: &[String],
    server_env: &std::collections::HashMap<String, String>,
    mcp_tool_name: &str,
    input: Value,
) -> Result<String> {
    let client = pool
        .get_or_create(server_key, server_command, server_args, server_env)
        .await?;
    let mut guard = client.lock().await;
    // 2.10: initialize exactly once per connection, not on every call.
    if guard.needs_initialize() {
        guard.initialize().await?;
    }
    let result = guard.call_tool(mcp_tool_name, input).await?;

    // Format the result
    match result {
        Value::String(s) => Ok(s),
        Value::Array(arr) => {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|v| {
                    if v.is_string() {
                        Some(v.as_str().unwrap_or("").to_string())
                    } else if v.get("type").and_then(|t| t.as_str()) == Some("text") {
                        v.get("text")
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    } else {
                        Some(serde_json::to_string_pretty(v).unwrap_or_default())
                    }
                })
                .collect();
            Ok(texts.join("\n"))
        }
        other => Ok(serde_json::to_string_pretty(&other).unwrap_or_default()),
    }
}
