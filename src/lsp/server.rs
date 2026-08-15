//! Per-process language server: JSON-RPC transport, document sync, indexing wait.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::time::{Duration, timeout};

use crate::lsp::format::extract_locations;
use crate::lsp::install::LanguageServerBinary;
use crate::lsp::status::{LspInstanceStatus, LspLifecycle};
use crate::lsp::uri::file_to_uri;
use crate::types::{LitecodeError, Result};

static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

/// Max automatic restarts after unexpected exit within the cooldown window.
pub(crate) const MAX_AUTO_RESTARTS: u32 = 2;
/// A server that survived at least this long is a healthy start: its exit
/// resets the restart budget (only rapid crash loops count against it).
pub(crate) const RESTART_COOLDOWN: Duration = Duration::from_secs(60);
/// Cap on a single LSP message body (defensive against a broken/malicious
/// server claiming a huge Content-Length).
const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
/// Cap on accumulated stderr (rotating buffer keeps the most recent bytes).
const MAX_LSP_STDERR_BYTES: usize = 64 * 1024;

/// Parse a `Content-Length:` header line, enforcing the message cap (2.11).
/// Returns `Ok(None)` when the line is not a Content-Length header.
fn parse_content_length(line: &str) -> Result<Option<usize>> {
    let Some(len_str) = line.trim().strip_prefix("Content-Length:") else {
        return Ok(None);
    };
    let parsed: usize = len_str
        .trim()
        .parse()
        .map_err(|e| LitecodeError::ToolExecution(format!("bad Content-Length: {e}")))?;
    if parsed > MAX_LSP_MESSAGE_BYTES {
        return Err(LitecodeError::ToolExecution(format!(
            "Content-Length {parsed} exceeds limit {MAX_LSP_MESSAGE_BYTES}"
        )));
    }
    Ok(Some(parsed))
}

/// Append a raw stderr line to the rotating buffer, keeping only the most
/// recent `MAX_LSP_STDERR_BYTES` (bound stderr accumulation, 2.11).
fn append_stderr_rotated(buf: &mut Vec<u8>, line: &[u8]) {
    buf.extend_from_slice(line);
    if buf.len() > MAX_LSP_STDERR_BYTES {
        let excess = buf.len() - MAX_LSP_STDERR_BYTES;
        buf.drain(..excess);
    }
}

pub(crate) struct OpenDocEntry {
    pub(crate) uri: String,
    opened_at: Instant,
}

const MAX_OPEN_DOCS: usize = 20;
const MIN_OPEN_DURATION: Duration = Duration::from_secs(30);
/// Max time to drain post-`initialized` server→client requests so servers
/// that pull `workspace/configuration` can start. Indexing continues after;
/// do not wait for rust-analyzer `quiescent` here.
const POST_INIT_DRAIN_BUDGET: Duration = Duration::from_secs(2);
/// Exit the post-init drain after this much silence on stdout.
const POST_INIT_DRAIN_QUIET: Duration = Duration::from_millis(150);
/// Hard cap on a single stdin write. A wedged server (full stdout pipe) must
/// not block the hub `op_gate` / Agent turn indefinitely.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Until this elapses (or a real nav hit / quiescent), empty navigation is
/// treated as inconclusive — covers csharp-ls / gopls / pyright / RA cold start.
const INDEX_GRACE: Duration = Duration::from_secs(20);

pub(crate) struct LspServer {
    pub(crate) _command: String,
    pub(crate) child: Child,
    pub(crate) stdin: ChildStdin,
    pub(crate) reader: BufReader<tokio::process::ChildStdout>,
    pub(crate) open_docs: VecDeque<OpenDocEntry>,
    pub(crate) open_docs_set: HashSet<String>,
    /// Per-document LSP versions. A language server may reject repeated
    /// didChange notifications with a fixed or regressing version.
    pub(crate) document_versions: HashMap<String, i32>,
    pub(crate) stderr_buf: Arc<tokio::sync::Mutex<Vec<u8>>>,
    /// When this process was spawned. Used to bound indexing-wait retries.
    pub(crate) spawned_at: Instant,
    /// True after quiescent (RA), a real nav hit, or [`INDEX_GRACE`] expiry.
    /// When false, empty navigation results are treated as inconclusive errors —
    /// never as a successful "No locations found".
    pub(crate) index_settled: bool,
    pub(crate) lifecycle: LspLifecycle,
    pub(crate) last_error: Option<String>,
    pub(crate) restart_count: u32,
    /// True when this process was killed by the I/O watchdog (not a natural exit).
    pub(crate) watchdog_kill: bool,
}

