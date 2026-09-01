//! Typical MCP host path against `tests/fixtures/mock_mcp_server.py`:
//!
//! Settings start (async hop) →schemas for the LLM list →sync `Tool::call`
//! from a current-thread runtime (the turn) →hub I/O.
//!
//! Direct `McpStdioClient` tests cover the JSON-RPC codec only.

use std::collections::HashMap;
use std::sync::Arc;

use litecode::mcp::{McpClient, McpConnectionPool, McpRunState, McpStdioClient};
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

fn echo_tool(pool: Arc<McpConnectionPool>, cmd: &str, args: &[String]) -> McpTool {
    McpTool::new(
        "echo".into(),
        serde_json::json!({"type": "object", "properties": {"greeting": {"type": "string"}}}),
        McpServerConnection {
            tool_name: "echo".into(),
            server_name: "mock".into(),
            command: cmd.to_string(),
            args: args.to_vec(),
            env: HashMap::new(),
            cwd: None,
            pool,
            timeout_secs: 60,
        },
    )
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
    let names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    assert!(names.contains(&"echo".to_string()));
    assert!(names.contains(&"crash".to_string()));
    assert!(names.contains(&"hang".to_string()));
    assert_eq!(
        tools
            .iter()
            .find(|tool| tool.name == "echo")
            .map(|tool| tool.description.as_str()),
        Some("Echo the arguments back")
    );
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

/// Restart replaces the live process; subsequent get_or_create uses the new child.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_restart_replaces_live_client() {
    let (cmd, args) = mock_command();
    let pool = Arc::new(McpConnectionPool::new());
    let def = litecode::config::schema::McpServerDefinition {
        command: cmd.clone(),
        args: args.clone(),
        env: HashMap::new(),
        transport: litecode::config::schema::McpTransport::Stdio,
        ..Default::default()
    };
    pool.start("mock", &def, None).await.expect("start");
    let first = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("first");
    pool.restart("mock", &def, None).await.expect("restart");
    let second = pool
        .get_or_create("mock", &cmd, &args, &HashMap::new())
        .await
        .expect("second");
    assert!(
        !Arc::ptr_eq(&first, &second),
        "restart must spawn a new client"
    );
    assert!(pool.child_alive("mock").await);
}

/// Product path: start on hub, list tools, then `Tool::call` on the turn's
/// current-thread runtime (must not panic / must round-trip echo).
#[tokio::test(flavor = "current_thread")]
async fn turn_runtime_start_list_and_tool_call() {
    let pool = Arc::new(McpConnectionPool::new());
    let def = mock_def();
    let tools = pool.start("mock", &def, None).await.expect("start");
    assert!(
        tools.iter().any(|tool| tool.name == "echo"),
        "start must return tools/list schemas, got {tools:?}"
    );
    let snap = pool.snapshot("mock").await;
    assert_eq!(snap.status, McpRunState::Running);
    assert!(snap.tools.iter().any(|tool| tool.name == "echo"));

    let schemas = pool.schemas("mock").await;
    assert!(
        schemas.iter().any(|tool| tool.name == "echo"),
        "schemas used by build_tool_list must include echo"
    );

    let (cmd, args) = mock_command();
    let tool = echo_tool(Arc::clone(&pool), &cmd, &args);
    let result = tool.call(serde_json::json!({"greeting": "hello"}));
    assert_eq!(
        result.level,
        ToolSignalLevel::Ok,
        "turn-runtime MCP call failed: {}",
        result.content
    );
    assert!(
        result.content.contains("hello"),
        "echo must reflect arguments, got: {}",
        result.content
    );

    pool.stop("mock").await;
    assert!(!pool.child_alive("mock").await);
    let after_stop = tool.call(serde_json::json!({"greeting": "again"}));
    assert_eq!(
        after_stop.level,
        ToolSignalLevel::Ok,
        "call after Stop must auto-start: {}",
        after_stop.content
    );
    assert!(after_stop.content.contains("again"));
}

#[tokio::test(flavor = "current_thread")]
async fn block_on_hub_from_turn_runtime_does_not_panic() {
    let pool = McpConnectionPool::new();
    let n = pool
        .block_on_hub(async { 7u8 })
        .expect("block_on_hub from current-thread runtime");
    assert_eq!(n, 7);
}

/// Catalog enable + agent bind →`build_tool_list` advertises MCP's own names
/// (`echo`), not `mcp_<id>`, and `Tool::call` works on the turn runtime.
#[tokio::test(flavor = "current_thread")]
async fn catalog_and_bind_exposes_echo_and_round_trips() {
    use litecode::config::TurnGuard;
    use litecode::config::resolved::{WorkspaceState, resolve};
    use litecode::config::schema::{
        ADAPTER_OPENAI_RESPONSES, AgentProfile, AgentToolBinding, GlobalSettings,
        McpServerDefinition, ProviderAuth, ProviderConnectionConfig, ProviderDefinition,
    };
    use litecode::engines::WorkspaceEngines;
    use litecode::ide_base::IdeBaseHandle;
    use litecode::llm::provider_from_definition;
    use litecode::optional::EngineManager;
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
            policy: litecode::permission::ToolPolicy::allow_all(),
            path_mode: litecode::permission::BindingPathMode::default(),
            last_applied_preset: None,
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
    let resolved = resolve(global, WorkspaceState::new(ws.path()));

    let pool = Arc::new(McpConnectionPool::new());
    let workspace_engines = WorkspaceEngines::new();
    let ide = IdeBaseHandle::open(ws.path(), Arc::new(workspace_engines.clone())).expect("ide");
    let provider = provider_from_definition(&ProviderDefinition {
        id: "test".into(),
        adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
        label: "test".into(),
        config: ProviderConnectionConfig {
            endpoint: "http://127.0.0.1:9".into(),
            api_key: "k".into(),
            auth: ProviderAuth::Bearer,
        },
    })
    .expect("provider");

    let tools = build_tool_list(
        &resolved,
        "default",
        provider,
        "k",
        0,
        tokio_util::sync::CancellationToken::new(),
        EngineManager::new(),
        workspace_engines,
        ide,
        "test-parent-session",
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        Arc::clone(&pool),
    )
    .await;
    let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    assert!(
        names.contains(&"echo".to_string()),
        "LLM list must use the server tool name, got {names:?}"
    );
    assert!(
        !names.contains(&"crash".to_string()),
        "MCP allowlist must hide unselected tools, got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("mcp_")),
        "catalog id must not be advertised as a tool, got {names:?}"
    );

    let echo = tools.iter().find(|t| t.name() == "echo").expect("echo");
    let result = echo.call(serde_json::json!({"greeting": "from-list"}));
    assert_eq!(
        result.level,
        ToolSignalLevel::Ok,
        "listed MCP tool call failed: {}",
        result.content
    );
    assert!(
        result.content.contains("from-list"),
        "got: {}",
        result.content
    );
}
