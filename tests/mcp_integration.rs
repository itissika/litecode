//! Typical MCP host path against `tests/fixtures/mock_mcp_server.py`:
//!
//! Settings start (async hop) →schemas for the LLM list →sync `Tool::call`
//! from a current-thread runtime (the turn) →hub I/O.
//!
//! Direct `McpStdioClient` tests cover the JSON-RPC codec only.

use std::collections::HashMap;
use std::sync::Arc;

use litecode::mcp::{McpClient, McpConnectionPool, McpStdioClient};
use litecode::tool::Tool;
use litecode::tools::mcp_tool::{McpServerConnection, McpTool};
use litecode::types::ToolSignalLevel;

fn mock_command() -> (String, Vec<String>) {
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock_mcp_server.py");
    ("python3".to_string(), vec![script.display().to_string()])
}

fn stdio_client() -> McpStdioClient {
    let (cmd, args) = mock_command();
    McpStdioClient::new(&cmd, &args, &HashMap::new(), None).expect("spawn mock mcp server")
}

fn mcp_client() -> McpClient {
    McpClient::Stdio(stdio_client())
}

fn mock_def() -> litecode::config::schema::McpServerDefinition {
    let (command, args) = mock_command();
    litecode::config::schema::McpServerDefinition {
        command,
        args,
        env: HashMap::new(),
        transport: litecode::config::schema::McpTransport::Stdio,
        ..Default::default()
    }
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
        pool.child_alive("mock").await,
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

    pool.kill_child("mock").await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !pool.child_alive("mock").await,
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
        pool.child_alive("mock").await,
        "rebuilt client must be alive"
    );
}

/// Catalog enable + agent bind →`build_tool_list` advertises
/// `mcp_<server>_<tool>` and `Tool::call` still uses the raw server name.
#[tokio::test(flavor = "current_thread")]
async fn catalog_and_bind_exposes_echo_and_round_trips() {
    use litecode::config::TurnGuard;
    use litecode::config::resolved::{WorkspaceState, resolve};
    use litecode::config::schema::{
        AgentProfile, AgentToolBinding, GlobalSettings, McpServerDefinition,
    };
    use litecode::engines::WorkspaceEngines;
    use litecode::ide_base::IdeBaseHandle;
    use litecode::optional::EngineManager;
    use litecode::runtime::RuntimeHandle;
    use litecode::session::manager::SessionManager;
    use litecode::tool::registry::build_tool_list;

    let (command, args) = mock_command();
    let mut global = GlobalSettings::default();
    global.mcp_servers.insert(
        "mock".into(),
        McpServerDefinition {
            command,
            args,
            env: HashMap::new(),
            transport: litecode::config::schema::McpTransport::Stdio,
            ..Default::default()
        },
    );
    let mut bindings = HashMap::new();
    bindings.insert(
        "mcp_mock".into(),
        AgentToolBinding {
            enabled: true,
            preset: None,
            allowed_tools: Some(vec!["echo".into()]),
        },
    );
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: bindings,
            ..Default::default()
        },
    );

    let ws = tempfile::TempDir::new().expect("ws");
    let resolved = resolve(
        global,
        WorkspaceState::new(ws.path()),
        litecode::provider_catalog::shared_for_db(&ws.path().join("global.db")).expect("catalog"),
    );

    let workspace_engines = WorkspaceEngines::new();
    let ide = IdeBaseHandle::open(ws.path(), Arc::new(workspace_engines.clone())).expect("ide");
    let runtime = RuntimeHandle::new(
        resolved.clone(),
        "default".into(),
        WorkspaceState::new(ws.path()),
        Arc::new(EngineManager::new()),
        Arc::new(workspace_engines),
        ide,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        ws.path().join("global.db"),
    );

    let permission = litecode::permission::PermissionEngine::resolver(resolved, "default", 0);
    let ctx = litecode::context_pipeline::Context {
        cwd: ws.path().to_path_buf(),
        workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(ws.path()),
        agents_md: None,
        claude_md: None,
    };
    let host = Arc::new(litecode::tools::code::CodeToolHost::new(
        permission,
        ctx,
        "default",
        std::sync::Arc::new(litecode::permission::RecordingPermissionSink::from_reply(
            true, false,
        )),
        ws.path().join(".litecode"),
        litecode::tool::write_lock::process_write_lock(),
        None,
    ));

    let tools = build_tool_list(
        &runtime,
        "default",
        0,
        tokio_util::sync::CancellationToken::new(),
        "test-parent-session",
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        Some(host),
    )
    .await
    .expect("build tool list");
    assert_eq!(tools.len(), 1, "code-only mode exposes run_code only");
    assert!(tools[0].is_code_transport());
    let names: Vec<String> = tools[0]
        .code_bindings()
        .expect("run_code bindings")
        .iter()
        .cloned()
        .collect();
    assert!(
        names.contains(&"mcp_mock.echo".to_string()),
        "LLM list must prefix the server namespace, got {names:?}"
    );
    assert!(
        !names.contains(&"echo".to_string()),
        "raw MCP name must not be advertised, got {names:?}"
    );
    assert!(
        !names.contains(&"crash".to_string()) && !names.contains(&"mcp_mock.crash".to_string()),
        "MCP allowlist must hide unselected tools, got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "mcp_mock"),
        "catalog id must not be advertised as a tool, got {names:?}"
    );

    let result = tools[0]
        .execute(
            serde_json::json!({
                "code": r#"return await tools.mcp_mock.echo({ greeting: "from-list" });"#
            }),
            litecode::tool::trait_::ToolExecutionContext {
                path_mode: litecode::workspace::ToolPathMode::Safe,
                workspace_root: ws.path().to_path_buf(),
                call_id: "call_mcp_code".into(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: 8000,
                session_id: "test-parent-session".into(),
                session: None,
            },
        )
        .await;
    assert_eq!(
        result.level,
        ToolSignalLevel::Ok,
        "listed MCP tool call failed: {}",
        result.content
    );
    assert!(
        result.content.contains("from-list"),
        "got content={}",
        result.content
    );
}