impl LspServer {
    pub(crate) async fn spawn(binary: &LanguageServerBinary, root_path: &Path) -> Result<Self> {
        let (program, args) =
            crate::lsp::install::ls_program_and_args(&binary.path, &binary.arguments);
        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(root_path);

        if let Some(env) = &binary.env {
            for (k, v) in env {
                cmd.env(k, v);
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            LitecodeError::ToolExecution(format!(
                "failed to spawn language server '{}': {e}",
                binary.path.display()
            ))
        })?;

        let stderr_buf: Arc<tokio::sync::Mutex<Vec<u8>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let buf = stderr_buf.clone();
            tokio::spawn(async move {
                let mut r = BufReader::new(stderr);
                let mut line = String::new();
                loop {
                    line.clear();
                    match r.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                tracing::debug!(stderr = %trimmed, "LSP stderr");
                                let mut buf = buf.lock().await;
                                append_stderr_rotated(&mut buf, line.as_bytes());
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let reader = BufReader::new(stdout);
        let command = binary.path.to_string_lossy().to_string();
        let mut server = Self {
            _command: command.clone(),
            child,
            stdin,
            reader,
            open_docs: VecDeque::new(),
            open_docs_set: HashSet::new(),
            document_versions: HashMap::new(),
            stderr_buf: stderr_buf.clone(),
            spawned_at: Instant::now(),
            index_settled: false,
            lifecycle: LspLifecycle::Starting,
            last_error: None,
            restart_count: 0,
            watchdog_kill: false,
        };

        let root_uri = file_to_uri(root_path);
        if let Err(e) = server.initialize(&root_uri, root_path).await {
            let stderr_content = {
                let buf = server.stderr_buf.lock().await;
                String::from_utf8_lossy(&buf).to_string()
            };
            let _ = server.child.start_kill();
            let _ = server.child.wait().await;
            let detail = if stderr_content.is_empty() {
                format!("LSP initialize failed: {e}")
            } else {
                format!("LSP initialize failed: {e}\nstderr:\n{stderr_content}")
            };
            return Err(LitecodeError::ToolExecution(detail));
        }
        // Drain/ack post-initialize server→client requests (configuration,
        // registerCapability). Indexing continues in the background.
        server.wait_until_ready_for_nav(&command).await;
        server.lifecycle = LspLifecycle::Running;
        Ok(server)
    }

    pub(crate) fn mark_failed(&mut self, err: impl Into<String>) {
        self.lifecycle = LspLifecycle::Failed;
        self.last_error = Some(err.into());
    }

    /// I/O watchdog: mark Failed, kill the child. Next `resolve_server_key`
    /// uses the existing auto-restart budget. Indexing slowness is not a stall.
    fn fail_and_kill(&mut self, err: impl Into<String>) {
        self.watchdog_kill = true;
        self.mark_failed(err);
        self.kill_child();
    }

    /// Ack pending server→client traffic so a busy indexer cannot fill the
    /// stdout pipe and stall the next stdin write.
    async fn drain_pending(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut n = 0u32;
        while Instant::now() < deadline && n < 64 {
            match timeout(Duration::from_millis(20), self.read_message()).await {
                Ok(Ok(msg)) => {
                    n += 1;
                    if Self::is_server_request(&msg) {
                        let _ = self.reply_server_request(&msg).await;
                    }
                    self.mark_settled_from_status(&msg);
                }
                _ => break,
            }
        }
    }

    pub(crate) fn is_process_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                if self.lifecycle != LspLifecycle::Failed && self.lifecycle != LspLifecycle::Stopped
                {
                    self.mark_failed("language server process exited");
                }
                false
            }
            Err(e) => {
                self.mark_failed(format!("language server process wait failed: {e}"));
                false
            }
        }
    }

