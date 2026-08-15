//! DeepSeek Responses wire adapter — vendor-tolerant Responses dialect.
//!
//! Official: [Responses API](https://api-docs.deepseek.com/zh-cn/guides/responses_api)
//! and [thinking mode](https://api-docs.deepseek.com/zh-cn/guides/thinking_mode).
//!
//! `reasoning.effort` is `none` / `low` / `high` / `max` (`none` disables thinking).
//! Vendor default is thinking on at `high`. Platform Low/Med/High map to
//! `low` / `high` / `max`. This adapter does **not** share the strict OpenAI
//! Responses path (unsupported params are ignored by the vendor; usage details
//! are hardened before authority serde).

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{Item, OutputItem, Response, ResponseStreamEvent};
use crate::config::schema::ProviderAuth;
use crate::types::{LitecodeError, Result, StreamEvents};

use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;

use super::responses_sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};

/// Platform Default context budget for this closed adapter.
pub(crate) const CONTEXT_WINDOW_DEFAULT: usize = 256_000;
/// Vendor maximum context window — session `context_mode = max`.
pub(crate) const CONTEXT_WINDOW_MAX: usize = 1_000_000;
/// Selectable wire model ids for Settings dropdown.
pub(crate) const API_MODEL_IDS: &[&str] = &["deepseek-v4-flash", "deepseek-v4-pro"];
/// Official DeepSeek Responses host. `/responses` is appended by [`normalize_endpoint`].
pub(crate) const DEFAULT_ENDPOINT: &str = "https://api.deepseek.com";

/// DeepSeek Responses-protocol provider (`adapter_id = deepseek_responses`).
pub struct DeepseekResponsesProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl DeepseekResponsesProvider {
    pub fn new(endpoint: String, auth: ProviderAuth) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint);
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
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

        let effort = resolve_deepseek_reasoning_effort(params);
        let mut body = serde_json::json!({
            "model": params.model,
            "instructions": params.instructions,
            "input": input,
            "stream": stream,
            "max_output_tokens": params.max_output_tokens,
            "reasoning": {
                "effort": effort,
            },
        });
        // Thinking mode ignores temperature; omit rather than send a no-op.
        if effort == "none" {
            body["temperature"] = serde_json::json!(params.temperature);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if params.json_output {
            body["text"] = serde_json::json!({
                "format": { "type": "json_object" }
            });
        }

        Ok(body)
    }
}

/// DeepSeek Responses `reasoning.effort`.
///
/// Platform three-tier (via `ModelRequest.reasoning_effort`): `low` / `high` / `max`.
/// Vendor default when unset is `high`.
fn resolve_deepseek_reasoning_effort(params: &ModelRequest) -> &'static str {
    match params.reasoning_effort.as_deref() {
        Some("none") => "none",
        Some("low") => "low",
        Some("max") => "max",
        Some("high") | Some("medium") => "high",
        _ => "high",
    }
}

/// Fill empty / incomplete usage objects so authority serde succeeds.
pub(crate) fn harden_deepseek_json(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("input_tokens") && map.contains_key("output_tokens") {
                let input = map.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                let output = map
                    .get("output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                map.entry("total_tokens")
                    .or_insert_with(|| Value::from(input.saturating_add(output)));
            }
            if let Some(Value::Object(details)) = map.get_mut("input_tokens_details") {
                details
                    .entry("cached_tokens")
                    .or_insert_with(|| Value::from(0u64));
            }
            if let Some(Value::Object(details)) = map.get_mut("output_tokens_details") {
                details
                    .entry("reasoning_tokens")
                    .or_insert_with(|| Value::from(0u64));
            }
            // DeepSeek Responses `reasoning.effort` includes `max`; OpenAI authority
            // serde only knows none/minimal/low/medium/high/xhigh.
            if matches!(map.get("effort"), Some(Value::String(s)) if s == "max") {
                map.insert("effort".into(), Value::String("xhigh".into()));
            }
            for child in map.values_mut() {
                harden_deepseek_json(child);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                harden_deepseek_json(child);
            }
        }
        _ => {}
    }
}

fn parse_response(text: &str) -> Result<Response> {
    let mut value: Value = serde_json::from_str(text)
        .map_err(|e| LitecodeError::Llm(format!("deserialize Response JSON: {e}")))?;
    harden_deepseek_json(&mut value);
    serde_json::from_value(value)
        .map_err(|e| LitecodeError::Llm(format!("deserialize Response: {e}")))
}

fn parse_stream_event(data: &str) -> Result<ResponseStreamEvent> {
    let mut value: Value = serde_json::from_str(data).map_err(|e| {
        LitecodeError::Llm(format!(
            "deserialize ResponseStreamEvent JSON: {e}; payload={data}"
        ))
    })?;
    harden_deepseek_json(&mut value);
    serde_json::from_value(value).map_err(|e| {
        LitecodeError::Llm(format!(
            "deserialize ResponseStreamEvent: {e}; payload={data}"
        ))
    })
}

