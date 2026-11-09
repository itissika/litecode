//! One process-wide HTTP runtime that owns every LLM connection.
//!
//! A turn runs on a current-thread Tokio runtime that is dropped when the turn
//! ends (`runtime::spawn_turn`). Hyper's connection task belongs to the runtime
//! that polled the request, so a connection opened by one turn cannot serve the
//! next one: its dispatcher died with the turn (`DispatchGone`). That is why
//! pooling had been disabled and every request paid a fresh TCP + TLS handshake.
//!
//! Here the whole request — send, headers, and the streamed body — runs as one
//! task on a runtime no turn owns, and the turn only awaits a channel of events
//! plus the task's result. Dependencies stay one-way: a turn calls into this
//! module and this module never calls back into a turn. A cancelling turn holds
//! the same token the task holds, so the task unwinds on its own.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use tokio::runtime::{Builder, Runtime};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::{AbortHandle, JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::llm::reconnect::{self, LlmReconnect};
use crate::provider_catalog::ResolvedModel;
use crate::types::{Item, LitecodeError, Result, StreamEvents};

/// Worker threads owned by the shared runtime. LLM traffic is a handful of
/// long-lived streaming bodies, not CPU work.
const HTTP_WORKER_THREADS: usize = 2;
/// Idle connections kept per host. Turns for several sessions run at once.
const POOL_MAX_IDLE_PER_HOST: usize = 8;
/// Idle connections older than this are closed even when never reused again.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// TCP-level keepalive so a half-dead link is noticed between requests.
const TCP_KEEPALIVE: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Upper bound for closing the runtime: an idle pooled connection may never
/// hold the process open longer than this.
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);
/// A cancelled call gets this long to unwind with its sealed partial items; the
/// grace only covers a body stuck inside a transport read.
const CANCEL_GRACE: Duration = Duration::from_millis(250);

/// Routing identity of one LLM endpoint.
///
/// A pooled connection belongs to an endpoint, not to a model: two models of one
/// provider share it, and a client built for another endpoint must never be
/// reused. A proxy or TLS policy is part of this identity the moment either
/// exists — it must key on the route, never on the model reference alone.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RouteKey {
    provider: String,
    endpoint: String,
}

