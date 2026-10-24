//! Chat Completions codec - one conversion pass into the authority shape.
//!
//! Provider differences arrive as catalog data: tiers, headers, extra_body,
//! `stream_usage`, `reasoning.key`. Nothing here reads a provider id.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{FunctionCallOutput, Item, MessageItem};
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::platform_knobs::{ThinkingSpec, ThinkingTier};
use crate::provider_catalog::ResolvedModel;
use crate::types::{LitecodeError, Result, StreamEvents};

use super::chat_synth::ChatSynth;
use super::http::{
    EmptyStreamAction, ReconnectBudget, llm_http_client, on_broken_stream, preserve_partial,
    send_cancellable,
};
use super::sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::{apply_auth, error_prefix, render_headers, user_agent};

pub(crate) struct ChatCompletionsCodec {
    client: Client,
    model: Arc<ResolvedModel>,
}

impl ChatCompletionsCodec {
    pub(crate) fn new(model: Arc<ResolvedModel>) -> Result<Self> {
        Ok(Self {
            client: llm_http_client()?,
            model,
        })
    }

    /// Vendor effort literal for this platform intent, if the model takes one.
    fn effort(&self, thinking: ThinkingSpec) -> Option<&str> {
        let tiers = self.model.reasoning.as_ref()?;
        match thinking {
            // Thinking off sends the catalog's own off literal when declared;
            // without one the whole control is omitted.
            ThinkingSpec::Off => self.model.reasoning_off.as_deref(),
            ThinkingSpec::Tier(ThinkingTier::Low) => Some(tiers.low.as_str()),
            ThinkingSpec::Tier(ThinkingTier::Medium) => Some(tiers.medium.as_str()),
            ThinkingSpec::Tier(ThinkingTier::High) => Some(tiers.high.as_str()),
        }
    }

    fn encode_body(&self, params: &ModelRequest, stream: bool) -> Result<Value> {
        let model = &self.model;
        let mut messages: Vec<Value> = Vec::new();
        if !params.instructions.trim().is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": params.instructions,
            }));
        }

        let input = &params.input;
        let reasoning_key = model.reasoning_key.as_str();
        // Tools and reasoning both mean the vendor reasons about this turn, so
        // the key is written on every assistant message of the replay.
        let replay_reasoning =
            !params.tools.is_empty() || input.iter().any(|item| matches!(item, Item::Reasoning(_)));
        let mut turn = AssistantTurn::default();

        for item in input {
            match item {
                Item::Reasoning(_) => {
                    let text = crate::types::item_text_preview(item);
                    if !text.is_empty() {
                        turn.push_reasoning(&text);
                    }
                }
                Item::FunctionCall(call) => {
                    turn.tool_calls.push(serde_json::json!({
                        "id": call.call_id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments,
                        }
                    }));
                }
                Item::FunctionCallOutput(output) => {
                    turn.flush(&mut messages, reasoning_key, replay_reasoning);
                    let content = match &output.output {
                        FunctionCallOutput::Text(text) => text.clone(),
                        FunctionCallOutput::Content(_) => crate::types::item_text_preview(item),
                    };
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": output.call_id,
                        "content": content,
                    }));
                }
                Item::Message(MessageItem::Input(_)) => {
                    turn.flush(&mut messages, reasoning_key, replay_reasoning);
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": crate::types::item_text_preview(item),
                    }));
                }
                Item::Message(MessageItem::Output(_)) => {
                    let text = crate::types::item_text_preview(item);
                    match &mut turn.content {
                        Some(existing) => {
                            if !existing.is_empty() && !text.is_empty() {
                                existing.push('\n');
                            }
                            existing.push_str(&text);
                        }
                        None => turn.content = Some(text),
                    }
                }
                _ => {}
            }
        }
        turn.flush(&mut messages, reasoning_key, replay_reasoning);

        let tools: Vec<Value> = params
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    }
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": params.model,
            "messages": messages,
            "stream": stream,
        });
        if params.max_output_tokens > 0 {
            body["max_tokens"] = Value::from(params.max_output_tokens);
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if stream && model.stream_usage {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }
        if let Some(effort) = self.effort(params.thinking) {
            body["reasoning_effort"] = Value::String(effort.to_string());
        }
        // JSON mode is per-request intent gated by a per-model capability.
        if params.json_output && model.json_output {
            body["response_format"] = serde_json::json!({ "type": "json_object" });
        }
        if let Value::Object(map) = &mut body {
            for (key, value) in &model.extra_body {
                map.insert(key.clone(), value.clone());
            }
        }
        Ok(body)
    }

    fn request(
        &self,
        body: &Value,
        api_key: &str,
        session_id: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .post(&self.model.request_url)
            .header("accept", "text/event-stream")
            .header("user-agent", user_agent());
        builder = apply_auth(builder, self.model.auth, api_key);
        for (name, value) in render_headers(&self.model.headers, session_id) {
            builder = builder.header(name, value);
        }
        builder.json(body)
    }
}