    pub(crate) fn status_snapshot(&self, project_root: &Path) -> LspInstanceStatus {
        LspInstanceStatus {
            command: self._command.clone(),
            project_root: project_root.display().to_string(),
            state: self.lifecycle,
            index_settled: self.index_settled,
            last_error: self.last_error.clone(),
            restart_count: self.restart_count,
        }
    }

    pub(crate) fn message_id_u64(value: &Value) -> Option<u64> {
        value.get("id").and_then(|id| {
            id.as_u64()
                .or_else(|| id.as_i64().and_then(|i| u64::try_from(i).ok()))
        })
    }

    pub(crate) async fn reply_server_request(&mut self, request: &Value) -> Result<()> {
        let Some(id) = request.get("id").cloned() else {
            return Ok(());
        };
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        // Minimal auto-acks so the server does not stall while we wait for our
        // own response. Values match common LSP client stubs.
        let result = match method {
            "workspace/configuration" => Self::configuration_response(request),
            "window/workDoneProgress/create"
            | "client/registerCapability"
            | "client/unregisterCapability"
            | "window/showMessageRequest"
            | "workspace/applyEdit" => Value::Null,
            _ => Value::Null,
        };
        self.write_raw(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .await
    }

    pub(crate) fn is_server_request(msg: &Value) -> bool {
        msg.get("method").is_some() && msg.get("id").is_some()
    }

    /// Build `workspace/configuration` result items from a server request.
    ///
    /// Section roots (e.g. `"csharp"`, `"rust-analyzer"`) get `{}` so servers can
    /// deserialize defaults. Nested/scalar keys and empty sections get `null`
    /// (LSP: no configuration available).
    pub(crate) fn configuration_response(request: &Value) -> Value {
        let items = request
            .pointer("/params/items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Value::Array(
            items
                .iter()
                .map(|item| {
                    let section = item.get("section").and_then(|s| s.as_str()).unwrap_or("");
                    if section.is_empty() || section.contains('.') {
                        Value::Null
                    } else {
                        Value::Object(serde_json::Map::new())
                    }
                })
                .collect(),
        )
    }

    pub(crate) fn is_quiescent_status(msg: &Value) -> bool {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        if method != "experimental/serverStatus" && method != "$/rust-analyzer/status" {
            return false;
        }
        msg.pointer("/params/quiescent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Empty nav may be reported as a genuine miss only when the index is ready.
    pub(crate) fn index_ready_for_empty_miss(&self) -> bool {
        self.index_settled || self.spawned_at.elapsed() >= INDEX_GRACE
    }

    /// After `initialized`: drain/ack server→client requests so servers that
    /// pull `workspace/configuration` can start. Same for every language —
    /// return as soon as stdout is quiet. Do not wait for index `quiescent`.
    pub(crate) async fn wait_until_ready_for_nav(&mut self, command: &str) {
        let _ = command;
        if self.index_settled {
            return;
        }
        let deadline = Instant::now() + POST_INIT_DRAIN_BUDGET;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let poll = remaining.min(POST_INIT_DRAIN_QUIET);
            match timeout(poll, self.read_message()).await {
                Ok(Ok(msg)) => {
                    if Self::is_server_request(&msg) {
                        let _ = self.reply_server_request(&msg).await;
                    }
                    if Self::is_quiescent_status(&msg) {
                        self.index_settled = true;
                        return;
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    // Quiet — handshake drain done; indexing continues off this path.
                    break;
                }
            }
        }
        // Do NOT mark settled on drain timeout — empty nav stays inconclusive
        // until INDEX_GRACE / a real hit / a later quiescent notification.
    }

    pub(crate) fn mark_settled_from_status(&mut self, msg: &Value) {
        if Self::is_quiescent_status(msg) {
            self.index_settled = true;
        }
    }

    /// Navigation that must not lie: empty while the index is unsettled is an error.
    pub(crate) async fn request_nav_with_retry(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        fn nav_empty(method: &str, result: &Value) -> bool {
            if method == "textDocument/hover" {
                return result.is_null()
                    || result
                        .get("contents")
                        .map(|c| match c {
                            Value::String(s) => s.trim().is_empty(),
                            Value::Object(o) => o
                                .get("value")
                                .and_then(|v| v.as_str())
                                .map(|s| s.trim().is_empty())
                                .unwrap_or(true),
                            Value::Array(a) => a.is_empty(),
                            _ => true,
                        })
                        .unwrap_or(true);
            }
            extract_locations(result).is_empty()
        }

        let mut result = self.send_request(method, params.clone()).await?;
        if !nav_empty(method, &result) {
            // A real hit is authoritative even before quiescent / grace.
            self.index_settled = true;
            return Ok(result);
        }

        if self.index_ready_for_empty_miss() {
            // Index known-ready (or grace elapsed): empty is a genuine miss.
            self.index_settled = true;
            return Ok(result);
        }

        // Unsettled index: retry, then fail honestly — never pretend "no symbol".
        for delay_ms in [500_u64, 1000, 2000, 3000] {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            result = self.send_request(method, params.clone()).await?;
            if !nav_empty(method, &result) {
                self.index_settled = true;
                return Ok(result);
            }
            if self.index_ready_for_empty_miss() {
                self.index_settled = true;
                return Ok(result);
            }
        }

        Err(LitecodeError::ToolExecution(format!(
            "language server index not ready (method={method}); result would be inconclusive. \
             Retry in a few seconds — do not treat this as 'symbol not found'."
        )))
    }

    pub(crate) async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed);
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        match tokio::time::timeout(Duration::from_secs(30), async {
            self.write_message(&request).await?;
            loop {
                let response = self.read_message().await?;
                // Server → client request: ack and keep waiting for our id.
                if Self::is_server_request(&response) {
                    self.reply_server_request(&response).await?;
                    continue;
                }
                // Notification (progress / diagnostics / serverStatus).
                if response.get("id").is_none() {
                    self.mark_settled_from_status(&response);
                    continue;
                }
                if Self::message_id_u64(&response) == Some(id) {
                    if let Some(error) = response.get("error") {
                        let msg = error["message"].as_str().unwrap_or("unknown error");
                        let code = error["code"].as_i64().unwrap_or(-1);
                        return Err(LitecodeError::ToolExecution(format!(
                            "LSP error (code {code}): {msg}"
                        )));
                    }
                    return Ok(response.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        })
        .await
        {
            Ok(inner) => inner,
            Err(_) => {
                self.fail_and_kill("LSP request timed out after 30s");
                Err(LitecodeError::ToolExecution(
                    "LSP request timed out after 30s".into(),
                ))
            }
        }
    }

    pub(crate) async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_message(&notification).await
    }

    pub(crate) async fn write_message(&mut self, value: &Value) -> Result<()> {
        self.drain_pending().await;
        if self.write_raw(value).await.is_ok() {
            return Ok(());
        }
        self.drain_pending().await;
        match self.write_raw(value).await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.fail_and_kill(e.to_string());
                Err(e)
            }
        }
    }

    async fn write_raw(&mut self, value: &Value) -> Result<()> {
        let body = serde_json::to_string(value)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let write = async {
            self.stdin.write_all(header.as_bytes()).await.map_err(|e| {
                LitecodeError::ToolExecution(format!("LSP write header failed: {e}"))
            })?;
            self.stdin
                .write_all(body.as_bytes())
                .await
                .map_err(|e| LitecodeError::ToolExecution(format!("LSP write body failed: {e}")))?;
            self.stdin
                .flush()
                .await
                .map_err(|e| LitecodeError::ToolExecution(format!("LSP flush failed: {e}")))?;
            Ok::<(), LitecodeError>(())
        };
        match timeout(WRITE_TIMEOUT, write).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(LitecodeError::ToolExecution("LSP write timed out".into())),
        }
    }

