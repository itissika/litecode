use std::collections::HashMap;
use std::path::Path;
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
    pub fn new(
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        cwd: Option<&Path>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        // Own process group so kill_process_tree can reach grandchildren
        // (servers that wrap `sh -c` / node spawn), matching custom tools.
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        let mut child = cmd.spawn().map_err(|e| {
            LitecodeError::ToolExecution(format!("failed to spawn MCP server '{}': {}", command, e))
        })?;
        // Windows Job Object (no-op on Unix): host crash must not leave the
        // server running. Call before the pid can be reused. Aligns with
        // ISSUE-040 / LSP / custom tools. Does not touch proc_lifetime.rs.
        crate::proc_lifetime::bind_child_to_parent(child.id().unwrap_or(0));

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
    pub async fn tool_schemas(&mut self) -> Result<Vec<crate::mcp::McpToolSchema>> {
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

    /// Kill the child process tree (used to tear down a stuck/timed-out server).
    ///
    /// Matches custom-tool `kill_child_tree`: Unix process-group SIGKILL,
    /// Windows `taskkill /T /F`. Protocol/request matching (ISSUE-034) is
    /// intentionally untouched.
    pub async fn kill(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.kill().await;
    }

    pub fn start_kill(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.start_kill();
    }
}

/// Best-effort group/tree kill for an MCP stdio server. Safe to call after the
/// direct child has already exited (Unix pgid still held by grandchildren).
fn kill_process_tree(child: &mut Child) {
    let Some(pid) = child.id() else {
        return;
    };
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = child;
        // Negative pid = whole group created via process_group(0).
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = child.start_kill();
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_reaps_grandchild_process() {
        let marker = std::env::temp_dir().join(format!("lc-mcp-gc-{}", ulid::Ulid::new()));
        let script = format!("(sleep 3; touch {}) & wait", marker.display());
        let mut client = McpStdioClient::new(
            "sh",
            &[String::from("-c"), script],
            &HashMap::new(),
            None,
        )
        .expect("spawn");
        // Let the grandchild start before we kill the tree.
        tokio::time::sleep(Duration::from_millis(100)).await;
        client.kill().await;
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(
            !marker.exists(),
            "MCP kill must reach grandchildren (process group / tree kill)"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drop_reaps_grandchild_process() {
        let marker = std::env::temp_dir().join(format!("lc-mcp-drop-{}", ulid::Ulid::new()));
        let script = format!("(sleep 3; touch {}) & wait", marker.display());
        {
            let _client = McpStdioClient::new(
                "sh",
                &[String::from("-c"), script],
                &HashMap::new(),
                None,
            )
            .expect("spawn");
            tokio::time::sleep(Duration::from_millis(100)).await;
            // Drop triggers start_kill + group kill.
        }
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(
            !marker.exists(),
            "MCP Drop must reach grandchildren"
        );
    }
}