impl LlmProvider for ChatCompletionsCodec {
    // The trait names this "endpoint"; the catalog calls the same thing the
    // request URL (base + protocol path).
    #[allow(clippy::misnamed_getters)]
    fn endpoint(&self) -> &str {
        &self.model.request_url
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(Self {
            client: self.client.clone(),
            model: Arc::clone(&self.model),
        })
    }

    fn clone_for_isolated_runtime(&self) -> Box<dyn LlmProvider> {
        match Self::new(Arc::clone(&self.model)) {
            Ok(codec) => Box::new(codec),
            Err(_) => self.box_clone(),
        }
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = self.encode_body(request, true)?;
            let dump = super::wire_dump::Capture::start(
                "chat",
                &self.model.request_url,
                request.session_id.as_deref(),
                &body,
            );
            let prefix = error_prefix(&self.model);
            let model = request.model.clone();
            let ingest = |value: &Value,
                          synth: &mut ChatSynth,
                          gate: &mut StreamContractGate,
                          acc: &mut StreamItemAccumulator,
                          on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>|
             -> Result<Option<Vec<Item>>> {
                let mut events = Vec::new();
                synth.ingest_chunk(value, &mut events);
                let mut last = None;
                for event in events {
                    if let Some(items) = forward_stream_event(gate, acc, event, on_event)? {
                        last = Some(items);
                    }
                }
                Ok(last)
            };

            let mut budget = ReconnectBudget::new();
            'open: loop {
                let resp = send_cancellable(
                    self.request(&body, api_key, request.session_id.as_deref()),
                    cancel,
                    "opening Chat Completions event stream",
                    &mut budget,
                )
                .await?;
                if let Some(dump) = &dump {
                    dump.status(resp.status().as_u16());
                }

                if !resp.status().is_success() {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if let Some(dump) = &dump {
                        dump.error_body(&text);
                    }
                    return Err(LitecodeError::Llm(format!(
                        "{prefix}: HTTP {status}: {text}"
                    )));
                }
                let resp = match check_event_stream_content_type(resp).await {
                    Ok(resp) => resp,
                    Err(error) => match on_broken_stream(&mut budget, cancel, false).await? {
                        EmptyStreamAction::Retry => continue 'open,
                        EmptyStreamAction::GiveUp => return Err(error),
                    },
                };

                let mut terminal_items: Option<Vec<Item>> = None;
                let mut reader = SseLineReader::new();
                let mut stream = resp.bytes_stream();
                let mut gate = StreamContractGate::new();
                let mut acc = StreamItemAccumulator::new();
                let mut synth = ChatSynth::new();
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
                            let chunk = match chunk {
                                Ok(chunk) => chunk,
                                Err(error) => {
                                    let failure = super::http::interrupted_stream_error(
                                        "reading Chat Completions event stream",
                                        &error,
                                        &acc,
                                    );
                                    match on_broken_stream(&mut budget, cancel, !acc.is_empty())
                                        .await?
                                    {
                                        EmptyStreamAction::Retry => continue 'open,
                                        EmptyStreamAction::GiveUp => return Err(preserve_partial(failure, &acc)),
                                    }
                                }
                            };
                            let lines = match reader.feed(&chunk) {
                                Ok(lines) => lines,
                                Err(error) => {
                                    match on_broken_stream(&mut budget, cancel, !acc.is_empty())
                                        .await?
                                    {
                                        EmptyStreamAction::Retry => continue 'open,
                                        EmptyStreamAction::GiveUp => return Err(preserve_partial(error, &acc)),
                                    }
                                }
                            };
                            for line in lines {
                                if let Some(dump) = &dump {
                                    dump.line(&line);
                                }
                                let Some(data) = sse_data_payload(&line) else {
                                    continue;
                                };
                                if data.trim() == "[DONE]" {
                                    continue;
                                }
                                let value: Value = match serde_json::from_str(data) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        let error = LitecodeError::Llm(format!(
                                            "{prefix}: Chat SSE JSON: {error}; payload={data}"
                                        ));
                                        match on_broken_stream(
                                            &mut budget,
                                            cancel,
                                            !acc.is_empty(),
                                        )
                                        .await?
                                        {
                                            EmptyStreamAction::Retry => continue 'open,
                                            EmptyStreamAction::GiveUp => return Err(preserve_partial(error, &acc)),
                                        }
                                    }
                                };
                                if let Some(items) = ingest(&value, &mut synth, &mut gate, &mut acc, &mut on_event)? {
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
                    let tail = match reader.finish() {
                        Ok(line) => line,
                        Err(error) => {
                            match on_broken_stream(&mut budget, cancel, !acc.is_empty()).await? {
                                EmptyStreamAction::Retry => continue 'open,
                                EmptyStreamAction::GiveUp => {
                                    return Err(preserve_partial(error, &acc));
                                }
                            }
                        }
                    };
                    if let Some(line) = tail {
                        if let Some(dump) = &dump {
                            dump.line(&line);
                        }
                        if let Some(data) = sse_data_payload(&line)
                            && data.trim() != "[DONE]"
                        {
                            let value: Value = match serde_json::from_str(data) {
                                Ok(value) => value,
                                Err(error) => {
                                    let error = LitecodeError::Llm(format!(
                                        "{prefix}: Chat SSE JSON: {error}; payload={data}"
                                    ));
                                    match on_broken_stream(&mut budget, cancel, !acc.is_empty())
                                        .await?
                                    {
                                        EmptyStreamAction::Retry => continue 'open,
                                        EmptyStreamAction::GiveUp => {
                                            return Err(preserve_partial(error, &acc));
                                        }
                                    }
                                }
                            };
                            if let Some(items) =
                                ingest(&value, &mut synth, &mut gate, &mut acc, &mut on_event)?
                            {
                                terminal_items = Some(items);
                            }
                        }
                    }
                    if terminal_items.is_none() {
                        // Chat chunks never carry a Responses terminal event.
                        let mut events = synth.finish_events(&model)?;
                        let mut last = None;
                        for event in events.drain(..) {
                            if let Some(items) =
                                forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                            {
                                last = Some(items);
                            }
                        }
                        if last.is_some() {
                            terminal_items = last;
                        }
                    }
                }

