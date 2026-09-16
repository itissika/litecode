//! Command Code Provider API adapter (GOAT / Pro / Max / Team / Provider plans).
//!
//! OpenAI Chat Completions wire at `https://api.commandcode.ai/provider/v1`
//! (`POST {base}/chat/completions`, `GET {base}/models`). Conversion lives in
//! [`super::chat_completions`]. This file owns Bearer auth, the LiteCode
//! user-agent, and error wrapping — no third-party client headers.
//!
//! Conformance notes (verified against the official docs and the live catalog):
//! - Auth is `Authorization: Bearer <key>` (docs: bearer for any route).
//! - `max_tokens` and `stream_options.include_usage` are documented request
//!   fields; usage arrives on a final chunk (the codec reads `usage` even when
//!   that chunk carries no choices).
//! - Anthropic models are served by the sibling `/messages` endpoint only and
//!   are rejected here with HTTP 400; they are filtered from the catalog and
//!   refused by `validate_model_config`.
//! - Undocumented knobs (`reasoning_effort`, `json_output`) are intentionally
//!   not sent; LiteCode's chat codec treats them as no-ops for this wire.

use std::future::Future;
use std::pin::Pin;

use reqwest::Client;
use tokio_util::sync::CancellationToken;

use crate::config::schema::ProviderAuth;
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::types::{Item, Result, StreamEvents};

use super::chat_completions::{
    ChatEncodeOpts, chat_post_url, complete_from_response, encode_chat_body, normalize_endpoint,
    stream_from_response,
};
use super::{llm_http_client, transport_error};

/// Official Provider API root. Empty Settings endpoint fills this.
pub(crate) const DEFAULT_ENDPOINT: &str = "https://api.commandcode.ai/provider/v1";

/// Neutral prefix: Command Code's own OpenAI error envelope (HTTP body) carries
/// the actionable detail — e.g. a Claude id sent here returns 400 pointing at
/// `/v1/messages` — so the prefix must not mislabel rate-limit or 5xx errors.
const ERROR_PREFIX: &str = "Command Code adapter";

pub struct CommandcodeProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl CommandcodeProvider {
    pub fn new(endpoint: String, auth: ProviderAuth) -> Result<Self> {
        let endpoint = normalize_endpoint(endpoint);
        // See `opencode::OpencodeProvider::new` — no wall-clock timeout: long
        // thinking outlives any fixed cap while the stream stays healthy.
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

/// Command Code sees LiteCode as LiteCode — Bearer key, LiteCode user-agent,
/// and nothing borrowed from another agent's client identity.
fn apply_commandcode_headers(
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

impl LlmProvider for CommandcodeProvider {
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
            let body = encode_chat_body(request, false, &ChatEncodeOpts::COMMANDCODE)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = apply_commandcode_headers(
                self.client.post(self.post_url()),
                header_name,
                header_value,
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| transport_error("sending Command Code response", &e))?;
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
            let body = encode_chat_body(request, true, &ChatEncodeOpts::COMMANDCODE)?;
            let (header_name, header_value) = self.auth_header(api_key);
            let resp = super::send_cancellable(
                apply_commandcode_headers(
                    self.client.post(self.post_url()),
                    header_name,
                    header_value,
                )
                .header("accept", "text/event-stream")
                .json(&body),
                cancel,
                "opening Command Code event stream",
            )
            .await?;
            stream_from_response(resp, &request.model, ERROR_PREFIX, on_event, cancel).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{Item, MessageItem, OutputMessage, OutputMessageContent};
    use crate::types::user_text;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "deepseek/deepseek-v4-flash".into(),
            instructions: "sys".into(),
            input: vec![user_text("hello")],
            tools: vec![],
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
        format!("http://{addr}/provider/v1")
    }

    #[test]
    fn default_endpoint_urls() {
        assert_eq!(
            chat_post_url(&normalize_endpoint(DEFAULT_ENDPOINT.into())),
            "https://api.commandcode.ai/provider/v1/chat/completions"
        );
        assert_eq!(
            super::super::chat_completions::models_get_url(&normalize_endpoint(DEFAULT_ENDPOINT.into())),
            "https://api.commandcode.ai/provider/v1/models"
        );
    }

    #[tokio::test]
    async fn complete_sends_bearer_litecode_ua_without_foreign_headers() {
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
        let provider = CommandcodeProvider::new(
            format!("http://{addr}/provider/v1"),
            ProviderAuth::Bearer,
        )
        .expect("provider");
        provider.complete(&sample_request(), "sk-cmd").await.expect("ok");
        let captured = captured.lock().unwrap();
        let raw = String::from_utf8_lossy(&captured);
        let lower = raw.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: bearer sk-cmd"),
            "missing bearer in {raw}"
        );
        assert!(
            lower.contains(&format!("user-agent: litecode/{}", env!("CARGO_PKG_VERSION"))),
            "missing litecode ua in {raw}"
        );
        assert!(
            !lower.contains("x-opencode-"),
            "must not send OpenCode headers in {raw}"
        );
        assert!(
            raw.contains("/provider/v1/chat/completions"),
            "must POST chat completions in {raw}"
        );
        assert!(
            lower.contains("\"stream\":false") || lower.contains("\"stream\": false"),
            "must send stream:false in {raw}"
        );
    }

    #[tokio::test]
    async fn stream_text_only() {
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let endpoint = serve_once(sse.into(), "200 OK", "text/event-stream").await;
        let provider =
            CommandcodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
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

    #[tokio::test]
    async fn stream_request_matches_official_shape() {
        // Docs "Streaming": `stream: true` + `stream_options.include_usage`.
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_cb = Arc::clone(&captured);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.expect("read");
            captured_cb.lock().unwrap().extend_from_slice(&buf[..n]);
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        let provider = CommandcodeProvider::new(
            format!("http://{addr}/provider/v1"),
            ProviderAuth::Bearer,
        )
        .expect("provider");
        let items = provider
            .complete_with_stream_events(
                &sample_request(),
                "sk-cmd",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("ok");
        assert!(!items.is_empty(), "stream produced no items");

        let captured = captured.lock().unwrap();
        let raw = String::from_utf8_lossy(&captured);
        let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains("\"stream\":true"),
            "must send stream:true in {raw}"
        );
        assert!(
            compact.contains("\"stream_options\":{\"include_usage\":true}"),
            "must opt into final usage chunk in {raw}"
        );
        assert!(
            compact.contains("\"max_tokens\":64"),
            "must send max_tokens (docs field) in {raw}"
        );
        assert!(
            !raw.to_ascii_lowercase().contains("x-opencode-"),
            "must not send OpenCode headers in {raw}"
        );
    }

    #[tokio::test]
    async fn http_400_uses_commandcode_prefix() {
        let endpoint = serve_once("nope".into(), "400 Bad Request", "application/json").await;
        let provider =
            CommandcodeProvider::new(endpoint, ProviderAuth::Bearer).expect("provider");
        let err = provider
            .complete(&sample_request(), "sk-test")
            .await
            .expect_err("fail");
        let msg = err.to_string();
        assert!(msg.contains("Command Code"), "{msg}");
        assert!(!msg.contains("OpenCode"), "{msg}");
        assert!(msg.contains("HTTP 400"), "{msg}");
    }
}