    pub(crate) async fn read_message(&mut self) -> Result<Value> {
        let mut content_length = 0usize;
        loop {
            let mut header_line = String::new();
            let bytes = self.reader.read_line(&mut header_line).await.map_err(|e| {
                let msg = format!("LSP read header failed: {e}");
                self.mark_failed(&msg);
                LitecodeError::ToolExecution(msg)
            })?;
            if bytes == 0 {
                let msg = "language server closed stdout".to_string();
                self.mark_failed(&msg);
                return Err(LitecodeError::ToolExecution(msg));
            }
            let trimmed = header_line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some(parsed) = parse_content_length(trimmed)? {
                content_length = parsed;
            }
        }
        if content_length == 0 {
            return Err(LitecodeError::ToolExecution(
                "missing Content-Length".into(),
            ));
        }
        let mut buf = vec![0u8; content_length];
        self.reader.read_exact(&mut buf).await.map_err(|e| {
            let msg = format!("LSP read body failed: {e}");
            self.mark_failed(&msg);
            LitecodeError::ToolExecution(msg)
        })?;
        let body = String::from_utf8(buf)
            .map_err(|e| LitecodeError::ToolExecution(format!("invalid utf8: {e}")))?;
        serde_json::from_str(&body)
            .map_err(|e| LitecodeError::ToolExecution(format!("invalid json: {e}")))
    }

