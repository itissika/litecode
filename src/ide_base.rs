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

    /// Sync a workspace file into the language server via the sole LspHub exit.
    ///
    /// No-op when LSP is not Warm, the path has no coverage, or the path is
    /// outside the workspace sandbox. External ALL-mode Agent paths deliberately
    /// do not call this method.
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
