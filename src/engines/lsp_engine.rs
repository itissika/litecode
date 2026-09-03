use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use crate::config::workspace::lsp_servers_from_engines;
use crate::lsp::LspHub;
use crate::lsp::deps::commands_for_server_ids;
use crate::types::{LitecodeError, Result};

pub struct LspEngine {
    hub: Arc<LspHub>,
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
    warmup_epoch: AtomicU64,
}

impl Default for LspEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LspEngine {
    pub fn new() -> Self {
        Self {
            hub: Arc::new(LspHub::new()),
            workspace_root: Arc::new(RwLock::new(None)),
            warmup_epoch: AtomicU64::new(0),
        }
    }

    pub fn hub(&self) -> Arc<LspHub> {
        Arc::clone(&self.hub)
    }

    pub fn set_workspace(&self, root: PathBuf) {
        self.hub.set_workspace(root.clone());
        if let Ok(mut guard) = self.workspace_root.write() {
            *guard = Some(root);
        }
    }

    fn warmup_still_valid(&self, epoch: u64) -> bool {
        self.warmup_epoch.load(Ordering::SeqCst) == epoch
    }
}

impl LspEngine {
    pub fn warmup(&self) -> Result<()> {
        let epoch = self.warmup_epoch.load(Ordering::SeqCst);
        let root = self
            .workspace_root
            .read()
            .map_err(|e| LitecodeError::Config(format!("workspace lock: {e}")))?
            .clone()
            .ok_or_else(|| LitecodeError::Config("lsp: workspace not set".into()))?;

        if !self.warmup_still_valid(epoch) {
            return Ok(());
        }

        let server_ids = lsp_servers_from_engines(&root);
        if server_ids.is_empty() {
            return Err(LitecodeError::Config(
                "lsp: not initialized for this workspace (enable it in Settings → Engines)".into(),
            ));
        }
        let commands = commands_for_server_ids(&root, &server_ids);
        if commands.is_empty() {
            return Err(LitecodeError::Config(
                "lsp: no language server commands resolved from configuration".into(),
            ));
        }
        // activate is metadata-only (no language-server I/O).
        self.hub.activate(&commands)?;
        Ok(())
    }

    pub fn stop(&self) {
        self.warmup_epoch.fetch_add(1, Ordering::SeqCst);
        let hub = Arc::clone(&self.hub);
        // Sync facade for engine lifecycle: always drive `hub.stop()` on a
        // dedicated thread so we never nest `Runtime::block_on` inside a caller
        // tokio runtime (e.g. `#[tokio::test]`). Language-server I/O runs on
        // the hub Runtime; callers only `.await` oneshots.
        if let Ok(handle) = std::thread::Builder::new()
            .name("lsp-engine-stop".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("lsp stop runtime");
                rt.block_on(hub.stop());
            })
        {
            let _ = handle.join();
        }
    }
}
