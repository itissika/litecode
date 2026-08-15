use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::config::schema::ToolReadiness;

/// Process-memory runtime catalog state. Never persisted to global DB.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCatalogState {
    /// Global-scope tool readiness (runtime-only, not in DB).
    #[serde(default)]
    pub readiness: HashMap<String, ToolReadiness>,
    /// Whether global init has been performed in this process lifetime.
    #[serde(default)]
    pub initialized: bool,
    /// LSP server names that have been initialized.
    #[serde(default)]
    pub lsp_servers: Vec<String>,
}
