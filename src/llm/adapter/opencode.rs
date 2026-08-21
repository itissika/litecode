//! OpenCode Chat-Completions adapter (Zen default host; Go via endpoint override).
//!
//! Conversion lives in [`super::chat_completions`]. This file owns Zen headers,
//! host, and error wrapping.

use std::future::Future;
use std::pin::Pin;

use reqwest::Client;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::schema::ProviderAuth;
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::types::{Item, Result, StreamEvents};

use super::chat_completions::{
    ChatEncodeOpts, chat_post_url, complete_from_response, encode_chat_body, normalize_endpoint,
    stream_from_response,
};
use super::{llm_http_client, transport_error};

/// Official Zen host. Empty Settings endpoint fills this; override for Go.
pub(crate) const DEFAULT_ENDPOINT: &str = "https://opencode.ai/zen/v1";

const ERROR_PREFIX: &str =
    "OpenCode adapter only speaks Chat Completions; this host's catalog may not include this id";

pub struct OpencodeProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl OpencodeProvider {
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

    fn post_url(&self) -> String {
        chat_post_url(&self.endpoint_url)
    }
}

fn zen_session_header(request: &ModelRequest) -> String {
    match request
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(id) => id.to_string(),
        None => "global".to_string(),
    }
}

/// OpenCode Zen headers aligned with `packages/opencode/src/session/llm/request.ts`.
/// `x-opencode-session` must be the Litecode conversation id, not a process-wide UUID.
fn apply_opencode_headers(
    builder: reqwest::RequestBuilder,
    header_name: String,
    header_value: String,
    session_id: &str,
    request_id: &str,
) -> reqwest::RequestBuilder {
    let user_agent = format!("opencode/{}", env!("CARGO_PKG_VERSION"));
    builder
        .header(header_name, header_value)
        .header("content-type", "application/json")
        .header("accept", "*/*")
        .header("user-agent", user_agent)
        .header("x-opencode-client", "cli")
        .header("x-opencode-project", "global")
        .header("x-opencode-session", session_id)
        .header("x-opencode-request", request_id)
}

impl LlmProvider for OpencodeProvider {
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
            let body = encode_chat_body(request, false, &ChatEncodeOpts::OPENCODE)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let request_id = Uuid::new_v4().to_string();
            let zen_session = zen_session_header(request);
            let resp = apply_opencode_headers(
                self.client.post(self.post_url()),
                header_name,
                header_value,
                &zen_session,
                &request_id,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| transport_error("sending OpenCode response", &e))?;
            complete_from_response(resp, ERROR_PREFIX).await
        })
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = encode_chat_body(request, true, &ChatEncodeOpts::OPENCODE)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let request_id = Uuid::new_v4().to_string();
            let zen_session = zen_session_header(request);
            let resp = apply_opencode_headers(
                self.client.post(self.post_url()),
                header_name,
                header_value,
                &zen_session,
                &request_id,
            )
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| transport_error("opening OpenCode event stream", &e))?;
            stream_from_response(resp, &request.model, ERROR_PREFIX, on_event, cancel).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        Item, MessageItem, OutputItem, OutputMessage, OutputMessageContent, ResponseStreamEvent,
    };
    use crate::llm::request::ToolDef;
    use crate::types::user_text;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_request(tools: Vec<ToolDef>) -> ModelRequest {
        ModelRequest {
            model: "deepseek-v4-flash-free".into(),
            instructions: "sys".into(),
            input: vec![user_text("hello")],
            tools,
            max_output_tokens: 64,
            temperature: 0.0,
            reasoning_effort: None,
            thinking_mode: None,
            json_output: false,
            session_id: Some("ses_test".into()),
        }
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
        format!("http://{addr}/v1")
    }

    #[test]
    fn normalize_keeps_zen_and_go_v1() {
        assert_eq!(
            normalize_endpoint("https://opencode.ai/zen/v1/".into()),
            "https://opencode.ai/zen/v1"
        );
        assert_eq!(
            normalize_endpoint("https://opencode.ai/zen/go/v1".into()),
            "https://opencode.ai/zen/go/v1"
        );
    }

    #[test]
    fn zen_session_uses_litecode_session_id() {
        let mut req = sample_request(vec![]);
        req.session_id = Some("ses_abc".into());
        assert_eq!(zen_session_header(&req), "ses_abc");
        req.session_id = None;
        assert_eq!(zen_session_header(&req), "global");
        req.session_id = Some("  ".into());
        assert_eq!(zen_session_header(&req), "global");
    }

    #[tokio::test]
    async fn stream_text_only() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let items = provider
            .complete_with_stream_events(
                &sample_request(vec![]),
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

    #[tokio::test]
    async fn stream_tool_call() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"p\\\":1}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
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
    async fn http_400_mentions_chat_only() {
        let endpoint = serve_once("nope".into(), "400 Bad Request", "application/json").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete(&sample_request(vec![]), "sk-test")
            .await
            .expect_err("fail");
        let msg = err.to_string();
        assert!(msg.contains("Chat Completions"), "{msg}");
        assert!(msg.contains("HTTP 400"), "{msg}");
    }

    #[tokio::test]
    async fn stream_usage_lands_on_completed() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":40}}}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider = OpencodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let seen: Arc<Mutex<Vec<ResponseStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_cb = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                seen_cb.lock().unwrap().push(ev);
            }));
        provider
            .complete_with_stream_events(
                &sample_request(vec![]),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        let events = seen.lock().unwrap().clone();
        let usage = events.iter().find_map(|e| match e {
            ResponseStreamEvent::ResponseCompleted(ev) => ev.response.usage.clone(),
            _ => None,
        });
        let usage = usage.expect("completed usage");
        assert_eq!(usage.input_tokens, 50);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.input_tokens_details.cached_tokens, 40);
    }

    #[tokio::test]
    async fn complete_sends_request_session_as_x_opencode_session() {
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_cb = Arc::clone(&captured);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            captured_cb.lock().unwrap().extend_from_slice(&buf[..n]);
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        let provider = OpencodeProvider::new(format!("http://{addr}/v1"), ProviderAuth::Bearer)
            .expect("provider");
        let mut req = sample_request(vec![]);
        req.session_id = Some("ses_parallel_a".into());
        provider.complete(&req, "sk-test").await.expect("ok");
        let captured = captured.lock().unwrap();
        let raw = String::from_utf8_lossy(&captured);
        assert!(
            raw.to_ascii_lowercase()
                .contains("x-opencode-session: ses_parallel_a"),
            "missing per-session header in {raw}"
        );
        assert!(
            !raw.to_ascii_lowercase()
                .contains("x-opencode-session: global"),
            "must not fall back to process-wide session in {raw}"
        );
    }
}
