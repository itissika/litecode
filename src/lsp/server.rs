//! Per-process language server: document sync on top of multiplexed JSON-RPC.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use serde_json::Value;
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio::time::Duration;

use crate::lsp::conn::{self, LspIo};
use crate::lsp::format::extract_locations;
use crate::lsp::install::LanguageServerBinary;
use crate::lsp::status::{LspInstanceStatus, LspLifecycle};
use crate::lsp::uri::{file_to_uri, publish_diagnostics_uri_matches};
use crate::types::{LitecodeError, Result};

/// Max automatic restarts after unexpected exit within the cooldown window.
pub(crate) const MAX_AUTO_RESTARTS: u32 = 2;
/// A server that survived at least this long is a healthy start: its exit
/// resets the restart budget (only rapid crash loops count against it).
pub(crate) const RESTART_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_LSP_STDERR_BYTES: usize = 64 * 1024;

const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];
const SEMANTIC_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];

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
const POST_INIT_DRAIN_QUIET: Duration = Duration::from_millis(150);
const INDEX_GRACE: Duration = Duration::from_secs(20);

struct DocState {
    open_docs: VecDeque<OpenDocEntry>,
    open_docs_set: HashSet<String>,
    document_versions: HashMap<String, i32>,
    document_text: HashMap<String, String>,
    uri_gates: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl DocState {
    fn new() -> Self {
        Self {
            open_docs: VecDeque::new(),
            open_docs_set: HashSet::new(),
            document_versions: HashMap::new(),
            document_text: HashMap::new(),
            uri_gates: HashMap::new(),
        }
    }

    fn gate(&mut self, uri: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.uri_gates
            .entry(uri.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}

pub(crate) struct LspServer {
    pub(crate) _command: String,
    child: tokio::sync::Mutex<Child>,
    pub(crate) io: Arc<LspIo>,
    docs: tokio::sync::Mutex<DocState>,
    pub(crate) stderr_buf: Arc<tokio::sync::Mutex<Vec<u8>>>,
    pub(crate) spawned_at: Instant,
    pub(crate) restart_count: u32,
    server_capabilities: std::sync::Mutex<Value>,
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
                    match tokio::io::AsyncBufReadExt::read_line(&mut r, &mut line).await {
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
        let pid = child.id().unwrap_or(0);
        let io = LspIo::start(stdin, stdout, pid);
        let command = binary.path.to_string_lossy().to_string();
        let server = Self {
            _command: command,
            child: tokio::sync::Mutex::new(child),
            io,
            docs: tokio::sync::Mutex::new(DocState::new()),
            stderr_buf: stderr_buf.clone(),
            spawned_at: Instant::now(),
            restart_count: 0,
            server_capabilities: std::sync::Mutex::new(Value::Null),
        };

        let root_uri = file_to_uri(root_path);
        if let Err(e) = server.initialize(&root_uri, root_path).await {
            let stderr_content = {
                let buf = server.stderr_buf.lock().await;
                String::from_utf8_lossy(&buf).to_string()
            };
            server.kill_child();
            let detail = if stderr_content.is_empty() {
                format!("LSP initialize failed: {e}")
            } else {
                format!("LSP initialize failed: {e}\nstderr:\n{stderr_content}")
            };
            return Err(LitecodeError::ToolExecution(detail));
        }
        tokio::time::sleep(POST_INIT_DRAIN_QUIET).await;
        if let Ok(mut g) = server.io.lifecycle.lock() {
            *g = LspLifecycle::Running;
        }
        Ok(server)
    }

    pub(crate) fn mark_failed(&self, err: impl Into<String>) {
        self.io.mark_failed(err);
    }

    pub(crate) fn is_process_alive(&self) -> bool {
        if self.io.io_failed.load(Ordering::SeqCst) {
            return false;
        }
        let Ok(mut child) = self.child.try_lock() else {
            return true;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => {
                self.io.mark_failed("language server process exited");
                false
            }
            Err(e) => {
                self.io
                    .mark_failed(format!("language server process wait failed: {e}"));
                false
            }
        }
    }

    pub(crate) fn lifecycle(&self) -> LspLifecycle {
        self.io
            .lifecycle
            .lock()
            .map(|g| *g)
            .unwrap_or(LspLifecycle::Failed)
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.io.last_error.lock().ok().and_then(|g| g.clone())
    }

    pub(crate) fn watchdog_kill(&self) -> bool {
        self.io.watchdog_kill.load(Ordering::SeqCst)
    }

    pub(crate) fn status_snapshot(&self, project_root: &Path) -> LspInstanceStatus {
        LspInstanceStatus {
            command: self._command.clone(),
            project_root: project_root.display().to_string(),
            state: self.lifecycle(),
            index_settled: self.io.index_settled.load(Ordering::SeqCst),
            last_error: self.last_error(),
            restart_count: self.restart_count,
        }
    }

    pub(crate) fn configuration_response(request: &Value) -> Value {
        conn::configuration_response(request)
    }

    pub(crate) fn index_ready_for_empty_miss(&self) -> bool {
        self.io.index_settled.load(Ordering::SeqCst) || self.spawned_at.elapsed() >= INDEX_GRACE
    }

    pub(crate) async fn request_nav_with_retry(
        &self,
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
            self.io.index_settled.store(true, Ordering::SeqCst);
            return Ok(result);
        }

        if self.index_ready_for_empty_miss() {
            self.io.index_settled.store(true, Ordering::SeqCst);
            return Ok(result);
        }

        for delay_ms in [500_u64, 1000, 2000, 3000] {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            result = self.send_request(method, params.clone()).await?;
            if !nav_empty(method, &result) {
                self.io.index_settled.store(true, Ordering::SeqCst);
                return Ok(result);
            }
            if self.index_ready_for_empty_miss() {
                self.io.index_settled.store(true, Ordering::SeqCst);
                return Ok(result);
            }
        }

        Err(LitecodeError::ToolExecution(format!(
            "language server index not ready (method={method}); result would be inconclusive. \
             Retry in a few seconds — do not treat this as 'symbol not found'."
        )))
    }

    pub(crate) async fn send_request(&self, method: &str, params: Value) -> Result<Value> {
        self.send_request_id(method, params, None).await
    }

    pub(crate) async fn send_request_id(
        &self,
        method: &str,
        params: Value,
        rpc_id: Option<u64>,
    ) -> Result<Value> {
        self.io.request(method, params, rpc_id).await
    }

    /// Wait until in-flight didOpen/didChange for `uri` has been enqueued, then
    /// send the request without holding the gate (so other requests can overlap).
    pub(crate) async fn send_request_synced(
        &self,
        uri: &str,
        method: &str,
        params: Value,
        rpc_id: Option<u64>,
    ) -> Result<Value> {
        {
            let gate = self.uri_gate(uri).await;
            let _g = gate.lock().await;
        }
        self.send_request_id(method, params, rpc_id).await
    }

    pub(crate) async fn send_notification(&self, method: &str, params: Value) -> Result<()> {
        self.io.notify(method, params).await
    }

    pub(crate) async fn initialize(&self, root_uri: &str, root_path: &Path) -> Result<()> {
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
                    "synchronization": {
                        "dynamicRegistration": false,
                        "willSave": false,
                        "willSaveWaitUntil": false,
                        "didSave": true
                    },
                    "definition": { "linkSupport": true },
                    "declaration": { "linkSupport": true },
                    "typeDefinition": { "linkSupport": true },
                    "implementation": { "linkSupport": true },
                    "references": {},
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "publishDiagnostics": { "relatedInformation": false },
                    "signatureHelp": {
                        "signatureInformation": {
                            "documentationFormat": ["plaintext", "markdown"],
                            "parameterInformation": { "labelOffsetSupport": true }
                        }
                    },
                    "documentHighlight": {},
                    "selectionRange": {},
                    "linkedEditingRange": {},
                    "inlayHint": {
                        "resolveSupport": {
                            "properties": ["tooltip", "textEdits", "label.location"]
                        }
                    },
                    "codeLens": {},
                    "completion": {
                        "contextSupport": true,
                        "completionItem": {
                            "snippetSupport": false,
                            "documentationFormat": ["plaintext", "markdown"]
                        }
                    },
                    "semanticTokens": {
                        "requests": { "range": false, "full": { "delta": false } },
                        "tokenTypes": SEMANTIC_TOKEN_TYPES,
                        "tokenModifiers": SEMANTIC_TOKEN_MODIFIERS,
                        "formats": ["relative"],
                        "overlappingTokenSupport": false,
                        "multilineTokenSupport": true
                    }
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
        let init_result = self.send_request("initialize", params).await?;
        if let Ok(mut g) = self.server_capabilities.lock() {
            *g = init_result
                .get("capabilities")
                .cloned()
                .unwrap_or(Value::Null);
        }
        self.send_notification("initialized", serde_json::json!({}))
            .await?;
        self.push_initial_configuration().await?;
        Ok(())
    }

    pub(crate) async fn push_initial_configuration(&self) -> Result<()> {
        let python_path = std::env::var("LITECODE_PYTHON_PATH").ok().or_else(|| {
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

    fn uses_incremental(&self) -> bool {
        let Ok(caps) = self.server_capabilities.lock() else {
            return false;
        };
        match &caps["textDocumentSync"] {
            Value::Number(n) => n.as_i64() == Some(2),
            Value::Object(o) => o.get("change").and_then(|v| v.as_i64()) == Some(2),
            _ => false,
        }
    }

    pub(crate) async fn evict_old_docs(&self) -> Result<()> {
        loop {
            let (uri, should_stop) = {
                let mut docs = self.docs.lock().await;
                if docs.open_docs.len() <= MAX_OPEN_DOCS {
                    break;
                }
                let oldest = &docs.open_docs[0];
                if oldest.opened_at.elapsed() < MIN_OPEN_DURATION {
                    break;
                }
                let entry = docs.open_docs.pop_front().unwrap();
                docs.open_docs_set.remove(&entry.uri);
                docs.document_versions.remove(&entry.uri);
                docs.document_text.remove(&entry.uri);
                self.io.forget_uri(&entry.uri);
                (entry.uri, false)
            };
            let _ = should_stop;
            self.send_notification(
                "textDocument/didClose",
                serde_json::json!({ "textDocument": { "uri": uri } }),
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) fn diagnostics_for_uri(&self, uri: &str) -> Value {
        self.io
            .diagnostics_for_uri(uri)
            .unwrap_or(Value::Array(vec![]))
    }

    pub(crate) async fn is_doc_open(&self, uri: &str) -> bool {
        let docs = self.docs.lock().await;
        docs.open_docs_set.contains(uri)
            || docs
                .open_docs_set
                .iter()
                .any(|u| publish_diagnostics_uri_matches(u, uri))
    }

    pub(crate) fn editor_client_caps(&self) -> Value {
        let caps = self
            .server_capabilities
            .lock()
            .map(|g| g.clone())
            .unwrap_or(Value::Null);
        let st = &caps["semanticTokensProvider"];
        let legend = st.get("legend");
        let token_types = legend
            .and_then(|l| l.get("tokenTypes"))
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                SEMANTIC_TOKEN_TYPES
                    .iter()
                    .map(|s| Value::String((*s).to_string()))
                    .collect()
            });
        let token_modifiers = legend
            .and_then(|l| l.get("tokenModifiers"))
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                SEMANTIC_TOKEN_MODIFIERS
                    .iter()
                    .map(|s| Value::String((*s).to_string()))
                    .collect()
            });
        let trigger_characters = caps["completionProvider"]["triggerCharacters"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![Value::String(".".into())]);
        serde_json::json!({
            "tokenTypes": token_types,
            "tokenModifiers": token_modifiers,
            "triggerCharacters": trigger_characters,
        })
    }

    async fn uri_gate(&self, uri: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut docs = self.docs.lock().await;
        docs.gate(uri)
    }

    pub(crate) async fn sync_document_from_disk(&self, file_path: &Path, uri: &str) -> Result<()> {
        let text = std::fs::read_to_string(file_path).map_err(|e| {
            LitecodeError::ToolExecution(format!("read '{}': {e}", file_path.display()))
        })?;
        self.sync_document_from_text(file_path, uri, &text).await
    }

    pub(crate) async fn sync_document_from_text(
        &self,
        file_path: &Path,
        uri: &str,
        text: &str,
    ) -> Result<()> {
        let gate = self.uri_gate(uri).await;
        let _g = gate.lock().await;
        let (payload, opened, version) = {
            let mut docs = self.docs.lock().await;
            if docs.open_docs_set.contains(uri) {
                if let Some(pos) = docs.open_docs.iter().position(|e| e.uri == uri) {
                    let entry = docs.open_docs.remove(pos).unwrap();
                    docs.open_docs.push_back(entry);
                }
                if docs.document_text.get(uri).map(String::as_str) == Some(text) {
                    return Ok(());
                }
                let old = docs
                    .document_text
                    .insert(uri.to_string(), text.to_string())
                    .unwrap_or_default();
                let version = docs
                    .document_versions
                    .entry(uri.to_string())
                    .and_modify(|version| *version = version.saturating_add(1))
                    .or_insert(2)
                    .to_owned();
                let incremental = self.uses_incremental();
                let change = if incremental {
                    let (end_line, end_col) = eof_position(&old);
                    serde_json::json!({
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end": { "line": end_line, "character": end_col }
                        },
                        "text": text
                    })
                } else {
                    serde_json::json!({ "text": text })
                };
                (
                    serde_json::json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [change]
                    }),
                    false,
                    version,
                )
            } else {
                let language_id = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(crate::lsp::project_root::lsp_language_id)
                    .unwrap_or("plaintext");
                docs.open_docs.push_back(OpenDocEntry {
                    uri: uri.to_string(),
                    opened_at: Instant::now(),
                });
                docs.open_docs_set.insert(uri.to_string());
                docs.document_versions.insert(uri.to_string(), 1);
                docs.document_text.insert(uri.to_string(), text.to_string());
                (
                    serde_json::json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": language_id,
                            "version": 1,
                            "text": text,
                        }
                    }),
                    true,
                    1,
                )
            }
        };
        self.io.note_doc_synced(uri, version);
        if opened {
            self.send_notification("textDocument/didOpen", payload)
                .await?;
            self.evict_old_docs().await?;
        } else {
            self.send_notification("textDocument/didChange", payload)
                .await?;
        }
        Ok(())
    }

    pub(crate) async fn wait_file_diagnostics(&self, uri: &str, budget: Duration) -> Value {
        self.io.wait_diagnostics(uri, budget).await
    }

    pub(crate) fn kill_child(&self) {
        self.io.abort_io();
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }

    pub(crate) async fn shutdown(&self) {
        let open_uris: Vec<String> = {
            let docs = self.docs.lock().await;
            docs.open_docs.iter().map(|e| e.uri.clone()).collect()
        };
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
        self.kill_child();
        self.set_lifecycle(LspLifecycle::Stopped);
    }

    pub(crate) async fn close_doc(&self, uri: &str) -> Result<()> {
        let gate = self.uri_gate(uri).await;
        let _g = gate.lock().await;
        {
            let mut docs = self.docs.lock().await;
            if !docs.open_docs_set.remove(uri) {
                return Ok(());
            }
            docs.open_docs.retain(|e| e.uri != uri);
            docs.document_versions.remove(uri);
            docs.document_text.remove(uri);
        }
        self.io.forget_uri(uri);
        self.send_notification(
            "textDocument/didClose",
            serde_json::json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    pub(crate) fn set_lifecycle(&self, state: LspLifecycle) {
        if let Ok(mut g) = self.io.lifecycle.lock() {
            *g = state;
        }
    }

    pub(crate) fn child_id(&self) -> Option<u32> {
        let pid = self.io.pid.load(Ordering::Relaxed);
        if pid == 0 { None } else { Some(pid) }
    }
}

fn eof_position(text: &str) -> (u32, u32) {
    let mut line = 0u32;
    let mut last_start = 0usize;
    for (i, c) in text.char_indices() {
        if c == '\n' {
            line += 1;
            last_start = i + 1;
        }
    }
    let last = &text[last_start..];
    (line, last.encode_utf16().count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::conn::{parse_content_length, publish_diagnostics_payload};

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
        let too_big = format!("Content-Length: {}", 16 * 1024 * 1024 + 1);
        let err = parse_content_length(&too_big).unwrap_err();
        assert!(err.to_string().contains("exceeds limit"), "got: {err}");
        assert!(parse_content_length("Content-Length: nope").is_err());
    }

    #[test]
    fn stderr_buffer_is_bounded_and_rotates() {
        let mut buf = Vec::new();
        append_stderr_rotated(&mut buf, b"line one\n");
        append_stderr_rotated(&mut buf, b"line two\n");
        assert_eq!(buf, b"line one\nline two\n");

        let big = "x".repeat(MAX_LSP_STDERR_BYTES);
        let mut buf = Vec::new();
        append_stderr_rotated(&mut buf, b"PREFIX ");
        append_stderr_rotated(&mut buf, big.as_bytes());
        assert!(
            buf.len() <= MAX_LSP_STDERR_BYTES,
            "rotated buffer must not exceed cap, got {}",
            buf.len()
        );
        let text = String::from_utf8_lossy(&buf);
        assert!(
            !text.contains("PREFIX"),
            "old prefix must be rotated out: {text}"
        );
        assert!(text.ends_with("x"), "recent bytes must survive: {text}");
    }

    #[test]
    fn publish_diagnostics_payload_extracts_array() {
        let msg = serde_json::json!({
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///a.rs",
                "diagnostics": [{ "message": "boom", "severity": 1 }]
            }
        });
        let (uri, version, diags) = publish_diagnostics_payload(&msg).expect("payload");
        assert_eq!(uri, "file:///a.rs");
        assert_eq!(version, None);
        assert_eq!(diags.as_array().map(|a| a.len()), Some(1));
        let versioned = serde_json::json!({
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": "file:///a.rs",
                "version": 3,
                "diagnostics": []
            }
        });
        let (_, ver, empty) = publish_diagnostics_payload(&versioned).expect("versioned");
        assert_eq!(ver, Some(3));
        assert_eq!(empty, serde_json::json!([]));
        assert!(
            publish_diagnostics_payload(&serde_json::json!({
                "method": "window/logMessage"
            }))
            .is_none()
        );
    }

    #[test]
    fn eof_position_tracks_utf16() {
        assert_eq!(eof_position(""), (0, 0));
        assert_eq!(eof_position("ab"), (0, 2));
        assert_eq!(eof_position("a\nb"), (1, 1));
    }
}