                if let Some(report) = synth.seam_report() {
                    // The vendor ended a thinking block mid-token and streamed the
                    // rest as `content`. Items stay a faithful copy of the stream;
                    // this line is the evidence trail for a vendor report.
                    tracing::warn!(
                        session = request.session_id.as_deref().unwrap_or_default(),
                        model = %model,
                        glued = report.glued,
                        reasoning_resumed = report.resumed,
                        samples = ?report.samples,
                        "chat stream split reasoning and content mid-stream"
                    );
                }

                if cancelled {
                    return resolve_stream_outcome(terminal_items, &acc, true);
                }
                match resolve_stream_outcome(terminal_items, &acc, false) {
                    Ok(items) => return Ok(items),
                    Err(error) if acc.is_empty() => {
                        match on_broken_stream(&mut budget, cancel, false).await? {
                            EmptyStreamAction::Retry => continue 'open,
                            EmptyStreamAction::GiveUp => return Err(preserve_partial(error, &acc)),
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        })
    }
}

#[derive(Default)]
struct AssistantTurn {
    reasoning: String,
    content: Option<String>,
    tool_calls: Vec<Value>,
}

impl AssistantTurn {
    fn push_reasoning(&mut self, text: &str) {
        if !self.reasoning.is_empty() {
            self.reasoning.push('\n');
        }
        self.reasoning.push_str(text);
    }

