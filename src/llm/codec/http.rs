//! Shared HTTP substrate for both codecs: client shape, cancellable send with
//! bounded retry, and transport diagnostics.
//!
//! Codec selection lives in [`super`]; the streaming contract that keeps
//! [`crate::types::StreamEvents`] ordered lives in
//! [`super::stream_contract::forward_stream_event`].

use crate::llm::reconnect::{LlmReconnect, LlmReconnectPhase};
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

/// Attempts already started against the shared connect + empty-stream budget.
pub(super) struct ReconnectBudget {
    used: usize,
}

impl ReconnectBudget {
    pub(super) fn new() -> Self {
        Self { used: 0 }
    }

    #[cfg(test)]
    pub(super) fn with_used(used: usize) -> Self {
        Self { used }
    }

    pub(super) fn max_attempts() -> u32 {
        (RETRY_MAX + 1) as u32
    }

    fn has_room(&self) -> bool {
        self.used <= RETRY_MAX
    }
}

/// What to do after the response body died before any streamed item.
pub(super) enum EmptyStreamAction {
    Retry,
    GiveUp,
}

/// Backoff for a 0-based retry index: `base * 2^attempt`, capped at
/// [`RETRY_MAX_DELAY`] (500ms, 1s, 2s, 4s, 8s, 8s, …).
fn retry_delay(attempt: usize) -> std::time::Duration {
    let factor = 1u32 << attempt.min(16);
    RETRY_BASE_DELAY.saturating_mul(factor).min(RETRY_MAX_DELAY)
}

fn notify(phase: LlmReconnectPhase, attempt: u32, delay_ms: u64) {
    crate::llm::reconnect::emit(LlmReconnect {
        phase,
        attempt,
        max_attempts: ReconnectBudget::max_attempts(),
        delay_ms,
    });
}

/// Send a request while remaining cancellable during connect/headers.
/// Dropping the pending `send()` future aborts the HTTP request.
///
/// Transient failures (connect/timeout/request errors, 408/502/503/504) retry up
/// to [`RETRY_MAX`] times with a doubling backoff; a non-clonable body is sent
/// once. Nothing has reached the model server on these paths, so a retry cannot
/// duplicate a generation. `budget` is shared with a later empty-stream loss so
/// both paths spend the same six attempts.
pub(super) async fn send_cancellable(
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
    stage: &str,
    budget: &mut ReconnectBudget,
) -> Result<reqwest::Response> {
    let template = request.try_clone();
    let mut first = Some(request);
    loop {
        if !budget.has_room() {
            notify(LlmReconnectPhase::Failed, budget.used as u32, 0);
            return Err(LitecodeError::Llm(format!(
                "{stage} failed: reconnect budget exhausted"
            )));
        }

        let attempt_index = budget.used;
        budget.used += 1;
        let attempt_n = budget.used as u32;
        if attempt_index > 0 {
            notify(LlmReconnectPhase::Connecting, attempt_n, 0);
        }

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
            Ok(response) => is_retryable_status(response.status()),
            Err(error) => error.is_connect() || error.is_timeout() || error.is_request(),
        };
        if !retry {
            // Cleared means the reconnect succeeded and the call continues.
            // A non-retryable status or transport error is a failure.
            if attempt_index > 0
                && result
                    .as_ref()
                    .is_ok_and(|response| response.status().is_success())
            {
                notify(LlmReconnectPhase::Cleared, attempt_n, 0);
            }
            return result.map_err(|error| transport_error(stage, &error));
        }
        if !budget.has_room() || template.is_none() {
            notify(LlmReconnectPhase::Failed, budget.used as u32, 0);
            return result.map_err(|error| transport_error(stage, &error));
        }

        let delay = retry_delay(attempt_index);
        tracing::warn!(
            stage,
            attempt = attempt_n,
            max_attempts = ReconnectBudget::max_attempts(),
            delay_ms = delay.as_millis() as u64,
            status = result
                .as_ref()
                .ok()
                .map(|response| response.status().as_u16()),
            "transient LLM HTTP failure; retrying"
        );
        notify(
            LlmReconnectPhase::Waiting,
            attempt_n + 1,
            delay.as_millis() as u64,
        );
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(LitecodeError::Canceled),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

/// Count an empty-body stream loss against the same budget as connect retries.
///
/// A generation that already produced items must not call this: replaying the
/// request would duplicate it. Cancellation does not emit `failed`.
pub(super) async fn on_empty_stream_loss(
    budget: &mut ReconnectBudget,
    cancel: &CancellationToken,
) -> Result<EmptyStreamAction> {
    if !budget.has_room() {
        notify(LlmReconnectPhase::Failed, budget.used as u32, 0);
        return Ok(EmptyStreamAction::GiveUp);
    }
    let delay = retry_delay(budget.used.saturating_sub(1));
    notify(
        LlmReconnectPhase::Waiting,
        (budget.used + 1) as u32,
        delay.as_millis() as u64,
    );
    tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(LitecodeError::Canceled),
        _ = tokio::time::sleep(delay) => {}
    }
    Ok(EmptyStreamAction::Retry)
}

