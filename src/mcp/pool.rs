//! Process-level MCP stdio pool.
//!
//! One dedicated runtime thread owns child processes and JSON-RPC I/O so the
//! turn's current-thread runtime never holds a `Child`. Settings hop via
//! `on_hub`; tool calls hop via [`McpConnectionPool::block_on_hub`]. Inner
//! helpers must not nest those hops (the hub has one worker).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::runtime::Handle;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::{McpClient, McpStdioClient};
use crate::config::schema::McpServerDefinition;
use crate::types::{LitecodeError, Result};

const START_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRunState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerSnapshot {
    pub status: McpRunState,
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for McpServerSnapshot {
    fn default() -> Self {
        Self {
            status: McpRunState::Stopped,
            tools: Vec::new(),
            error: None,
        }
    }
}

struct Entry {
    status: McpRunState,
    error: Option<String>,
    client: Option<Arc<Mutex<McpClient>>>,
    schemas: Vec<(String, serde_json::Value)>,
}

impl Entry {
    fn stopped() -> Self {
        Self {
            status: McpRunState::Stopped,
            error: None,
            client: None,
            schemas: Vec::new(),
        }
    }

    fn snapshot(&self) -> McpServerSnapshot {
        McpServerSnapshot {
            status: self.status,
            tools: self.schemas.iter().map(|(n, _)| n.clone()).collect(),
            error: self.error.clone(),
        }
    }
}

type Inner = Arc<Mutex<HashMap<String, Entry>>>;

/// Process-level MCP stdio connections. One child per server id until Stop/Restart.
pub struct McpConnectionPool {
    inner: Inner,
    handle: Handle,
    shutdown: CancellationToken,
}

impl McpConnectionPool {
    pub fn new() -> Self {
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("litecode-mcp".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .thread_name("litecode-mcp-worker")
                    .build()
                    .expect("mcp runtime");
                let _ = tx.send(rt.handle().clone());
                rt.block_on(stop.cancelled());
            })
            .expect("mcp hub thread");
        let handle = rx.recv().expect("mcp hub handle");
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            handle,
            shutdown,
        }
    }

    async fn on_hub<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.await
            .map_err(|_| LitecodeError::ToolExecution("MCP hub runtime unavailable".into()))
    }

    /// Drive a future on the MCP hub from a sync caller.
    ///
    /// Always waits on a side thread: turn runtime is current-thread, so
    /// `blocking_recv` on the caller would panic.
    pub fn block_on_hub<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.handle.spawn(async move {
            let _ = tx.send(fut.await);
        });
        let (otx, orx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = otx.send(rx.blocking_recv());
        });
        orx.recv()
            .map_err(|_| LitecodeError::ToolExecution("MCP hub runtime unavailable".into()))?
            .map_err(|_| LitecodeError::ToolExecution("MCP hub runtime unavailable".into()))
    }

    pub async fn snapshot(&self, id: &str) -> McpServerSnapshot {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        self.on_hub(async move {
            inner
                .lock()
                .await
                .get(&id)
                .map(Entry::snapshot)
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    pub async fn snapshots(&self) -> HashMap<String, McpServerSnapshot> {
        let inner = Arc::clone(&self.inner);
        self.on_hub(async move {
            inner
                .lock()
                .await
                .iter()
                .map(|(id, e)| (id.clone(), e.snapshot()))
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    pub async fn schemas(&self, id: &str) -> Vec<(String, serde_json::Value)> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        self.on_hub(async move {
            inner
                .lock()
                .await
                .get(&id)
                .map(|e| e.schemas.clone())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default()
    }

    /// Spawn (or reuse a live process) and handshake. Idempotent while Running.
    /// Returns `tools/list` schemas used to instantiate LLM tools.
    pub async fn start(
        &self,
        id: &str,
        def: &McpServerDefinition,
    ) -> Result<Vec<(String, serde_json::Value)>> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        let def = def.clone();
        self.on_hub(async move { start_inner(&inner, &id, &def).await })
            .await?
    }

    pub async fn stop(&self, id: &str) {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        let _ = self
            .on_hub(async move {
                stop_inner(&inner, &id).await;
            })
            .await;
    }

    pub async fn restart(
        &self,
        id: &str,
        def: &McpServerDefinition,
    ) -> Result<Vec<(String, serde_json::Value)>> {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        let def = def.clone();
        self.on_hub(async move {
            stop_inner(&inner, &id).await;
            start_inner(&inner, &id, &def).await
        })
        .await?
    }

    pub async fn stop_all(&self) {
        let inner = Arc::clone(&self.inner);
        let _ = self
            .on_hub(async move {
                stop_all_inner(&inner).await;
            })
            .await;
    }

    /// Start if needed, then return the live client. Used by tests.
    pub async fn get_or_create(
        &self,
        server_key: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Arc<Mutex<McpClient>>> {
        let inner = Arc::clone(&self.inner);
        let server_key = server_key.to_string();
        let def = McpServerDefinition {
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            transport: crate::config::schema::McpTransport::Stdio,
        };
        self.on_hub(async move { get_or_create_inner(&inner, &server_key, &def).await })
            .await?
    }

    /// Must run on the MCP hub (via [`Self::block_on_hub`] / [`Self::on_hub`]).
    pub(crate) async fn call_on_hub(
        &self,
        server_key: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<String> {
        let def = McpServerDefinition {
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
            transport: crate::config::schema::McpTransport::Stdio,
        };
        let client = get_or_create_inner(&self.inner, server_key, &def).await?;
        let mut guard = client.lock().await;
        if guard.needs_initialize() {
            guard.initialize().await?;
        }
        let result = guard.call_tool(tool_name, input).await?;
        Ok(format_mcp_result(result))
    }

    pub(crate) async fn stop_on_hub(&self, id: &str) {
        stop_inner(&self.inner, id).await;
    }

    pub async fn kill_child(&self, id: &str) {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        let _ = self
            .on_hub(async move {
                let client = inner.lock().await.get(&id).and_then(|e| e.client.clone());
                if let Some(client) = client {
                    client.lock().await.kill().await;
                }
            })
            .await;
    }

    pub async fn child_alive(&self, id: &str) -> bool {
        let inner = Arc::clone(&self.inner);
        let id = id.to_string();
        self.on_hub(async move {
            let client = inner.lock().await.get(&id).and_then(|e| e.client.clone());
            match client {
                Some(c) => c.lock().await.is_alive().await,
                None => false,
            }
        })
        .await
        .unwrap_or(false)
    }
}

impl Default for McpConnectionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for McpConnectionPool {
    fn drop(&mut self) {
        if let Ok(mut map) = self.inner.try_lock() {
            for (_, entry) in map.drain() {
                if let Some(client) = entry.client {
                    if let Ok(mut guard) = client.try_lock() {
                        guard.start_kill();
                    }
                }
            }
        }
        self.shutdown.cancel();
    }
}

async fn start_inner(
    inner: &Inner,
    id: &str,
    def: &McpServerDefinition,
) -> Result<Vec<(String, serde_json::Value)>> {
    if matches!(
        def.transport,
        crate::config::schema::McpTransport::Remote { .. }
    ) {
        return Err(LitecodeError::ToolExecution(
            "stdio MCP lifecycle currently supports stdio servers".into(),
        ));
    }
    if def.command.trim().is_empty() {
        return Err(LitecodeError::ToolExecution(
            "MCP stdio server command must not be empty".into(),
        ));
    }
    loop {
        {
            let mut map = inner.lock().await;
            let entry = map.entry(id.to_string()).or_insert_with(Entry::stopped);
            match entry.status {
                McpRunState::Running => {
                    if let Some(client) = &entry.client {
                        let alive = client.lock().await.is_alive().await;
                        if alive {
                            return Ok(entry.schemas.clone());
                        }
                    }
                    *entry = Entry::stopped();
                }
                McpRunState::Starting => {
                    drop(map);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
                McpRunState::Stopped | McpRunState::Error => {
                    *entry = Entry {
                        status: McpRunState::Starting,
                        error: None,
                        client: None,
                        schemas: Vec::new(),
                    };
                }
            }
        }
        break;
    }

    let handshake = tokio::time::timeout(
        START_TIMEOUT,
        spawn_and_list(&def.command, &def.args, &def.env),
    )
    .await;

    let mut map = inner.lock().await;
    let entry = map.entry(id.to_string()).or_insert_with(Entry::stopped);
    match handshake {
        Ok(Ok((client, schemas))) => {
            *entry = Entry {
                status: McpRunState::Running,
                error: None,
                client: Some(Arc::new(Mutex::new(client))),
                schemas: schemas.clone(),
            };
            Ok(schemas)
        }
        Ok(Err(e)) => {
            let msg = e.to_string();
            *entry = Entry {
                status: McpRunState::Error,
                error: Some(msg.clone()),
                client: None,
                schemas: Vec::new(),
            };
            Err(LitecodeError::ToolExecution(msg))
        }
        Err(_) => {
            let msg = format!(
                "MCP start timed out after {}s ({})",
                START_TIMEOUT.as_secs(),
                def.command
            );
            *entry = Entry {
                status: McpRunState::Error,
                error: Some(msg.clone()),
                client: None,
                schemas: Vec::new(),
            };
            Err(LitecodeError::ToolExecution(msg))
        }
    }
}

async fn stop_inner(inner: &Inner, id: &str) {
    let client = {
        let mut map = inner.lock().await;
        let Some(entry) = map.get_mut(id) else {
            return;
        };
        let client = entry.client.take();
        *entry = Entry::stopped();
        client
    };
    if let Some(client) = client {
        client.lock().await.kill().await;
    }
}

async fn stop_all_inner(inner: &Inner) {
    let ids: Vec<String> = inner.lock().await.keys().cloned().collect();
    for id in ids {
        stop_inner(inner, &id).await;
    }
}

async fn get_or_create_inner(
    inner: &Inner,
    server_key: &str,
    def: &McpServerDefinition,
) -> Result<Arc<Mutex<McpClient>>> {
    start_inner(inner, server_key, def).await?;
    let map = inner.lock().await;
    let entry = map.get(server_key).ok_or_else(|| {
        LitecodeError::ToolExecution(format!("MCP server '{server_key}' is not available"))
    })?;
    entry.client.clone().ok_or_else(|| {
        LitecodeError::ToolExecution(format!("MCP server '{server_key}' is not available"))
    })
}

async fn spawn_and_list(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<(McpClient, Vec<(String, serde_json::Value)>)> {
    let mut client = McpClient::Stdio(McpStdioClient::new(command, args, env)?);
    let schemas = client.tool_schemas().await?;
    Ok((client, schemas))
}

fn format_mcp_result(result: serde_json::Value) -> String {
    match result {
        serde_json::Value::String(s) => s,
        serde_json::Value::Array(arr) => arr
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
            .collect::<Vec<_>>()
            .join("\n"),
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    }
}
