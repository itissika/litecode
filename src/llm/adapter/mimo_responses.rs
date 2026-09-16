//! MiMo Responses wire adapter — vendor-tolerant Responses dialect.
//!
//! MiMo returns Responses-shaped JSON/SSE but may omit required fields inside
//! `usage.*_tokens_details` (empty objects). This adapter hardens those shapes
//! before authority serde — it does **not** share the strict OpenAI Responses path.
//!
//! ## Official wire alignment
//!
//! - [Responses API](https://mimo.mi.com/docs/en-US/api/chat/responses): `reasoning.effort`
//!   — `none` off; `low`/`medium`/`high` on (vendor: identical behavior today).
//! - [Deep Thinking](https://mimo.mi.com/docs/en-US/quick-start/usage-guide/text-generation/deep-thinking):
//!   `mimo-v2.5` / `mimo-v2.5-pro` default to thinking **enabled**; multi-turn tool
//!   examples use thinking on with tools and require authority `Item::Reasoning`
//!   round-trip. Do **not** force `effort: none` when `tools` is non-empty — that
//!   contradicts the Deep Thinking tool-call walkthrough.

use std::future::Future;
use std::pin::Pin;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{Item, ResponseStreamEvent};
use crate::config::schema::ProviderAuth;
use crate::types::{LitecodeError, Result, StreamEvents};

use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::platform_knobs::{ThinkingSpec, ThinkingTier};

use super::reasoning_replay::ensure_reasoning_replay;
use super::responses_sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::{llm_http_client, transport_error};

/// Platform Default context budget for this closed adapter (economic / capability tradeoff).
pub(crate) const CONTEXT_WINDOW_DEFAULT: usize = 256_000;
/// Vendor maximum context window — used when session `context_mode = max`.
pub(crate) const CONTEXT_WINDOW_MAX: usize = 1_000_000;
/// Selectable wire model ids for Settings dropdown.
pub(crate) const API_MODEL_IDS: &[&str] = &["mimo-v2.5", "mimo-v2.5-pro"];
/// Official MiMo Responses host (pay-as-you-go). `/responses` is appended by
/// [`normalize_endpoint`]. Token-plan hosts remain user-overridable in Settings.
pub(crate) const DEFAULT_ENDPOINT: &str = "https://api.xiaomimimo.com/v1";

/// MiMo Responses-protocol provider (`adapter_id = mimo_responses`).
pub struct MimoResponsesProvider {
    client: Client,
    endpoint_url: String,
    auth: ProviderAuth,
}

impl MimoResponsesProvider {
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

    fn build_body(params: &ModelRequest, stream: bool) -> Result<Value> {
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

        let effort = resolve_mimo_reasoning_effort(params);
        // Thinking mode + tools: MiMo requires every assistant turn's reasoning
        // to be passed back, otherwise 400 (Deep Thinking docs, "Multi-turn
        // Conversation Pass-through Requirements"). Turns without recorded
        // reasoning (compaction summary) get a placeholder item.
        let input_items =
            ensure_reasoning_replay(&params.input, !tools.is_empty(), effort != "none");
        let input: Vec<Value> = input_items
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| LitecodeError::Llm(format!("serialize input items: {e}")))?;

        let mut body = serde_json::json!({
            "model": params.model,
            "instructions": params.instructions,
            "input": input,
            "stream": stream,
            "max_output_tokens": params.max_output_tokens,
            "temperature": params.temperature,
            "reasoning": {
                "effort": effort,
            },
        });
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }

        Ok(body)
    }
}

/// MiMo Responses `reasoning.effort`.
///
/// Vendor default is thinking on; `none` only for [`ThinkingSpec::Off`] or platform Low.
fn resolve_mimo_reasoning_effort(params: &ModelRequest) -> &'static str {
    match params.thinking {
        ThinkingSpec::Off | ThinkingSpec::Tier(ThinkingTier::Low) => "none",
        ThinkingSpec::Tier(ThinkingTier::Medium) => "medium",
        ThinkingSpec::Tier(ThinkingTier::High) => "high",
    }
}

/// Fill empty / incomplete `*_tokens_details` objects so authority serde succeeds.
pub(crate) fn harden_mimo_json(value: &mut Value) {
    match value {
        Value::Object(map) => {
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
            for child in map.values_mut() {
                harden_mimo_json(child);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                harden_mimo_json(child);
            }
        }
        _ => {}
    }
}

