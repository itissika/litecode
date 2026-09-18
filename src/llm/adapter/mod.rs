//! Private provider wire adapters. Not `pub` from `llm`.
//!
//! # Adapter selection
//!
//! Product adapters are registered in [`registry`] (`openai_responses`,
//! `deepseek_responses`, `mimo_responses`, `opencode`, `ark_coding`,
//! `commandcode`). OpenAI / DeepSeek / MiMo / Ark Coding speak Responses
//! JSON/SSE; OpenCode and Command Code use the Chat Completions codec in
//! [`chat_completions`]. Vendor-tolerant hardening stays in this directory.
//!
//! # Streaming contract
//!
//! All live authority [`crate::types::StreamEvents`] leave this module through
//! [`stream_contract::forward_stream_event`]: tool `function_call_arguments.delta`
//! is never forwarded before an `output_item.added` for that item id (synthesized
//! when the provider omits it).

mod ark_coding;
mod chat_completions;
mod commandcode;
mod deepseek_responses;
mod mimo_responses;
mod openai_responses;
mod opencode;
mod registry;
mod reasoning_replay;
mod responses_sse;
mod stream_contract;

use crate::config::schema::ProviderDefinition;
use crate::llm::provider::LlmProvider;
use crate::types::{LitecodeError, Result};
use tokio_util::sync::CancellationToken;

/// Build an HTTP client safe to share across short-lived agent runtimes.
///
/// Agent turns run on per-turn Tokio runtimes. Keeping an idle pooled
/// connection after its originating runtime exits leaves Hyper's dispatcher
/// task unavailable to the next turn (`DispatchGone`). Disable idle pooling;
/// an active stream remains unaffected.
pub(super) fn llm_http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()?)
}

/// First backoff for a transient failure while opening a stream.
const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_millis(500);
/// Cap for the doubling backoff — long waits stop helping once a link is gone.
const RETRY_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(8);
/// Retries after the first attempt (total attempts = `RETRY_MAX + 1`).
const RETRY_MAX: usize = 5;

/// Backoff for a 0-based retry index: `base * 2^attempt`, capped at
/// [`RETRY_MAX_DELAY`] (500ms, 1s, 2s, 4s, 8s, 8s, …).
fn retry_delay(attempt: usize) -> std::time::Duration {
    let factor = 1u32 << attempt.min(16);
    RETRY_BASE_DELAY.saturating_mul(factor).min(RETRY_MAX_DELAY)
}

/// Send a request while remaining cancellable during connect/headers.
/// Dropping the pending `send()` future aborts the HTTP request.
///
/// Transient failures (connect/timeout/request errors, 408/502/503/504) retry up
/// to [`RETRY_MAX`] times with a doubling backoff; a non-clonable body is sent
/// once. Nothing has reached the model server on these paths, so a retry cannot
/// duplicate a generation.
pub(super) async fn send_cancellable(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    stage: &str,
) -> Result<reqwest::Response> {
    let template = request.try_clone();
    let mut first = Some(request);
    for attempt in 0..=RETRY_MAX {
        let request = if let Some(request) = first.take() {
            request
        } else if let Some(request) = template
            .as_ref()
            .and_then(reqwest::RequestBuilder::try_clone)
        {
            request
        } else {
            return Err(LitecodeError::Llm(format!(
                "{stage} failed: request body cannot be cloned for retry"
            )));
        };

        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(LitecodeError::Canceled),
            result = request.send() => result,
        };
        let retry = match &result {
            Ok(response) => matches!(
                response.status(),
                reqwest::StatusCode::REQUEST_TIMEOUT
                    | reqwest::StatusCode::BAD_GATEWAY
                    | reqwest::StatusCode::SERVICE_UNAVAILABLE
                    | reqwest::StatusCode::GATEWAY_TIMEOUT
            ),
            Err(error) => error.is_connect() || error.is_timeout() || error.is_request(),
        };
        if !retry || attempt == RETRY_MAX || template.is_none() {
            return result.map_err(|error| transport_error(stage, &error));
        }

        let delay = retry_delay(attempt);
        tracing::warn!(
            stage,
            attempt = attempt + 1,
            max_attempts = RETRY_MAX + 1,
            delay_ms = delay.as_millis() as u64,
            status = result
                .as_ref()
                .ok()
                .map(|response| response.status().as_u16()),
            "transient LLM HTTP failure; retrying"
        );
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(LitecodeError::Canceled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
    unreachable!("bounded LLM HTTP retry loop")
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn retry_delay_doubles_then_caps() {
        let ms = |n: u64| std::time::Duration::from_millis(n);
        assert_eq!(retry_delay(0), ms(500));
        assert_eq!(retry_delay(1), ms(1_000));
        assert_eq!(retry_delay(2), ms(2_000));
        assert_eq!(retry_delay(3), ms(4_000));
        assert_eq!(retry_delay(4), ms(8_000));
        assert_eq!(retry_delay(5), ms(8_000));
        // No shift overflow far past the retry budget.
        assert_eq!(retry_delay(64), ms(8_000));
        assert_eq!(retry_delay(usize::MAX), ms(8_000));
    }

    #[tokio::test]
    async fn send_cancellable_returns_canceled_when_token_fires() {
        let client = llm_http_client().unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = send_cancellable(
            client.post("http://127.0.0.1:1/never-connected"),
            &cancel,
            "test send",
        )
        .await;
        assert!(matches!(result, Err(LitecodeError::Canceled)));
    }

    #[tokio::test]
    async fn send_cancellable_retries_transient_status_before_stream_starts() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for status in ["503 Service Unavailable", "200 OK"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).await.unwrap();
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let client = llm_http_client().unwrap();
        let response = send_cancellable(
            client
                .post(format!("http://{address}/responses"))
                .body("{}"),
            &CancellationToken::new(),
            "test send",
        )
        .await
        .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn send_cancellable_does_not_retry_explicit_client_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });

        let client = llm_http_client().unwrap();
        let response = send_cancellable(
            client
                .post(format!("http://{address}/responses"))
                .body("{}"),
            &CancellationToken::new(),
            "test send",
        )
        .await
        .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn llm_client_does_not_reuse_idle_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).await.unwrap();
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .unwrap();
            }
        });

        let client = llm_http_client().unwrap();
        let url = format!("http://{address}/");
        client
            .get(&url)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        client
            .get(&url)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();

        server.await.unwrap();
    }
}

