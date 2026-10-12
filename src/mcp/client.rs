use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::config::schema::DEFAULT_MCP_TOOL_TIMEOUT_SECS;
use crate::types::{LitecodeError, Result};

/// Default per-request read timeout; aligns with [`DEFAULT_MCP_TOOL_TIMEOUT_SECS`].
pub const DEFAULT_MCP_REQUEST_TIMEOUT: Duration =
    Duration::from_secs(DEFAULT_MCP_TOOL_TIMEOUT_SECS);

/// Max consecutive non-JSON stdout lines before declaring the connection dirty.
const MAX_BAD_LINES: u32 = 32;

pub struct McpStdioClient {
    child: Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    next_id: AtomicU64,
    initialized: bool,
    /// After a cancelled/timed-out/failed read, the BufReader may hold a stale
    /// response. Callers must rebuild (kill + respawn) rather than reuse.
    dirty: bool,
    request_timeout: Duration,
}

/// Sets `*flag = true` on drop unless disarmed — covers turn-cancel dropping
/// the read future mid-`read_line` (ISSUE-034 misnumbering).
struct DirtyOnDrop<'a> {
    flag: &'a mut bool,
    armed: bool,
}

impl Drop for DirtyOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            *self.flag = true;
        }
    }
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
            dirty: false,
            request_timeout: DEFAULT_MCP_REQUEST_TIMEOUT,
        })
    }

    /// Per-request bound for the JSON-RPC read loop (ISSUE-034).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.request_timeout = if timeout.is_zero() {
            DEFAULT_MCP_REQUEST_TIMEOUT
        } else {
            timeout
        };
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// True when the stdio buffer may contain a stale response; rebuild required.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        if self.dirty {
            return Err(LitecodeError::ToolExecution(
                "MCP connection is dirty; server must be rebuilt".into(),
            ));
        }

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

        self.stdin.write_all(line.as_bytes()).await.map_err(|e| {
            self.dirty = true;
            LitecodeError::ToolExecution(format!("MCP stdin write failed: {e}"))
        })?;
        self.stdin.flush().await.map_err(|e| {
            self.dirty = true;
            LitecodeError::ToolExecution(format!("MCP stdin flush failed: {e}"))
        })?;

        // After the request is on the wire, any incomplete read leaves a
        // response that would mis-number the next call (ISSUE-034).
        let timeout = self.request_timeout;
        let method_owned = method.to_string();
        {
            let dirty = &mut self.dirty;
            let reader = &mut self.reader;
            let mut guard = DirtyOnDrop {
                flag: dirty,
                armed: true,
            };
            let outcome = tokio::time::timeout(timeout, read_matching_response(reader, id)).await;
            match outcome {
                Ok(Ok(value)) => {
                    guard.armed = false;
                    Ok(value)
                }
                Ok(Err(e)) => {
                    // leave armed → dirty
                    Err(e)
                }
                Err(_) => Err(LitecodeError::ToolExecution(format!(
                    "MCP request timed out after {}s ({method_owned})",
                    timeout.as_secs()
                ))),
            }
        }
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
    /// Windows `taskkill /T /F`.
    pub async fn kill(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.kill().await;
        self.dirty = true;
    }

    pub fn start_kill(&mut self) {
        kill_process_tree(&mut self.child);
        let _ = self.child.start_kill();
        self.dirty = true;
    }
}

/// Read stdout lines until a JSON-RPC response with matching `id` arrives.
/// Skips notifications (no id) and responses for other ids (ISSUE-034).
async fn read_matching_response(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    want_id: u64,
) -> Result<Value> {
    let mut bad_lines = 0u32;
    loop {
        let mut response_line = String::new();
        let bytes_read = reader.read_line(&mut response_line).await?;
        if bytes_read == 0 {
            return Err(LitecodeError::ToolExecution(
                "MCP server closed stdout unexpectedly".into(),
            ));
        }

        let response: Value = match serde_json::from_str(response_line.trim()) {
            Ok(v) => {
                bad_lines = 0;
                v
            }
            Err(_) => {
                bad_lines += 1;
                if bad_lines >= MAX_BAD_LINES {
                    return Err(LitecodeError::ToolExecution(
                        "MCP server sent too many non-JSON lines".into(),
                    ));
                }
                continue;
            }
        };

        // Notifications have no id (or null id).
        let Some(resp_id) = response.get("id") else {
            continue;
        };
        if resp_id.is_null() {
            continue;
        }
        if !json_rpc_id_matches(resp_id, want_id) {
            // Stale / out-of-order response for another request — skip.
            continue;
        }

        if let Some(error) = response.get("error") {
            let msg = error["message"].as_str().unwrap_or("unknown error");
            let code = error["code"].as_i64().unwrap_or(-1);
            return Err(LitecodeError::ToolExecution(format!(
                "MCP error (code {}): {}",
                code, msg
            )));
        }

        return Ok(response.get("result").cloned().unwrap_or(Value::Null));
    }
}