impl RouteKey {
    pub(crate) fn of(model: &ResolvedModel) -> Self {
        Self {
            provider: model.provider_id.clone(),
            endpoint: model.request_url.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new(provider: &str, endpoint: &str) -> Self {
        Self {
            provider: provider.to_string(),
            endpoint: endpoint.to_string(),
        }
    }
}

/// What the HTTP task reports back to its turn, in stream order.
enum HttpNotice {
    Event(StreamEvents),
    Reconnect(LlmReconnect),
}

/// The runtime and the connection pools it drives.
pub(crate) struct HttpRuntime {
    /// `None` once shut down: a late request fails instead of rebuilding.
    runtime: Mutex<Option<Runtime>>,
    clients: Mutex<HashMap<RouteKey, Client>>,
}

impl HttpRuntime {
    fn new() -> Result<Self> {
        Ok(Self {
            runtime: Mutex::new(Some(build_runtime()?)),
            clients: Mutex::new(HashMap::new()),
        })
    }

    /// The client for one route, created on first use.
    ///
    /// Credentials are never part of this cache: they are attached per request.
    fn client(&self, route: &RouteKey) -> Result<Client> {
        let mut clients = self
            .clients
            .lock()
            .map_err(|_| LitecodeError::Llm("the LLM HTTP client cache is poisoned".into()))?;
        if let Some(client) = clients.get(route) {
            return Ok(client.clone());
        }
        let client = Client::builder()
            .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .tcp_keepalive(TCP_KEEPALIVE)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()?;
        clients.insert(route.clone(), client.clone());
        Ok(client)
    }

    fn spawn<F>(&self, work: F) -> Result<JoinHandle<Result<Vec<Item>>>>
    where
        F: Future<Output = Result<Vec<Item>>> + Send + 'static,
    {
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| LitecodeError::Llm("the LLM HTTP runtime is poisoned".into()))?;
        let runtime = runtime
            .as_ref()
            .ok_or_else(|| LitecodeError::Llm("the LLM HTTP runtime is shut down".to_string()))?;
        Ok(runtime.spawn(work))
    }

    /// Run one whole model call on the shared runtime.
    ///
    /// `work` gets the client for `route`, a sink for stream events, and the
    /// cancellation token. It must own everything it touches: it is `'static`
    /// because it outlives the turn's stack.
    pub(crate) async fn complete<'a, F, Fut>(
        &self,
        route: &RouteKey,
        on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &CancellationToken,
        work: F,
    ) -> Result<Vec<Item>>
    where
        F: FnOnce(Client, Option<Box<dyn FnMut(StreamEvents) + Send>>, CancellationToken) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<Vec<Item>>> + Send,
    {
        let client = self.client(route)?;
        // The sink is a synchronous callback, so the task cannot apply channel
        // backpressure; the turn drains in-line and never waits on anything but
        // this channel, so the two stay within one event of each other.
        let (tx, notices) = unbounded_channel::<HttpNotice>();
        let events = tx.clone();
        let token = cancel.clone();
        let task = self.spawn(async move {
            let sink: Box<dyn FnMut(StreamEvents) + Send> = Box::new(move |event| {
                let _ = events.send(HttpNotice::Event(event));
            });
            // A turn installs a task-local sink for reconnect notices; a task on
            // another runtime cannot see it, so forward them over the channel.
            let forward: Arc<dyn Fn(LlmReconnect) + Send + Sync> = Arc::new(move |notice| {
                let _ = tx.send(HttpNotice::Reconnect(notice));
            });
            reconnect::scope(forward, work(client, Some(sink), token)).await
        })?;
        drive(task, notices, on_event, cancel).await
    }

    /// Close the pool and the runtime, waiting no longer than
    /// [`SHUTDOWN_BUDGET`].
    pub(crate) fn shutdown(&self) {
        let runtime = self.runtime.lock().ok().and_then(|mut slot| slot.take());
        let Some(runtime) = runtime else {
            return;
        };
        // Drop the clients first so every idle pooled connection is closed
        // before anyone waits on the runtime driving them.
        if let Ok(mut clients) = self.clients.lock() {
            clients.clear();
        }
        let started = Instant::now();
        runtime.shutdown_timeout(SHUTDOWN_BUDGET);
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            budget_ms = SHUTDOWN_BUDGET.as_millis() as u64,
            "closed the LLM HTTP runtime"
        );
    }
}

/// Build the runtime on a plain thread: building a runtime from inside one is a
/// panic, and the first model call always arrives from a turn runtime.
fn build_runtime() -> Result<Runtime> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("litecode-http-init".to_string())
        .spawn(move || {
            let built = Builder::new_multi_thread()
                .worker_threads(HTTP_WORKER_THREADS)
                .thread_name("litecode-http")
                .enable_all()
                .build();
            let _ = tx.send(built.map_err(|error| error.to_string()));
        })
        .map_err(|error| {
            LitecodeError::Llm(format!("cannot start the LLM HTTP runtime: {error}"))
        })?;
    match rx.recv() {
        Ok(Ok(runtime)) => Ok(runtime),
        Ok(Err(message)) => Err(LitecodeError::Llm(format!(
            "cannot build the LLM HTTP runtime: {message}"
        ))),
        Err(_) => Err(LitecodeError::Llm(
            "the LLM HTTP runtime init thread died".to_string(),
        )),
    }
}

/// Abort the HTTP task if the turn drops the call instead of awaiting it.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Forward the HTTP task's notices to the turn until the task closes the channel.
async fn forward<'a>(
    notices: &mut UnboundedReceiver<HttpNotice>,
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
) {
    while let Some(notice) = notices.recv().await {
        match notice {
            HttpNotice::Event(event) => {
                if let Some(on_event) = on_event.as_mut() {
                    on_event(event);
                }
            }
            HttpNotice::Reconnect(notice) => reconnect::emit(notice),
        }
    }
}