/// Preserve useful transport diagnostics without exposing credentials or bodies.
///
/// `reqwest::Error::Display` may include a full request URL. Keep only its
/// origin and path, intentionally discarding query and fragment components.
pub(super) fn transport_error(stage: &str, error: &reqwest::Error) -> LitecodeError {
    use std::error::Error as _;

    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_request() {
        "request"
    } else if error.is_body() {
        "response_body"
    } else if error.is_decode() {
        "decode"
    } else {
        "transport"
    };
    let url = error
        .url()
        .map(|url| {
            let authority = match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().unwrap_or("<unknown-host>")),
                None => url.host_str().unwrap_or("<unknown-host>").to_string(),
            };
            format!("{}://{}{}", url.scheme(), authority, url.path())
        })
        .unwrap_or_else(|| "<unavailable>".to_string());
    let mut causes = Vec::new();
    let mut source = error.source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    let cause = if causes.is_empty() {
        "<unavailable>".to_string()
    } else {
        causes.join(": ")
    };

    tracing::warn!(
        stage,
        kind,
        timeout = error.is_timeout(),
        connect = error.is_connect(),
        request = error.is_request(),
        body = error.is_body(),
        decode = error.is_decode(),
        url,
        cause,
        "LLM HTTP transport failed"
    );
    LitecodeError::Llm(format!(
        "{stage} failed ({kind}; timeout={}; connect={}; request={}; body={}; decode={}; url={url}; cause={cause})",
        error.is_timeout(),
        error.is_connect(),
        error.is_request(),
        error.is_body(),
        error.is_decode(),
    ))
}

fn interrupted_stream_error(
    stage: &str,
    error: &reqwest::Error,
    acc: &stream_contract::StreamItemAccumulator,
) -> LitecodeError {
    let error = transport_error(stage, error);
    if acc.is_empty() {
        return error;
    }
    let LitecodeError::Llm(message) = error else {
        unreachable!("transport_error always returns LitecodeError::Llm")
    };
    LitecodeError::LlmStreamInterrupted {
        message,
        partial: acc.seal_incomplete(),
    }
}

/// Construct a boxed provider from a provider row (adapter_id selects the wire).
pub(super) fn from_definition(def: &ProviderDefinition) -> Result<Box<dyn LlmProvider>> {
    registry::build_client(def)
}

/// Public registry surface for settings API / validation (re-exported via `llm`).
pub mod public {
    pub use super::registry::{
        AdapterDescriptor, FieldSchema, FieldType, adapter_default_capabilities,
        apply_owned_modality_capabilities, catalog_supported_ids, closed_api_model_ids,
        closed_context_windows, closed_default_endpoint, has_remote_model_catalog, is_known_adapter,
        list_adapters, parse_model_config, parse_provider_config, provider_ready,
        validate_model_config, validate_provider_config,
    };
}

pub(crate) use chat_completions::{
    models_get_url as chat_models_url, parse_model_catalog as parse_chat_model_catalog,
};
