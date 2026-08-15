use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::McpClient;
use crate::types::{LitecodeError, Result};

/// Pool of MCP client connections keyed by server identity.
/// Reuses existing connections across multiple tool calls.
pub struct McpConnectionPool {
    clients: Mutex<HashMap<String, Arc<Mutex<McpClient>>>>,
    /// Servers that failed to start — fast-fail without retrying.
    failed_servers: Mutex<HashSet<String>>,
}

impl McpConnectionPool {
    pub fn new() -> Self {
        Self {
            clients: Mutex::new(HashMap::new()),
            failed_servers: Mutex::new(HashSet::new()),
        }
    }

    /// Get or create an MCP client for the given server key.
    /// If a client exists and is alive, returns it.
    /// Otherwise creates a new client and stores it.
    pub async fn get_or_create(
        &self,
        server_key: &str,
        command: &str,
        args: &[String],
        env: &HashMap<String, String>,
    ) -> Result<Arc<Mutex<McpClient>>> {
        // Check if server is known to be unavailable
        {
            let failed = self.failed_servers.lock().await;
            if failed.contains(server_key) {
                return Err(LitecodeError::ToolExecution(format!(
                    "MCP server '{}' is not available",
                    server_key
                )));
            }
        }

        let mut clients = self.clients.lock().await;
        if let Some(client) = clients.get(server_key) {
            // 2.10: if the cached client's server process has crashed, drop it and
            // rebuild so the next call gets a fresh connection instead of reusing
            // a dead one.
            let alive = client.lock().await.is_alive().await;
            if alive {
                return Ok(Arc::clone(client));
            }
            clients.remove(server_key);
        }

        // Create new stdio client
        let stdio_client = match super::McpStdioClient::new(command, args, env) {
            Ok(c) => c,
            Err(e) => {
                // Mark as failed so future calls get a clear error
                let mut failed = self.failed_servers.lock().await;
                failed.insert(server_key.to_string());
                return Err(e);
            }
        };
        let mcp_client = McpClient::Stdio(stdio_client);
        let client = Arc::new(Mutex::new(mcp_client));
        clients.insert(server_key.to_string(), Arc::clone(&client));
        Ok(client)
    }
}