/// A broken event stream: retry while nothing was produced, otherwise surface Retry.
///
/// Replaying a request that already yielded items would duplicate the generation.
pub(super) async fn on_broken_stream(
    budget: &mut ReconnectBudget,
    cancel: &CancellationToken,
    produced_items: bool,
) -> Result<EmptyStreamAction> {
    if produced_items {
        note_terminal_transport_failure(budget);
        return Ok(EmptyStreamAction::GiveUp);
    }
    on_empty_stream_loss(budget, cancel).await
}

/// Stream died after tokens were already produced. Surface Retry immediately.
pub(super) fn note_terminal_transport_failure(budget: &ReconnectBudget) {
    notify(LlmReconnectPhase::Failed, budget.used.max(1) as u32, 0);
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

/// Keep items already streamed when a later line or content type fails.
pub(super) fn preserve_partial(
    error: LitecodeError,
    acc: &super::stream_contract::StreamItemAccumulator,
) -> LitecodeError {
    if acc.is_empty() {
        return error;
    }
    match error {
        LitecodeError::Llm(message) => LitecodeError::LlmStreamInterrupted {
            message,
            partial: acc.seal_incomplete(),
        },
        other => other,
    }
}

pub(super) fn interrupted_stream_error(
    stage: &str,
    error: &reqwest::Error,
    acc: &super::stream_contract::StreamItemAccumulator,
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

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    #[test]
    fn retry_delay_is_the_documented_backoff() {
        assert_eq!(retry_delay(0), std::time::Duration::from_millis(500));
    }

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
            &mut ReconnectBudget::new(),
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
            &mut ReconnectBudget::new(),
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
            &mut ReconnectBudget::new(),
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

    fn collect_notices() -> (
        std::sync::Arc<dyn Fn(crate::llm::reconnect::LlmReconnect) + Send + Sync>,
        std::sync::Arc<std::sync::Mutex<Vec<crate::llm::reconnect::LlmReconnect>>>,
    ) {
        let notices = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink_notices = std::sync::Arc::clone(&notices);
        let sink: std::sync::Arc<dyn Fn(crate::llm::reconnect::LlmReconnect) + Send + Sync> =
            std::sync::Arc::new(move |notice| sink_notices.lock().unwrap().push(notice));
        (sink, notices)
    }

    #[tokio::test]
    async fn first_success_emits_no_reconnect_notice() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let (sink, notices) = collect_notices();
        let client = llm_http_client().unwrap();
        let response = crate::llm::reconnect::scope(sink, async move {
            send_cancellable(
                client
                    .post(format!("http://{address}/responses"))
                    .body("{}"),
                &CancellationToken::new(),
                "test send",
                &mut ReconnectBudget::new(),
            )
            .await
        })
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(notices.lock().unwrap().is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn retry_emits_waiting_then_connecting_then_cleared() {
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
        let (sink, notices) = collect_notices();
        let client = llm_http_client().unwrap();
        let response = crate::llm::reconnect::scope(sink, async move {
            send_cancellable(
                client
                    .post(format!("http://{address}/responses"))
                    .body("{}"),
                &CancellationToken::new(),
                "test send",
                &mut ReconnectBudget::new(),
            )
            .await
        })
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let got = notices.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].phase, LlmReconnectPhase::Waiting);
        assert_eq!(got[0].attempt, 2);
        assert_eq!(got[0].max_attempts, 6);
        assert_eq!(got[0].delay_ms, 500);
        assert_eq!(got[1].phase, LlmReconnectPhase::Connecting);
        assert_eq!(got[1].attempt, 2);
        assert_eq!(got[2].phase, LlmReconnectPhase::Cleared);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn non_retryable_status_after_a_retry_does_not_clear() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for status in ["503 Service Unavailable", "400 Bad Request"] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).await.unwrap();
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let (sink, notices) = collect_notices();
        let client = llm_http_client().unwrap();
        let response = crate::llm::reconnect::scope(sink, async move {
            send_cancellable(
                client
                    .post(format!("http://{address}/responses"))
                    .body("{}"),
                &CancellationToken::new(),
                "test send",
                &mut ReconnectBudget::new(),
            )
            .await
        })
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let got = notices.lock().unwrap().clone();
        assert!(
            got.iter()
                .all(|notice| notice.phase != LlmReconnectPhase::Cleared),
            "a rejected retry must not clear the bubble, got {got:?}"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn exhausted_budget_emits_failed_without_another_wait() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (sink, notices) = collect_notices();
        let client = llm_http_client().unwrap();
        let response = crate::llm::reconnect::scope(sink, async move {
            send_cancellable(
                client
                    .post(format!("http://{address}/responses"))
                    .body("{}"),
                &CancellationToken::new(),
                "test send",
                &mut ReconnectBudget::with_used(RETRY_MAX),
            )
            .await
        })
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let got = notices.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|notice| notice.phase == LlmReconnectPhase::Failed
                    && notice.attempt == 6
                    && notice.max_attempts == 6)
        );
        assert!(
            got.iter()
                .all(|notice| notice.phase != LlmReconnectPhase::Waiting)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_while_waiting_does_not_emit_failed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let cancel = CancellationToken::new();
        let cancel_sleep = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            cancel_sleep.cancel();
        });
        let (sink, notices) = collect_notices();
        let client = llm_http_client().unwrap();
        let result = crate::llm::reconnect::scope(sink, async move {
            send_cancellable(
                client
                    .post(format!("http://{address}/responses"))
                    .body("{}"),
                &cancel,
                "test send",
                &mut ReconnectBudget::new(),
            )
            .await
        })
        .await;
        assert!(matches!(result, Err(LitecodeError::Canceled)));
        let got = notices.lock().unwrap().clone();
        assert!(
            got.iter()
                .any(|notice| notice.phase == LlmReconnectPhase::Waiting)
        );
        assert!(
            got.iter()
                .all(|notice| notice.phase != LlmReconnectPhase::Failed)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn empty_stream_loss_on_a_spent_budget_emits_failed() {
        let (sink, notices) = collect_notices();
        let action = crate::llm::reconnect::scope(sink, async {
            on_empty_stream_loss(
                &mut ReconnectBudget::with_used(RETRY_MAX + 1),
                &CancellationToken::new(),
            )
            .await
        })
        .await
        .unwrap();
        assert!(matches!(action, EmptyStreamAction::GiveUp));
        let got = notices.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].phase, LlmReconnectPhase::Failed);
        assert_eq!(got[0].attempt, 6);
    }
}