fn join(joined: std::result::Result<Result<Vec<Item>>, JoinError>) -> Result<Vec<Item>> {
    match joined {
        Ok(result) => result,
        Err(error) if error.is_cancelled() => Err(LitecodeError::Canceled),
        Err(error) => Err(LitecodeError::Llm(format!(
            "the LLM HTTP task failed: {error}"
        ))),
    }
}

/// Drive one call from the turn side: events in order, cancellation immediate.
async fn drive<'a>(
    mut task: JoinHandle<Result<Vec<Item>>>,
    mut notices: UnboundedReceiver<HttpNotice>,
    mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
    cancel: &CancellationToken,
) -> Result<Vec<Item>> {
    let _abort = AbortOnDrop(task.abort_handle());
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            // The task holds the same token and unwinds with its sealed items;
            // wait for that, but never without a bound.
            if let Ok(joined) = tokio::time::timeout(CANCEL_GRACE, &mut task).await {
                return join(joined);
            }
            task.abort();
            return Err(LitecodeError::Canceled);
        }
        () = forward(&mut notices, &mut on_event) => {}
    }
    join(task.await)
}

static GLOBAL: OnceLock<Arc<HttpRuntime>> = OnceLock::new();
static CLOSED: AtomicBool = AtomicBool::new(false);
/// Serialises the first build so a losing racer never drops a runtime inside
/// an async context (Tokio forbids that).
static BUILD: Mutex<()> = Mutex::new(());

/// The process-wide runtime every model call goes through.
pub(crate) fn global() -> Result<Arc<HttpRuntime>> {
    if CLOSED.load(Ordering::Acquire) {
        return Err(LitecodeError::Llm(
            "the LLM HTTP runtime is shut down".to_string(),
        ));
    }
    if let Some(runtime) = GLOBAL.get() {
        return Ok(Arc::clone(runtime));
    }
    let _building = BUILD
        .lock()
        .map_err(|_| LitecodeError::Llm("the LLM HTTP runtime build is poisoned".into()))?;
    if let Some(runtime) = GLOBAL.get() {
        return Ok(Arc::clone(runtime));
    }
    let built = Arc::new(HttpRuntime::new()?);
    let _ = GLOBAL.set(Arc::clone(&built));
    Ok(built)
}

