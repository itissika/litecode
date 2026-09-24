//! Responses codec - the authority shape.
//!
//! One implementation serves every provider whose catalog entry declares
//! `endpoint_type = "responses"`. Vendor differences arrive as catalog data
//! (tiers, headers, extra_body) or as named quirks; there is no provider id
//! branch anywhere in this file.
//!
//! The codec also owns the transcript-to-wire boundary: the session log is free
//! to hold lifecycle markers and repeated snapshots of one provider item, and
//! [`normalize_input_items`] reduces that to the single clean copy the dialect
//! accepts before anything is serialized.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{
    FunctionCallOutput, InputContent, Item, MessageItem, ResponseStreamEvent,
};
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::platform_knobs::{ThinkingSpec, ThinkingTier};
use crate::provider_catalog::{ProviderQuirk, ResolvedModel};
use crate::session::media_tokens::classify_input_file;
use crate::types::{LitecodeError, Result, StreamEvents};

use super::http::{llm_http_client, send_cancellable};
use super::sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::{apply_auth, error_prefix, render_headers, user_agent};
use super::{http::interrupted_stream_error, replay::ensure_reasoning_replay, responses_harden};

pub(crate) struct ResponsesCodec {
    client: Client,
    model: Arc<ResolvedModel>,
}

impl ResponsesCodec {
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

    /// Whether this request makes the vendor think. A tier whose literal is the
    /// model's declared off literal (MiMo's low = "none") does not.
    fn thinking_active(&self, thinking: ThinkingSpec) -> bool {
        match self.effort(thinking) {
            Some(literal) => Some(literal) != self.model.reasoning_off.as_deref(),
            None => false,
        }
    }

    fn build_body(&self, params: &ModelRequest, stream: bool) -> Result<Value> {
        let model = &self.model;
        let effort = self.effort(params.thinking);
        let thinking_active = self.thinking_active(params.thinking);

        let mut input_items = params.input.clone();
        // Chat Completions synthesizes reasoning shells for the transcript, but
        // their cc_rs IDs do not identify replayable Responses reasoning. Drop
        // them before vendor-specific reasoning replay can fill any required gap.
        input_items.retain(|item| {
            !matches!(item, Item::Reasoning(reasoning)
                if reasoning.id.as_deref().is_some_and(|id| id.starts_with("cc_rs_")))
        });
        if model.has_quirk(ProviderQuirk::ReasoningReplay) {
            // Replay whenever this request thinks: a vendor whose default is
            // thinking-on must never see a reasoning-less assistant turn.
            input_items = ensure_reasoning_replay(
                &input_items,
                !params.tools.is_empty(),
                thinking_active,
            );
        }
        let input: Vec<Value> = input_items
            .iter()
            .map(serialize_input_item)
            .collect::<std::result::Result<Vec<Value>, _>>()
            .map_err(|error| LitecodeError::Llm(format!("serialize input items: {error}")))?;
        let input = normalize_input_items(input);

        let tools: Vec<Value> = params
            .tools
            .iter()
            .map(|tool| {
                serde_json::json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.input_schema,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": params.model,
            "instructions": params.instructions,
            "input": input,
            "stream": stream,
            "max_output_tokens": params.max_output_tokens,
        });

        if let Some(effort) = effort {
            if model.has_quirk(ProviderQuirk::ThinkingTypeSwitch) {
                // Doubao: a non-thinking control's literal IS the switch value;
                // a thinking control enables the switch and carries its effort.
                if thinking_active {
                    body["thinking"] = serde_json::json!({ "type": "enabled" });
                    body["reasoning"] = serde_json::json!({ "effort": effort });
                } else {
                    body["thinking"] = serde_json::json!({ "type": effort });
                }
            } else {
                body["reasoning"] = serde_json::json!({ "effort": effort });
            }
        }

        let send_temperature = model.temperature
            && !(model.has_quirk(ProviderQuirk::OmitTemperatureWhenThinking) && thinking_active);
        if send_temperature {
            body["temperature"] = serde_json::json!(params.temperature);
        }

        // An empty tools array is omitted for every provider: it carries no
        // information and some vendors reject it.
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        // JSON mode is per-request intent gated by a per-model capability:
        // declaring the capability must never force every turn into JSON.
        if params.json_output && model.json_output {
            body["text"] = serde_json::json!({ "format": { "type": "json_object" } });
        }

        if let Value::Object(map) = &mut body {
            for (key, value) in &model.extra_body {
                map.insert(key.clone(), value.clone());
            }
        }
        Ok(body)
    }

