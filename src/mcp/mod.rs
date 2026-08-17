pub mod client;
pub mod pool;
#[cfg(feature = "remote-mcp")]
pub mod remote;

pub use client::McpStdioClient;
pub use pool::{McpConnectionPool, McpRunState, McpServerSnapshot};
#[cfg(feature = "remote-mcp")]
pub use remote::McpRemoteClient;

use serde_json::Value;

use crate::types::Result;

pub enum McpClient {
    Stdio(McpStdioClient),
    #[cfg(feature = "remote-mcp")]
    Remote(McpRemoteClient),
}

impl McpClient {
    pub async fn initialize(&mut self) -> Result<Value> {
        match self {
            McpClient::Stdio(c) => c.initialize().await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(c) => c.initialize().await,
        }
    }

    pub async fn list_tools(&mut self) -> Result<Value> {
        match self {
            McpClient::Stdio(c) => c.list_tools().await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(c) => c.list_tools().await,
        }
    }

    /// Whether this client still needs its one-time `initialize` handshake.
    pub fn needs_initialize(&mut self) -> bool {
        match self {
            McpClient::Stdio(c) => c.needs_initialize(),
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(_) => true,
        }
    }

    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        match self {
            McpClient::Stdio(c) => c.call_tool(name, arguments).await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(c) => c.call_tool(name, arguments).await,
        }
    }

    pub async fn tool_schemas(&mut self) -> Result<Vec<(String, Value)>> {
        match self {
            McpClient::Stdio(c) => c.tool_schemas().await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(c) => c.tool_schemas().await,
        }
    }

    /// True while the underlying connection is usable (stdio child alive;
    /// remote is always considered alive).
    pub async fn is_alive(&mut self) -> bool {
        match self {
            McpClient::Stdio(c) => c.is_alive().await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(_) => true,
        }
    }

    /// Kill the underlying server process (no-op for remote).
    pub async fn kill(&mut self) {
        match self {
            McpClient::Stdio(c) => c.kill().await,
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(_) => {}
        }
    }

    pub fn start_kill(&mut self) {
        match self {
            McpClient::Stdio(c) => c.start_kill(),
            #[cfg(feature = "remote-mcp")]
            McpClient::Remote(_) => {}
        }
    }
}
