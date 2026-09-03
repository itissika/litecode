//! Config gate: document identity, commit ack, and generationed notify.
//!
//! `SettingsWriter` is the process lock + storage dispatcher. This module owns
//! the types that make a second write path fail to type-check.

use serde::{Deserialize, Serialize};

/// Persistable settings document. Not catalog, not runtime status.
pub type PersistDoc = DocId;

/// Evaluation / projection. Never a `commit` target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalView {
    AvailableTools,
    McpRuntime,
    EngineUsable,
    InstallProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DocId {
    #[serde(rename = "providers")]
    Providers,
    #[serde(rename = "models")]
    Models,
    #[serde(rename = "agents")]
    Agents,
    #[serde(rename = "log")]
    Log,
    #[serde(rename = "websearch")]
    Websearch,
    #[serde(rename = "custom_tools.global")]
    CustomToolsGlobal,
    #[serde(rename = "custom_tools.workspace")]
    CustomToolsWorkspace,
    #[serde(rename = "mcp.global")]
    McpGlobal,
    #[serde(rename = "mcp.workspace")]
    McpWorkspace,
    #[serde(rename = "engines")]
    Engines,
    #[serde(rename = "excludes")]
    Excludes,
}

impl DocId {
    pub const ALL: &'static [DocId] = &[
        DocId::Providers,
        DocId::Models,
        DocId::Agents,
        DocId::Log,
        DocId::Websearch,
        DocId::CustomToolsGlobal,
        DocId::CustomToolsWorkspace,
        DocId::McpGlobal,
        DocId::McpWorkspace,
        DocId::Engines,
        DocId::Excludes,
    ];

    pub fn reloads_global(self) -> bool {
        matches!(
            self,
            DocId::Providers
                | DocId::Models
                | DocId::Agents
                | DocId::Log
                | DocId::Websearch
                | DocId::CustomToolsGlobal
                | DocId::McpGlobal
        )
    }

    pub fn reloads_workspace(self) -> bool {
        matches!(
            self,
            DocId::CustomToolsWorkspace | DocId::McpWorkspace | DocId::Engines | DocId::Excludes
        )
    }

    pub fn needs_engine_reconcile(docs: &[DocId]) -> bool {
        docs.contains(&DocId::Engines)
    }

    pub fn apply_plan(docs: &[DocId]) -> ApplyPlan {
        ApplyPlan {
            reload_global: docs.iter().copied().any(DocId::reloads_global),
            reload_workspace: docs.iter().copied().any(DocId::reloads_workspace),
            reconcile_engines: Self::needs_engine_reconcile(docs),
        }
    }
}

/// What `RuntimeHandle::apply` may do for a commit. Engines reconcile is never implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyPlan {
    pub reload_global: bool,
    pub reload_workspace: bool,
    pub reconcile_engines: bool,
}

/// Successful gate commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitAck {
    pub generation: u64,
    pub docs: Vec<DocId>,
    pub restart_required: bool,
}

impl CommitAck {
    pub fn revision(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_reconcile_only_when_named() {
        assert!(!DocId::needs_engine_reconcile(&[DocId::Log]));
        assert!(!DocId::needs_engine_reconcile(&[
            DocId::CustomToolsWorkspace,
            DocId::McpWorkspace
        ]));
        assert!(DocId::needs_engine_reconcile(&[DocId::Engines]));
        assert!(DocId::needs_engine_reconcile(&[DocId::Log, DocId::Engines]));
        let log = DocId::apply_plan(&[DocId::Log]);
        assert!(log.reload_global);
        assert!(!log.reload_workspace);
        assert!(!log.reconcile_engines);
        let tools = DocId::apply_plan(&[DocId::CustomToolsWorkspace]);
        assert!(!tools.reload_global);
        assert!(tools.reload_workspace);
        assert!(!tools.reconcile_engines);
    }

    #[test]
    fn doc_id_wire_names() {
        assert_eq!(
            serde_json::to_string(&DocId::CustomToolsWorkspace).unwrap(),
            "\"custom_tools.workspace\""
        );
        assert_eq!(
            serde_json::from_str::<DocId>("\"engines\"").unwrap(),
            DocId::Engines
        );
        let _persist: PersistDoc = DocId::Log;
        let _eval = EvalView::AvailableTools;
        assert_ne!(
            serde_json::to_string(&DocId::Agents).unwrap(),
            "\"available-tools\""
        );
    }
}
