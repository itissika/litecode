//! The runtime-owned graph of shared editor services.
//!
//! Agent orchestration consumes this handle; it does not create a parallel
//! workspace, terminal, or engine graph.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engines::WorkspaceEngines;
use crate::terminal::TerminalHub;
use crate::workspace::{WorkspaceError, WorkspaceService};

#[derive(Clone)]
pub struct IdeBaseHandle {
    pub workspace: Arc<WorkspaceService>,
    pub engines: Arc<WorkspaceEngines>,
    pub terminal: Arc<TerminalHub>,
}

impl IdeBaseHandle {
    pub fn new(
        workspace: Arc<WorkspaceService>,
        engines: Arc<WorkspaceEngines>,
        terminal: Arc<TerminalHub>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workspace,
            engines,
            terminal,
        })
    }

    /// Single construction helper for CLI/tests: one workspace service graph.
    pub fn open(
        workspace_root: impl Into<PathBuf>,
        engines: Arc<WorkspaceEngines>,
    ) -> Result<Arc<Self>, WorkspaceError> {
        let workspace = WorkspaceService::new(workspace_root.into())?;
        Ok(Self::new(workspace, engines, Arc::new(TerminalHub::new())))
    }

    /// Apply canonical buffer/write text into the language server.
    /// No-op when LSP is not Warm, the path has no coverage, or the path is
    /// outside the workspace sandbox.
    pub async fn apply_document_if_ready(self: &Arc<Self>, abs_path: &Path, text: &str) {
        if !self.engines.is_warmed("lsp") {
            return;
        }
        let hub = self.engines.lsp_hub();
        if !hub.file_has_lsp_coverage(abs_path) {
            return;
        }
        if self.workspace.sandbox().rel_path(abs_path).is_err() {
            return;
        }
        if let Err(error) = hub.apply_document(abs_path, text).await {
            tracing::debug!(
                error = %error,
                path = %abs_path.display(),
                "LSP document apply failed"
            );
        }
    }

    /// Bootstrap from disk if the document is not already open, or close it
    /// when the file is gone. Does not overwrite an open document from disk.
    pub async fn sync_document_if_ready(self: &Arc<Self>, abs_path: &Path) {
        if !self.engines.is_warmed("lsp") {
            return;
        }
        let hub = self.engines.lsp_hub();
        if !hub.file_has_lsp_coverage(abs_path) {
            return;
        }
        if self.workspace.sandbox().rel_path(abs_path).is_err() {
            return;
        }
        if let Err(error) = hub.sync_document(abs_path).await {
            tracing::debug!(
                error = %error,
                path = %abs_path.display(),
                "LSP document sync failed"
            );
        }
    }
}