fn json_rpc_id_matches(resp_id: &Value, want: u64) -> bool {
    match resp_id {
        Value::Number(n) => n.as_u64() == Some(want) || n.as_i64() == Some(want as i64),
        Value::String(s) => s.parse::<u64>().ok() == Some(want),
        _ => false,
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

    fn mock_cmd() -> (String, Vec<String>, HashMap<String, String>) {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mock_mcp_server.py");
        (
            "python3".into(),
            vec![script.display().to_string()],
            HashMap::new(),
        )
    }

    fn mock_cmd_env(mode: &str) -> (String, Vec<String>, HashMap<String, String>) {
        let (cmd, args, mut env) = mock_cmd();
        env.insert("MCP_MOCK_MODE".into(), mode.into());
        (cmd, args, env)
    }

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

    /// R1: notification before the real response must not be treated as the result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r1_skips_notification_before_response() {
        let (cmd, args, env) = mock_cmd_env("notify_first");
        let mut client = McpStdioClient::new(&cmd, &args, &env, None).expect("spawn");
        client.initialize().await.expect("initialize");
        let out = client
            .call_tool("echo", serde_json::json!({"greeting": "after-notify"}))
            .await
            .expect("echo after notification");
        let text = out["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("after-notify"),
            "must match by id and skip notification, got: {text}"
        );
        assert!(!client.is_dirty(), "successful match must not dirty");
        // Next call must still work (no misnumbering from leftover response).
        let out2 = client
            .call_tool("echo", serde_json::json!({"greeting": "second"}))
            .await
            .expect("second echo");
        assert!(
            out2["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .contains("second"),
            "second call must not receive a stale response"
        );
    }

    /// R2: no reply → bounded timeout error + dirty (pool must rebuild).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_hang_times_out_and_marks_dirty() {
        let (cmd, args, env) = mock_cmd();
        let mut client = McpStdioClient::new(&cmd, &args, &env, None).expect("spawn");
        client.set_request_timeout(Duration::from_secs(2));
        client.initialize().await.expect("initialize");
        let started = std::time::Instant::now();
        let err = client
            .call_tool("hang", serde_json::json!({}))
            .await
            .expect_err("hang must time out");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(5),
            "timeout must be bounded, took {elapsed:?}"
        );
        assert!(
            err.to_string().contains("timed out"),
            "error should mention timeout, got: {err}"
        );
        assert!(client.is_dirty(), "timeout must mark connection dirty");
        // Reuse must fail closed until rebuild.
        let reuse = client
            .call_tool("echo", serde_json::json!({"greeting": "nope"}))
            .await;
        assert!(
            reuse.is_err(),
            "dirty client must refuse further requests"
        );
    }

    /// R3: cancel mid-request → dirty; rebuilt client serves the next request correctly.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r3_cancel_marks_dirty_next_client_ok() {
        let (cmd, args, env) = mock_cmd_env("slow");
        let mut client = McpStdioClient::new(&cmd, &args, &env, None).expect("spawn");
        client.set_request_timeout(Duration::from_secs(30));
        client.initialize().await.expect("initialize");

        // Inner scope so the call_tool future (and its &mut client borrow) end
        // before we assert is_dirty / drop(client) — avoids E0502/E0505.
        {
            let call = client.call_tool("echo", serde_json::json!({"greeting": "slow-one"}));
            tokio::pin!(call);
            tokio::select! {
                res = &mut call => {
                    panic!("slow call should have been cancelled, got {res:?}");
                }
                _ = tokio::time::sleep(Duration::from_millis(300)) => {
                    // Falling through drops `call` → DirtyOnDrop marks dirty.
                }
            }
        }
        assert!(
            client.is_dirty(),
            "cancel during read must mark dirty to prevent misnumbering"
        );

        // Rebuild (simulates pool kill + get_or_create).
        drop(client);
        let (cmd, args, env) = mock_cmd();
        let mut fresh = McpStdioClient::new(&cmd, &args, &env, None).expect("respawn");
        fresh.initialize().await.expect("re-initialize");
        let out = fresh
            .call_tool("echo", serde_json::json!({"greeting": "after-cancel"}))
            .await
            .expect("echo on fresh client");
        assert!(
            out["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .contains("after-cancel"),
            "fresh client must not see the cancelled request's response"
        );
    }
}