fn normalize_endpoint(endpoint: String) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        return trimmed.to_string();
    }
    if let Ok(parsed) = url::Url::parse(trimmed) {
        let path = parsed.path();
        if path.is_empty() || path == "/" || path == "/v1" || path.ends_with("/v1") {
            let full = format!("{trimmed}/responses");
            tracing::info!("endpoint normalized: {trimmed} -> {full}");
            return full;
        }
    }
    trimmed.to_string()
}

fn output_items_to_items(output: Vec<OutputItem>) -> Vec<Item> {
    output.into_iter().map(Item::from).collect()
}

impl LlmProvider for DeepseekResponsesProvider {
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
            let resp = self
                .client
                .post(&self.endpoint_url)
                .header(header_name, header_value)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| LitecodeError::Llm(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(LitecodeError::Llm(format!("HTTP {status}: {text}")));
            }

            let text = resp
                .text()
                .await
                .map_err(|e| LitecodeError::Llm(e.to_string()))?;
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
            let resp = self
                .client
                .post(&self.endpoint_url)
                .header(header_name, header_value)
                .header("content-type", "application/json")
                .header("accept", "text/event-stream")
                .json(&body)
                .send()
                .await
                .map_err(|e| LitecodeError::Llm(e.to_string()))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(LitecodeError::Llm(format!("HTTP {status}: {text}")));
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
                        let chunk = chunk.map_err(|e| LitecodeError::Llm(e.to_string()))?;
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