/// `execute` must return a pending future so a caller timeout can fire while
/// the hub call is still running.
#[tokio::test(flavor = "current_thread")]
async fn execute_stays_pending_while_hang_blocks_on_hub() {
    use std::time::{Duration, Instant};

    let pool = Arc::new(McpConnectionPool::new());
    let def = mock_def();
    pool.start("mock", &def, None).await.expect("start");

    let (cmd, args) = mock_command();
    let tool = McpTool::new(
        "mock",
        "hang".into(),
        serde_json::json!({"type": "object", "properties": {}}),
        McpServerConnection {
            tool_name: "hang".into(),
            server_name: "mock".into(),
            command: cmd,
            args,
            env: HashMap::new(),
            cwd: None,
            pool: Arc::clone(&pool),
            timeout_secs: 3,
        },
    );

    let started = Instant::now();
    let raced = tokio::time::timeout(
        Duration::from_secs(1),
        tool.execute(
            serde_json::json!({}),
            litecode::tool::trait_::ToolExecutionContext {
                path_mode: litecode::workspace::ToolPathMode::Safe,
                workspace_root: std::path::PathBuf::from("."),
                call_id: "call_hang".into(),
                cancel: tokio_util::sync::CancellationToken::new(),
                output_limit: 8000,
                session_id: String::new(),
                session: None,
            },
        ),
    )
    .await;
    let elapsed = started.elapsed();
    pool.stop("mock").await;

    assert!(
        raced.is_err(),
        "execute must stay pending so a caller timeout can fire, got {raced:?} after {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "caller timeout did not fire ahead of the hub timeout: {elapsed:?}"
    );
}
