//! LSP Hub: process pool, routing, document sync orchestration.
//!
//! Language-server stdin/stdout is multiplexed in [`crate::lsp::conn`]. Callers
//! `.await` a oneshot; they must never `block_on` the hub Runtime (Agent
//! `current_thread` deadlock). Spawn of LS processes hops onto [`Self::handle`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;
use tokio::runtime::{Handle, Runtime};
use tokio::time::Duration;

use crate::lsp::deps;
use crate::lsp::format::{format_action_result, format_error_diagnostics_block};
use crate::lsp::server::{LspServer, MAX_AUTO_RESTARTS, RESTART_COOLDOWN};

/// Restart budget after an LSP server exit: a server that survived at least
/// `RESTART_COOLDOWN` and then *exited on its own* resets the budget. Watchdog
/// kills (write/RPC timeout) always count — otherwise a process that lived
/// >60s before we killed it would respawn forever.
fn effective_restart_count(prior_restarts: u32, lifetime: Duration, watchdog_kill: bool) -> u32 {
    if !watchdog_kill && lifetime >= RESTART_COOLDOWN {
        0
    } else {
        prior_restarts
    }
}
use crate::lsp::server_map::{program_from_command, server_command_for_ext};
use crate::lsp::status::{LspInstanceStatus, LspLifecycle};
use crate::lsp::uri::{canonical_project_root, file_to_uri, uri_to_path};
use crate::types::{LitecodeError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LspDiagFeedback {
    /// No Error diagnostics within budget, or hub inactive / timed out waiting.
    Silence,
    /// At least one Error-severity diagnostic for the file.
    Errors(String),
    /// Hard failure (path scope, missing LS, spawn) — caller may surface a short notice.
    Unavailable(String),
}

pub(crate) fn short_feedback_reason(err: &str) -> String {
    let trimmed = err.trim();
    const MAX: usize = 160;
    if trimmed.len() <= MAX {
        trimmed.to_string()
    } else {
        format!("{}…", &trimmed[..MAX])
    }
}

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) struct ServerKey {
    command: String,
    project_root: PathBuf,
}

struct LspHubInner {
    workspace_root: Option<PathBuf>,
    servers: HashMap<ServerKey, Arc<LspServer>>,
    configured_commands: HashSet<String>,
    active: bool,
    last_used: HashMap<ServerKey, Instant>,
    idle_timeout: Duration,
}

/// Shared LSP backend: lazy-spawned LS processes keyed by (command, project_root).
pub struct LspHub {
    inner: Mutex<LspHubInner>,
    // `ManuallyDrop` so `Drop` can leak the runtime instead of dropping it from
    // within an async context (which panics tokio). See `Drop for LspHub`.
    rt: std::mem::ManuallyDrop<Runtime>,
    handle: Handle,
}

