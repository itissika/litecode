use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::types::{LitecodeError, Result};

pub struct McpStdioClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    next_id: AtomicU64,
    initialized: bool,
}

impl McpStdioClient {
    pub fn new(command: &str, args: &[String], env: &HashMap<String, String>) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            LitecodeError::ToolExecution(format!("failed to spawn MCP server '{}': {}", command, e))
        })?;

        let stdin = child.stdin.take().expect("stdin piped but missing");
        let stdout = child.stdout.take().expect("stdout piped but missing");
        let reader = BufReader::new(stdout);

        Ok(Self {
            child,
            stdin,
            reader,
            next_id: AtomicU64::new(1),
            initialized: false,
        })
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_request_id();
        let mut request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(p) = params {
            request["params"] = p;
        }

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');

        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        let mut response_line = String::new();
        let bytes_read = self.reader.read_line(&mut response_line).await?;
        if bytes_read == 0 {
            return Err(LitecodeError::ToolExecution(
                "MCP server closed stdout unexpectedly".into(),
            ));
        }

        let response: Value = serde_json::from_str(response_line.trim())?;

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

    pub async fn initialize(&mut self) -> Result<Value> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "litecode",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let result = self.send_request("initialize", Some(params)).await;
        if result.is_ok() {
            self.initialized = true;
        }
        result
    }

    /// Whether this client still needs its one-time `initialize` handshake.
    pub fn needs_initialize(&self) -> bool {
        !self.initialized
    }

    pub async fn list_tools(&mut self) -> Result<Value> {
        self.send_request("tools/list", Some(serde_json::json!({})))
            .await
    }

    /// Fetch tool schemas from MCP server (initializes if needed, calls tools/list).
    /// Returns Vec of (tool_name, inputSchema).
    pub async fn tool_schemas(&mut self) -> Result<Vec<(String, Value)>> {
        self.initialize().await?;
        let result = self.list_tools().await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|tool| {
                        let name = tool.get("name")?.as_str()?.to_string();
                        let schema = tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or(serde_json::json!({"type": "object", "properties": {}}));
                        Some((name, schema))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(tools)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });
        self.send_request("tools/call", Some(params)).await
    }

    /// True while the spawned server process is still running.
    pub async fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    /// Kill the child process (used to tear down a stuck/timed-out server).
    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