    fn parse_stream_event(&self, data: &str) -> Result<ResponseStreamEvent> {
        let mut value: Value = serde_json::from_str(data).map_err(|error| {
            LitecodeError::Llm(format!(
                "deserialize ResponseStreamEvent JSON: {error}; payload={data}"
            ))
        })?;
        responses_harden::harden(&mut value, self.model.usage_patch);
        serde_json::from_value(value).map_err(|error| {
            LitecodeError::Llm(format!(
                "deserialize ResponseStreamEvent: {error}; payload={data}"
            ))
        })
    }

    fn request(&self, body: &Value, api_key: &str, session_id: Option<&str>) -> reqwest::RequestBuilder {
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

/// Serialize one authority item for the Responses wire.
///
/// Media parts are normalized to the vendor-documented part names: an
/// `input_file` that classifies as image/video/audio becomes
/// `input_image`/`input_video`/`input_audio` with the matching url key. A
/// model that does not declare that modality never receives such a part
/// (catalog validation refuses the declaration), so this stays a codec
/// invariant instead of a per-vendor switch.
fn serialize_input_item(item: &Item) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(item)?;
    let (parts, key) = match item {
        Item::Message(MessageItem::Input(message)) => (message.content.as_slice(), "content"),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Content(parts) => (parts.as_slice(), "output"),
            FunctionCallOutput::Text(_) => return Ok(value),
        },
        _ => return Ok(value),
    };
    let mapped: Vec<Value> = parts
        .iter()
        .map(map_input_content)
        .collect::<std::result::Result<_, _>>()?;
    value[key] = Value::Array(mapped);
    Ok(value)
}

fn map_input_content(content: &InputContent) -> serde_json::Result<Value> {
    let InputContent::InputFile(file) = content else {
        return serde_json::to_value(content);
    };
    let Some(url) = file.file_data.as_deref().or(file.file_url.as_deref()) else {
        return serde_json::to_value(content);
    };
    let (part_type, url_key) = match classify_input_file(file) {
        Some("image") => ("input_image", "image_url"),
        Some("video") => ("input_video", "video_url"),
        Some("audio") => ("input_audio", "audio_url"),
        _ => return serde_json::to_value(content),
    };
    let mut mapped = serde_json::Map::new();
    mapped.insert("type".into(), Value::from(part_type));
    mapped.insert(url_key.into(), Value::from(url));
    Ok(Value::Object(mapped))
}

/// Normalize the replayed `input` array for the Responses wire.
///
/// The transcript is a session-owned log: it keeps lifecycle markers and it may
/// legitimately hold one provider item twice (a live `output_item.added`
/// snapshot persisted while the stream ran, then the authoritative terminal
/// payload appended after it — a reasoning item's `encrypted_content` only
/// completes at the end of the stream, so the two copies are not byte-equal).
/// What the wire accepts is narrower, and that boundary is this codec's job:
///
/// * A provider item `id` is a logical identity and may appear **once** in
///   `input`; a second copy is rejected (OpenAI: `Duplicate item found with id
///   rs_…`). The last copy carries the complete payload, so it wins — at the
///   position of the first copy, which is where the item was produced relative
///   to its surrounding tool call/output pair.
/// * `status` is populated when items are returned *from* the API and is not
///   accepted back on input (OpenAI: `Unknown parameter: 'input[1].status'`).
///   Whatever the vendor's dialect, the field carries no model-visible
///   information, so it never goes out.
///
/// Items without an id are passed through without collapsing: host-built parts,
/// `function_call_output`, and synthesized assistant text (empty id) are not
/// identities. Empty ids and Chat Completions' synthetic message/function-call
/// ids are omitted from the wire rather than presented as Responses identities.
fn normalize_input_items(items: Vec<Value>) -> Vec<Value> {
    // The complete copy of every provider id — the last one in the transcript.
    let mut complete: HashMap<String, Value> = HashMap::new();
    for item in &items {
        if let Some(id) = wire_item_id(item) {
            complete.insert(id.to_string(), item.clone());
        }
    }
    let mut emitted: HashSet<String> = HashSet::new();
    let mut input: Vec<Value> = Vec::with_capacity(items.len());
    for mut item in items {
        if let Some(id) = wire_item_id(&item).map(str::to_string) {
            if !emitted.insert(id.clone()) {
                continue;
            }
            // First copy keeps the slot; the complete payload goes out.
            if let Some(complete_item) = complete.get(&id) {
                item = complete_item.clone();
            }
        }
        if let Value::Object(map) = &mut item {
            map.remove("status");
            if map
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| {
                    id.trim().is_empty()
                        || (map.get("type").and_then(Value::as_str) == Some("message")
                            && id.starts_with("cc_msg_"))
                        || (map.get("type").and_then(Value::as_str) == Some("function_call")
                            && id.starts_with("cc_fc_"))
                })
            {
                map.remove("id");
            }
        }
        input.push(item);
    }
    input
}