impl LspHub {
    pub fn new() -> Self {
        let rt = Runtime::new().expect("lsp runtime");
        let handle = rt.handle().clone();
        Self {
            inner: Mutex::new(LspHubInner {
                workspace_root: None,
                servers: HashMap::new(),
                configured_commands: HashSet::new(),
                active: false,
                last_used: HashMap::new(),
                idle_timeout: Duration::from_secs(
                    std::env::var("LITECODE_LSP_IDLE_TIMEOUT_SECS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(600),
                ),
            }),
            rt: std::mem::ManuallyDrop::new(rt),
            handle,
        }
    }

    async fn spawn_ls(&self, binary: crate::lsp::install::LanguageServerBinary, root: PathBuf) -> Result<LspServer> {
        self.handle
            .spawn(async move { LspServer::spawn(&binary, &root).await })
            .await
            .map_err(|e| LitecodeError::ToolExecution(format!("lsp spawn join: {e}")))?
    }

    pub fn set_workspace(&self, root: PathBuf) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.workspace_root = Some(crate::config::path::strip_verbatim(&root));
        }
    }

    /// Current workspace root in LAP form (if set).
    pub fn workspace_root(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.workspace_root.clone())
    }

    pub fn is_active(&self) -> bool {
        self.inner.lock().map(|g| g.active).unwrap_or(false)
    }

    /// True when `abs_path`'s extension maps to a language server configured for this workspace.
    /// Does not check engine Warm — callers gate on `EngineState` separately.
    pub fn file_has_lsp_coverage(&self, abs_path: &Path) -> bool {
        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Some(cmd) = server_command_for_ext(ext) else {
            return false;
        };
        self.inner
            .lock()
            .ok()
            .is_some_and(|inner| inner.active && inner.configured_commands.contains(&cmd))
    }

    /// Per-instance status snapshots (Running / Failed / indexing, …).
    pub fn instance_statuses(&self) -> Vec<LspInstanceStatus> {
        let servers: Vec<(ServerKey, Arc<LspServer>)> = {
            let Ok(inner) = self.inner.lock() else {
                return Vec::new();
            };
            inner
                .servers
                .iter()
                .map(|(k, s)| (k.clone(), Arc::clone(s)))
                .collect()
        };
        servers
            .iter()
            .map(|(key, server)| server.status_snapshot(&key.project_root))
            .collect()
    }

    #[cfg(test)]
    pub fn set_configured_commands_for_test(&self, commands: &[String]) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.configured_commands = commands.iter().cloned().collect();
            inner.active = true;
        }
    }

    /// Verify configured LS binaries and mark hub active (no spawn).
    pub fn activate(&self, commands: &[String]) -> Result<()> {
        for cmd in commands {
            let program = program_from_command(cmd);
            deps::command_runnable_command(cmd).map_err(|e| {
                LitecodeError::Config(format!("language server '{program}' not runnable: {e}"))
            })?;
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
        inner.configured_commands = commands.iter().cloned().collect();
        inner.active = true;
        Ok(())
    }

    async fn resolve_server_key(&self, abs_path: &Path) -> Result<ServerKey> {
        let abs_path = crate::config::path::strip_verbatim(abs_path);
        // ---- Phase 7b: lazy idle check + workspace/active validation ----
        let workspace_root = {
            let inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            if !inner.active {
                return Err(LitecodeError::ToolExecution("LSP hub not active".into()));
            }
            let root = inner
                .workspace_root
                .clone()
                .ok_or_else(|| LitecodeError::Config("lsp: workspace not set".into()))?;
            let root = crate::config::path::canon_abs_lossy(&root);
            if !crate::config::path::is_under(&abs_path, &root) {
                return Err(LitecodeError::ToolExecution(format!(
                    "path outside LSP workspace (root={}, path={}). \
                     Pass a workspace-relative path (e.g. src/main.rs) or an absolute path under that root.",
                    root.display(),
                    abs_path.display()
                )));
            }
            root
        };

        let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let cmd = server_command_for_ext(ext)
            .ok_or_else(|| LitecodeError::ToolExecution(format!("no LS for .{ext}")))?;

        let program = program_from_command(&cmd);
        let project_root = crate::lsp::project_root::project_root_for_file(
            &abs_path,
            &program,
            Some(&workspace_root),
        )?;
        let key = ServerKey {
            command: cmd.clone(),
            project_root: canonical_project_root(&project_root),
        };

        // Reap *other* idle instances. Never shut down the server this request
        // needs — that was kill-then-spawn on the same key.
        let need_shutdown: Vec<Arc<LspServer>> = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            if inner.idle_timeout.is_zero() {
                Vec::new()
            } else {
                let mut to_remove: Vec<ServerKey> = Vec::new();
                for (idle_key, last_used) in &inner.last_used {
                    if idle_key != &key && last_used.elapsed() > inner.idle_timeout {
                        to_remove.push(idle_key.clone());
                    }
                }
                let removed: Vec<_> = to_remove
                    .iter()
                    .filter_map(|k| inner.servers.remove(k))
                    .collect();
                for k in &to_remove {
                    inner.last_used.remove(k);
                }
                removed
            }
        };
        for server in need_shutdown {
            server.shutdown().await;
        }

        let existing = {
            let inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            if !inner.configured_commands.contains(&cmd) {
                return Err(LitecodeError::ToolExecution(format!(
                    "language server '{cmd}' not configured for this workspace"
                )));
            }
            inner.servers.get(&key).cloned()
        };

        if let Some(existing) = existing {
            if existing.is_process_alive()
                && !matches!(
                    existing.lifecycle(),
                    LspLifecycle::Failed | LspLifecycle::Stopped
                )
            {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                inner.last_used.insert(key.clone(), Instant::now());
                return Ok(key);
            }
            let prior_restarts = existing.restart_count;
            let last_err = existing
                .last_error()
                .unwrap_or_else(|| "language server not running".into());
            let watchdog_kill = existing.watchdog_kill();
            let lifetime = existing.spawned_at.elapsed();
            // Remove dead instance; auto-restart below if under budget.
            {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                inner.servers.remove(&key);
                inner.last_used.remove(&key);
            }
            let effective_prior = effective_restart_count(prior_restarts, lifetime, watchdog_kill);
            if effective_prior >= MAX_AUTO_RESTARTS {
                return Err(LitecodeError::ToolExecution(format!(
                    "language server '{cmd}' failed and exhausted auto-restarts ({effective_prior}): {last_err}"
                )));
            }
            let binary = deps::resolve_server_binary(&cmd).map_err(|e| {
                LitecodeError::Config(format!(
                    "language server '{program}' could not be resolved: {e}"
                ))
            })?;
            match self.spawn_ls(binary, key.project_root.clone()).await {
                Ok(mut server) => {
                    server.restart_count = effective_prior + 1;
                    let mut inner = self
                        .inner
                        .lock()
                        .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                    inner.servers.insert(key.clone(), Arc::new(server));
                    inner.last_used.insert(key.clone(), Instant::now());
                    return Ok(key);
                }
                Err(e) => return Err(e),
            }
        }

        let binary = deps::resolve_server_binary(&cmd).map_err(|e| {
            LitecodeError::Config(format!(
                "language server '{program}' could not be resolved: {e}"
            ))
        })?;

        match self.spawn_ls(binary, key.project_root.clone()).await {
            Ok(server) => {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                inner
                    .servers
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(server));
                inner.last_used.insert(key.clone(), Instant::now());
                Ok(key)
            }
            Err(e) => Err(e),
        }
    }

    /// Explicit restart of one server instance (resets auto-restart budget).
    pub async fn restart_server(
        self: &Arc<Self>,
        command: &str,
        project_root: &Path,
    ) -> Result<()> {
        let key = ServerKey {
            command: command.to_string(),
            project_root: canonical_project_root(project_root),
        };
        let old = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            if !inner.active {
                return Err(LitecodeError::ToolExecution("LSP hub not active".into()));
            }
            if !inner.configured_commands.contains(command) {
                return Err(LitecodeError::ToolExecution(format!(
                    "language server '{command}' not configured for this workspace"
                )));
            }
            inner.servers.remove(&key)
        };
        if let Some(old) = old {
            old.set_lifecycle(LspLifecycle::Restarting);
            old.shutdown().await;
        }
        let program = program_from_command(command);
        let binary = deps::resolve_server_binary(command).map_err(|e| {
            LitecodeError::Config(format!(
                "language server '{program}' could not be resolved: {e}"
            ))
        })?;
        let server = self.spawn_ls(binary, key.project_root.clone()).await?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
        inner.servers.insert(key.clone(), Arc::new(server));
        inner.last_used.insert(key, Instant::now());
        Ok(())
    }

    /// Explicit restart of every running instance.
    pub async fn restart_all(self: &Arc<Self>) -> Result<()> {
        let keys: Vec<ServerKey> = {
            let inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            inner.servers.keys().cloned().collect()
        };
        for key in keys {
            self.restart_server(&key.command, &key.project_root).await?;
        }
        Ok(())
    }

    pub async fn stop(self: &Arc<Self>) {
        let servers: Vec<Arc<LspServer>> = {
            let mut inner = match self.inner.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            inner.active = false;
            inner.configured_commands.clear();
            inner.servers.drain().map(|(_, s)| s).collect()
        };
        for server in servers {
            server.shutdown().await;
        }
    }

    /// PIDs of spawned language-server child processes (for memory telemetry).
    pub fn language_server_pids(&self) -> Vec<u32> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner.servers.values().filter_map(|s| s.child_id()).collect()
    }

    /// Sync (open/change/close) a document from disk through the hub I/O exit.
    pub async fn sync_document(self: &Arc<Self>, abs_path: &Path) -> Result<()> {
        self.sync_document_work(abs_path).await
    }

    async fn sync_document_work(&self, abs_path: &Path) -> Result<()> {
        if !self.is_active() {
            return Ok(());
        }

        let abs_path = crate::config::path::strip_verbatim(abs_path);

        // If the file no longer exists, send didClose and return.
        if !abs_path.exists() {
            let key = match self.resolve_server_key(&abs_path).await {
                Ok(k) => k,
                Err(_) => return Ok(()),
            };
            let uri = file_to_uri(&abs_path);
            let server = {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                inner.servers.get(&key).cloned()
            };
            if let Some(server) = server {
                if let Err(e) = server.close_doc(&uri).await {
                    tracing::warn!(
                        error = %e,
                        path = %abs_path.display(),
                        "LSP didClose after file deletion failed"
                    );
                }
            }
            return Ok(());
        }

        let key = self.resolve_server_key(&abs_path).await?;
        let uri = file_to_uri(&abs_path);
        let server = {
            let inner = self
                .inner
                .lock()
                .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
            inner.servers.get(&key).cloned().ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "language server '{}' not running",
                    key.command
                ))
            })?
        };
        server.sync_document_from_disk(&abs_path, &uri).await
    }

    /// Execute a file-scoped LSP request after any in-flight didOpen/didChange
    /// for that URI has been enqueued on stdin.
    async fn request_synced_document(
        &self,
        method: &str,
        params: Value,
        abs_path: &Path,
        rpc_id: Option<u64>,
    ) -> Result<Value> {
        let abs_path = crate::config::path::strip_verbatim(abs_path);
        let key = self.resolve_server_key(&abs_path).await?;
        let uri = file_to_uri(&abs_path);
        let server = self.server_arc(&key)?;
        if !server.is_doc_open(&uri).await {
            server.sync_document_from_disk(&abs_path, &uri).await?;
        }
        server.send_request_synced(&uri, method, params, rpc_id).await
    }

    fn server_arc(&self, key: &ServerKey) -> Result<Arc<LspServer>> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
        inner.servers.get(key).cloned().ok_or_else(|| {
            LitecodeError::ToolExecution(format!(
                "language server '{}' not running",
                key.command
            ))
        })
    }

    pub async fn request(self: &Arc<Self>, method: &str, params: Value) -> Result<Value> {
        self.request_ex(method, params, None).await
    }

    pub async fn request_ex(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        rpc_id: Option<u64>,
    ) -> Result<Value> {
        if method == "$/cancelRequest" {
            let id = params
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| LitecodeError::ToolExecution("$/cancelRequest requires id".into()))?;
            let servers: Vec<Arc<LspServer>> = {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                inner.servers.values().cloned().collect()
            };
            for server in servers {
                server.io.cancel(id);
            }
            return Ok(Value::Null);
        }

        if method == "litecode/getDiagnostics" {
            let uri = params.get("uri").and_then(|u| u.as_str()).ok_or_else(|| {
                LitecodeError::ToolExecution("litecode/getDiagnostics requires uri".into())
            })?;
            let path = uri_to_path(uri)
                .ok_or_else(|| LitecodeError::ToolExecution(format!("invalid uri: {uri}")))?;
            let key = self.resolve_server_key(&path).await?;
            let server = self.server_arc(&key)?;
            if !server.is_doc_open(uri).await {
                server.sync_document_from_disk(&path, uri).await?;
            }
            return Ok(serde_json::json!({
                "diagnostics": server.diagnostics_for_uri(uri)
            }));
        }

        if method == "litecode/didChange" {
            let uri = params.get("uri").and_then(|u| u.as_str()).ok_or_else(|| {
                LitecodeError::ToolExecution("litecode/didChange requires uri".into())
            })?;
            let text = params.get("text").and_then(|t| t.as_str()).ok_or_else(|| {
                LitecodeError::ToolExecution("litecode/didChange requires text".into())
            })?;
            let path = uri_to_path(uri)
                .ok_or_else(|| LitecodeError::ToolExecution(format!("invalid uri: {uri}")))?;
            let key = self.resolve_server_key(&path).await?;
            let server = self.server_arc(&key)?;
            server.sync_document_from_text(&path, uri, text).await?;
            return Ok(serde_json::json!({
                "diagnostics": server.diagnostics_for_uri(uri)
            }));
        }

        if method == "litecode/serverCapabilities" {
            let uri = params.get("uri").and_then(|u| u.as_str()).ok_or_else(|| {
                LitecodeError::ToolExecution("litecode/serverCapabilities requires uri".into())
            })?;
            let path = uri_to_path(uri)
                .ok_or_else(|| LitecodeError::ToolExecution(format!("invalid uri: {uri}")))?;
            let key = self.resolve_server_key(&path).await?;
            let server = self.server_arc(&key)?;
            return Ok(server.editor_client_caps());
        }

        if method.starts_with("textDocument/")
            && let Some(path) = uri_path_from_params(&params)
        {
            return self
                .request_synced_document(method, params, &path, rpc_id)
                .await;
        }

        if method == "shutdown" || method == "exit" {
            let server = {
                let inner = self
                    .inner
                    .lock()
                    .map_err(|e| LitecodeError::Config(format!("lsp hub lock: {e}")))?;
                let key = inner.servers.keys().next().cloned().ok_or_else(|| {
                    LitecodeError::ToolExecution("no language servers".into())
                })?;
                inner.servers.get(&key).cloned().expect("key from pool")
            };
            return server.send_request(method, params).await;
        }

        Err(LitecodeError::ToolExecution(
            "cannot route LSP request: missing textDocument.uri".into(),
        ))
    }

    pub async fn tool_action(
        self: &Arc<Self>,
        action: &str,
        file_path: &Path,
        line: Option<u64>,
        character: Option<u64>,
    ) -> Result<String> {
        self.tool_action_with_query(action, file_path, line, character, None)
            .await
    }

    pub async fn tool_action_with_query(
        self: &Arc<Self>,
        action: &str,
        file_path: &Path,
        line: Option<u64>,
        character: Option<u64>,
        query: Option<&str>,
    ) -> Result<String> {
        self.tool_action_async(action, file_path, line, character, query)
            .await
    }

    /// Post-write/edit feedback: **only** file-local Error diagnostics, and only when
    /// the hub is already active and the language server returns at least one Error.
    ///
    /// Prefer [`Self::file_error_diagnostics_feedback_ex`] when the caller needs to
    /// distinguish hard failures from clean silence.
    pub async fn file_error_diagnostics_feedback(
        self: &Arc<Self>,
        file_path: &Path,
    ) -> Option<String> {
        match self.file_error_diagnostics_feedback_ex(file_path).await {
            LspDiagFeedback::Errors(text) => Some(text),
            LspDiagFeedback::Silence | LspDiagFeedback::Unavailable(_) => None,
        }
    }

    /// Structured diagnostics feedback for write/edit.
    ///
    /// - [`LspDiagFeedback::Errors`] — actionable Error-severity diagnostics
    /// - [`LspDiagFeedback::Unavailable`] — path/spawn/hub hard failure (caller may notice)
    /// - [`LspDiagFeedback::Silence`] — inactive hub, timeout, or no Error diagnostics
    pub async fn file_error_diagnostics_feedback_ex(
        self: &Arc<Self>,
        file_path: &Path,
    ) -> LspDiagFeedback {
        if !self.is_active() {
            return LspDiagFeedback::Silence;
        }
        const BUDGET: Duration = Duration::from_millis(750);
        match tokio::time::timeout(BUDGET, self.collect_file_error_diagnostics(file_path)).await {
            Ok(Ok(Some(text))) if !text.is_empty() => LspDiagFeedback::Errors(text),
            Ok(Ok(_)) => LspDiagFeedback::Silence,
            Ok(Err(e)) => LspDiagFeedback::Unavailable(short_feedback_reason(&e.to_string())),
            Err(_) => LspDiagFeedback::Silence,
        }
    }

    async fn collect_file_error_diagnostics(&self, file_path: &Path) -> Result<Option<String>> {
        let file_path = crate::config::path::strip_verbatim(file_path);
        let uri = file_to_uri(&file_path);
        let key = self.resolve_server_key(&file_path).await?;
        let server = self.server_arc(&key)?;
        server.sync_document_from_disk(&file_path, &uri).await?;
        let found = server
            .wait_file_diagnostics(&uri, Duration::from_millis(600))
            .await;
        Ok(format_error_diagnostics_block(&found))
    }

    async fn tool_action_async(
        &self,
        action: &str,
        file_path: &Path,
        line: Option<u64>,
        character: Option<u64>,
        query: Option<&str>,
    ) -> Result<String> {
        let action = normalize_lsp_action(action);
        let file_path = crate::config::path::strip_verbatim(file_path);
        let uri = file_to_uri(&file_path);
        let lsp_line = line.map(|l| l.saturating_sub(1)).unwrap_or(0);
        let lsp_char = character.map(|c| c.saturating_sub(1)).unwrap_or(0);
        let position = serde_json::json!({ "line": lsp_line, "character": lsp_char });

        let key = self.resolve_server_key(&file_path).await?;
        let server = self.server_arc(&key)?;

        let result = match action {
            "goToDefinition" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                let params = serde_json::json!({
                    "textDocument": { "uri": uri },
                    "position": position
                });
                match server
                    .request_nav_with_retry("textDocument/definition", params.clone())
                    .await
                {
                    Ok(result) if !crate::lsp::format::extract_locations(&result).is_empty() => {
                        result
                    }
                    Ok(empty) => {
                        // Settled empty definition — try typeDefinition before confirming miss.
                        match server
                            .request_nav_with_retry("textDocument/typeDefinition", params)
                            .await
                        {
                            Ok(typed)
                                if !crate::lsp::format::extract_locations(&typed).is_empty() =>
                            {
                                typed
                            }
                            Ok(_) => empty,
                            Err(_) => empty,
                        }
                    }
                    Err(inconclusive) => {
                        // Index not ready for definition — typeDefinition may still hit.
                        match server
                            .request_nav_with_retry("textDocument/typeDefinition", params)
                            .await
                        {
                            Ok(typed)
                                if !crate::lsp::format::extract_locations(&typed).is_empty() =>
                            {
                                typed
                            }
                            _ => return Err(inconclusive),
                        }
                    }
                }
            }
            "findReferences" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .request_nav_with_retry(
                        "textDocument/references",
                        serde_json::json!({
                            "textDocument": { "uri": uri },
                            "position": position,
                            "context": { "includeDeclaration": true }
                        }),
                    )
                    .await?
            }
            "hover" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .request_nav_with_retry(
                        "textDocument/hover",
                        serde_json::json!({
                            "textDocument": { "uri": uri },
                            "position": position
                        }),
                    )
                    .await?
            }
            "goToImplementation" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .request_nav_with_retry(
                        "textDocument/implementation",
                        serde_json::json!({
                            "textDocument": { "uri": uri },
                            "position": position
                        }),
                    )
                    .await?
            }
            "documentSymbol" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .send_request(
                        "textDocument/documentSymbol",
                        serde_json::json!({ "textDocument": { "uri": uri } }),
                    )
                    .await?
            }
            "workspaceSymbol" => {
                // file_path selects/starts the LS; query filters workspace symbols.
                // Opening the document is optional once the workspace is initialized.
                if file_path.is_file() {
                    server.sync_document_from_disk(&file_path, &uri).await?;
                }
                server
                    .send_request(
                        "workspace/symbol",
                        serde_json::json!({ "query": query.unwrap_or("") }),
                    )
                    .await?
            }
            "prepareCallHierarchy" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .send_request(
                        "textDocument/prepareCallHierarchy",
                        serde_json::json!({
                            "textDocument": { "uri": uri },
                            "position": position
                        }),
                    )
                    .await?
            }
            "incomingCalls" | "outgoingCalls" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                let items = server
                    .send_request(
                        "textDocument/prepareCallHierarchy",
                        serde_json::json!({
                            "textDocument": { "uri": uri },
                            "position": position
                        }),
                    )
                    .await?;
                let Some(item) = items.as_array().and_then(|a| a.first()).cloned() else {
                    return Ok("No call hierarchy item at position".into());
                };
                let method = if action == "incomingCalls" {
                    "callHierarchy/incomingCalls"
                } else {
                    "callHierarchy/outgoingCalls"
                };
                server
                    .send_request(method, serde_json::json!({ "item": item }))
                    .await?
            }
            "diagnostics" => {
                server.sync_document_from_disk(&file_path, &uri).await?;
                server
                    .wait_file_diagnostics(&uri, Duration::from_secs(3))
                    .await
            }
            other => {
                return Err(LitecodeError::ToolExecution(format!(
                    "unknown lsp action: {other}"
                )));
            }
        };

        Ok(format_action_result(action, &result))
    }
}