    pub(crate) async fn initialize(&mut self, root_uri: &str, root_path: &Path) -> Result<()> {
        let folder_name = root_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("workspace");
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "rootPath": root_path.to_string_lossy(),
            "capabilities": {
                "workspace": {
                    "workspaceFolders": true,
                    "configuration": true
                },
                "textDocument": {
                    "definition": { "linkSupport": true },
                    "typeDefinition": { "linkSupport": true },
                    "implementation": { "linkSupport": true },
                    "references": {},
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "publishDiagnostics": { "relatedInformation": false }
                },
                "window": {
                    "workDoneProgress": true
                },
                "experimental": {
                    "serverStatusNotification": true
                }
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": folder_name
            }]
        });
        self.send_request("initialize", params).await?;
        self.send_notification("initialized", serde_json::json!({}))
            .await?;
        // Push initial workspace configuration.
        self.push_initial_configuration().await?;
        Ok(())
    }

    pub(crate) async fn push_initial_configuration(&mut self) -> Result<()> {
        // Push pythonPath for Pyright from environment variable.
        let python_path = std::env::var("LITECODE_PYTHON_PATH").ok().or_else(|| {
            // Auto-detect .venv if exists.
            let venv = Path::new(".venv");
            if venv.is_dir() {
                Some(venv.to_string_lossy().to_string())
            } else {
                None
            }
        });

        if let Some(python_path) = python_path {
            let settings = serde_json::json!({
                "settings": {
                    "python": {
                        "pythonPath": python_path
                    }
                }
            });
            self.send_notification("workspace/didChangeConfiguration", settings)
                .await?;
        }

        Ok(())
    }

    /// Evict oldest documents past the max limit, respecting min retention.
    pub(crate) async fn evict_old_docs(&mut self) -> Result<()> {
        while self.open_docs.len() > MAX_OPEN_DOCS {
            let oldest = &self.open_docs[0];
            // Only evict if past min retention.
            if oldest.opened_at.elapsed() < MIN_OPEN_DURATION {
                break;
            }
            let entry = self.open_docs.pop_front().unwrap();
            self.open_docs_set.remove(&entry.uri);
            self.document_versions.remove(&entry.uri);
            self.send_notification(
                "textDocument/didClose",
                serde_json::json!({ "textDocument": { "uri": &entry.uri } }),
            )
            .await?;
        }
        Ok(())
    }

    /// Promote an existing doc to the back of the LRU queue (mark as recently used).
    pub(crate) fn touch_doc(&mut self, uri: &str) {
        if let Some(pos) = self.open_docs.iter().position(|e| e.uri == uri) {
            let entry = self.open_docs.remove(pos).unwrap();
            self.open_docs.push_back(entry);
        }
    }

    pub(crate) async fn sync_document_from_disk(
        &mut self,
        file_path: &Path,
        uri: &str,
    ) -> Result<()> {
        let text = std::fs::read_to_string(file_path).map_err(|e| {
            LitecodeError::ToolExecution(format!("read '{}': {e}", file_path.display()))
        })?;
        if self.open_docs_set.contains(uri) {
            self.touch_doc(uri);
            let version = self
                .document_versions
                .entry(uri.to_string())
                .and_modify(|version| *version = version.saturating_add(1))
                .or_insert(2)
                .to_owned();
            self.send_notification(
                "textDocument/didChange",
                serde_json::json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [{ "text": text }]
                }),
            )
            .await?;
        } else {
            let language_id = file_path
                .extension()
                .and_then(|e| e.to_str())
                .map(crate::lsp::project_root::lsp_language_id)
                .unwrap_or("plaintext");
            self.send_notification(
                "textDocument/didOpen",
                serde_json::json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": 1,
                        "text": text,
                    }
                }),
            )
            .await?;
            self.open_docs.push_back(OpenDocEntry {
                uri: uri.to_string(),
                opened_at: Instant::now(),
            });
            self.open_docs_set.insert(uri.to_string());
            self.document_versions.insert(uri.to_string(), 1);
            self.evict_old_docs().await?;
        }
        Ok(())
    }

    pub(crate) async fn collect_notifications(&mut self, duration: Duration) -> Result<Vec<Value>> {
        let mut notifications = Vec::new();
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match timeout(remaining, self.read_message()).await {
                Ok(Ok(msg)) => {
                    // Must ack server→client requests (e.g. workspace/configuration).
                    // Swallowing them leaves csharp-ls / similar blocked on solution load.
                    if Self::is_server_request(&msg) {
                        let _ = self.reply_server_request(&msg).await;
                        continue;
                    }
                    self.mark_settled_from_status(&msg);
                    notifications.push(msg);
                }
                Ok(Err(_)) | Err(_) => break,
            }
        }
        Ok(notifications)
    }

    /// Synchronously kill the child process (no async wait). Used from `Drop`
    /// so no LSP process outlives the hub (2.11); the stderr reader task then
    /// reaches EOF and finishes on its own.
    pub(crate) fn kill_child(&mut self) {
        let _ = self.child.start_kill();
    }

    pub(crate) async fn shutdown(&mut self) {
        let open_uris: Vec<String> = self.open_docs.iter().map(|e| e.uri.clone()).collect();
        for uri in &open_uris {
            let _ = self
                .send_notification(
                    "textDocument/didClose",
                    serde_json::json!({ "textDocument": { "uri": uri } }),
                )
                .await;
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            self.send_request("shutdown", serde_json::json!({})),
        )
        .await;
        let _ = self.send_notification("exit", serde_json::json!({})).await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        self.lifecycle = LspLifecycle::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_length_parses_and_caps() {
        assert_eq!(parse_content_length("not a header").unwrap(), None);
        assert_eq!(
            parse_content_length("Content-Length: 42").unwrap(),
            Some(42)
        );
        assert_eq!(
            parse_content_length("  Content-Length: 7  ").unwrap(),
            Some(7)
        );
        // Oversized messages are rejected before any allocation.
        let too_big = format!("Content-Length: {}", MAX_LSP_MESSAGE_BYTES + 1);
        let err = parse_content_length(&too_big).unwrap_err();
        assert!(err.to_string().contains("exceeds limit"), "got: {err}");
        // Malformed lengths are rejected.
        assert!(parse_content_length("Content-Length: nope").is_err());
    }

    #[test]
    fn stderr_buffer_is_bounded_and_rotates() {
        // Below the cap the buffer accumulates verbatim.
        let mut buf = Vec::new();
        append_stderr_rotated(&mut buf, b"line one\n");
        append_stderr_rotated(&mut buf, b"line two\n");
        assert_eq!(buf, b"line one\nline two\n");

        // Overflow keeps only the most recent MAX_LSP_STDERR_BYTES (2.11 bound).
        let big = "x".repeat(MAX_LSP_STDERR_BYTES);
        let mut buf = Vec::new();
        append_stderr_rotated(&mut buf, b"PREFIX ");
        append_stderr_rotated(&mut buf, big.as_bytes());
        assert!(
            buf.len() <= MAX_LSP_STDERR_BYTES,
            "rotated buffer must not exceed cap, got {}",
            buf.len()
        );
        // The oldest prefix was rotated out; the recent tail survives.
        let text = String::from_utf8_lossy(&buf);
        assert!(
            !text.contains("PREFIX"),
            "old prefix must be rotated out: {text}"
        );
        assert!(text.ends_with("x"), "recent bytes must survive: {text}");
    }
}
