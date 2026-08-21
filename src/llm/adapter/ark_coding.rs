//! Volcengine Ark Coding Plan — OpenAI Responses gateway.
//!
//! Dedicated Coding Plan host (`/api/coding/v3`), not the general Ark `/api/v3`.
//! POST `{base}/responses`. This file owns Bearer auth, LiteCode user-agent,
//! `store: false`, and Doubao `thinking` / `reasoning.effort` dialect.

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{Item, OutputItem, Response, ResponseStreamEvent};
use crate::config::schema::ProviderAuth;
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::types::{LitecodeError, Result, StreamEvents};

use super::responses_sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::{llm_http_client, transport_error};

/// Settings / catalog root (Codex `base_url`). [`normalize_endpoint`] appends `/responses`.
pub(crate) const DEFAULT_ENDPOINT: &str = "https://ark.cn-beijing.volces.com/api/coding/v3";

const ERROR_PREFIX: &str = "Ark Coding Plan adapter";

pub struct ArkCodingProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl ArkCodingProvider {
    pub fn new(endpoint: String, auth: ProviderAuth) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint);
        // reqwest `.timeout` is a wall-clock cap on connect + full SSE body.
        // Long thinking outlives 120s while the stream is still healthy; user
        // cancel already covers "nothing happening". Idle `read_timeout` would
        // mis-kill silent thinking. Highest-ROI follow-up is retry on transport
        // timeout — shelved; dropping the cap is enough for now.
        let client = llm_http_client()?;
        Ok(Self {
            client,
            endpoint_url: endpoint,
            auth,
        })
    }

    fn auth_header(&self, api_key: &str) -> (String, String) {
        match self.auth {
            ProviderAuth::Bearer => ("Authorization".to_string(), format!("Bearer {api_key}")),
            ProviderAuth::ApiKey => ("api-key".to_string(), api_key.to_string()),
        }
    }

    fn build_body(params: &ModelRequest, stream: bool) -> Result<Value> {
        let input: Vec<Value> = params
            .input
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| LitecodeError::Llm(format!("serialize input items: {e}")))?;

        let tools: Vec<Value> = params
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": params.model,
            "instructions": params.instructions,
            "input": input,
            "tools": tools,
            "stream": stream,
            "max_output_tokens": params.max_output_tokens,
            "temperature": params.temperature,
            "store": false,
        });
        apply_ark_thinking(&mut body, params);
        Ok(body)
    }
}

/// Official Doubao Responses thinking dialect (doc 1956279).
///
/// Platform Low → `thinking.type=disabled` and no `reasoning.effort` (`disabled` +
/// `low|medium|high` is a 400). Medium/High → `enabled` plus `reasoning.effort`.
/// Coding Plan probe (P1a–c) accepted these combos on `doubao-seed-2.1-turbo`;
/// Kimi / MiniMax / GLM returned HTTP 200 with the same fields, so they are not omitted.
fn apply_ark_thinking(body: &mut Value, params: &ModelRequest) {
    if params.thinking_mode.as_deref() == Some("disabled") {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
        return;
    }
    match params.reasoning_effort.as_deref() {
        Some("low") | Some("none") | Some("minimal") => {
            body["thinking"] = serde_json::json!({ "type": "disabled" });
        }
        Some("medium") => {
            body["thinking"] = serde_json::json!({ "type": "enabled" });
            body["reasoning"] = serde_json::json!({ "effort": "medium" });
        }
        Some("high") | Some("max") => {
            body["thinking"] = serde_json::json!({ "type": "enabled" });
            body["reasoning"] = serde_json::json!({ "effort": "high" });
        }
        _ => {}
    }
}

/// Ark SSE omits OpenAI-required fields on early events (`output`, `status`,
/// reasoning `summary`, function `arguments`, etc.).
fn harden_ark_json(value: &mut Value) {
    harden_ark_value(value, None);
}

