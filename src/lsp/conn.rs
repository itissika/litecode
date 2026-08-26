//! Multiplexed JSON-RPC transport: one stdin writer, one stdout dispatcher.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio::time::{Duration, timeout};

use crate::lsp::status::LspLifecycle;
use crate::lsp::uri::publish_diagnostics_uri_matches;
use crate::types::{LitecodeError, Result};

pub(crate) static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

const MAX_LSP_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn request_timeout() -> Duration {
    let secs = std::env::var("LITECODE_LSP_REQUEST_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30u64);
    Duration::from_secs(secs.max(1))
}

pub(crate) fn parse_content_length(line: &str) -> Result<Option<usize>> {
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

pub(crate) fn message_id_u64(value: &Value) -> Option<u64> {
    value.get("id").and_then(|id| {
        id.as_u64()
            .or_else(|| id.as_i64().and_then(|i| u64::try_from(i).ok()))
    })
}

pub(crate) fn is_server_request(msg: &Value) -> bool {
    msg.get("method").is_some() && msg.get("id").is_some()
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

pub(crate) fn publish_diagnostics_payload(msg: &Value) -> Option<(String, Option<i32>, Value)> {
    if msg["method"].as_str() != Some("textDocument/publishDiagnostics") {
        return None;
    }
    let uri = msg["params"]["uri"].as_str()?.to_string();
    let version = msg["params"].get("version").and_then(json_i32);
    let diags = msg["params"]
        .get("diagnostics")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    Some((uri, version, diags))
}

/// Editor-facing diagnostic push (VS Code DiagnosticCollection.set).
#[derive(Debug, Clone)]
pub struct LspDiagnosticEvent {
    pub uri: String,
    pub version: Option<i32>,
    pub diagnostics: Value,
}

fn json_i32(value: &Value) -> Option<i32> {
    value.as_i64().and_then(|n| i32::try_from(n).ok())
}

#[derive(Clone)]
struct DiagEntry {
    diags: Value,
    version: Option<i32>,
}

fn upsert_uri_map<V>(map: &mut HashMap<String, V>, uri: &str, value: V) {
    let key = map
        .keys()
        .find(|k| publish_diagnostics_uri_matches(k, uri))
        .cloned()
        .unwrap_or_else(|| uri.to_string());
    map.insert(key, value);
}

fn map_get_matching<'a, V>(map: &'a HashMap<String, V>, uri: &str) -> Option<&'a V> {
    map.iter()
        .find(|(k, _)| publish_diagnostics_uri_matches(k, uri))
        .map(|(_, v)| v)
}

/// Cached publish covers the last didOpen/didChange we sent.
/// Missing versions are never current — silence beats a stale Error.
fn cache_covers_sync(publish_version: Option<i32>, synced: Option<i32>) -> bool {
    match (publish_version, synced) {
        (Some(got), Some(want)) => got >= want,
        _ => false,
    }
}

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

fn ack_server_request(request: &Value) -> Option<Value> {
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let result = match method {
        "workspace/configuration" => configuration_response(request),
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability"
        | "window/showMessageRequest"
        | "workspace/applyEdit" => Value::Null,
        _ => Value::Null,
    };
    Some(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
}

type RpcResult = std::result::Result<Value, LitecodeError>;

pub(crate) struct LspIo {
    pub(crate) outbound: mpsc::UnboundedSender<Value>,
    pending: Mutex<HashMap<u64, oneshot::Sender<RpcResult>>>,
    diagnostics: Mutex<HashMap<String, DiagEntry>>,
    synced_version: Mutex<HashMap<String, i32>>,
    diag_waiters: Mutex<Vec<(String, oneshot::Sender<()>)>>,
    pub(crate) index_settled: AtomicBool,
    pub(crate) io_failed: AtomicBool,
    pub(crate) watchdog_kill: AtomicBool,
    pub(crate) lifecycle: Mutex<LspLifecycle>,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) pid: AtomicU32,
    writer_abort: Mutex<Option<AbortHandle>>,
    reader_abort: Mutex<Option<AbortHandle>>,
    diag_tx: Option<broadcast::Sender<LspDiagnosticEvent>>,
}

impl LspIo {
    pub(crate) fn start(
        stdin: ChildStdin,
        stdout: tokio::process::ChildStdout,
        pid: u32,
        diag_tx: Option<broadcast::Sender<LspDiagnosticEvent>>,
    ) -> Arc<Self> {
        let (outbound, outbound_rx) = mpsc::unbounded_channel();
        let io = Arc::new(Self {
            outbound: outbound.clone(),
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            synced_version: Mutex::new(HashMap::new()),
            diag_waiters: Mutex::new(Vec::new()),
            index_settled: AtomicBool::new(false),
            io_failed: AtomicBool::new(false),
            watchdog_kill: AtomicBool::new(false),
            lifecycle: Mutex::new(LspLifecycle::Starting),
            last_error: Mutex::new(None),
            pid: AtomicU32::new(pid),
            writer_abort: Mutex::new(None),
            reader_abort: Mutex::new(None),
            diag_tx,
        });

        let writer_io = Arc::clone(&io);
        let writer = tokio::spawn(async move {
            write_loop(stdin, outbound_rx, writer_io).await;
        });
        let reader_io = Arc::clone(&io);
        let reader = tokio::spawn(async move {
            read_loop(BufReader::new(stdout), outbound, reader_io).await;
        });
        if let Ok(mut g) = io.writer_abort.lock() {
            *g = Some(writer.abort_handle());
        }
        if let Ok(mut g) = io.reader_abort.lock() {
            *g = Some(reader.abort_handle());
        }
        io
    }

    pub(crate) fn abort_io(&self) {
        if let Ok(g) = self.writer_abort.lock()
            && let Some(h) = g.as_ref()
        {
            h.abort();
        }
        if let Ok(g) = self.reader_abort.lock()
            && let Some(h) = g.as_ref()
        {
            h.abort();
        }
    }

    pub(crate) fn mark_failed(&self, err: impl Into<String>) {
        self.io_failed.store(true, Ordering::SeqCst);
        if let Ok(mut g) = self.lifecycle.lock() {
            if *g != LspLifecycle::Stopped {
                *g = LspLifecycle::Failed;
            }
        }
        if let Ok(mut g) = self.last_error.lock() {
            *g = Some(err.into());
        }
        self.fail_all_pending("language server I/O failed");
    }

    fn fail_all_pending(&self, msg: &str) {
        let waiters = self
            .pending
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        for (_, tx) in waiters {
            let _ = tx.send(Err(LitecodeError::ToolExecution(msg.into())));
        }
    }

    pub(crate) fn ingest_notification(&self, msg: &Value) {
        if let Some((uri, version, diags)) = publish_diagnostics_payload(msg) {
            if let Ok(mut g) = self.diagnostics.lock() {
                upsert_uri_map(
                    &mut g,
                    &uri,
                    DiagEntry {
                        diags: diags.clone(),
                        version,
                    },
                );
            }
            if let Some(tx) = &self.diag_tx {
                let _ = tx.send(LspDiagnosticEvent {
                    uri: uri.clone(),
                    version,
                    diagnostics: diags,
                });
            }
            // Late / unversioned publishes must not abort a wait for the
            // current document version; only a covering publish wakes waiters.
            if self.diagnostics_current(&uri) {
                self.wake_diag_waiters(&uri);
            }
        }
        if is_quiescent_status(msg) {
            self.index_settled.store(true, Ordering::SeqCst);
        }
    }

    pub(crate) fn note_doc_synced(&self, uri: &str, version: i32) {
        if let Ok(mut g) = self.synced_version.lock() {
            upsert_uri_map(&mut g, uri, version);
        }
    }

    pub(crate) fn forget_uri(&self, uri: &str) {
        if let Ok(mut g) = self.diagnostics.lock() {
            g.retain(|k, _| !publish_diagnostics_uri_matches(k, uri));
        }
        if let Ok(mut g) = self.synced_version.lock() {
            g.retain(|k, _| !publish_diagnostics_uri_matches(k, uri));
        }
    }

    fn wake_diag_waiters(&self, uri: &str) {
        let Ok(mut g) = self.diag_waiters.lock() else {
            return;
        };
        let mut rest = Vec::new();
        for (u, tx) in g.drain(..) {
            if publish_diagnostics_uri_matches(&u, uri) {
                let _ = tx.send(());
            } else {
                rest.push((u, tx));
            }
        }
        *g = rest;
    }

    pub(crate) async fn wait_diagnostics(&self, uri: &str, budget: Duration) -> Value {
        if self.diagnostics_current(uri) {
            return self
                .diagnostics_for_uri(uri)
                .unwrap_or(Value::Array(vec![]));
        }
        let (tx, rx) = oneshot::channel();
        if let Ok(mut g) = self.diag_waiters.lock() {
            g.push((uri.to_string(), tx));
        }
        if self.diagnostics_current(uri) {
            return self
                .diagnostics_for_uri(uri)
                .unwrap_or(Value::Array(vec![]));
        }
        let _ = timeout(budget, rx).await;
        // After timeout, never return a cache that predates this document version.
        if self.diagnostics_current(uri) {
            self.diagnostics_for_uri(uri)
                .unwrap_or(Value::Array(vec![]))
        } else {
            Value::Array(vec![])
        }
    }

    fn diagnostics_current(&self, uri: &str) -> bool {
        let Some(entry) = self.diag_entry(uri) else {
            return false;
        };
        cache_covers_sync(entry.version, self.synced_version_for_uri(uri))
    }

    pub(crate) fn diagnostics_are_current(&self, uri: &str) -> bool {
        self.diagnostics_current(uri)
    }

    fn synced_version_for_uri(&self, uri: &str) -> Option<i32> {
        let Ok(g) = self.synced_version.lock() else {
            return None;
        };
        map_get_matching(&g, uri).copied()
    }

    fn diag_entry(&self, uri: &str) -> Option<DiagEntry> {
        let Ok(g) = self.diagnostics.lock() else {
            return None;
        };
        map_get_matching(&g, uri).cloned()
    }

    pub(crate) fn diagnostics_for_uri(&self, uri: &str) -> Option<Value> {
        self.diag_entry(uri).map(|e| e.diags)
    }

    pub(crate) fn complete_response(&self, msg: &Value) {
        let Some(id) = message_id_u64(msg) else {
            return;
        };
        let tx = self.pending.lock().ok().and_then(|mut g| g.remove(&id));
        let Some(tx) = tx else {
            return;
        };
        if let Some(error) = msg.get("error") {
            let err_msg = error["message"].as_str().unwrap_or("unknown error");
            let code = error["code"].as_i64().unwrap_or(-1);
            let _ = tx.send(Err(LitecodeError::ToolExecution(format!(
                "LSP error (code {code}): {err_msg}"
            ))));
        } else {
            let _ = tx.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
        }
    }

    pub(crate) fn enqueue(&self, value: Value) -> Result<()> {
        self.outbound
            .send(value)
            .map_err(|_| LitecodeError::ToolExecution("LSP stdin closed".into()))
    }

    pub(crate) async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.enqueue(serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    pub(crate) async fn request(
        &self,
        method: &str,
        params: Value,
        rpc_id: Option<u64>,
    ) -> Result<Value> {
        let id = rpc_id.unwrap_or_else(|| NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        {
            let mut g = self
                .pending
                .lock()
                .map_err(|e| LitecodeError::ToolExecution(format!("lsp pending lock: {e}")))?;
            g.insert(id, tx);
        }
        if let Err(e) = self.enqueue(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })) {
            if let Ok(mut g) = self.pending.lock() {
                g.remove(&id);
            }
            return Err(e);
        }
        match timeout(request_timeout(), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(LitecodeError::ToolExecution("LSP request cancelled".into())),
            Err(_) => {
                if let Ok(mut g) = self.pending.lock() {
                    g.remove(&id);
                }
                let _ = self.notify_now("$/cancelRequest", serde_json::json!({ "id": id }));
                Err(LitecodeError::ToolExecution(format!(
                    "LSP request timed out after {}s",
                    request_timeout().as_secs()
                )))
            }
        }
    }

    pub(crate) fn cancel(&self, id: u64) {
        if let Ok(mut g) = self.pending.lock()
            && let Some(tx) = g.remove(&id)
        {
            let _ = tx.send(Err(LitecodeError::ToolExecution(
                "LSP request cancelled".into(),
            )));
        }
        let _ = self.notify_now("$/cancelRequest", serde_json::json!({ "id": id }));
    }

    fn notify_now(&self, method: &str, params: Value) -> Result<()> {
        self.enqueue(serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }
}

/// Apply one inbound JSON-RPC message. Returns an optional stdin ack.
pub(crate) fn dispatch_message(io: &LspIo, msg: &Value) -> Option<Value> {
    if is_server_request(msg) {
        if is_quiescent_status(msg) {
            io.index_settled.store(true, Ordering::SeqCst);
        }
        return ack_server_request(msg);
    }
    if msg.get("id").is_none() {
        io.ingest_notification(msg);
        return None;
    }
    io.complete_response(msg);
    None
}

async fn write_loop(mut stdin: ChildStdin, mut rx: mpsc::UnboundedReceiver<Value>, io: Arc<LspIo>) {
    while let Some(value) = rx.recv().await {
        if let Err(e) = write_raw(&mut stdin, &value).await {
            io.mark_failed(e.to_string());
            break;
        }
    }
}

async fn write_raw(stdin: &mut ChildStdin, value: &Value) -> Result<()> {
    let body = serde_json::to_string(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let write = async {
        stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| LitecodeError::ToolExecution(format!("LSP write header failed: {e}")))?;
        stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| LitecodeError::ToolExecution(format!("LSP write body failed: {e}")))?;
        stdin
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

async fn read_loop(
    mut reader: BufReader<tokio::process::ChildStdout>,
    outbound: mpsc::UnboundedSender<Value>,
    io: Arc<LspIo>,
) {
    loop {
        match read_message(&mut reader).await {
            Ok(msg) => {
                if let Some(ack) = dispatch_message(&io, &msg) {
                    let _ = outbound.send(ack);
                }
            }
            Err(e) => {
                io.mark_failed(e.to_string());
                break;
            }
        }
    }
}

pub(crate) async fn read_message(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> Result<Value> {
    let mut content_length = 0usize;
    loop {
        let mut header_line = String::new();
        let bytes = reader
            .read_line(&mut header_line)
            .await
            .map_err(|e| LitecodeError::ToolExecution(format!("LSP read header failed: {e}")))?;
        if bytes == 0 {
            return Err(LitecodeError::ToolExecution(
                "language server closed stdout".into(),
            ));
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
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| LitecodeError::ToolExecution(format!("LSP read body failed: {e}")))?;
    let body = String::from_utf8(buf)
        .map_err(|e| LitecodeError::ToolExecution(format!("invalid utf8: {e}")))?;
    serde_json::from_str(&body)
        .map_err(|e| LitecodeError::ToolExecution(format!("invalid json: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn test_io() -> Arc<LspIo> {
        test_io_diag(None)
    }

    fn test_io_diag(diag_tx: Option<broadcast::Sender<LspDiagnosticEvent>>) -> Arc<LspIo> {
        let (outbound, rx) = mpsc::unbounded_channel();
        std::mem::forget(rx);
        Arc::new(LspIo {
            outbound,
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            synced_version: Mutex::new(HashMap::new()),
            diag_waiters: Mutex::new(Vec::new()),
            index_settled: AtomicBool::new(false),
            io_failed: AtomicBool::new(false),
            watchdog_kill: AtomicBool::new(false),
            lifecycle: Mutex::new(LspLifecycle::Running),
            last_error: Mutex::new(None),
            pid: AtomicU32::new(0),
            writer_abort: Mutex::new(None),
            reader_abort: Mutex::new(None),
            diag_tx,
        })
    }

    fn publish(uri: &str, version: Option<i32>, diags: Value) -> Value {
        let mut params = serde_json::json!({
            "uri": uri,
            "diagnostics": diags,
        });
        if let Some(version) = version {
            params["version"] = serde_json::json!(version);
        }
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": params,
        })
    }

    fn error_diag() -> Value {
        serde_json::json!([{ "message": "boom", "severity": 1 }])
    }

    #[test]
    fn dispatch_out_of_order_responses() {
        let io = test_io();
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        io.pending.lock().unwrap().insert(1, tx1);
        io.pending.lock().unwrap().insert(2, tx2);
        assert!(
            dispatch_message(
                &io,
                &serde_json::json!({"jsonrpc":"2.0","id":2,"result":"second"})
            )
            .is_none()
        );
        assert!(
            dispatch_message(
                &io,
                &serde_json::json!({"jsonrpc":"2.0","id":1,"result":"first"})
            )
            .is_none()
        );
        assert_eq!(
            rx2.try_recv().unwrap().unwrap(),
            Value::String("second".into())
        );
        assert_eq!(
            rx1.try_recv().unwrap().unwrap(),
            Value::String("first".into())
        );
    }

    #[test]
    fn configuration_ack_is_queued() {
        let io = test_io();
        let ack = dispatch_message(
            &io,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "workspace/configuration",
                "params": { "items": [{ "section": "csharp" }] }
            }),
        );
        assert_eq!(ack.unwrap()["result"], serde_json::json!([{}]));
    }

    #[test]
    fn cache_covers_sync_requires_matching_version() {
        assert!(cache_covers_sync(Some(2), Some(2)));
        assert!(cache_covers_sync(Some(3), Some(2)));
        assert!(!cache_covers_sync(Some(1), Some(2)));
        assert!(!cache_covers_sync(None, Some(2)));
        assert!(!cache_covers_sync(Some(1), None));
        assert!(!cache_covers_sync(None, None));
    }

    fn stale_error() -> Value {
        serde_json::json!([{ "message": "cannot find foo", "severity": 1 }])
    }

    fn new_error() -> Value {
        serde_json::json!([{ "message": "unused import", "severity": 1 }])
    }

    /// Agent-fix sequence that previously echoed last round's Error:
    /// v1 error → didChange v2 (fixed) with no fresh publish yet → must be
    /// silence, not the v1 message. A late v1 publish must not resurrect it.
    #[tokio::test]
    async fn edit_fix_must_not_echo_previous_round_error() {
        let io = test_io();
        let uri = "file:///lib.rs";

        io.note_doc_synced(uri, 1);
        io.ingest_notification(&publish(uri, Some(1), stale_error()));
        let round1 = io.wait_diagnostics(uri, Duration::from_millis(50)).await;
        assert_eq!(
            round1,
            stale_error(),
            "current v1 errors are valid feedback"
        );

        // Round 2: the file is fixed; LS has not published v2 yet.
        // Old wait_diagnostics returned v1 errors immediately here.
        io.note_doc_synced(uri, 2);
        let round2 = io.wait_diagnostics(uri, Duration::from_millis(40)).await;
        assert_eq!(
            round2,
            Value::Array(vec![]),
            "stale v1 errors must not be reported after didChange v2; got {round2}"
        );
        assert!(
            !round2.to_string().contains("cannot find foo"),
            "must not leak previous-round message: {round2}"
        );

        io.ingest_notification(&publish(uri, Some(1), stale_error()));
        let after_late_v1 = io.wait_diagnostics(uri, Duration::from_millis(40)).await;
        assert_eq!(
            after_late_v1,
            Value::Array(vec![]),
            "late v1 publish must not resurrect the old error"
        );

        io.ingest_notification(&publish(uri, Some(2), Value::Array(vec![])));
        let round3 = io.wait_diagnostics(uri, Duration::from_millis(50)).await;
        assert_eq!(round3, Value::Array(vec![]));
    }

    #[tokio::test]
    async fn wait_ignores_stale_publish_and_keeps_waiting_for_current() {
        let io = test_io();
        let uri = "file:///lib.rs";
        io.note_doc_synced(uri, 1);
        io.ingest_notification(&publish(uri, Some(1), stale_error()));
        io.note_doc_synced(uri, 2);

        let io_pub = Arc::clone(&io);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            io_pub.ingest_notification(&publish(uri, Some(1), stale_error()));
            tokio::time::sleep(Duration::from_millis(20)).await;
            io_pub.ingest_notification(&publish(uri, Some(2), new_error()));
        });

        let started = Instant::now();
        let found = io.wait_diagnostics(uri, Duration::from_millis(400)).await;
        assert_eq!(
            found,
            new_error(),
            "must skip late v1 and return the v2 error, not silence-or-stale"
        );
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "should complete on v2, not eat the full budget"
        );
    }

    #[tokio::test]
    async fn wait_returns_current_errors_without_blocking() {
        let io = test_io();
        let uri = "file:///a.rs";
        io.note_doc_synced(uri, 1);
        io.ingest_notification(&publish(uri, Some(1), error_diag()));

        let started = Instant::now();
        let found = io.wait_diagnostics(uri, Duration::from_millis(400)).await;
        assert_eq!(found, error_diag());
        assert!(
            started.elapsed() < Duration::from_millis(80),
            "current errors must not wait out the budget"
        );
    }

    #[tokio::test]
    async fn wait_returns_current_clean_cache_without_blocking() {
        let io = test_io();
        let uri = "file:///a.rs";
        io.note_doc_synced(uri, 2);
        io.ingest_notification(&publish(uri, Some(2), Value::Array(vec![])));

        let started = Instant::now();
        let found = io.wait_diagnostics(uri, Duration::from_millis(400)).await;
        assert_eq!(found, Value::Array(vec![]));
        assert!(
            started.elapsed() < Duration::from_millis(80),
            "current clean publish must not wait out the budget"
        );
    }

    #[tokio::test]
    async fn unversioned_publish_after_change_is_silence_not_stale_errors() {
        let io = test_io();
        let uri = "file:///a.rs";
        io.note_doc_synced(uri, 1);
        io.ingest_notification(&publish(uri, Some(1), error_diag()));
        io.note_doc_synced(uri, 2);
        io.ingest_notification(&publish(uri, None, error_diag()));

        let found = io.wait_diagnostics(uri, Duration::from_millis(30)).await;
        assert_eq!(
            found,
            Value::Array(vec![]),
            "unversioned publish must not be treated as covering didChange"
        );
    }

    #[test]
    fn diagnostics_are_current_rejects_stale_cache() {
        let io = test_io();
        let uri = "file:///lib.rs";
        io.note_doc_synced(uri, 1);
        io.ingest_notification(&publish(uri, Some(1), stale_error()));
        assert!(io.diagnostics_are_current(uri));
        io.note_doc_synced(uri, 2);
        assert!(
            !io.diagnostics_are_current(uri),
            "v1 cache must not cover v2 sync"
        );
    }

    #[test]
    fn publish_diagnostics_fans_out_to_subscribers() {
        let (tx, mut rx) = broadcast::channel(8);
        let io = test_io_diag(Some(tx));
        io.ingest_notification(&publish(
            "file:///a.rs",
            Some(2),
            serde_json::json!([{ "message": "x", "severity": 1 }]),
        ));
        let ev = rx.try_recv().expect("editor fan-out");
        assert_eq!(ev.uri, "file:///a.rs");
        assert_eq!(ev.version, Some(2));
        assert_eq!(ev.diagnostics.as_array().map(|a| a.len()), Some(1));
        assert!(
            io.diagnostics_for_uri("file:///a.rs").is_some(),
            "agent cache must still be written"
        );
    }
}