/// The provider-assigned identity of a wire item, when it has one.
fn wire_item_id(item: &Value) -> Option<&str> {
    item.get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

impl LlmProvider for ResponsesCodec {
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

    /// Native Responses SSE (`stream: true`). Final Items come from
    /// `response.completed` / `response.incomplete` output, or a cancel seal of
    /// the Items already opened on the stream.
    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async move {
            let body = self.build_body(request, true)?;
            let dump = super::wire_dump::Capture::start(
                "responses",
                &self.model.request_url,
                request.session_id.as_deref(),
                &body,
            );
            let resp = send_cancellable(
                self.request(&body, api_key, request.session_id.as_deref()),
                cancel,
                "opening Responses event stream",
            )
            .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                return Err(LitecodeError::Llm(format!(
                    "{}: HTTP {status}: {text}",
                    error_prefix(&self.model)
                )));
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
                        let chunk = chunk.map_err(|error| {
                            interrupted_stream_error("reading Responses event stream", &error, &acc)
                        })?;
                        for line in reader.feed(&chunk)? {
                            if let Some(dump) = &dump {
                                dump.line(&line);
                            }
                            let Some(data) = sse_data_payload(&line) else {
                                continue;
                            };
                            let event = self.parse_stream_event(data)?;
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
                    if let Some(dump) = &dump {
                        dump.line(&line);
                    }
                    if let Some(data) = sse_data_payload(&line) {
                        let event = self.parse_stream_event(data)?;
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
        Item, MessageItem, OutputMessage, OutputStatus, OutputTextContent, ResponseTextDeltaEvent,
    };
    use crate::llm::request::ToolDef;
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::{assistant_text, user_text};
    use std::path::Path;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// `ENDPOINT` is replaced by streaming tests that need a local listener.
    const OPENAI_PROVIDER: &str = r#"
version = 1
[[providers]]
id = "openai"
name = "OpenAI"
endpoint = "ENDPOINT"
endpoint_type = "responses"
tiers = { low = "low", medium = "medium", high = "high" }
"#;

    const DEEPSEEK_PROVIDER: &str = r#"
version = 1
[[providers]]
id = "deepseek"
name = "DeepSeek"
endpoint = "https://api.deepseek.com"
endpoint_type = "responses"
quirks = ["omit_temperature_when_thinking", "reasoning_replay"]
"#;

    const ARK_PROVIDER: &str = r#"
version = 1
[[providers]]
id = "ark"
name = "Ark"
endpoint = "https://ark.example.com/api/coding/v3"
endpoint_type = "responses"
quirks = ["thinking_type_switch"]
"#;

    /// A provider that declares no reasoning tiers at all.
    const PLAIN_PROVIDER: &str = r#"
version = 1
[[providers]]
id = "plain"
name = "Plain"
endpoint = "https://plain.example.com/v1"
endpoint_type = "responses"
"#;

    fn plain(entry: &str) -> Arc<ResolvedModel> {
        let entry = format!("provider_id = \"plain\"\n{entry}");
        model(PLAIN_PROVIDER, &entry, "plain/m")
    }

    fn model(provider: &str, entry: &str, reference: &str) -> Arc<ResolvedModel> {
        let text = format!("{provider}\n[[models]]\nid = \"m\"\n{entry}\n");
        let catalog = ProviderCatalog::parse(&text, Path::new("test-catalog.toml")).unwrap();
        Arc::clone(catalog.model(reference).expect("model"))
    }

    fn openai(entry: &str) -> Arc<ResolvedModel> {
        let entry = format!("provider_id = \"openai\"\n{entry}");
        let provider = OPENAI_PROVIDER.replace("ENDPOINT", "https://api.openai.com/v1");
        model(&provider, &entry, "openai/m")
    }

    fn deepseek(entry: &str) -> Arc<ResolvedModel> {
        let entry = format!("provider_id = \"deepseek\"\n{entry}");
        model(DEEPSEEK_PROVIDER, &entry, "deepseek/m")
    }

    fn ark(entry: &str) -> Arc<ResolvedModel> {
        let entry = format!("provider_id = \"ark\"\n{entry}");
        model(ARK_PROVIDER, &entry, "ark/m")
    }

    fn sample_request() -> ModelRequest {
        ModelRequest {
            model: "m".into(),
            instructions: "test".into(),
            input: vec![],
            tools: vec![],
            max_output_tokens: 64,
            temperature: 0.0,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: None,
        }
    }

    fn tool(name: &str) -> ToolDef {
        ToolDef {
            name: name.into(),
            description: "d".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn request_body(codec: &ResponsesCodec, request: &ModelRequest) -> Value {
        codec.build_body(request, true).unwrap()
    }

    #[test]
    fn plain_model_sends_the_authority_shape() {
        let codec = ResponsesCodec::new(openai("")).unwrap();
        let body = request_body(&codec, &sample_request());
        assert_eq!(body["model"], "m");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_output_tokens"], 64);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert!(body.get("tools").is_none(), "empty tools are omitted");
        assert!(body.get("store").is_none());
    }

    #[test]
    fn thinking_off_omits_the_whole_control() {
        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.thinking = ThinkingSpec::Off;
        let body = request_body(&codec, &request);
        assert!(body.get("reasoning").is_none(), "{body}");
    }

    #[test]
    fn thinking_off_sends_the_declared_off_literal() {
        let codec = ResponsesCodec::new(deepseek(
            "reasoning = { tiers = { off = \"none\", low = \"low\", medium = \"high\", high = \"max\" } }",
        ))
        .unwrap();
        let mut request = sample_request();
        request.thinking = ThinkingSpec::Off;
        assert_eq!(
            request_body(&codec, &request)["reasoning"]["effort"],
            "none"
        );
    }

    #[test]
    fn model_without_tiers_never_sends_reasoning() {
        let codec = ResponsesCodec::new(plain("")).unwrap();
        let body = request_body(&codec, &sample_request());
        assert!(body.get("reasoning").is_none(), "{body}");
    }

    #[test]
    fn tiers_map_each_platform_level_to_its_own_literal() {
        let codec = ResponsesCodec::new(deepseek(
            "reasoning = { tiers = { low = \"low\", medium = \"high\", high = \"max\" } }",
        ))
        .unwrap();
        for (tier, literal) in [
            (ThinkingTier::Low, "low"),
            (ThinkingTier::Medium, "high"),
            (ThinkingTier::High, "max"),
        ] {
            let mut request = sample_request();
            request.thinking = ThinkingSpec::Tier(tier);
            assert_eq!(request_body(&codec, &request)["reasoning"]["effort"], literal);
        }
    }

    #[test]
    fn deepseek_omits_temperature_while_thinking_and_keeps_it_when_off() {
        let codec = ResponsesCodec::new(deepseek(
            "reasoning = { tiers = { off = \"none\", low = \"low\", medium = \"high\", high = \"max\" } }",
        ))
        .unwrap();
        let with_thinking = request_body(&codec, &sample_request());
        assert!(with_thinking.get("temperature").is_none(), "{with_thinking}");
        let mut request = sample_request();
        request.thinking = ThinkingSpec::Off;
        assert_eq!(request_body(&codec, &request)["temperature"], 0.0);
    }

    #[test]
    fn ark_uses_the_thinking_switch_and_store_extra_body() {
        let codec = ResponsesCodec::new(ark(
            "reasoning = { tiers = { off = \"disabled\", low = \"disabled\", medium = \"medium\", high = \"high\" } }\nextra_body = { store = false }",
        ))
        .unwrap();

        let mut request = sample_request();
        request.thinking = ThinkingSpec::Tier(ThinkingTier::Low);
        let low = request_body(&codec, &request);
        assert_eq!(low["thinking"]["type"], "disabled");
        assert!(low.get("reasoning").is_none(), "{low}");
        assert_eq!(low["store"], false);

        // Off uses the same switch value as Low; thinking stays off.
        request.thinking = ThinkingSpec::Off;
        let off = request_body(&codec, &request);
        assert_eq!(off["thinking"]["type"], "disabled");
        assert!(off.get("reasoning").is_none(), "{off}");

        request.thinking = ThinkingSpec::Tier(ThinkingTier::High);
        let high = request_body(&codec, &request);
        assert_eq!(high["thinking"]["type"], "enabled");
        assert_eq!(high["reasoning"]["effort"], "high");
    }

    #[test]
    fn json_output_is_intent_gated_by_capability() {
        // Intent + capability: the model declares JSON support and this request asks for it.
        let codec = ResponsesCodec::new(deepseek("json_output = true\n")).unwrap();
        let mut request = sample_request();
        request.json_output = true;
        let body = request_body(&codec, &request);
        assert_eq!(body["text"]["format"]["type"], "json_object");

        // Capability alone must never force every turn into JSON mode.
        let body = request_body(&codec, &sample_request());
        assert!(body.get("text").is_none(), "{body}");

        // Intent without the capability is dropped, not sent blind.
        let codec = ResponsesCodec::new(deepseek("")).unwrap();
        let mut request = sample_request();
        request.json_output = true;
        assert!(request_body(&codec, &request).get("text").is_none());
    }

    #[test]
    fn tools_use_the_responses_function_shape() {
        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.tools = vec![tool("read")];
        let body = request_body(&codec, &request);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "read");
        assert!(body["tools"][0].get("function").is_none());
    }

    /// A provider id is one logical item: when the transcript holds a live
    /// snapshot plus the completed payload, one clean copy goes out — the
    /// complete payload, at the position where the item was first produced.
    #[test]
    fn repeated_item_ids_collapse_to_the_complete_copy_in_place() {
        use crate::authority::responses::{
            FunctionCallOutputItemParam, FunctionToolCall, ReasoningItem, ReasoningItemContent,
            ReasoningTextContent,
        };

        fn reasoning(encrypted: &str) -> Item {
            Item::Reasoning(ReasoningItem {
                id: Some("rs_1".into()),
                summary: vec![],
                content: Some(vec![ReasoningItemContent::ReasoningText(
                    ReasoningTextContent {
                        text: "think".into(),
                    },
                )]),
                encrypted_content: Some(encrypted.into()),
                status: Some(OutputStatus::Completed),
            })
        }

        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.input = vec![
            user_text("hi"),
            reasoning("gAAAA-live-snapshot"),
            Item::FunctionCall(FunctionToolCall {
                arguments: "{}".into(),
                call_id: "call_1".into(),
                namespace: None,
                name: "read".into(),
                id: Some("fc_1".into()),
                status: Some(OutputStatus::Completed),
            }),
            reasoning("gAAAA-complete"),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "call_1".into(),
                output: FunctionCallOutput::Text("ok".into()),
                id: None,
                status: Some(OutputStatus::Completed),
            }),
        ];

        let body = request_body(&codec, &request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 4, "{input:?}");
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["id"], "rs_1");
        assert_eq!(input[1]["encrypted_content"], "gAAAA-complete");
        assert_eq!(input[2]["id"], "fc_1");
        assert_eq!(input[3]["type"], "function_call_output");
    }

    /// `status` is populated when items are returned *from* the API; a replay
    /// that carries it is rejected as an unknown parameter.
    #[test]
    fn output_only_status_never_reaches_the_wire() {
        use crate::authority::responses::ReasoningItem;

        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.input = vec![
            user_text("hi"),
            assistant_text("done"),
            Item::Reasoning(ReasoningItem {
                id: Some("rs_1".into()),
                summary: vec![],
                content: None,
                encrypted_content: None,
                status: Some(OutputStatus::InProgress),
            }),
        ];

        let body = request_body(&codec, &request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "{input:?}");
        for item in input {
            assert!(item.get("status").is_none(), "{item}");
        }
    }

    /// An empty id is not an identity: synthesized assistant turns and host-built
    /// outputs must never collapse into each other or send an invalid id.
    #[test]
    fn items_without_ids_are_never_collapsed() {
        use crate::authority::responses::FunctionCallOutputItemParam;

        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.input = vec![
            assistant_text("one"),
            assistant_text("two"),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: "call_1".into(),
                output: FunctionCallOutput::Text("ok".into()),
                id: None,
                status: None,
            }),
        ];

        let body = request_body(&codec, &request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "{body}");
        for item in input {
            assert!(item.get("id").is_none(), "{item}");
        }
        assert_eq!(input[0]["content"][0]["text"], "one");
        assert_eq!(input[1]["content"][0]["text"], "two");
    }

    #[test]
    fn compact_summary_does_not_send_an_empty_id() {
        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.input = vec![
            crate::context_pipeline::summary::compact_summary_message("decisions", false),
            user_text("continue"),
        ];

        let body = request_body(&codec, &request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2, "{body}");
        assert!(input[0].get("id").is_none(), "{}", input[0]);
        assert_eq!(input[0]["role"], "assistant");
    }

    #[test]
    fn chat_history_does_not_replay_synthetic_responses_ids() {
        use crate::authority::responses::{
            FunctionCallOutputItemParam, FunctionToolCall, ReasoningItem,
        };

        let codec = ResponsesCodec::new(openai("")).unwrap();
        let mut request = sample_request();
        request.input = vec![
            user_text("hi"),
            Item::Reasoning(ReasoningItem {
                id: Some("cc_rs_fbef1944fce743688ba5cd474a3f35f6".into()),
                summary: vec![],
                content: None,
                encrypted_content: None,
                status: Some(OutputStatus::Completed),
            }),
            Item::Message(MessageItem::Output(OutputMessage {
                id: "cc_msg_123".into(),
                role: crate::authority::responses::AssistantRole::Assistant,
                status: OutputStatus::Completed,
                phase: None,
                content: vec![crate::authority::responses::OutputMessageContent::OutputText(
                    OutputTextContent {
                        text: "I'll use a tool".into(),
                        annotations: vec![],
                        logprobs: None,
                    },
                )],
            })),
            Item::FunctionCall(FunctionToolCall {
                id: Some("cc_fc_123".into()),
                call_id: "call_123".into(),
                name: "read".into(),
                arguments: "{}".into(),
                status: Some(OutputStatus::Completed),
                namespace: None,
            }),
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                id: None,
                call_id: "call_123".into(),
                output: FunctionCallOutput::Text("ok".into()),
                status: None,
            }),
            user_text("continue"),
        ];

        let input = request_body(&codec, &request)["input"].as_array().unwrap().clone();
        assert_eq!(input.len(), 5, "{input:?}");
        assert_eq!(input[1]["content"][0]["text"], "I'll use a tool");
        assert!(input[1].get("id").is_none(), "{input:?}");
        assert_eq!(input[2]["type"], "function_call");
        assert!(input[2].get("id").is_none(), "{input:?}");
        assert_eq!(input[2]["call_id"], input[3]["call_id"]);
        assert_eq!(input[4]["role"], "user");
    }

    #[test]
    fn reasoning_replay_quirk_fills_the_gap_after_chat_reasoning_is_removed() {
        use crate::authority::responses::ReasoningItem;

        let codec = ResponsesCodec::new(deepseek("reasoning = { tiers = { off = \"none\", low = \"low\", medium = \"high\", high = \"max\" } }")).unwrap();
        let mut request = sample_request();
        request.tools = vec![tool("read")];
        request.input = vec![
            user_text("hi"),
            Item::Reasoning(ReasoningItem {
                id: Some("cc_rs_123".into()),
                summary: vec![],
                content: None,
                encrypted_content: None,
                status: Some(OutputStatus::Completed),
            }),
            assistant_text("done"),
        ];

        let input = request_body(&codec, &request)["input"].as_array().unwrap().clone();
        assert_eq!(input.len(), 3, "{input:?}");
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["id"], "rs_replay_0");
        assert_eq!(input[2]["role"], "assistant");
    }

    #[test]
    fn replay_is_quirk_gated_and_synthesizes_missing_reasoning() {
        let request = {
            let mut request = sample_request();
            request.tools = vec![tool("read")];
            request.input = vec![
                user_text("hi"),
                Item::Message(MessageItem::Output(OutputMessage {
                    id: "msg_1".into(),
                    role: crate::authority::responses::AssistantRole::Assistant,
                    status: OutputStatus::Completed,
                    phase: None,
                    content: vec![crate::authority::responses::OutputMessageContent::OutputText(
                        OutputTextContent {
                            text: "hello".into(),
                            annotations: vec![],
                            logprobs: None,
                        },
                    )],
                })),
            ];
            request
        };

        let codec = ResponsesCodec::new(deepseek("reasoning = { tiers = { off = \"none\", low = \"low\", medium = \"high\", high = \"max\" } }")).unwrap();
        let body = request_body(&codec, &request);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3, "{input:?}");
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(
            input[1]["content"][0]["text"],
            super::super::replay::REPLAY_REASONING_PLACEHOLDER
        );

        // A provider without the quirk keeps the input untouched.
        let codec = ResponsesCodec::new(openai("")).unwrap();
        let body = request_body(&codec, &request);
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
    }

    /// MiMo's Low literal is its off literal, so a Low request is not "thinking"
    /// and must not force reasoning replay.
    #[test]
    fn a_tier_whose_literal_is_off_does_not_force_replay() {
        const MIMO_PROVIDER: &str = r#"
version = 1
[[providers]]
id = "mimo"
name = "MiMo"
endpoint = "https://mimo.example/v1"
endpoint_type = "responses"
quirks = ["reasoning_replay"]
"#;
        let entry = "reasoning = { tiers = { off = \"none\", low = \"none\", medium = \"medium\", high = \"high\" } }";
        let text = format!(
            "{MIMO_PROVIDER}\n[[models]]\nid = \"m\"\nprovider_id = \"mimo\"\n{entry}\n"
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("test-catalog.toml")).unwrap();
        let codec = ResponsesCodec::new(Arc::clone(catalog.model("mimo/m").unwrap())).unwrap();

        let mut request = sample_request();
        request.tools = vec![tool("read")];
        request.input = vec![
            user_text("hi"),
            Item::Message(MessageItem::Output(OutputMessage {
                id: "msg_1".into(),
                role: crate::authority::responses::AssistantRole::Assistant,
                status: OutputStatus::Completed,
                phase: None,
                content: vec![crate::authority::responses::OutputMessageContent::OutputText(
                    OutputTextContent {
                        text: "hello".into(),
                        annotations: vec![],
                        logprobs: None,
                    },
                )],
            })),
        ];

        request.thinking = ThinkingSpec::Tier(ThinkingTier::Low);
        assert_eq!(
            request_body(&codec, &request)["input"].as_array().unwrap().len(),
            2,
            "low maps to the off literal: no replay"
        );

        request.thinking = ThinkingSpec::Tier(ThinkingTier::Medium);
        assert_eq!(
            request_body(&codec, &request)["input"].as_array().unwrap().len(),
            3,
            "medium thinks: replay is synthesized"
        );
    }

    #[test]
    fn media_parts_use_vendor_documented_names() {
        use crate::authority::responses::{FunctionCallOutputItemParam, InputFileContent};

        let file = |name: &str| {
            InputContent::InputFile(InputFileContent {
                file_data: None,
                file_id: None,
                file_url: Some(format!("https://example.com/{name}")),
                filename: Some(name.into()),
                detail: None,
            })
        };
        let item = Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: "c1".into(),
            output: FunctionCallOutput::Content(vec![
                file("clip.mp4"),
                file("sound.mp3"),
                file("shot.webp"),
                file("doc.pdf"),
            ]),
            id: None,
            status: None,
        });
        let value = serialize_input_item(&item).unwrap();
        assert_eq!(value["output"][0]["type"], "input_video");
        assert_eq!(value["output"][0]["video_url"], "https://example.com/clip.mp4");
        assert_eq!(value["output"][1]["type"], "input_audio");
        assert_eq!(value["output"][2]["type"], "input_image");
        assert_eq!(value["output"][3]["type"], "input_file");
    }

    #[test]
    fn request_headers_come_from_the_catalog() {
        let text = r#"
version = 1
[[providers]]
id = "p"
name = "P"
endpoint = "https://x.example/v1"
endpoint_type = "responses"
headers = { "x-session" = "{{session_id}}", "x-static" = "1" }

[[models]]
id = "m"
provider_id = "p"
"#;
        let catalog = ProviderCatalog::parse(text, Path::new("t.toml")).unwrap();
        let resolved = Arc::clone(catalog.model("p/m").unwrap());
        let codec = ResponsesCodec::new(resolved).unwrap();
        let request = codec
            .request(&serde_json::json!({}), "sk-test", Some("ses_1"))
            .build()
            .unwrap();
        assert_eq!(request.headers()["x-session"], "ses_1");
        assert_eq!(request.headers()["x-static"], "1");
        assert_eq!(request.headers()["authorization"], "Bearer sk-test");
        assert_eq!(request.headers()["user-agent"], user_agent());
        assert_eq!(request.url().as_str(), "https://x.example/v1/responses");
    }

    #[test]
    fn session_header_falls_back_to_global() {
        let text = r#"
version = 1
[[providers]]
id = "p"
name = "P"
endpoint = "https://x.example/v1"
endpoint_type = "responses"
headers = { "x-session" = "{{session_id}}" }

[[models]]
id = "m"
provider_id = "p"
"#;
        let catalog = ProviderCatalog::parse(text, Path::new("t.toml")).unwrap();
        let codec = ResponsesCodec::new(Arc::clone(catalog.model("p/m").unwrap())).unwrap();
        let request = codec
            .request(&serde_json::json!({}), "k", None)
            .build()
            .unwrap();
        assert_eq!(request.headers()["x-session"], "global");
    }

    #[test]
    fn usage_patch_is_applied_before_deserialization() {
        let codec = ResponsesCodec::new(deepseek(
            "usage_patch = \"map_max_effort_to_xhigh\"\nreasoning = { tiers = { low = \"low\", medium = \"high\", high = \"max\" } }",
        ))
        .unwrap();
        let event = serde_json::json!({
            "type": "response.created",
            "sequence_number": 1,
            "response": {
                "id": "r",
                "object": "response",
                "created_at": 1,
                "model": "m",
                "status": "in_progress",
                "reasoning": { "effort": "max" },
                "output": []
            }
        })
        .to_string();
        let parsed = codec.parse_stream_event(&event).expect("hardened event");
        let serialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(serialized["response"]["reasoning"]["effort"], "xhigh");
    }

    // ── streaming ────────────────────────────────────────────────────────────

    async fn serve_once(body: String, content_type: &str, declared_len: Option<usize>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let content_type = content_type.to_string();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let length = declared_len.unwrap_or(body.len());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{address}/v1")
    }

    fn completed_response() -> Value {
        serde_json::json!({
            "id": "resp_1",
            "object": "response",
            "created_at": 1,
            "model": "m",
            "status": "completed",
            "output": [
                {"type": "message", "id": "msg_1", "role": "assistant", "status": "completed",
                 "content": [{"type": "output_text", "text": "hi", "annotations": []}]},
                {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "bash",
                 "arguments": "{}", "status": "completed"}
            ],
            "usage": {
                "input_tokens": 3,
                "output_tokens": 2,
                "total_tokens": 5,
                "input_tokens_details": {"cached_tokens": 0},
                "output_tokens_details": {"reasoning_tokens": 0}
            }
        })
    }

    fn sse_fixture() -> String {
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
            "response": completed_response()
        });
        format!("data: {delta}\n\ndata: {completed}\n\n")
    }

    #[tokio::test]
    async fn stream_events_are_forwarded_and_items_come_from_the_terminal() {
        let endpoint = serve_once(sse_fixture(), "text/event-stream", None).await;
        let codec = codec_at(OPENAI_PROVIDER, &endpoint);
        let seen: Arc<Mutex<Vec<crate::types::StreamEvents>>> = Arc::new(Mutex::new(Vec::new()));
        let collector = Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| collector.lock().unwrap().push(event)));

        let items = codec
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("stream ok");

        let events = seen.lock().unwrap().clone();
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseStreamEvent::ResponseOutputTextDelta(ResponseTextDeltaEvent { item_id, .. })
                if item_id == "msg_1"
        )));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ResponseStreamEvent::ResponseCompleted(_)))
        );
        assert!(items.iter().any(|item| matches!(
            item,
            Item::Message(MessageItem::Output(OutputMessage { id, .. })) if id == "msg_1"
        )));
    }

    /// The request URL comes from the catalog, so a streaming test points a
    /// throwaway catalog at the local listener.
    fn codec_at(provider_toml: &str, endpoint: &str) -> ResponsesCodec {
        let text = format!(
            "{}\n[[models]]\nid = \"m\"\nprovider_id = \"openai\"\n",
            provider_toml.replace("ENDPOINT", endpoint)
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("test-catalog.toml")).unwrap();
        ResponsesCodec::new(Arc::clone(catalog.model("openai/m").unwrap())).unwrap()
    }

    #[tokio::test]
    async fn cancel_after_a_delta_seals_incomplete() {
        let delta = serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "hello partial"
        });
        let endpoint = serve_once(format!("data: {delta}\n\n"), "text/event-stream", None).await;
        let codec = codec_at(OPENAI_PROVIDER, &endpoint);
        let cancel = CancellationToken::new();
        let cancel_in_callback = cancel.clone();
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |_| cancel_in_callback.cancel()));
        let items = codec
            .complete_with_stream_events(&sample_request(), "sk-test", on_event, &cancel)
            .await
            .expect("opened stream seals incomplete on cancel");
        assert_eq!(items.len(), 1);
        match &items[0] {
            Item::Message(MessageItem::Output(message)) => {
                assert_eq!(message.status, OutputStatus::Incomplete);
                assert_eq!(crate::types::item_text_preview(&items[0]), "hello partial");
            }
            other => panic!("expected message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_before_any_event_is_canceled() {
        let endpoint = serve_once(String::new(), "text/event-stream", None).await;
        let codec = codec_at(OPENAI_PROVIDER, &endpoint);
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = codec
            .complete_with_stream_events(&sample_request(), "sk-test", None, &cancel)
            .await
            .expect_err("no opened items");
        assert!(matches!(error, LitecodeError::Canceled));
    }

    #[tokio::test]
    async fn transport_disconnect_returns_partial_items_with_an_explicit_error() {
        let delta = serde_json::json!({
            "type": "response.output_text.delta",
            "sequence_number": 1,
            "item_id": "msg_1",
            "output_index": 0,
            "content_index": 0,
            "delta": "recover me"
        });
        let endpoint = serve_once(format!("data: {delta}\n\n"), "text/event-stream", Some(4096))
            .await;
        let codec = codec_at(OPENAI_PROVIDER, &endpoint);
        let error = codec
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("truncated response is an explicit failure");
        let LitecodeError::LlmStreamInterrupted { partial, .. } = error else {
            panic!("expected interrupted stream error, got {error:?}");
        };
        assert_eq!(partial.len(), 1);
        assert_eq!(crate::types::item_text_preview(&partial[0]), "recover me");
    }

    #[tokio::test]
    async fn non_event_stream_content_type_surfaces_the_proxy_body() {
        let body = r#"{"error":{"message":"upstream exploded"}}"#;
        let endpoint = serve_once(body.to_string(), "application/json", None).await;
        let codec = codec_at(OPENAI_PROVIDER, &endpoint);
        let message = codec
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("must fail with the proxy body")
            .to_string();
        assert!(message.contains("text/event-stream"), "{message}");
        assert!(message.contains("upstream exploded"), "{message}");
    }

    #[tokio::test]
    async fn http_errors_name_the_catalog_provider() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad",
                )
                .await;
        });
        let codec = codec_at(OPENAI_PROVIDER, &format!("http://{address}/v1"));
        let message = codec
            .complete_with_stream_events(
                &sample_request(),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("400")
            .to_string();
        assert!(message.contains("provider 'OpenAI'"), "{message}");
        assert!(message.contains("HTTP 400"), "{message}");
    }
}