    fn is_empty(&self) -> bool {
        self.reasoning.is_empty() && self.content.is_none() && self.tool_calls.is_empty()
    }

    fn flush(&mut self, messages: &mut Vec<Value>, reasoning_key: &str, replay_reasoning: bool) {
        if self.is_empty() {
            return;
        }
        let mut object = Map::new();
        object.insert("role".into(), Value::String("assistant".into()));
        if !self.tool_calls.is_empty() {
            let content = match &self.content {
                Some(text) if !text.is_empty() => Value::String(text.clone()),
                _ => Value::Null,
            };
            object.insert("content".into(), content);
            object.insert(
                "tool_calls".into(),
                Value::Array(std::mem::take(&mut self.tool_calls)),
            );
        } else {
            object.insert(
                "content".into(),
                Value::String(self.content.take().unwrap_or_default()),
            );
        }
        if replay_reasoning || !self.reasoning.is_empty() {
            object.insert(
                reasoning_key.to_string(),
                Value::String(std::mem::take(&mut self.reasoning)),
            );
        }
        self.content = None;
        messages.push(Value::Object(object));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{OutputMessage, OutputMessageContent, ResponseStreamEvent};
    use crate::llm::request::ToolDef;
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::user_text;
    use std::path::Path;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const PROVIDER: &str = r#"
version = 1
[[providers]]
id = "zen"
name = "OpenCode Zen"
endpoint = "ENDPOINT"
endpoint_type = "chat_completions"
auth = "bearer"
headers = { "x-opencode-session" = "{{session_id}}" }
"#;

    fn codec(entry: &str) -> ChatCompletionsCodec {
        let text = format!(
            "{}\n[[models]]\nid = \"m\"\nprovider_id = \"zen\"\n{entry}\n",
            PROVIDER.replace("ENDPOINT", "https://opencode.ai/zen/v1")
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("t.toml")).unwrap();
        ChatCompletionsCodec::new(Arc::clone(catalog.model("zen/m").unwrap())).unwrap()
    }

    fn tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: "d".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "m".into(),
            instructions: "sys".into(),
            input: vec![user_text("hi")],
            tools: vec![],
            max_output_tokens: 64,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: Some("ses_1".into()),
            purpose: crate::llm::RequestPurpose::Main,
        }
    }

    #[test]
    fn checkpoint_serializes_as_a_user_message() {
        let checkpoint = crate::session::working::CompactionCheckpoint {
            summary: "## Primary Request and Intent\n- ship checkpoint identity".into(),
            from: 1,
            to: 2,
        };
        let mut request = sample_request();
        request.input = vec![checkpoint.model_item()];
        let body = codec("").encode_body(&request, true).unwrap();
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages[1]["role"], "user", "{body}");
        let text = messages[1]["content"].as_str().expect("user content");
        assert_eq!(text.matches("<compacted-summary>").count(), 1, "{body}");
        assert!(text.contains("ship checkpoint identity"));
    }

    #[test]
    fn tools_and_reasoning_replay_into_one_assistant_turn() {
        use crate::authority::responses::{
            AssistantRole, FunctionToolCall, OutputStatus, ReasoningItem, ReasoningItemContent,
            ReasoningTextContent,
        };

        let mut request = sample_request();
        request.tools = vec![tool("read")];
        request.input = vec![
            Item::Reasoning(ReasoningItem {
                id: Some("rs_1".into()),
                summary: vec![],
                content: Some(vec![ReasoningItemContent::ReasoningText(
                    ReasoningTextContent {
                        text: "think".into(),
                    },
                )]),
                encrypted_content: None,
                status: Some(OutputStatus::Completed),
            }),
            Item::Message(MessageItem::Output(OutputMessage {
                id: "msg_1".into(),
                role: AssistantRole::Assistant,
                status: OutputStatus::Completed,
                phase: None,
                content: vec![OutputMessageContent::OutputText(
                    crate::authority::responses::OutputTextContent {
                        text: "hello".into(),
                        annotations: vec![],
                        logprobs: None,
                    },
                )],
            })),
            Item::FunctionCall(FunctionToolCall {
                id: Some("fc_1".into()),
                call_id: "call_1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                status: Some(OutputStatus::Completed),
                namespace: None,
            }),
            Item::FunctionCallOutput(crate::authority::responses::FunctionCallOutputItemParam {
                call_id: "call_1".into(),
                output: FunctionCallOutput::Text("ok".into()),
                id: None,
                status: None,
            }),
        ];

        let body = codec("").encode_body(&request, true).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "hello");
        assert_eq!(messages[1]["reasoning_content"], "think");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["content"], "ok");
        assert_eq!(body["tools"][0]["function"]["name"], "read");
    }

    /// Regression guard: the model's own earlier reasoning goes back as its
    /// reasoning text, whatever id the item carries.
    #[test]
    fn recorded_reasoning_is_replayed_as_reasoning_content() {
        use crate::authority::responses::{
            FunctionCallOutputItemParam, FunctionToolCall, ReasoningItem, ReasoningItemContent,
            ReasoningTextContent,
        };
        let mut request = sample_request();
        request.tools = vec![tool("read")];
        request.input = vec![
            user_text("hi"),
            Item::Reasoning(ReasoningItem {
                id: Some("cc_rs_b0dd1e2387cd43649a76f14a949f06a9".into()),
                summary: vec![],
                content: Some(vec![ReasoningItemContent::ReasoningText(
                    ReasoningTextContent {
                        text: "my own thinking".into(),
                    },
                )]),
                encrypted_content: None,
                status: None,
            }),
            Item::FunctionCall(FunctionToolCall {
                id: Some("cc_fc_1".into()),
                call_id: "call_1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                status: None,
                namespace: None,
            }),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                id: None,
                call_id: "call_1".into(),
                output: FunctionCallOutput::Text("ok".into()),
                status: None,
            }),
        ];
        let body = codec("").encode_body(&request, true).unwrap();
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(
            messages[2]["reasoning_content"], "my own thinking",
            "{body}"
        );
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
    }

    // ── streaming ────────────────────────────────────────────────────────────

    async fn serve_once(body: String, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let content_type = content_type.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        address.to_string()
    }

    fn local_codec(address: &str) -> ChatCompletionsCodec {
        let text = format!(
            "{}\n[[models]]\nid = \"m\"\nprovider_id = \"zen\"\n",
            PROVIDER.replace("ENDPOINT", &format!("http://{address}/v1"))
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("t.toml")).unwrap();
        ChatCompletionsCodec::new(Arc::clone(catalog.model("zen/m").unwrap())).unwrap()
    }

    fn chunk(delta: Value) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({
                "id": "c1",
                "object": "chat.completion.chunk",
                "model": "m",
                "choices": [{ "index": 0, "delta": delta }]
            })
        )
    }

    #[tokio::test]
    async fn stream_text_only_becomes_an_assistant_message() {
        let body = format!(
            "{}data: [DONE]\n\n",
            chunk(serde_json::json!({"content": "hi"}))
        );
        let address = serve_once(body, "text/event-stream").await;
        let items = local_codec(&address)
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("stream");
        assert_eq!(items.len(), 1);
        match &items[0] {
            Item::Message(MessageItem::Output(message)) => {
                assert_eq!(crate::types::item_text_preview(&items[0]), "hi");
                assert_eq!(
                    message.role,
                    crate::authority::responses::AssistantRole::Assistant
                );
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_tool_call_opens_the_item_before_the_arguments_delta() {
        let first = chunk(serde_json::json!({
            "tool_calls": [{ "index": 0, "id": "call_1", "function": { "name": "read", "arguments": "" } }]
        }));
        let second = chunk(serde_json::json!({
            "tool_calls": [{ "index": 0, "function": { "arguments": "{\"path\":\"a\"}" } }]
        }));
        let body = format!("{first}{second}data: [DONE]\n\n");
        let address = serve_once(body, "text/event-stream").await;

        let seen: Arc<Mutex<Vec<StreamEvents>>> = Arc::new(Mutex::new(Vec::new()));
        let collector = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| collector.lock().unwrap().push(event)));

        let items = local_codec(&address)
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("stream");

        let events = seen.lock().unwrap().clone();
        let added = events.iter().position(|event| {
            matches!(
                event,
                ResponseStreamEvent::ResponseOutputItemAdded(added)
                    if matches!(&added.item, crate::authority::responses::OutputItem::FunctionCall(call)
                        if call.name == "read")
            )
        });
        let delta = events.iter().position(|event| {
            matches!(
                event,
                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(_)
            )
        });
        assert!(
            added.is_some() && delta.is_some() && added.unwrap() < delta.unwrap(),
            "added must precede the arguments delta: {events:?}"
        );
        assert!(items.iter().any(|item| matches!(
            item,
            Item::FunctionCall(call) if call.name == "read" && call.arguments.contains("a")
        )));
    }

    #[tokio::test]
    async fn stream_usage_lands_on_the_terminal_response() {
        let usage = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            serde_json::json!({
                "id": "c1",
                "object": "chat.completion.chunk",
                "model": "m",
                "choices": [],
                "usage": { "prompt_tokens": 50, "completion_tokens": 3,
                           "prompt_tokens_details": { "cached_tokens": 40 } }
            })
        );
        let address = serve_once(usage, "text/event-stream").await;
        let seen: Arc<Mutex<Vec<StreamEvents>>> = Arc::new(Mutex::new(Vec::new()));
        let collector = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| collector.lock().unwrap().push(event)));
        local_codec(&address)
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("stream");
        let events = seen.lock().unwrap().clone();
        let completed = events
            .iter()
            .find_map(|event| match event {
                ResponseStreamEvent::ResponseCompleted(completed) => Some(completed),
                _ => None,
            })
            .expect("terminal response");
        let usage = completed.response.usage.as_ref().expect("usage");
        assert_eq!(usage.input_tokens, 50);
        assert_eq!(usage.output_tokens, 3);
        assert_eq!(usage.input_tokens_details.cached_tokens, 40);
    }

    #[tokio::test]
    async fn stream_whitespace_only_content_leaves_no_message_item() {
        let body = format!(
            "{}{}data: [DONE]\n\n",
            chunk(serde_json::json!({ "reasoning_content": "think" })),
            chunk(serde_json::json!({ "content": "\n\n" })),
        );
        let address = serve_once(body, "text/event-stream").await;
        let items = local_codec(&address)
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("stream");
        assert_eq!(items.len(), 1, "{items:?}");
        assert!(matches!(&items[0], Item::Reasoning(_)), "{items:?}");
    }
}