            if !cancelled {
                if let Some(line) = reader.finish()? {
                    if let Some(data) = sse_data_payload(&line) {
                        let event = parse_stream_event(data)?;
                        if let Some(items) =
                            forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                        {
                            terminal_items = Some(items);
                        }
                    }
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
        Item, MessageItem, OutputMessage, OutputStatus, ReasoningItem,
        ResponseFunctionCallArgumentsDeltaEvent, ResponseReasoningTextDeltaEvent,
        ResponseStreamEvent, ResponseTextDeltaEvent,
    };
    use crate::config::schema::ProviderAuth;
    use crate::llm::request::{ModelRequest, ToolDef};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    fn sample_request(tools: Vec<ToolDef>) -> ModelRequest {
        ModelRequest {
            model: "deepseek-v4-flash".into(),
            instructions: "test".into(),
            input: vec![],
            tools,
            max_output_tokens: 64,
            temperature: 0.7,
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
        }
    }

    fn completed_response_json() -> String {
        serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1,
            "model": "deepseek-v4-flash",
            "status": "completed",
            "store": false,
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "summary": [],
                    "content": [{"type": "reasoning_text", "text": "think"}]
                },
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
                    "name": "bash",
                    "arguments": "{\"command\":\"ls\"}",
                    "status": "completed"
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "input_tokens_details": {},
                "output_tokens_details": {}
            }
        })
        .to_string()
    }

    fn sse_with_event_fields() -> String {
        let text_delta = serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_1",
            "output_index": 1,
            "content_index": 0,
            "delta": "hi"
        });
        let reasoning_delta = serde_json::json!({
            "type": "response.reasoning_text.delta",
            "sequence_number": 2,
            "item_id": "rs_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "think"
        });
        let fc_delta = serde_json::json!({
            "type": "response.function_call_arguments.delta",
            "sequence_number": 3,
            "item_id": "fc_1",
            "output_index": 2,
            "delta": "{\"command\":\"ls\"}"
        });
        let completed = serde_json::json!({
            "type": "response.completed",
            "sequence_number": 4,
            "response": serde_json::from_str::<serde_json::Value>(&completed_response_json()).unwrap()
        });
        // Official DeepSeek stream: event: field + data JSON; no data: [DONE].
        format!(
            "event: response.output_text.delta\ndata: {text_delta}\n\n\
             event: response.reasoning_text.delta\ndata: {reasoning_delta}\n\n\
             event: response.function_call_arguments.delta\ndata: {fc_delta}\n\n\
             event: response.completed\ndata: {completed}\n\n"
        )
    }

    async fn serve_once(body: String, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let content_type = content_type.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}/v1")
    }

    fn item_id(item: &Item) -> Option<String> {
        match item {
            Item::Message(MessageItem::Output(OutputMessage { id, .. })) => Some(id.clone()),
            Item::Reasoning(ReasoningItem { id, .. }) => id.clone(),
            Item::FunctionCall(fc) => fc.id.clone(),
            _ => None,
        }
    }

    #[test]
    fn reasoning_effort_defaults_to_high() {
        let body = DeepseekResponsesProvider::build_body(&sample_request(vec![]), false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_effort_maps_platform_tiers() {
        let mut req = sample_request(vec![]);
        req.reasoning_effort = Some("low".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("temperature").is_none());

        req.reasoning_effort = Some("high".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");

        req.reasoning_effort = Some("max".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "max");

        req.reasoning_effort = Some("none".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "none");
        assert_eq!(body["temperature"], 0.7);
    }

    #[test]
    fn thinking_mode_does_not_override_effort() {
        let mut req = sample_request(vec![]);
        req.thinking_mode = Some("disabled".into());
        req.reasoning_effort = Some("high".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn medium_effort_maps_to_vendor_high() {
        let mut req = sample_request(vec![]);
        req.reasoning_effort = Some("medium".into());
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn omits_empty_tools_and_sets_json_format() {
        let mut req = sample_request(vec![]);
        req.json_output = true;
        let body = DeepseekResponsesProvider::build_body(&req, false).unwrap();
        assert!(body.get("tools").is_none());
        assert_eq!(body["text"]["format"]["type"], "json_object");
    }

    #[test]
    fn function_tools_use_responses_shape() {
        let tools = vec![ToolDef {
            name: "read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({}),
        }];
        let body = DeepseekResponsesProvider::build_body(&sample_request(tools), true).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "read");
        assert!(body["tools"][0].get("function").is_none());
    }

    #[test]
    fn normalize_root_to_responses() {
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com".into()),
            "https://api.deepseek.com/responses"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com/v1".into()),
            "https://api.deepseek.com/v1/responses"
        );
        assert_eq!(
            normalize_endpoint("https://api.deepseek.com/v1/responses".into()),
            "https://api.deepseek.com/v1/responses"
        );
    }

    #[test]
    fn harden_fills_empty_token_details() {
        let mut v = serde_json::json!({
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "input_tokens_details": {},
                "output_tokens_details": {}
            }
        });
        harden_deepseek_json(&mut v);
        assert_eq!(v["usage"]["input_tokens_details"]["cached_tokens"], 0);
        assert_eq!(v["usage"]["output_tokens_details"]["reasoning_tokens"], 0);
        assert_eq!(v["usage"]["total_tokens"], 3);
    }

    #[test]
    fn harden_maps_deepseek_max_effort_to_xhigh() {
        let mut v = serde_json::json!({
            "reasoning": { "effort": "max", "summary": null }
        });
        harden_deepseek_json(&mut v);
        assert_eq!(v["reasoning"]["effort"], "xhigh");
    }

    #[test]
    fn parse_response_created_with_effort_max() {
        let data = serde_json::json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": {
                "id": "d1ed54e9-da81-4eca-9a61-4a23542948e2",
                "object": "response",
                "created_at": 1786634511,
                "status": "in_progress",
                "model": "deepseek-v4-pro",
                "output": [],
                "reasoning": { "effort": "max", "summary": null },
                "store": false,
                "parallel_tool_calls": true
            }
        })
        .to_string();
        parse_stream_event(&data).expect("DeepSeek effort=max must deserialize after harden");
    }

    #[tokio::test]
    async fn stream_parses_event_field_sse_without_done() {
        let body = sse_with_event_fields();
        let endpoint = serve_once(body, "text/event-stream").await;
        let provider =
            DeepseekResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));

        let items = provider
            .complete_with_stream_events(
                &sample_request(vec![]),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("stream ok");

        let events = seen.lock().unwrap().clone();
        assert!(events.iter().any(|e| matches!(
            e,
            ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent { item_id, .. })
                if item_id == "msg_1"
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            ResponseStreamEvent::ResponseReasoningTextDelta(ResponseReasoningTextDeltaEvent {
                item_id,
                ..
            }) if item_id == "rs_1"
        )));
        assert!(events.iter().any(|e| matches!(
            e,
            ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
                ResponseFunctionCallArgumentsDeltaEvent { item_id, .. }
            ) if item_id == "fc_1"
        )));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ResponseStreamEvent::ResponseCompleted(_)))
        );
        let ids: Vec<_> = items.iter().filter_map(item_id).collect();
        assert!(ids.contains(&"msg_1".to_string()));
        assert!(ids.contains(&"rs_1".to_string()));
        assert!(ids.contains(&"fc_1".to_string()));
    }

    #[tokio::test]
    async fn cancel_after_text_delta_seals_incomplete() {
        let body = format!(
            "event: response.output_text.delta\ndata: {}\n\n",
            serde_json::json!({
                "type": "response.output_text.delta",
                "sequence_number": 1,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "hello partial"
            })
        );
        let endpoint = serve_once(body, "text/event-stream").await;
        let provider =
            DeepseekResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let cancel = CancellationToken::new();
        let cancel_cb = cancel.clone();
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |_ev| {
                cancel_cb.cancel();
            }));
        let items = provider
            .complete_with_stream_events(&sample_request(vec![]), "sk-test", on_event, &cancel)
            .await
            .expect("opened stream seals incomplete on cancel");
        assert_eq!(items.len(), 1);
        match &items[0] {
            Item::Message(MessageItem::Output(msg)) => {
                assert_eq!(msg.status, OutputStatus::Incomplete);
                assert_eq!(crate::types::item_text_preview(&items[0]), "hello partial");
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_non_stream_hardens_usage_and_returns_items() {
        let body = completed_response_json();
        let endpoint = serve_once(body, "application/json").await;
        let provider =
            DeepseekResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete(&sample_request(vec![]), "sk-test")
            .await
            .expect("complete ok");
        let ids: Vec<_> = items.iter().filter_map(item_id).collect();
        assert!(ids.contains(&"msg_1".to_string()));
        assert!(ids.contains(&"rs_1".to_string()));
        assert!(ids.contains(&"fc_1".to_string()));
    }
}