fn parse_stream_event(data: &str) -> Result<ResponseStreamEvent> {
    let mut value: Value = serde_json::from_str(data).map_err(|e| {
        LitecodeError::Llm(format!(
            "deserialize ResponseStreamEvent JSON: {e}; payload={data}"
        ))
    })?;
    harden_mimo_json(&mut value);
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

impl LlmProvider for MimoResponsesProvider {
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
            let resp = super::send_cancellable(
                self.client
                    .post(&self.endpoint_url)
                    .header(header_name, header_value)
                    .header("content-type", "application/json")
                    .header("accept", "text/event-stream")
                    .json(&body),
                cancel,
                "opening MiMo event stream",
            )
            .await?;

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
                            transport_error("reading MiMo event stream", &e)
                        })?;
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

            if !cancelled
                && let Some(line) = reader.finish()?
                && let Some(data) = sse_data_payload(&line)
            {
                let event = parse_stream_event(data)?;
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
    use crate::llm::request::{ModelRequest, ToolDef};
    use crate::platform_knobs::{ThinkingSpec, ThinkingTier};

    fn sample_request(tools: Vec<ToolDef>) -> ModelRequest {
        ModelRequest {
            model: "mimo-v2.5".into(),
            instructions: "test".into(),
            input: vec![],
            tools,
            max_output_tokens: 64,
            temperature: 0.0,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: None,
        }
    }

    #[test]
    fn reasoning_effort_defaults_to_medium_vendor_default() {
        let body = MimoResponsesProvider::build_body(&sample_request(vec![]), false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");
    }

    #[test]
    fn reasoning_effort_keeps_thinking_with_tools() {
        let tools = vec![ToolDef {
            name: "read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({}),
        }];
        let mut req = sample_request(tools);
        req.thinking = ThinkingSpec::Tier(ThinkingTier::Medium);
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert!(body.get("tools").is_some());

        req.thinking = ThinkingSpec::Tier(ThinkingTier::High);
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn reasoning_effort_omits_empty_tools_array() {
        let body = MimoResponsesProvider::build_body(&sample_request(vec![]), false).unwrap();
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn reasoning_effort_maps_thinking_mode() {
        let mut req = sample_request(vec![]);
        req.thinking = ThinkingSpec::Tier(ThinkingTier::Medium);
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");

        req.thinking = ThinkingSpec::Tier(ThinkingTier::High);
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");

        req.thinking = ThinkingSpec::Off;
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    #[test]
    fn replay_synthesizes_reasoning_for_reasoning_less_assistant_segment() {
        // Post-compaction shape: assistant summary message with no reasoning item.
        let tools = vec![ToolDef {
            name: "read".into(),
            description: "read".into(),
            input_schema: serde_json::json!({}),
        }];
        let mut req = sample_request(tools);
        req.input = vec![
            crate::types::user_text("hi"),
            crate::types::assistant_text("[Conversation summary]\nkept"),
        ];
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        let input = body["input"].as_array().expect("input array");
        assert_eq!(input.len(), 3, "one reasoning item must be inserted");
        assert_eq!(input[1]["type"], "reasoning");
        assert_eq!(input[1]["content"][0]["type"], "reasoning_text");
        assert_eq!(input[2]["type"], "message");
        assert_eq!(input[2]["role"], "assistant");
    }

    #[test]
    fn replay_skipped_without_tools_or_thinking() {
        let mut req = sample_request(vec![]);
        req.input = vec![
            crate::types::user_text("hi"),
            crate::types::assistant_text("summary"),
        ];
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(
            body["input"].as_array().expect("input array").len(),
            2,
            "no tools → vendor ignores reasoning, no insertion"
        );

        // Thinking explicitly disabled.
        let mut req = sample_request(vec![]);
        req.thinking = ThinkingSpec::Off;
        req.input = vec![
            crate::types::user_text("hi"),
            crate::types::assistant_text("summary"),
        ];
        let body = MimoResponsesProvider::build_body(&req, false).unwrap();
        assert_eq!(
            body["input"].as_array().expect("input array").len(),
            2,
            "thinking off → no insertion"
        );
    }

    #[test]
    fn harden_fills_empty_input_tokens_details() {
        let mut v = serde_json::json!({
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "input_tokens_details": {},
                "output_tokens_details": {}
            }
        });
        harden_mimo_json(&mut v);
        assert_eq!(v["usage"]["input_tokens_details"]["cached_tokens"], 0);
        assert_eq!(v["usage"]["output_tokens_details"]["reasoning_tokens"], 0);
    }
}