/// Close the process-wide runtime. Called once on the way out of `main` so an
/// idle pooled connection can never hold the process open; bounded by
/// [`SHUTDOWN_BUDGET`]. A no-op when no model call ever ran.
pub fn shutdown() {
    CLOSED.store(true, Ordering::Release);
    if let Some(runtime) = GLOBAL.get() {
        runtime.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::llm::ModelRequest;
    use crate::llm::codec::chat::ChatCompletionsCodec;
    use crate::llm::provider::LlmProvider;
    use crate::llm::reconnect::{self, LlmReconnectPhase};
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::user_text;

    /// A blocking HTTP server that counts TCP accepts.
    ///
    /// It answers every request on a connection and leaves the connection open,
    /// so a client that reuses its pooled connection shows up as one accept and
    /// several requests.
    struct TestServer {
        address: SocketAddr,
        accepts: Arc<AtomicUsize>,
        requests: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl TestServer {
        fn start<F>(respond: F) -> Self
        where
            F: Fn(&mut TcpStream, usize) + Send + Sync + 'static,
        {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("address");
            let accepts = Arc::new(AtomicUsize::new(0));
            let requests = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let respond = Arc::new(respond);
            let loop_accepts = Arc::clone(&accepts);
            let loop_requests = Arc::clone(&requests);
            let loop_stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for connection in listener.incoming() {
                    if loop_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut stream) = connection else {
                        continue;
                    };
                    loop_accepts.fetch_add(1, Ordering::AcqRel);
                    let respond = Arc::clone(&respond);
                    let requests = Arc::clone(&loop_requests);
                    std::thread::spawn(move || {
                        let mut index = 0;
                        while read_request(&mut stream) {
                            respond(&mut stream, index);
                            requests.fetch_add(1, Ordering::AcqRel);
                            index += 1;
                        }
                    });
                }
            });
            Self {
                address,
                accepts,
                requests,
                stop,
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            let _ = TcpStream::connect(self.address);
        }
    }

    /// Read one request head plus its body. False on a closed connection.
    fn read_request(stream: &mut TcpStream) -> bool {
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => return false,
                Ok(_) => head.push(byte[0]),
            }
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
        let length = text
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if length > 0 {
            let mut body = vec![0_u8; length];
            if stream.read_exact(&mut body).is_err() {
                return false;
            }
        }
        true
    }

    fn sse(stream: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }

    fn status(stream: &mut TcpStream, status: &str) {
        let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n");
        let _ = stream.write_all(response.as_bytes());
    }

    fn chat_chunk(text: &str) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "c1",
                "object": "chat.completion.chunk",
                "model": "m",
                "choices": [{ "index": 0, "delta": { "content": text } }]
            })
        )
    }

    fn chat_codec(address: SocketAddr) -> ChatCompletionsCodec {
        let text = format!(
            "version = 1\n[[providers]]\nid = \"zen\"\nname = \"Zen\"\nendpoint = \"http://{address}/v1\"\nendpoint_type = \"chat_completions\"\nauth = \"bearer\"\n\n[[models]]\nid = \"m\"\nprovider_id = \"zen\"\n"
        );
        let catalog =
            ProviderCatalog::parse(&text, Path::new("http-runtime-test.toml")).expect("catalog");
        ChatCompletionsCodec::new(Arc::clone(catalog.model("zen/m").expect("model")))
    }

    fn request() -> ModelRequest {
        ModelRequest {
            model: "m".into(),
            instructions: "sys".into(),
            input: vec![user_text("hi")],
            tools: vec![],
            max_output_tokens: 64,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: Some("ses_http".into()),
            purpose: crate::llm::RequestPurpose::Main,
        }
    }

    fn turn() -> Runtime {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("turn runtime")
    }

    fn text_of(items: &[crate::types::Item]) -> String {
        crate::types::item_text_preview(&items[0]).to_string()
    }

    #[test]
    fn a_pooled_connection_outlives_the_turn_runtime_that_opened_it() {
        let body = format!("{}data: [DONE]\n\n", chat_chunk("hi"));
        let server = TestServer::start(move |stream, _| sse(stream, &body));
        let codec = chat_codec(server.address);
        let cancel = CancellationToken::new();

        let first = turn()
            .block_on(codec.complete_with_stream_events(&request(), "sk-test", None, &cancel))
            .expect("first call");
        assert_eq!(text_of(&first), "hi");
        // The turn runtime dies here; the connection must not.
        let second = turn()
            .block_on(codec.complete_with_stream_events(&request(), "sk-test", None, &cancel))
            .expect("second call");
        assert_eq!(text_of(&second), "hi");

        assert_eq!(
            server.accepts.load(Ordering::Acquire),
            1,
            "the second turn must reuse the connection the first one opened"
        );
        assert_eq!(server.requests.load(Ordering::Acquire), 2);
    }

    #[test]
    fn cancelling_mid_stream_returns_at_once_and_keeps_delta_order() {
        let server = TestServer::start(|stream, _| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
            );
            for text in ["a", "b", "c"] {
                let chunk = chat_chunk(text);
                let framed = format!("{:x}\r\n{chunk}\r\n", chunk.len());
                let _ = stream.write_all(framed.as_bytes());
                let _ = stream.flush();
            }
            // The vendor keeps the stream open; the turn must not wait for it.
            std::thread::sleep(Duration::from_secs(5));
        });
        let codec = chat_codec(server.address);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            trigger.cancel();
        });

        let started = Instant::now();
        let items = turn()
            .block_on(codec.complete_with_stream_events(&request(), "sk-test", None, &cancel))
            .expect("a cancelled call keeps its sealed items");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cancellation must not wait for the server: {:?}",
            started.elapsed()
        );
        assert_eq!(text_of(&items), "abc", "deltas must arrive in order");
    }

    #[test]
    fn reconnect_notices_reach_the_turns_task_local_scope() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let ok_body = format!("{}data: [DONE]\n\n", chat_chunk("hi"));
        let server = TestServer::start(move |stream, _| {
            if counter.fetch_add(1, Ordering::AcqRel) == 0 {
                status(stream, "503 Service Unavailable");
            } else {
                sse(stream, &ok_body);
            }
        });
        let codec = chat_codec(server.address);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        let sink: Arc<dyn Fn(LlmReconnect) + Send + Sync> =
            Arc::new(move |notice| sink_seen.lock().unwrap().push(notice));

        let items = turn()
            .block_on(reconnect::scope(sink, async {
                codec
                    .complete_with_stream_events(
                        &request(),
                        "sk-test",
                        None,
                        &CancellationToken::new(),
                    )
                    .await
            }))
            .expect("the retry succeeds");
        assert_eq!(text_of(&items), "hi");

        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter()
                .any(|notice| notice.phase == LlmReconnectPhase::Waiting),
            "{seen:?}"
        );
        assert!(
            seen.iter()
                .any(|notice| notice.phase == LlmReconnectPhase::Cleared),
            "{seen:?}"
        );
    }

    #[test]
    fn a_changed_route_never_reuses_the_old_client() {
        let body = format!("{}data: [DONE]\n\n", chat_chunk("hi"));
        let server = TestServer::start(move |stream, _| sse(stream, &body));
        let runtime = HttpRuntime::new().expect("runtime");
        let url = format!("http://{}/v1", server.address);
        let cancel = CancellationToken::new();
        for provider in ["zen", "other"] {
            let route = RouteKey::new(provider, &url);
            let url = url.clone();
            turn()
                .block_on(runtime.complete(
                    &route,
                    None,
                    &cancel,
                    move |client, _sink, _token| async move {
                        let response = client
                            .get(&url)
                            .send()
                            .await
                            .map_err(|error| LitecodeError::Llm(error.to_string()))?;
                        let _ = response.text().await;
                        Ok(Vec::new())
                    },
                ))
                .expect("request");
        }
        assert_eq!(
            server.accepts.load(Ordering::Acquire),
            2,
            "a different route must not inherit the first route's client"
        );
    }

    #[test]
    fn shutdown_closes_an_idle_pool_and_returns_within_the_budget() {
        let body = format!("{}data: [DONE]\n\n", chat_chunk("hi"));
        let server = TestServer::start(move |stream, _| sse(stream, &body));
        let runtime = HttpRuntime::new().expect("runtime");
        let url = format!("http://{}/v1", server.address);
        let route = RouteKey::new("zen", &url);
        let cancel = CancellationToken::new();
        for _ in 0..2 {
            let url = url.clone();
            turn()
                .block_on(runtime.complete(
                    &route,
                    None,
                    &cancel,
                    move |client, _sink, _token| async move {
                        let response = client
                            .get(&url)
                            .send()
                            .await
                            .map_err(|error| LitecodeError::Llm(error.to_string()))?;
                        let _ = response.text().await;
                        Ok(Vec::new())
                    },
                ))
                .expect("request");
        }
        assert_eq!(server.accepts.load(Ordering::Acquire), 1);

        let started = Instant::now();
        runtime.shutdown();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "shutdown with an idle pooled connection must be bounded: {:?}",
            started.elapsed()
        );

        let late = turn().block_on(runtime.complete(
            &route,
            None,
            &cancel,
            move |_client, _sink, _token| async move { Ok(Vec::new()) },
        ));
        assert!(late.is_err(), "a shut-down runtime must refuse new work");
    }
}
