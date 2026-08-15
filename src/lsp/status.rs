//! Unified language-server instance status (all languages share these types).

use serde::{Deserialize, Serialize};

/// Lifecycle of one language-server process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspLifecycle {
    Starting,
    Running,
    Restarting,
    Failed,
    Stopped,
}

/// Snapshot of one instance for Hub aggregation / engines detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspInstanceStatus {
    pub command: String,
    pub project_root: String,
    pub state: LspLifecycle,
    pub index_settled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub restart_count: u32,
}
