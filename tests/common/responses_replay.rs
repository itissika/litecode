//! Local HTTP replay of OpenAI Responses SSE fixtures (product path).
//!
//! Responses bodies live under `tests/fixtures/sse/responses/`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// Read a Responses SSE fixture by stem name (no extension).
pub fn fixture_responses_sse(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sse/responses")
        .join(format!("{name}.txt"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing Responses SSE fixture {} ({e})", path.display()))
}

/// Minimal text-only completed Responses SSE (also committed as fixture).
pub fn text_only_completed_sse() -> String {
    let text_delta = serde_json::json!({
        "type": "response.output_text.delta",
        "sequence_number": 1,
        "item_id": "msg_text_1",
        "output_index": 0,
        "content_index": 0,
        "delta": "Hello from Responses replay"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 2,
        "response": {
            "id": "resp_text_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "output": [{
                "type": "message",
                "id": "msg_text_1",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": "Hello from Responses replay",
                    "annotations": []
                }]
            }]
        }
    });
    format!("data: {text_delta}\n\ndata: {completed}\n\n")
}

/// Function-call Responses SSE (read tool) — early name via output_item.added, then args.
pub fn tool_call_completed_sse() -> String {
    let added = serde_json::json!({
        "type": "response.output_item.added",
        "sequence_number": 1,
        "output_index": 0,
        "item": {
            "type": "function_call",
            "id": "fc_read_1",
            "call_id": "call_read_1",
            "name": "read",
            "arguments": "",
            "status": "in_progress"
        }
    });
    let fc_delta = serde_json::json!({
        "type": "response.function_call_arguments.delta",
        "sequence_number": 2,
        "item_id": "fc_read_1",
        "output_index": 0,
        "delta": "{\"path\":\"test.txt\"}"
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "sequence_number": 3,
        "response": {
            "id": "resp_tool_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "id": "fc_read_1",
                "call_id": "call_read_1",
                "name": "read",
                "arguments": "{\"path\":\"test.txt\"}",
                "status": "completed"
            }]
        }
    });
    format!("data: {added}\n\ndata: {fc_delta}\n\ndata: {completed}\n\n")
}

/// Serve a queue of Responses SSE bodies over HTTP/1.1 (`text/event-stream`).
/// Returns base endpoint `http://127.0.0.1:PORT/v1` suitable for
/// `ProviderDefinition.endpoint` / `provider_from_definition` (normalizes to `/responses`).
pub async fn serve_responses_queue(bodies: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let queue = Arc::new(Mutex::new(bodies));
    let served = Arc::new(AtomicUsize::new(0));
    let queue_bg = Arc::clone(&queue);
    let served_bg = Arc::clone(&served);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let body = {
                let mut q = queue_bg.lock().await;
                if q.is_empty() {
                    // Repeat last-served pattern: empty → 500 so tests fail loudly
                    // rather than silently hanging on Chat dialect.
                    drop(q);
                    let resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 19\r\nConnection: close\r\n\r\nno more SSE bodies";
                    let _ = socket.write_all(resp.as_bytes()).await;
                    let _ = socket.shutdown().await;
                    continue;
                }
                served_bg.fetch_add(1, Ordering::SeqCst);
                q.remove(0)
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{addr}/v1")
}
