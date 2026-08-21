//! OpenAI Responses protocol adapter — Items only; HTTP via reqwest + response-types JSON.
//! Native SSE (`stream: true`) forwards authority `ResponseStreamEvent` through
//! [`super::stream_contract`] (product tool-name ordering).

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::authority::responses::{Item, OutputItem, Response, ResponseStreamEvent};
use crate::config::schema::ProviderAuth;
use crate::types::{LitecodeError, Result, StreamEvents};

use tokio_util::sync::CancellationToken;

use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;

use super::responses_sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::transport_error;

/// Responses-protocol provider (`protocol = openai_responses`).
pub struct OpenaiResponsesProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl OpenaiResponsesProvider {
    pub fn new(endpoint: String, auth: ProviderAuth) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint);
        // reqwest `.timeout` is a wall-clock cap on connect + full SSE body.
        // Long thinking outlives 120s while the stream is still healthy; user
        // cancel already covers "nothing happening". Idle `read_timeout` would
        // mis-kill silent thinking. Highest-ROI follow-up is retry on transport
        // timeout — shelved; dropping the cap is enough for now.
        let client = Client::builder()
            // .timeout(std::time::Duration::from_secs(120))
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

    /// Build Responses request body. Only difference for stream vs non-stream is `"stream"`.
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
        });

        if let Some(reasoning_effort) = &params.reasoning_effort {
            body["reasoning"] = serde_json::json!({ "effort": reasoning_effort });
        }

        Ok(body)
    }
}

