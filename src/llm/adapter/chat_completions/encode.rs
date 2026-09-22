use serde_json::{Map, Value};

use crate::authority::responses::{FunctionCallOutput, Item, MessageItem};
use crate::config::schema::{ADAPTER_COMMANDCODE, ADAPTER_DEEPSEEK_RESPONSES};
use crate::llm::request::ModelRequest;
use crate::platform_knobs::{ThinkingSpec, ThinkingTier, map_thinking_to_wire};
use crate::types::Result;

pub(crate) const REASONING_CONTENT_KEY: &str = "reasoning_content";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningWriteKey {
    ReasoningContent,
}

impl ReasoningWriteKey {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReasoningContent => REASONING_CONTENT_KEY,
        }
    }
}

/// Which models of a Chat Completions host may receive `reasoning_effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningEffort {
    /// The vendor normalizes one effort vocabulary across its whole catalog, so
    /// every model can carry the platform tier (Command Code `low`/`high`/`max`).
    VendorWide,
    /// The vendor derives effort per model, so only models whose published
    /// metadata declares a vocabulary are sent. Every other model keeps the
    /// vendor default and sends no field.
    DeclaredPerModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChatEncodeOpts {
    pub include_stream_usage: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub reasoning_write_key: ReasoningWriteKey,
}

impl ChatEncodeOpts {
    pub(crate) const OPENCODE: Self = Self {
        include_stream_usage: true,
        reasoning_effort: Some(ReasoningEffort::DeclaredPerModel),
        reasoning_write_key: ReasoningWriteKey::ReasoningContent,
    };

    pub(crate) const COMMANDCODE: Self = Self {
        include_stream_usage: true,
        reasoning_effort: Some(ReasoningEffort::VendorWide),
        reasoning_write_key: ReasoningWriteKey::ReasoningContent,
    };
}

/// `reasoning_effort` vocabulary a model's own published metadata declares,
/// cheapest first. `None` means the field must be omitted: the host validates
/// the value per model and rejects an undeclared one with HTTP 400.
///
/// Deliberately narrow — only models whose declared vocabulary is exactly the
/// platform's `low`/`high`/`max`. The rest differ (`glm-5.2` is `high`/`max`,
/// `kimi-k3` is `max` alone, `hy3` is `none`/`low`/`high`, `mimo-v2.5` declares
/// none at all), and the same id can even declare differently on Zen and Go,
/// so a wide table would be wrong more often than it is useful.
fn declared_efforts(model: &str) -> Option<&'static [&'static str]> {
    let model = model.trim().to_ascii_lowercase();
    let model = model.as_str();
    // Zen/Go DeepSeek ids carry the `v4` generation marker (`deepseek-v4-pro`,
    // `deepseek-v4.1-flash`, `deepseek-v4-flash-vision-exp`). Matching the
    // generation rather than a bare `deepseek` prefix keeps ids whose
    // vocabulary is unverified (the undocumented `deepseek-flash`, the retired
    // `deepseek-reasoner` / `r1` / `v3` line) on the vendor default.
    if model.starts_with("deepseek-v4") {
        // The flash line declares `low|high|max`; `deepseek-v4-pro` floors at
        // `high` and rejects `low`.
        return Some(if model == "deepseek-v4-pro" {
            &["high", "max"]
        } else {
            &["low", "high", "max"]
        });
    }
    // GLM 5.3 declares the same `low|high|max` vocabulary as the flash line.
    if matches!(model, "glm-5.3" | "glm-5.3-flash") {
        return Some(&["low", "high", "max"]);
    }
    None
}

