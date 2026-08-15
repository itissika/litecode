//! MCP stdio integration (2.10): initialize-once, crash-rebuild, liveness.
//!
//! Drives a real `McpStdioClient` / `McpConnectionPool` against the mock
//! `tests/fixtures/mock_mcp_server.py`. The 60s timeout-kill path is not run
//! here (it would slow the suite by a minute); it is covered by the pool
//! liveness/kill primitives exercised below plus the `run_mcp_call` timeout
//! wiring, which are the pieces whose regression would reintroduce a leak.

use std::collections::HashMap;
use std::sync::Arc;

use litecode::mcp::{McpClient, McpConnectionPool, McpStdioClient};

fn mock_command() -> (String, Vec<String>) {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock_mcp_server.py");
    ("python3".to_string(), vec![script.display().to_string()])
}

fn stdio_client() -> McpStdioClient {
    let (cmd, args) = mock_command();
    McpStdioClient::new(&cmd, &args, &HashMap::new()).expect("spawn mock mcp server")
}

fn mcp_client() -> McpClient {
    McpClient::Stdio(stdio_client())
}

/// initialize happens exactly once per connection and flips `needs_initialize`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_is_single_shot_per_connection() {
    let mut client = mcp_client();
    assert!(client.needs_initialize(), "fresh client needs initialize");
    client.initialize().await.expect("initialize succeeds");
    assert!(
        !client.needs_initialize(),
        "needs_initialize must be false after one successful initialize"
    );
    // A second initialize is a no-op guard (client stays initialized).
    client.initialize().await.expect("idempotent initialize");
    assert!(!client.needs_initialize());
}

/// `initialize` + `tools/list` returns the mock server's tool catalog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_tools_returns_mock_catalog() {
    let mut client = mcp_client();
    let tools = client.tool_schemas().await.expect("tool_schemas");
    let names: Vec<String> = tools.iter().map(|(n, _)| n.clone()).collect();
    assert!(names.contains(&"echo".to_string()));
    assert!(names.contains(&"crash".to_string()));
    assert!(names.contains(&"hang".to_string()));
}

/// `tools/call` round-trips through the mock server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_tool_round_trips() {
    let mut client = mcp_client();
    client.initialize().await.expect("initialize");
    let out = client
        .call_tool("echo", serde_json::json!({"greeting": "hello"}))
        .await
        .expect("echo call");
    let text = out["content"][0]["text"]
        .as_str()
        .expect("mock echo returns text content");
    assert!(
        text.contains("hello"),
        "echo must reflect arguments, got: {text}"
    );
}

/// Pool reuses the same live client for the same server key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_reuses_live_client_for_same_key() {
    let (cmd, args) = mock_command();
    let pool = Arc::new(McpConnectionPool::new());
    let a = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("first get");
    let b = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("second get");
    assert!(Arc::ptr_eq(&a, &b), "pool must reuse the cached client");
    assert!(
        a.lock().await.is_alive().await,
        "cached client must be alive"
    );
}

/// When the server process crashes, the pool drops the dead client and rebuilds
/// a fresh one on the next call (2.10 crash-removal).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_rebuilds_after_crash() {
    let (cmd, args) = mock_command();
    let pool = Arc::new(McpConnectionPool::new());
    let client = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("initial get");

    // Kill the underlying server; the cached client must now report dead.
    client.lock().await.kill().await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !client.lock().await.is_alive().await,
        "cached client must be detected dead after process kill"
    );

    // Next get_or_create drops the dead entry and returns a fresh live client.
    let rebuilt = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("rebuilt get");
    assert!(
        !Arc::ptr_eq(&client, &rebuilt),
        "a dead client must be replaced, not reused"
    );
    assert!(
        rebuilt.lock().await.is_alive().await,
        "rebuilt client must be alive"
    );
}
