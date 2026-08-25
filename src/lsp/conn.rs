//! Multiplexed JSON-RPC transport: one stdin writer, one stdout dispatcher.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};
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

pub(crate) fn publish_diagnostics_payload(msg: &Value) -> Option<(String, Value)> {
    if msg["method"].as_str() != Some("textDocument/publishDiagnostics") {
        return None;
    }
    let uri = msg["params"]["uri"].as_str()?.to_string();
    let diags = msg["params"]
        .get("diagnostics")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    Some((uri, diags))
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
    pub(crate) diagnostics: Mutex<HashMap<String, Value>>,
    diag_waiters: Mutex<Vec<(String, oneshot::Sender<()>)>>,
    pub(crate) index_settled: AtomicBool,
    pub(crate) io_failed: AtomicBool,
    pub(crate) watchdog_kill: AtomicBool,
    pub(crate) lifecycle: Mutex<LspLifecycle>,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) pid: AtomicU32,
    writer_abort: Mutex<Option<AbortHandle>>,
    reader_abort: Mutex<Option<AbortHandle>>,
}

impl LspIo {
    pub(crate) fn start(
        stdin: ChildStdin,
        stdout: tokio::process::ChildStdout,
        pid: u32,
    ) -> Arc<Self> {
        let (outbound, outbound_rx) = mpsc::unbounded_channel();
        let io = Arc::new(Self {
            outbound: outbound.clone(),
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            diag_waiters: Mutex::new(Vec::new()),
            index_settled: AtomicBool::new(false),
            io_failed: AtomicBool::new(false),
            watchdog_kill: AtomicBool::new(false),
            lifecycle: Mutex::new(LspLifecycle::Starting),
            last_error: Mutex::new(None),
            pid: AtomicU32::new(pid),
            writer_abort: Mutex::new(None),
            reader_abort: Mutex::new(None),
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
        if let Some((uri, diags)) = publish_diagnostics_payload(msg) {
            if let Ok(mut g) = self.diagnostics.lock() {
                g.insert(uri.clone(), diags);
            }
            self.wake_diag_waiters(&uri);
        }
        if is_quiescent_status(msg) {
            self.index_settled.store(true, Ordering::SeqCst);
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
        if let Some(existing) = self.diagnostics_for_uri(uri)
            && diagnostics_have_errors(&existing)
        {
            return existing;
        }
        let (tx, rx) = oneshot::channel();
        if let Ok(mut g) = self.diag_waiters.lock() {
            g.push((uri.to_string(), tx));
        }
        let _ = timeout(budget, rx).await;
        self.diagnostics_for_uri(uri)
            .unwrap_or(Value::Array(vec![]))
    }

    pub(crate) fn diagnostics_for_uri(&self, uri: &str) -> Option<Value> {
        let Ok(g) = self.diagnostics.lock() else {
            return None;
        };
        for (k, v) in g.iter() {
            if publish_diagnostics_uri_matches(k, uri) {
                return Some(v.clone());
            }
        }
        None
    }

    pub(crate) fn complete_response(&self, msg: &Value) {
        let Some(id) = message_id_u64(msg) else {
            return;
        };
        let tx = self
            .pending
            .lock()
            .ok()
            .and_then(|mut g| g.remove(&id));
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
            Ok(Err(_)) => Err(LitecodeError::ToolExecution(
                "LSP request cancelled".into(),
            )),
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

async fn write_loop(
    mut stdin: ChildStdin,
    mut rx: mpsc::UnboundedReceiver<Value>,
    io: Arc<LspIo>,
) {
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

fn diagnostics_have_errors(diags: &Value) -> bool {
    diags.as_array().is_some_and(|arr| {
        arr.iter()
            .any(|d| d.get("severity").and_then(|s| s.as_i64()) == Some(1))
    })
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

    #[test]
    fn dispatch_out_of_order_responses() {
        let (outbound, _rx) = mpsc::unbounded_channel();
        let io = Arc::new(LspIo {
            outbound,
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            diag_waiters: Mutex::new(Vec::new()),
            index_settled: AtomicBool::new(false),
            io_failed: AtomicBool::new(false),
            watchdog_kill: AtomicBool::new(false),
            lifecycle: Mutex::new(LspLifecycle::Running),
            last_error: Mutex::new(None),
            pid: AtomicU32::new(0),
            writer_abort: Mutex::new(None),
            reader_abort: Mutex::new(None),
        });
        let (tx1, mut rx1) = oneshot::channel();
        let (tx2, mut rx2) = oneshot::channel();
        io.pending.lock().unwrap().insert(1, tx1);
        io.pending.lock().unwrap().insert(2, tx2);
        assert!(dispatch_message(
            &io,
            &serde_json::json!({"jsonrpc":"2.0","id":2,"result":"second"})
        )
        .is_none());
        assert!(dispatch_message(
            &io,
            &serde_json::json!({"jsonrpc":"2.0","id":1,"result":"first"})
        )
        .is_none());
        assert_eq!(rx2.try_recv().unwrap().unwrap(), Value::String("second".into()));
        assert_eq!(rx1.try_recv().unwrap().unwrap(), Value::String("first".into()));
    }

    #[test]
    fn configuration_ack_is_queued() {
        let (outbound, _rx) = mpsc::unbounded_channel();
        let io = Arc::new(LspIo {
            outbound,
            pending: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(HashMap::new()),
            diag_waiters: Mutex::new(Vec::new()),
            index_settled: AtomicBool::new(false),
            io_failed: AtomicBool::new(false),
            watchdog_kill: AtomicBool::new(false),
            lifecycle: Mutex::new(LspLifecycle::Running),
            last_error: Mutex::new(None),
            pid: AtomicU32::new(0),
            writer_abort: Mutex::new(None),
            reader_abort: Mutex::new(None),
        });
        let ack = dispatch_message(
            &io,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "workspace/configuration",
                "params": { "items": [{ "section": "csharp" }] }
            }),
        );
        assert_eq!(
            ack.unwrap()["result"],
            serde_json::json!([{}])
        );
    }
}