fn harden_ark_value(value: &mut Value, event_type: Option<&str>) {
    match value {
        Value::Object(map) => {
            let ty = map.get("type").and_then(Value::as_str).map(str::to_owned);
            let event = ty.as_deref().or(event_type);
            fill_ark_typed_object(map);
            if let Some(Value::Object(resp)) = map.get_mut("response") {
                fill_ark_response_object(resp, event);
            }
            if map.get("object").and_then(Value::as_str) == Some("response") {
                fill_ark_response_object(map, event);
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    harden_ark_value(child, event);
                }
            }
        }
        Value::Array(arr) => {
            for child in arr {
                harden_ark_value(child, event_type);
            }
        }
        _ => {}
    }
}

fn fill_ark_typed_object(map: &mut Map<String, Value>) {
    match map.get("type").and_then(Value::as_str) {
        Some("reasoning") => {
            map.entry("summary")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        Some("function_call") => {
            map.entry("arguments")
                .or_insert_with(|| Value::String(String::new()));
            if !map.contains_key("call_id") {
                let fallback = map
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                map.insert("call_id".into(), Value::String(fallback));
            }
            map.entry("name")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("message") => {
            map.entry("content")
                .or_insert_with(|| Value::Array(Vec::new()));
            map.entry("role")
                .or_insert_with(|| Value::String("assistant".into()));
            map.entry("status")
                .or_insert_with(|| Value::String("in_progress".into()));
        }
        Some("summary_text") => {
            map.entry("text")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("output_text") => {
            map.entry("text")
                .or_insert_with(|| Value::String(String::new()));
            map.entry("annotations")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        _ => {}
    }
}

fn fill_ark_response_object(map: &mut Map<String, Value>, event_type: Option<&str>) {
    map.entry("output")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !map.contains_key("status") {
        let status = match event_type {
            Some("response.completed") => "completed",
            Some("response.incomplete") => "incomplete",
            Some("response.failed") => "failed",
            Some("response.cancelled") => "cancelled",
            _ => "in_progress",
        };
        map.insert("status".into(), Value::String(status.into()));
    }
}

fn parse_response(text: &str) -> Result<Response> {
    let mut value: Value = serde_json::from_str(text)
        .map_err(|e| LitecodeError::Llm(format!("deserialize Response JSON: {e}")))?;
    harden_ark_json(&mut value);
    serde_json::from_value(value)
        .map_err(|e| LitecodeError::Llm(format!("deserialize Response: {e}")))
}

fn parse_stream_event(data: &str) -> Result<ResponseStreamEvent> {
    let mut value: Value = serde_json::from_str(data).map_err(|e| {
        LitecodeError::Llm(format!(
            "deserialize ResponseStreamEvent JSON: {e}; payload={data}"
        ))
    })?;
    harden_ark_json(&mut value);
    serde_json::from_value(value).map_err(|e| {
        LitecodeError::Llm(format!(
            "deserialize ResponseStreamEvent: {e}; payload={data}"
        ))
    })
}

/// Coding Plan OpenAI root is `/api/coding/v3` (not `/v1`). Always POST `/responses`.
fn normalize_endpoint(endpoint: String) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        return trimmed.to_string();
    }
    format!("{trimmed}/responses")
}

fn output_items_to_items(output: Vec<OutputItem>) -> Vec<Item> {
    output.into_iter().map(Item::from).collect()
}

fn apply_ark_headers(
    builder: reqwest::RequestBuilder,
    header_name: String,
    header_value: String,
) -> reqwest::RequestBuilder {
    let user_agent = format!("litecode/{}", env!("CARGO_PKG_VERSION"));
    builder
        .header(header_name, header_value)
        .header("content-type", "application/json")
        .header("user-agent", user_agent)
}

fn http_err(status: reqwest::StatusCode, text: String) -> LitecodeError {
    LitecodeError::Llm(format!("{ERROR_PREFIX}: HTTP {status}: {text}"))
}

impl LlmProvider for ArkCodingProvider {
    fn endpoint(&self) -> &str {
        &self.endpoint_url
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(Self {
            client: self.client.clone(),
            endpoint_url: self.endpoint_url.clone(),
            auth: self.auth,
        })
    }

    fn clone_for_isolated_runtime(&self) -> Box<dyn LlmProvider> {
        match Self::new(self.endpoint_url.clone(), self.auth) {
            Ok(p) => Box::new(p),
            Err(_) => self.box_clone(),
        }
    }

    fn complete<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = Self::build_body(request, false)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = apply_ark_headers(
                self.client.post(&self.endpoint_url),
                header_name,
                header_value,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| transport_error("sending Ark Coding Plan response", &e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(http_err(status, text));
            }

            let text = resp
                .text()
                .await
                .map_err(|e| transport_error("reading Ark Coding Plan response", &e))?;
            let response = parse_response(&text)?;
            Ok(output_items_to_items(response.output))
        })
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = Self::build_body(request, true)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = apply_ark_headers(
                self.client.post(&self.endpoint_url),
                header_name,
                header_value,
            )
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| transport_error("opening Ark Coding Plan event stream", &e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(http_err(status, text));
            }
            let resp = check_event_stream_content_type(resp).await?;

            let mut terminal_items: Option<Vec<Item>> = None;
            let mut reader = SseLineReader::new();
            let mut stream = resp.bytes_stream();
            let mut gate = StreamContractGate::new();
            let mut acc = StreamItemAccumulator::new();
            let mut cancelled = cancel.is_cancelled();

            while !cancelled {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        break;
                    }
                    chunk = stream.next() => {
                        let Some(chunk) = chunk else { break; };
                        let chunk = chunk.map_err(|e| {
                            transport_error("reading Ark Coding Plan event stream", &e)
                        })?;
                        for line in reader.feed(&chunk)? {
                            let Some(data) = sse_data_payload(&line) else {
                                continue;
                            };
                            let event = parse_stream_event(data)?;
                            if let Some(items) =
                                forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                            {
                                terminal_items = Some(items);
                            }
                            if cancel.is_cancelled() {
                                cancelled = true;
                                break;
                            }
                        }
                    }
                }
            }

            if !cancelled
                && let Some(line) = reader.finish()?
                && let Some(data) = sse_data_payload(&line)
            {
                let event = parse_stream_event(data)?;
                if let Some(items) =
                    forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                {
                    terminal_items = Some(items);
                }
            }

            resolve_stream_outcome(terminal_items, &acc, cancelled)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        Item, MessageItem, OutputItem, OutputMessage, OutputMessageContent, ResponseStreamEvent,
    };
    use crate::config::schema::ProviderAuth;
    use crate::llm::request::ModelRequest;
    use crate::types::user_text;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "doubao-seed-2.1-turbo".into(),
            instructions: "sys".into(),
            input: vec![user_text("hello")],
            tools: vec![],
            max_output_tokens: 64,
            temperature: 0.0,
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
            session_id: Some("ses_ark".into()),
        }
    }

    fn completed_response_json() -> String {
        serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1,
            "model": "doubao-seed-2.1-turbo",
            "status": "completed",
            "output": [
                {
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{"type": "output_text", "text": "hi", "annotations": []}]
                },
                {
                    "type": "function_call",
                    "id": "fc_1",
                    "call_id": "call_1",
                    "name": "read",
                    "arguments": "{\"p\":1}",
                    "status": "completed"
                }
            ]
        })
        .to_string()
    }

    fn sse_tool_call() -> String {
        let added = serde_json::json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "read",
                "arguments": "",
                "status": "in_progress"
            }
        });
        let delta = serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "sequence_number": 2,
            "item_id": "fc_1",
            "output_index": 0,
            "delta": "{\"p\":1}"
        });
        let completed = serde_json::json!({
            "type": "response.completed",
            "sequence_number": 3,
            "response": serde_json::from_str::<serde_json::Value>(&completed_response_json()).unwrap()
        });
        format!("data: {added}\n\ndata: {delta}\n\ndata: {completed}\n\n")
    }

    fn sse_text_only() -> String {
        let created = serde_json::json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": {
                "created_at": 1,
                "id": "resp_1",
                "max_output_tokens": 64,
                "model": "doubao-seed-2.1-turbo",
                "object": "response",
                "thinking": { "type": "enabled" },
                "store": false
            }
        });
        let delta = serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hi"
        });
        let completed = serde_json::json!({
            "type": "response.completed",
            "sequence_number": 2,
            "response": {
                "id": "resp_1",
                "object": "response",
                "created_at": 1,
                "model": "doubao-seed-2.1-turbo",
                "status": "completed",
                "output": [{
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "status": "completed",
                    "content": [{"type": "output_text", "text": "hi", "annotations": []}]
                }]
            }
        });
        format!("data: {created}\n\ndata: {delta}\n\ndata: {completed}\n\n")
    }

    async fn serve_once(body: String, status: &str, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let content_type = content_type.to_string();
        let status = status.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}/api/coding/v3")
    }

    #[test]
    fn default_coding_plan_urls() {
        assert_eq!(
            normalize_endpoint(DEFAULT_ENDPOINT.into()),
            "https://ark.cn-beijing.volces.com/api/coding/v3/responses"
        );
        assert_eq!(
            normalize_endpoint(format!("{DEFAULT_ENDPOINT}/")),
            "https://ark.cn-beijing.volces.com/api/coding/v3/responses"
        );
        assert_eq!(
            normalize_endpoint("https://ark.cn-beijing.volces.com/api/coding/v3/responses".into()),
            "https://ark.cn-beijing.volces.com/api/coding/v3/responses"
        );
    }

    #[test]
    fn build_body_store_false_and_thinking_dialect() {
        let mut req = sample_request();
        let none = ArkCodingProvider::build_body(&req, false).unwrap();
        assert_eq!(none["store"], false);
        assert!(none.get("thinking").is_none());
        assert!(none.get("reasoning").is_none());
        assert_eq!(none["stream"], false);

        req.reasoning_effort = Some("low".into());
        let low = ArkCodingProvider::build_body(&req, true).unwrap();
        assert_eq!(low["thinking"]["type"], "disabled");
        assert!(low.get("reasoning").is_none());
        assert_eq!(low["stream"], true);

        req.reasoning_effort = Some("medium".into());
        let med = ArkCodingProvider::build_body(&req, false).unwrap();
        assert_eq!(med["thinking"]["type"], "enabled");
        assert_eq!(med["reasoning"]["effort"], "medium");

        req.reasoning_effort = Some("high".into());
        let high = ArkCodingProvider::build_body(&req, false).unwrap();
        assert_eq!(high["thinking"]["type"], "enabled");
        assert_eq!(high["reasoning"]["effort"], "high");

        req.thinking_mode = Some("disabled".into());
        req.reasoning_effort = Some("high".into());
        let off = ArkCodingProvider::build_body(&req, false).unwrap();
        assert_eq!(off["thinking"]["type"], "disabled");
        assert!(off.get("reasoning").is_none());
    }

    #[test]
    fn kimi_still_gets_thinking_fields() {
        let req = ModelRequest {
            model: "kimi-k2.7-code".into(),
            instructions: String::new(),
            input: vec![],
            tools: vec![],
            max_output_tokens: 16,
            temperature: 0.0,
            reasoning_effort: Some("medium".into()),
            thinking_mode: None,
            json_output: false,
            session_id: None,
        };
        let body = ArkCodingProvider::build_body(&req, false).unwrap();
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning"]["effort"], "medium");
    }

    #[tokio::test]
    async fn complete_uses_bearer_litecode_ua_without_opencode_headers() {
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_cb = Arc::clone(&captured);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            captured_cb.lock().unwrap().extend_from_slice(&buf[..n]);
            let body = completed_response_json();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        let provider =
            ArkCodingProvider::new(format!("http://{addr}/api/coding/v3"), ProviderAuth::Bearer)
                .expect("provider");
        provider
            .complete(&sample_request(), "sk-ark")
            .await
            .expect("ok");
        let captured = captured.lock().unwrap();
        let raw = String::from_utf8_lossy(&captured);
        let lower = raw.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: bearer sk-ark"),
            "missing bearer in {raw}"
        );
        assert!(
            lower.contains(&format!(
                "user-agent: litecode/{}",
                env!("CARGO_PKG_VERSION")
            )),
            "missing litecode ua in {raw}"
        );
        assert!(
            !lower.contains("x-opencode-"),
            "must not send OpenCode headers in {raw}"
        );
        assert!(raw.contains("/responses"), "must POST responses in {raw}");
        assert!(
            !raw.contains("/chat/completions"),
            "must not POST chat completions in {raw}"
        );
        assert!(
            raw.contains("\"store\":false") || raw.contains("\"store\": false"),
            "must send store:false in {raw}"
        );
    }

    #[tokio::test]
    async fn stream_tool_call_orders_added_before_delta() {
        let endpoint = serve_once(sse_tool_call(), "200 OK", "text/event-stream").await;
        let provider = ArkCodingProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));
        let items = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        assert!(
            items
                .iter()
                .any(|i| matches!(i, Item::FunctionCall(fc) if fc.name == "read")),
            "got {items:?}"
        );
        let events = seen.lock().unwrap().clone();
        let added = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseOutputItemAdded(ev)
                    if matches!(&ev.item, OutputItem::FunctionCall(fc) if fc.name == "read")
            )
        });
        let delta = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(_)
            )
        });
        assert!(added.is_some() && delta.is_some());
        assert!(added.unwrap() < delta.unwrap());
    }

    #[tokio::test]
    async fn http_error_uses_ark_prefix() {
        let endpoint = serve_once("nope".into(), "400 Bad Request", "application/json").await;
        let provider = ArkCodingProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete(&sample_request(), "sk-test")
            .await
            .expect_err("fail");
        let msg = err.to_string();
        assert!(msg.contains("Ark Coding Plan"), "{msg}");
        assert!(!msg.contains("OpenCode"), "{msg}");
        assert!(msg.contains("HTTP 400"), "{msg}");
    }

    #[tokio::test]
    async fn stream_json_content_type_fails() {
        let endpoint = serve_once(
            r#"{"error":"not sse"}"#.into(),
            "200 OK",
            "application/json",
        )
        .await;
        let provider = ArkCodingProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("fail");
        let msg = err.to_string();
        assert!(
            msg.contains("not sse") || msg.contains("event-stream") || msg.contains("JSON"),
            "{msg}"
        );
    }

    #[test]
    fn harden_created_event_fills_output_and_status() {
        let raw = serde_json::json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": {
                "created_at": 1,
                "id": "resp_1",
                "max_output_tokens": 128000,
                "model": "deepseek-v4-flash",
                "object": "response",
                "thinking": { "type": "enabled" },
                "store": false
            }
        });
        let event = parse_stream_event(&raw.to_string()).expect("ark created");
        assert!(matches!(event, ResponseStreamEvent::ResponseCreated(_)));
    }

    #[test]
    fn harden_reasoning_item_added_fills_summary() {
        let raw = serde_json::json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "sequence_number": 2,
            "item": {
                "id": "rs_1",
                "type": "reasoning",
                "status": "in_progress"
            }
        });
        let event = parse_stream_event(&raw.to_string()).expect("ark reasoning added");
        match event {
            ResponseStreamEvent::ResponseOutputItemAdded(ev) => {
                assert!(matches!(ev.item, OutputItem::Reasoning(_)));
            }
            other => panic!("expected output_item.added, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_text_only_with_ark_created() {
        let endpoint = serve_once(sse_text_only(), "200 OK", "text/event-stream").await;
        let provider = ArkCodingProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        let text: String = items
            .iter()
            .filter_map(|i| match i {
                Item::Message(MessageItem::Output(OutputMessage { content, .. })) => {
                    content.iter().find_map(|c| match c {
                        OutputMessageContent::OutputText(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                }
                _ => None,
            })
            .collect();
        assert!(text.contains("hi"), "got {items:?}");
    }
}