pub(crate) fn normalize_lsp_action(action: &str) -> &str {
    match action {
        "definition" | "goToDefinition" => "goToDefinition",
        "references" | "findReferences" => "findReferences",
        "implementation" | "goToImplementation" => "goToImplementation",
        other => other,
    }
}

pub(crate) fn uri_path_from_params(params: &Value) -> Option<PathBuf> {
    let uri = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(|u| u.as_str())
        .or_else(|| params.get("uri").and_then(|u| u.as_str()))?;
    uri_to_path(uri)
}

impl Default for LspHub {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LspHub {
    fn drop(&mut self) {
        // 2.11: shut down the LSP servers so no child process / stderr task
        // outlives the hub (previously the runtime — and its running servers —
        // were leaked outright).
        //
        // Dropping a tokio `Runtime` from within an async runtime context
        // panics, and `block_on` from inside a runtime deadlocks the worker, so
        // `LspHub` (routinely dropped inside a runtime, e.g. when an
        // `AgentRuntime`'s engine handle is released at the end of a turn) must
        // not block_on here. Two paths:
        //
        // - Outside a runtime: perform a proper blocking shutdown (graceful
        //   didClose + shutdown + kill + wait), which also lets the stderr
        //   readers EOF.
        // - Inside a runtime: kill every child synchronously; the stderr reader
        //   tasks then reach EOF and finish on the (leaked) runtime.
        let inside_runtime = tokio::runtime::Handle::try_current().is_ok();
        let servers: Vec<Arc<LspServer>> = self
            .inner
            .lock()
            .ok()
            .map(|mut inner| inner.servers.drain().map(|(_, v)| v).collect())
            .unwrap_or_default();
        if inside_runtime {
            for server in &servers {
                server.kill_child();
            }
        } else {
            (*self.rt).block_on(async {
                for server in servers {
                    server.shutdown().await;
                }
            });
        }
        // `rt` is `ManuallyDrop`, so `drop_in_place::<LspHub>` will NOT drop the
        // field on its own. We extract it here and `forget` it; this avoids the
        // double-drop that `ptr::read` + `forget` would otherwise cause (the
        // field would be dropped again as an uninitialized value).
        // SAFETY: `ManuallyDrop::take` moves the `Runtime` out; we never touch
        // `self.rt` again, and `forget` prevents its `Drop` from running.
        let rt = unsafe { std::mem::ManuallyDrop::take(&mut self.rt) };
        std::mem::forget(rt);
    }
}

pub type SharedLspHub = Arc<LspHub>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_budget_resets_after_cooldown() {
        // A server that survived the cooldown is a healthy start: exit resets the budget.
        assert_eq!(
            effective_restart_count(2, RESTART_COOLDOWN, false),
            0,
            "healthy lifetime must reset the restart budget"
        );
        assert_eq!(
            effective_restart_count(0, RESTART_COOLDOWN, false),
            0,
            "reset from zero stays zero"
        );
        assert_eq!(
            effective_restart_count(2, RESTART_COOLDOWN, true),
            2,
            "watchdog kill must not reset the budget"
        );
        // Rapid crash loop keeps counting.
        assert_eq!(effective_restart_count(0, Duration::from_secs(5), false), 0);
        assert_eq!(effective_restart_count(1, Duration::from_secs(5), false), 1);
        assert_eq!(effective_restart_count(2, Duration::from_secs(5), false), 2);
        // Near-zero lifetime counts as a crash loop (well below cooldown).
        assert_eq!(
            effective_restart_count(2, Duration::from_millis(0), false),
            2
        );
    }
}