/// Wire `reasoning_effort` for this model, or `None` to keep the vendor default.
///
/// `ThinkingSpec::Off` never reaches here — callers omit the field outright
/// rather than send `none`, which some hosts reject (Zen returns 400 for
/// models whose upstream omits `none` from its effort literal).
fn resolve_reasoning_effort(
    policy: ReasoningEffort,
    model: &str,
    tier: ThinkingTier,
) -> Option<String> {
    match policy {
        ReasoningEffort::VendorWide => map_thinking_to_wire(ADAPTER_COMMANDCODE, tier).1,
        ReasoningEffort::DeclaredPerModel => {
            let value = map_thinking_to_wire(ADAPTER_DEEPSEEK_RESPONSES, tier).1?;
            let declared = declared_efforts(model)?;
            if declared.contains(&value.as_str()) {
                return Some(value);
            }
            // The platform's Low tier sits below some models' floor
            // (`deepseek-v4-pro` starts at `high`). Clamp up to the cheapest
            // declared value rather than send one the host rejects.
            declared.first().map(|floor| (*floor).to_string())
        }
    }
}

fn item_text(item: &Item) -> String {
    crate::types::item_text_preview(item)
}

pub(crate) fn encode_chat_body(
    params: &ModelRequest,
    stream: bool,
    opts: &ChatEncodeOpts,
) -> Result<Value> {
    let mut messages: Vec<Value> = Vec::new();
    if !params.instructions.trim().is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": params.instructions,
        }));
    }

    let reasoning_key = opts.reasoning_write_key.as_str();
    let replay_reasoning = !params.tools.is_empty()
        || params
            .input
            .iter()
            .any(|item| matches!(item, Item::Reasoning(_)));
    let mut turn = AssistantTurn::default();

    for item in &params.input {
        match item {
            Item::Reasoning(_) => {
                let text = item_text(item);
                if !text.is_empty() {
                    turn.push_reasoning(&text);
                }
            }
            Item::FunctionCall(fc) => {
                turn.tool_calls.push(serde_json::json!({
                    "id": fc.call_id,
                    "type": "function",
                    "function": {
                        "name": fc.name,
                        "arguments": fc.arguments,
                    }
                }));
            }
            Item::FunctionCallOutput(out) => {
                turn.flush(&mut messages, reasoning_key, replay_reasoning);
                let content = match &out.output {
                    FunctionCallOutput::Text(s) => s.clone(),
                    FunctionCallOutput::Content(_) => item_text(item),
                };
                messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": out.call_id,
                    "content": content,
                }));
            }
            Item::Message(MessageItem::Input(_)) => {
                turn.flush(&mut messages, reasoning_key, replay_reasoning);
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": item_text(item),
                }));
            }
            Item::Message(MessageItem::Output(_)) => {
                let text = item_text(item);
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
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect();

    let mut body = serde_json::json!({
        "model": params.model,
        "messages": messages,
        "stream": stream,
        "temperature": params.temperature,
    });
    if params.max_output_tokens > 0 {
        body["max_tokens"] = Value::from(params.max_output_tokens);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if stream && opts.include_stream_usage {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    if let (Some(policy), ThinkingSpec::Tier(tier)) = (opts.reasoning_effort, params.thinking) {
        if let Some(effort) = resolve_reasoning_effort(policy, &params.model, tier) {
            body["reasoning_effort"] = Value::String(effort);
        }
    }
    Ok(body)
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
        let mut obj = Map::new();
        obj.insert("role".into(), Value::String("assistant".into()));
        if !self.tool_calls.is_empty() {
            let content = match &self.content {
                Some(s) if !s.is_empty() => Value::String(s.clone()),
                _ => Value::Null,
            };
            obj.insert("content".into(), content);
            obj.insert(
                "tool_calls".into(),
                Value::Array(std::mem::take(&mut self.tool_calls)),
            );
        } else {
            obj.insert(
                "content".into(),
                Value::String(self.content.take().unwrap_or_default()),
            );
        }
        if replay_reasoning || !self.reasoning.is_empty() {
            obj.insert(
                reasoning_key.to_string(),
                Value::String(std::mem::take(&mut self.reasoning)),
            );
        }
        self.content = None;
        messages.push(Value::Object(obj));
    }
}
