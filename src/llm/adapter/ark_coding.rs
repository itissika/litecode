//! Volcengine Ark Coding Plan — OpenAI Chat Completions gateway.
//!
//! Dedicated Coding Plan host (`/api/coding/v3`), not the general Ark `/api/v3`.
//! Conversion lives in [`super::chat_completions`]. This file owns Bearer auth,
//! LiteCode user-agent, and error wrapping.

use std::future::Future;
use std::pin::Pin;

use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::config::schema::ProviderAuth;
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::types::{Item, LitecodeError, Result, StreamEvents};

use super::chat_completions::{
    ChatEncodeOpts, chat_post_url, complete_from_response, encode_chat_body, normalize_endpoint,
    stream_from_response,
};

pub(crate) const DEFAULT_ENDPOINT: &str = "https://ark.cn-beijing.volces.com/api/coding/v3";

const ERROR_PREFIX: &str = "Ark Coding Plan adapter only speaks Chat Completions";

pub struct ArkCodingProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl ArkCodingProvider {
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

    fn post_url(&self) -> String {
        chat_post_url(&self.endpoint_url)
    }
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
            let body = encode_chat_body(request, false, &ChatEncodeOpts::ARK)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp =
                apply_ark_headers(self.client.post(self.post_url()), header_name, header_value)
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| LitecodeError::Llm(e.to_string()))?;
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
            let body = encode_chat_body(request, true, &ChatEncodeOpts::ARK)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp =
                apply_ark_headers(self.client.post(self.post_url()), header_name, header_value)
                    .header("accept", "text/event-stream")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| LitecodeError::Llm(e.to_string()))?;
            stream_from_response(resp, &request.model, ERROR_PREFIX, on_event, cancel).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::chat_completions::{chat_post_url, models_get_url};
    use super::*;
    use crate::authority::responses::{
        Item, MessageItem, OutputItem, OutputMessage, OutputMessageContent, ResponseStreamEvent,
    };
    use crate::types::user_text;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "ark-code-latest".into(),
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
            chat_post_url(DEFAULT_ENDPOINT),
            "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
        );
        assert_eq!(
            models_get_url(DEFAULT_ENDPOINT),
            "https://ark.cn-beijing.volces.com/api/coding/v3/models"
        );
        assert_eq!(
            normalize_endpoint(format!("{DEFAULT_ENDPOINT}/")),
            DEFAULT_ENDPOINT
        );
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
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
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
        assert!(
            raw.contains("/chat/completions"),
            "must POST chat completions in {raw}"
        );
    }

    #[tokio::test]
    async fn stream_tool_call_orders_added_before_delta() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"p\\\":1}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
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

    #[tokio::test]
    async fn stream_text_only() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
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