fn normalize_endpoint(endpoint: String) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        return trimmed.to_string();
    }
    if let Ok(parsed) = url::Url::parse(trimmed) {
        let path = parsed.path();
        // Empty root or API version root → append /responses.
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

impl LlmProvider for OpenaiResponsesProvider {
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

    /// Explicit non-stream path (`stream: false`). Degradation = call this, not silently disable SSE.
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
                .map_err(|e| transport_error("sending OpenAI response", &e))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(LitecodeError::Llm(format!("HTTP {status}: {text}")));
            }

            let response: Response = resp
                .json()
                .await
                .map_err(|e| LitecodeError::Llm(format!("deserialize Response: {e}")))?;

            Ok(output_items_to_items(response.output))
        })
    }

    /// Native Responses SSE (`stream: true`). Final Items come from
    /// `response.completed` or `response.incomplete` output, or a cancel seal
    /// of Items already opened on the stream.
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
                .map_err(|e| transport_error("opening OpenAI event stream", &e))?;

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
                        let chunk = chunk.map_err(|e| {
                            transport_error("reading OpenAI event stream", &e)
                        })?;
                        for line in reader.feed(&chunk)? {
                            let Some(data) = sse_data_payload(&line) else {
                                continue;
                            };
                            let event: ResponseStreamEvent = serde_json::from_str(data).map_err(|e| {
                                LitecodeError::Llm(format!(
                                    "deserialize ResponseStreamEvent: {e}; payload={data}"
                                ))
                            })?;
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
                let event: ResponseStreamEvent = serde_json::from_str(data).map_err(|e| {
                    LitecodeError::Llm(format!(
                        "deserialize ResponseStreamEvent: {e}; payload={data}"
                    ))
                })?;
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
        Item, MessageItem, OutputMessage, OutputStatus, ReasoningItem,
        ResponseFunctionCallArgumentsDeltaEvent, ResponseReasoningTextDeltaEvent,
        ResponseStreamEvent, ResponseTextDeltaEvent,
    };
    use crate::config::schema::ProviderAuth;
    use crate::llm::request::ModelRequest;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "gpt-4o".into(),
            instructions: "test".into(),
            input: vec![],
            tools: vec![],
            max_output_tokens: 64,
            temperature: 0.0,
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
            session_id: None,
        }
    }

    fn completed_response_json() -> String {
        serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1,
            "model": "gpt-4o",
            "status": "completed",
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
            ]
        })
        .to_string()
    }

    fn sse_fixture_with_completed() -> String {
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
        format!(
            "data: {text_delta}\n\ndata: {reasoning_delta}\n\ndata: {fc_delta}\n\ndata: {completed}\n\n"
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
    fn normalize_root_to_responses() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1".into()),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1/".into()),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn normalize_keeps_explicit_responses() {
        assert_eq!(
            normalize_endpoint("https://api.openai.com/v1/responses".into()),
            "https://api.openai.com/v1/responses"
        );
    }

    #[test]
    fn build_body_stream_flag_only_difference() {
        let req = sample_request();
        let off = OpenaiResponsesProvider::build_body(&req, false).unwrap();
        let on = OpenaiResponsesProvider::build_body(&req, true).unwrap();
        assert_eq!(off["stream"], false);
        assert_eq!(on["stream"], true);
        let mut off_obj = off.as_object().unwrap().clone();
        let mut on_obj = on.as_object().unwrap().clone();
        off_obj.remove("stream");
        on_obj.remove("stream");
        assert_eq!(off_obj, on_obj);
    }

    #[tokio::test]
    async fn stream_events_forwarded_and_items_from_completed() {
        let body = sse_fixture_with_completed();
        let endpoint = serve_once(body, "text/event-stream").await;
        let provider =
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let request = sample_request();
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));

        let items = provider
            .complete_with_stream_events(&request, "sk-test", on_event, &CancellationToken::new())
            .await
            .expect("stream ok");

        let events = seen.lock().unwrap().clone();
        assert!(events.iter().any(|e| matches!(
            e,
            ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent {
                item_id,
                ..
            }) if item_id == "msg_1"
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
        // Fixture omits output_item.added — gate must synthesize before the delta.
        let added_pos = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseOutputItemAdded(ev)
                    if matches!(&ev.item, crate::authority::responses::OutputItem::FunctionCall(fc)
                        if fc.id.as_deref() == Some("fc_1") || fc.call_id == "fc_1")
            )
        });
        let delta_pos = events.iter().position(|e| {
            matches!(
                e,
                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(
                    ResponseFunctionCallArgumentsDeltaEvent { item_id, .. }
                ) if item_id == "fc_1"
            )
        });
        assert!(
            added_pos.is_some() && delta_pos.is_some() && added_pos.unwrap() < delta_pos.unwrap(),
            "gate must emit output_item.added before function_call_arguments.delta; events={events:?}"
        );
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
    async fn stream_without_completed_seals_incomplete() {
        let body = format!(
            "data: {}\n\n",
            serde_json::json!({
                "type": "response.output_text.delta",
                "sequence_number": 1,
                "item_id": "msg_1",
                "output_index": 0,
                "content_index": 0,
                "delta": "hi"
            })
        );
        let endpoint = serve_once(body, "text/event-stream").await;
        let provider =
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("opened stream seals incomplete");
        assert_eq!(items.len(), 1);
        match &items[0] {
            Item::Message(MessageItem::Output(msg)) => {
                assert_eq!(msg.status, OutputStatus::Incomplete);
                assert_eq!(crate::types::item_text_preview(&items[0]), "hi");
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_after_text_delta_seals_incomplete() {
        let body = format!(
            "data: {}\n\n",
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
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let cancel = CancellationToken::new();
        let cancel_cb = cancel.clone();
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |_ev| {
                cancel_cb.cancel();
            }));
        let items = provider
            .complete_with_stream_events(&sample_request(), "sk-test", on_event, &cancel)
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
    async fn cancel_before_any_event_is_canceled() {
        let endpoint = serve_once(String::new(), "text/event-stream").await;
        let provider =
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = provider
            .complete_with_stream_events(&sample_request(), "sk-test", None, &cancel)
            .await
            .expect_err("no opened items");
        assert!(matches!(err, LitecodeError::Canceled));
    }

    #[tokio::test]
    async fn non_event_stream_content_type_surfaces_proxy_error_body() {
        // A 200 with a JSON error body (misbehaving proxy) must surface the body
        // explicitly instead of being silently consumed as an empty stream.
        let body = r#"{"error":{"message":"upstream exploded","code":"bad_gateway"}}"#;
        let endpoint = serve_once(body.to_string(), "application/json").await;
        let provider =
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("must fail with the proxy body");
        let msg = err.to_string();
        assert!(msg.contains("text/event-stream"), "got: {msg}");
        assert!(msg.contains("upstream exploded"), "got: {msg}");
    }

    #[tokio::test]
    async fn complete_non_stream_returns_output_items() {
        let body = completed_response_json();
        let endpoint = serve_once(body, "application/json").await;
        let provider =
            OpenaiResponsesProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete(&sample_request(), "sk-test")
            .await
            .expect("complete ok");
        let ids: Vec<_> = items.iter().filter_map(item_id).collect();
        assert!(ids.contains(&"msg_1".to_string()));
        assert!(ids.contains(&"rs_1".to_string()));
        assert!(ids.contains(&"fc_1".to_string()));
    }
}
