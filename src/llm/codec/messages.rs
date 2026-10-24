//! Anthropic Messages codec.
//!
//! Authority items go in, authority stream events come out. The wire dialect
//! (roles, tool pairing, thinking signatures, cache breakpoints) stays here.
//! Nothing in this file reads a provider id.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputFileContent,
    InputImageContent, InputMessage, InputRole, Item, MessageItem, OutputMessageContent,
    ReasoningItem, ReasoningItemContent,
};
use crate::llm::provider::LlmProvider;
use crate::llm::request::ModelRequest;
use crate::platform_knobs::{ThinkingSpec, ThinkingTier};
use crate::provider_catalog::{Modality, ResolvedModel};
use crate::session::media_tokens::classify_input_file;
use crate::types::{LitecodeError, Result, StreamEvents};

use super::http::{
    EmptyStreamAction, ReconnectBudget, llm_http_client, on_broken_stream, preserve_partial,
    send_cancellable,
};
use super::messages_synth::{MessagesSynth, REDACTED_PREFIX};
use super::sse::{SseLineReader, check_event_stream_content_type, sse_data_payload};
use super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::{apply_auth, error_prefix, render_headers, user_agent};

fn adaptive_thinking(display: Option<&str>) -> Value {
    let mut thinking = serde_json::json!({"type": "adaptive"});
    if let Some(display) = display {
        thinking["display"] = serde_json::json!(display);
    }
    // Replay already drops signatures whose prefix was rewritten. This only
    // covers a mismatch that slips past it, and gateways may not forward it.
    thinking["block_binding"] = serde_json::json!({"prefix_mismatch_behavior": "drop_block"});
    thinking
}

fn thinking_binding_header(existing: Option<&str>) -> String {
    let Some(existing) = existing.map(str::trim).filter(|value| !value.is_empty()) else {
        return THINKING_BINDING_BETA.to_string();
    };
    if existing
        .split(',')
        .any(|part| part.trim() == THINKING_BINDING_BETA)
    {
        return existing.to_string();
    }
    format!("{existing},{THINKING_BINDING_BETA}")
}

const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Lets a rewritten prefix drop the thinking blocks bound to the old one.
/// Required whenever `thinking.block_binding` is sent. Valid only beside
/// `thinking.type: "adaptive"` on Sonnet 5.5 and Haiku 5.5.
const THINKING_BINDING_BETA: &str = "thinking-binding-controls-2026-08-01";
const NO_RESULT: &str = "[no result recorded]";
const ORPHAN_RESULT: &str = "[tool result without matching call]";

pub(crate) struct MessagesCodec {
    client: Client,
    model: Arc<ResolvedModel>,
}

impl MessagesCodec {
    pub(crate) fn new(model: Arc<ResolvedModel>) -> Result<Self> {
        Ok(Self {
            client: llm_http_client()?,
            model,
        })
    }

    fn encode_body(&self, params: &ModelRequest, stream: bool) -> Result<Value> {
        let (mut messages, calls, results) = fold_items(&self.model, &params.input);
        place_results(&mut messages, calls, results);
        let mut messages = prune_and_merge(messages);
        // A request that ends on the assistant is prefill, which these models
        // reject. A period is a real text block; an empty one is rejected too.
        let needs_user = match messages.last() {
            None => true,
            Some(message) => message.assistant,
        };
        if needs_user {
            messages.push(Draft {
                assistant: false,
                blocks: vec![Block::text(".")],
            });
        }
        reorder(&mut messages);
        mark_cache(&mut messages);

        let max_tokens = if params.max_output_tokens > 0 {
            params.max_output_tokens
        } else {
            self.model.max_output
        };
        let mut body = serde_json::json!({
            "model": params.model,
            "max_tokens": max_tokens,
            "stream": stream,
            "messages": messages
                .iter()
                .map(|message| serde_json::json!({
                    "role": if message.assistant { "assistant" } else { "user" },
                    "content": message.blocks.iter().map(render_block).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        });
        let instructions = params.instructions.trim();
        if !instructions.is_empty() {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": instructions,
                "cache_control": {"type": "ephemeral"},
            }]);
        }
        if !params.tools.is_empty() {
            body["tools"] = Value::Array(
                params
                    .tools
                    .iter()
                    .map(|tool| {
                        serde_json::json!({
                            "name": tool.name,
                            "description": tool.description,
                            "input_schema": with_object_type(&tool.input_schema),
                        })
                    })
                    .collect(),
            );
        }
        // Signed blocks only reach this codec when the port kept them, which it
        // does only for an enabled thinking intent. A thinking-off request
        // therefore never carries a block the off literal cannot sit beside.
        let (thinking, output_config) = self.thinking_fields(params.thinking);
        if let Some(thinking) = thinking {
            body["thinking"] = thinking;
        }
        if let Some(output_config) = output_config {
            body["output_config"] = output_config;
        }
        if let Value::Object(map) = &mut body {
            for (key, value) in &self.model.extra_body {
                map.insert(key.clone(), value.clone());
            }
        }
        Ok(body)
    }

    /// Catalog thinking controls. No model-id branch: off, effort, and display
    /// all come from the resolved model.
    fn thinking_fields(&self, spec: ThinkingSpec) -> (Option<Value>, Option<Value>) {
        let Some(tiers) = &self.model.reasoning else {
            return (None, None);
        };
        let effort = match spec {
            ThinkingSpec::Off => {
                return match self.model.reasoning_off.as_deref() {
                    Some(off) if !off.is_empty() => (Some(serde_json::json!({"type": off})), None),
                    // No off literal means the model stays adaptive. The prefix
                    // check then defaults to error, so send drop_block anyway.
                    _ => (Some(adaptive_thinking(None)), None),
                };
            }
            ThinkingSpec::Tier(ThinkingTier::Low) => tiers.low.as_str(),
            ThinkingSpec::Tier(ThinkingTier::Medium) => tiers.medium.as_str(),
            ThinkingSpec::Tier(ThinkingTier::High) => tiers.high.as_str(),
        };
        if self.model.reasoning_off.as_deref() == Some(effort) {
            return (Some(serde_json::json!({"type": effort})), None);
        }
        let thinking = adaptive_thinking(
            self.model
                .reasoning_summary
                .as_deref()
                .filter(|display| !display.is_empty()),
        );
        (Some(thinking), Some(serde_json::json!({"effort": effort})))
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
        let mut catalog_beta = None;
        for (name, value) in render_headers(&self.model.headers, session_id) {
            if name.eq_ignore_ascii_case("anthropic-beta") {
                catalog_beta = Some(value);
                continue;
            }
            builder = builder.header(name, value);
        }
        if body
            .get("thinking")
            .and_then(|thinking| thinking.get("block_binding"))
            .is_some()
        {
            builder = builder.header(
                "anthropic-beta",
                thinking_binding_header(catalog_beta.as_deref()),
            );
        } else if let Some(beta) = catalog_beta {
            builder = builder.header("anthropic-beta", beta);
        }
        // Set after catalog headers so a reserved name cannot replace the version.
        builder
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(body)
    }

    fn interpret(
        synth: &mut MessagesSynth,
        data: &str,
        prefix: &str,
    ) -> Result<Vec<crate::authority::responses::ResponseStreamEvent>> {
        let value: Value = serde_json::from_str(data).map_err(|error| {
            LitecodeError::Llm(format!(
                "{prefix}: Messages SSE JSON: {error}; payload={data}"
            ))
        })?;
        if value.get("type").and_then(Value::as_str) == Some("error") {
            return Err(LitecodeError::Llm(format!(
                "{prefix}: {}",
                error_detail(&value)
            )));
        }
        synth.ingest(&value)
    }
}

impl LlmProvider for MessagesCodec {
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
                "messages",
                &self.model.request_url,
                request.session_id.as_deref(),
                &body,
            );
            let prefix = error_prefix(&self.model);
            let mut budget = ReconnectBudget::new();
            'open: loop {
                let resp = send_cancellable(
                    self.request(&body, api_key, request.session_id.as_deref()),
                    cancel,
                    "opening Messages event stream",
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
                    return Err(http_error(&prefix, status, &text));
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
                let mut synth = MessagesSynth::new(&request.model);
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
                                        "reading Messages event stream",
                                        &error,
                                        &acc,
                                    );
                                    match on_broken_stream(&mut budget, cancel, !acc.is_empty()).await? {
                                        EmptyStreamAction::Retry => continue 'open,
                                        EmptyStreamAction::GiveUp => {
                                            return Err(preserve_partial(failure, &acc));
                                        }
                                    }
                                }
                            };
                            let lines = match reader.feed(&chunk) {
                                Ok(lines) => lines,
                                Err(error) => {
                                    match on_broken_stream(&mut budget, cancel, !acc.is_empty()).await? {
                                        EmptyStreamAction::Retry => continue 'open,
                                        EmptyStreamAction::GiveUp => {
                                            return Err(preserve_partial(error, &acc));
                                        }
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
                                let events = match Self::interpret(&mut synth, data, &prefix) {
                                    Ok(events) => events,
                                    Err(error) => {
                                        match on_broken_stream(&mut budget, cancel, !acc.is_empty()).await? {
                                            EmptyStreamAction::Retry => continue 'open,
                                            EmptyStreamAction::GiveUp => {
                                                return Err(preserve_partial(error, &acc));
                                            }
                                        }
                                    }
                                };
                                for event in events {
                                    if let Some(items) = forward_stream_event(
                                        &mut gate,
                                        &mut acc,
                                        event,
                                        &mut on_event,
                                    )? {
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
                        if let Some(data) = sse_data_payload(&line) {
                            let events = match Self::interpret(&mut synth, data, &prefix) {
                                Ok(events) => events,
                                Err(error) => {
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
                            for event in events {
                                if let Some(items) =
                                    forward_stream_event(&mut gate, &mut acc, event, &mut on_event)?
                                {
                                    terminal_items = Some(items);
                                }
                            }
                        }
                    }
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

fn http_error(prefix: &str, status: reqwest::StatusCode, text: &str) -> LitecodeError {
    let detail = serde_json::from_str::<Value>(text)
        .ok()
        .as_ref()
        .map(error_detail)
        .filter(|detail| !detail.is_empty())
        .unwrap_or_else(|| text.to_string());
    LitecodeError::Llm(format!("{prefix}: HTTP {status}: {detail}"))
}

fn error_detail(value: &Value) -> String {
    let err = value.get("error").unwrap_or(value);
    match (
        err.get("type").and_then(Value::as_str),
        err.get("message").and_then(Value::as_str),
    ) {
        (Some(kind), Some(message)) => format!("{kind}: {message}"),
        _ => value.to_string(),
    }
}

fn with_object_type(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut map = map.clone();
            map.entry("type".to_string())
                .or_insert_with(|| serde_json::json!("object"));
            Value::Object(map)
        }
        _ => serde_json::json!({"type": "object"}),
    }
}

struct Draft {
    assistant: bool,
    blocks: Vec<Block>,
}

struct Block {
    body: Body,
    cache: bool,
}

enum Body {
    Text(String),
    Media(Value),
    Thinking {
        text: String,
        signature: String,
    },
    Redacted {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: Value,
        is_error: bool,
    },
}

impl Block {
    fn text(text: impl Into<String>) -> Self {
        Self {
            body: Body::Text(text.into()),
            cache: false,
        }
    }

    fn media(value: Value) -> Self {
        Self {
            body: Body::Media(value),
            cache: false,
        }
    }

    fn thinking(text: impl Into<String>, signature: impl Into<String>) -> Self {
        Self {
            body: Body::Thinking {
                text: text.into(),
                signature: signature.into(),
            },
            cache: false,
        }
    }

    fn redacted(data: impl Into<String>) -> Self {
        Self {
            body: Body::Redacted { data: data.into() },
            cache: false,
        }
    }

    fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        Self {
            body: Body::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            },
            cache: false,
        }
    }

    fn tool_result(id: impl Into<String>, content: Value, is_error: bool) -> Self {
        Self {
            body: Body::ToolResult {
                id: id.into(),
                content,
                is_error,
            },
            cache: false,
        }
    }

    fn is_cacheable(&self) -> bool {
        match &self.body {
            Body::Text(text) => !text.is_empty(),
            Body::Media(_) | Body::ToolUse { .. } | Body::ToolResult { .. } => true,
            Body::Thinking { .. } | Body::Redacted { .. } => false,
        }
    }
}

fn fold_items(
    model: &ResolvedModel,
    items: &[Item],
) -> (Vec<Draft>, Vec<(usize, String)>, HashMap<String, Block>) {
    let call_ids: std::collections::HashSet<&str> = items
        .iter()
        .filter_map(|item| match item {
            Item::FunctionCall(call) => Some(call.call_id.as_str()),
            _ => None,
        })
        .collect();
    let mut messages = Vec::new();
    let mut calls = Vec::new();
    let mut results = HashMap::new();
    for item in items {
        match item {
            Item::Message(MessageItem::Input(message)) => {
                push(&mut messages, false, input_blocks(model, &message.content));
            }
            Item::Message(MessageItem::Output(message)) => {
                let mut blocks = Vec::new();
                for part in &message.content {
                    match part {
                        OutputMessageContent::OutputText(text) => {
                            if !text.text.is_empty() {
                                blocks.push(Block::text(text.text.clone()));
                            }
                        }
                        OutputMessageContent::Refusal(refusal) => {
                            if !refusal.refusal.is_empty() {
                                blocks.push(Block::text(refusal.refusal.clone()));
                            }
                        }
                    }
                }
                push(&mut messages, true, blocks);
            }
            Item::Reasoning(reasoning) => {
                if let Some(block) = reasoning_block(reasoning) {
                    push(&mut messages, true, vec![block]);
                }
            }
            Item::FunctionCall(call) => {
                let block = Block::tool_use(
                    call.call_id.clone(),
                    call.name.clone(),
                    object_or_empty(&call.arguments),
                );
                if let Some(index) = push(&mut messages, true, vec![block]) {
                    calls.push((index, call.call_id.clone()));
                }
            }
            Item::FunctionCallOutput(output) => {
                if !call_ids.contains(output.call_id.as_str()) {
                    let text = format!("{ORPHAN_RESULT}\n{}", output_preview(output));
                    push(&mut messages, false, vec![Block::text(text)]);
                    continue;
                }
                results.insert(
                    output.call_id.clone(),
                    Block::tool_result(output.call_id.clone(), output_value(model, output), false),
                );
                // The result is placed later, next to its call. Leave a user
                // boundary now so a later assistant turn does not merge into
                // the one that issued the call.
                if messages.last().is_some_and(|message| message.assistant) {
                    messages.push(Draft {
                        assistant: false,
                        blocks: Vec::new(),
                    });
                }
            }
            _ => {}
        }
    }
    (messages, calls, results)
}

fn push(messages: &mut Vec<Draft>, assistant: bool, blocks: Vec<Block>) -> Option<usize> {
    if blocks.is_empty() {
        return None;
    }
    if let Some(last) = messages.last_mut() {
        if last.assistant == assistant {
            last.blocks.extend(blocks);
            return Some(messages.len() - 1);
        }
    }
    messages.push(Draft { assistant, blocks });
    Some(messages.len() - 1)
}

/// `tool_result` has to be the first blocks of the user message that follows
/// the assistant message carrying the matching `tool_use`.
fn place_results(
    messages: &mut Vec<Draft>,
    calls: Vec<(usize, String)>,
    mut results: HashMap<String, Block>,
) {
    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (index, id) in calls {
        grouped.entry(index).or_default().push(id);
    }
    for (index, ids) in grouped.into_iter().rev() {
        let blocks: Vec<Block> = ids
            .into_iter()
            .map(|id| {
                results.remove(&id).unwrap_or_else(|| {
                    Block::tool_result(id, Value::String(NO_RESULT.to_string()), true)
                })
            })
            .collect();
        let next = index + 1;
        if messages.get(next).is_some_and(|message| !message.assistant) {
            let mut rest = std::mem::take(&mut messages[next].blocks);
            messages[next].blocks = blocks;
            messages[next].blocks.append(&mut rest);
        } else {
            messages.insert(
                next,
                Draft {
                    assistant: false,
                    blocks,
                },
            );
        }
    }
    if !results.is_empty() {
        let blocks = results
            .into_values()
            .map(|block| match block.body {
                Body::ToolResult { content, .. } => {
                    Block::text(format!("{ORPHAN_RESULT}\n{}", content_string(&content)))
                }
                other => Block {
                    body: other,
                    cache: false,
                },
            })
            .collect();
        push(messages, false, blocks);
    }
}

fn prune_and_merge(mut messages: Vec<Draft>) -> Vec<Draft> {
    for message in &mut messages {
        message.blocks.retain(|block| match &block.body {
            Body::Text(text) => !text.is_empty(),
            _ => true,
        });
    }
    messages.retain(|message| !message.blocks.is_empty());
    let mut merged = Vec::new();
    for message in messages {
        if merged
            .last()
            .is_some_and(|last: &Draft| last.assistant == message.assistant)
        {
            merged.last_mut().unwrap().blocks.extend(message.blocks);
        } else {
            merged.push(message);
        }
    }
    merged
}

/// Thinking blocks are fences. A signature covers the blocks before it, so a
/// tool call may move to the end of its own stretch and must not cross a fence.
fn reorder(messages: &mut [Draft]) {
    for message in messages.iter_mut().filter(|message| message.assistant) {
        message.blocks = fence_tool_uses(std::mem::take(&mut message.blocks));
    }
}

fn fence_tool_uses(blocks: Vec<Block>) -> Vec<Block> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut segment = Vec::new();
    for block in blocks {
        if matches!(block.body, Body::Thinking { .. } | Body::Redacted { .. }) {
            flush_tool_segment(&mut out, &mut segment);
            out.push(block);
        } else {
            segment.push(block);
        }
    }
    flush_tool_segment(&mut out, &mut segment);
    out
}

fn flush_tool_segment(out: &mut Vec<Block>, segment: &mut Vec<Block>) {
    let mut rest = Vec::new();
    let mut tools = Vec::new();
    for block in segment.drain(..) {
        if matches!(block.body, Body::ToolUse { .. }) {
            tools.push(block);
        } else {
            rest.push(block);
        }
    }
    out.extend(rest);
    out.extend(tools);
}

fn mark_cache(messages: &mut [Draft]) {
    if let Some(message) = messages.last_mut() {
        mark_last_cacheable(message);
    }
    let users: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| !message.assistant)
        .map(|(index, _)| index)
        .collect();
    if users.len() >= 2 {
        let index = users[users.len() - 2];
        mark_last_cacheable(&mut messages[index]);
    }
}

fn mark_last_cacheable(message: &mut Draft) {
    if let Some(block) = message
        .blocks
        .iter_mut()
        .rev()
        .find(|block| block.is_cacheable())
    {
        block.cache = true;
    }
}

fn render_block(block: &Block) -> Value {
    let mut value = match &block.body {
        Body::Text(text) => serde_json::json!({"type": "text", "text": text}),
        Body::Media(value) => value.clone(),
        Body::Thinking { text, signature } => {
            serde_json::json!({"type": "thinking", "thinking": text, "signature": signature})
        }
        Body::Redacted { data } => {
            serde_json::json!({"type": "redacted_thinking", "data": data})
        }
        Body::ToolUse { id, name, input } => {
            serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        Body::ToolResult {
            id,
            content,
            is_error,
        } => {
            let mut value =
                serde_json::json!({"type": "tool_result", "tool_use_id": id, "content": content});
            if *is_error {
                value["is_error"] = serde_json::json!(true);
            }
            value
        }
    };
    if block.cache {
        if let Value::Object(map) = &mut value {
            map.insert(
                "cache_control".to_string(),
                serde_json::json!({"type": "ephemeral"}),
            );
        }
    }
    value
}

fn reasoning_block(reasoning: &ReasoningItem) -> Option<Block> {
    let signature = reasoning
        .encrypted_content
        .as_deref()
        .filter(|signature| !signature.is_empty())?;
    if let Some(data) = signature.strip_prefix(REDACTED_PREFIX) {
        return Some(Block::redacted(data));
    }
    let text = reasoning
        .content
        .as_ref()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| match part {
                    ReasoningItemContent::ReasoningText(text) => Some(text.text.as_str()),
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Some(Block::thinking(text, signature))
}

fn object_or_empty(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => serde_json::json!({}),
    }
}

fn input_blocks(model: &ResolvedModel, parts: &[InputContent]) -> Vec<Block> {
    parts
        .iter()
        .filter_map(|part| part_block(model, part))
        .collect()
}

fn part_block(model: &ResolvedModel, part: &InputContent) -> Option<Block> {
    match part {
        InputContent::InputText(text) => {
            if text.text.is_empty() {
                None
            } else {
                Some(Block::text(text.text.clone()))
            }
        }
        InputContent::InputImage(image) => Some(image_block(model, image)),
        InputContent::InputFile(file) => Some(file_block(model, file)),
    }
}

fn image_block(model: &ResolvedModel, image: &InputImageContent) -> Block {
    let part = InputContent::InputImage(image.clone());
    let Some(url) = image.image_url.as_deref().filter(|url| !url.is_empty()) else {
        return degrade(&part);
    };
    if !model.supports(Modality::Image) {
        return degrade(&part);
    }
    source_block("image", url).unwrap_or_else(|| degrade(&part))
}

fn file_block(model: &ResolvedModel, file: &InputFileContent) -> Block {
    let part = InputContent::InputFile(file.clone());
    let url = file
        .file_data
        .as_deref()
        .or(file.file_url.as_deref())
        .unwrap_or("");
    if looks_like_pdf(file) {
        if model.supports(Modality::Pdf) {
            if let Some(block) = source_block("document", url) {
                return force_pdf_media_type(block);
            }
        }
        return degrade(&part);
    }
    if classify_input_file(file) == Some("image")
        && model.supports(Modality::Image)
        && let Some(block) = source_block("image", url)
    {
        return block;
    }
    degrade(&part)
}

fn source_block(kind: &str, url: &str) -> Option<Block> {
    let source = if let Some((media_type, data)) = split_data_url(url) {
        serde_json::json!({"type": "base64", "media_type": media_type, "data": data})
    } else if url.starts_with("https://") || url.starts_with("http://") {
        serde_json::json!({"type": "url", "url": url})
    } else {
        return None;
    };
    Some(Block::media(
        serde_json::json!({"type": kind, "source": source}),
    ))
}

fn force_pdf_media_type(mut block: Block) -> Block {
    if let Body::Media(value) = &mut block.body {
        if let Some(media_type) = value.pointer_mut("/source/media_type") {
            *media_type = serde_json::json!("application/pdf");
        }
    }
    block
}

fn split_data_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media_type = meta.split(';').next()?.trim();
    if media_type.is_empty() || data.is_empty() {
        return None;
    }
    Some((media_type.to_string(), data.to_string()))
}

fn looks_like_pdf(file: &InputFileContent) -> bool {
    let filename = file.filename.as_deref().unwrap_or("").to_ascii_lowercase();
    if filename.ends_with(".pdf") {
        return true;
    }
    let url = file.file_url.as_deref().unwrap_or("").to_ascii_lowercase();
    if url.contains(".pdf") || url.contains("application/pdf") {
        return true;
    }
    let data = file.file_data.as_deref().unwrap_or("");
    let head = &data[..data.len().min(128)];
    let head = head.to_ascii_lowercase();
    head.contains("application/pdf") || head.contains(".pdf")
}

/// Image and file parts have no text preview. The label keeps the turn from
/// dropping the part when this model cannot accept the modality.
fn degrade(part: &InputContent) -> Block {
    let preview =
        crate::types::item_text_preview(&Item::Message(MessageItem::Input(InputMessage {
            content: vec![part.clone()],
            role: InputRole::User,
            status: None,
        })));
    if !preview.is_empty() {
        return Block::text(preview);
    }
    let label = match part {
        InputContent::InputFile(file) => file
            .filename
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "[file]".to_string()),
        InputContent::InputImage(_) => "[image]".to_string(),
        InputContent::InputText(_) => "[text]".to_string(),
    };
    Block::text(label)
}

fn output_value(model: &ResolvedModel, output: &FunctionCallOutputItemParam) -> Value {
    match &output.output {
        FunctionCallOutput::Text(text) => Value::String(text.clone()),
        FunctionCallOutput::Content(parts) => {
            let blocks = input_blocks(model, parts);
            let only_text = blocks
                .iter()
                .all(|block| matches!(block.body, Body::Text(_)));
            if blocks.is_empty() {
                Value::String(String::new())
            } else if only_text {
                Value::String(
                    blocks
                        .iter()
                        .filter_map(|block| match &block.body {
                            Body::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            } else {
                Value::Array(blocks.iter().map(render_block).collect())
            }
        }
    }
}

fn output_preview(output: &FunctionCallOutputItemParam) -> String {
    match &output.output {
        FunctionCallOutput::Text(text) => text.clone(),
        FunctionCallOutput::Content(parts) => parts
            .iter()
            .filter_map(|part| match part {
                InputContent::InputText(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn content_string(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        AssistantRole, FunctionToolCall, OutputMessage, OutputStatus, OutputTextContent,
        ReasoningTextContent, ResponseStreamEvent,
    };
    use crate::llm::request::{ModelRequest, ToolDef};
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
endpoint_type = "messages"
auth = "bearer"
headers = { "x-opencode-session" = "{{session_id}}" }
"#;

    const REASONING: &str = r#"modalities = ["text", "image", "pdf"]
reasoning = { summary = "summarized", tiers = { low = "low", medium = "high", high = "max" } }
"#;

    fn codec_at(endpoint: &str, tail: &str) -> MessagesCodec {
        let text = format!(
            "{}\n[[models]]\nid = \"m\"\nprovider_id = \"zen\"\ncontext_window = 256000\nmax_output = 128000\n{tail}\n",
            PROVIDER.replace("ENDPOINT", endpoint)
        );
        let catalog = ProviderCatalog::parse(&text, Path::new("t.toml")).unwrap();
        MessagesCodec::new(Arc::clone(catalog.model("zen/m").unwrap())).unwrap()
    }

    fn codec() -> MessagesCodec {
        codec_at("https://opencode.ai/inference/anthropic/v1", REASONING)
    }

    fn request(input: Vec<Item>) -> ModelRequest {
        ModelRequest {
            model: "claude-sonnet-5-5".into(),
            instructions: "sys".into(),
            input,
            tools: vec![ToolDef {
                name: "read".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({"properties": {"path": {"type": "string"}}}),
            }],
            max_output_tokens: 64,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: Some("ses_1".into()),
            purpose: crate::llm::RequestPurpose::Main,
        }
    }

    fn thinking(text: &str, signature: &str) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some("rs".into()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent { text: text.into() },
            )]),
            encrypted_content: Some(signature.into()),
            status: Some(OutputStatus::Completed),
        })
    }

    fn assistant(text: &str) -> Item {
        Item::Message(MessageItem::Output(OutputMessage {
            id: "m".into(),
            role: AssistantRole::Assistant,
            status: OutputStatus::Completed,
            phase: None,
            content: vec![
                crate::authority::responses::OutputMessageContent::OutputText(OutputTextContent {
                    text: text.into(),
                    annotations: vec![],
                    logprobs: None,
                }),
            ],
        }))
    }

    fn call(id: &str, name: &str, arguments: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            id: Some(id.into()),
            call_id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
            status: Some(OutputStatus::Completed),
            namespace: None,
        })
    }

    fn output_text(id: &str, text: &str) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: id.into(),
            output: FunctionCallOutput::Text(text.into()),
            id: None,
            status: None,
        })
    }

    fn without_cache_control(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut next = serde_json::Map::new();
                for (key, child) in map {
                    if key == "cache_control" {
                        continue;
                    }
                    next.insert(key.clone(), without_cache_control(child));
                }
                Value::Object(next)
            }
            Value::Array(items) => Value::Array(items.iter().map(without_cache_control).collect()),
            other => other.clone(),
        }
    }

    /// The trailing "." is a prefill pad, not history. Drop it before comparing
    /// one turn's messages with the next turn's.
    fn history_messages(body: &Value) -> Vec<Value> {
        let mut messages = body["messages"].as_array().cloned().unwrap_or_default();
        if let Some(last) = messages.last() {
            let pad = last.get("role").and_then(Value::as_str) == Some("user")
                && last.pointer("/content/0/text").and_then(Value::as_str) == Some(".")
                && last
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| blocks.len() == 1);
            if pad {
                messages.pop();
            }
        }
        messages
            .into_iter()
            .map(|message| without_cache_control(&message))
            .collect()
    }

    /// One conversation, two turns. The earlier request's messages are a prefix
    /// of the later one: history is append-only, including signed thinking.
    #[test]
    fn later_turn_extends_the_previous_messages_prefix() {
        let first = codec()
            .encode_body(
                &request(vec![
                    user_text("q1"),
                    thinking("old", "sig-old"),
                    assistant("a1"),
                ]),
                true,
            )
            .unwrap();
        let second = codec()
            .encode_body(
                &request(vec![
                    user_text("q1"),
                    thinking("old", "sig-old"),
                    assistant("a1"),
                    user_text("q2"),
                    thinking("new", "sig-new"),
                    assistant("a2"),
                ]),
                true,
            )
            .unwrap();
        let prev = history_messages(&first);
        let next = history_messages(&second);
        assert!(
            next.starts_with(&prev),
            "closed-turn thinking left the later request\nprev={prev:?}\nnext={next:?}"
        );
    }

    #[test]
    fn thinking_and_tool_signatures_round_trip_inside_the_open_turn() {
        let body = codec()
            .encode_body(
                &request(vec![
                    user_text("read a.rs"),
                    thinking("Need the file.", "EqQBCg=="),
                    assistant("Reading."),
                    call("toolu_01", "read", r#"{"path":"a.rs"}"#),
                    output_text("toolu_01", "contents"),
                    thinking("Done.", "EqQBCg==second"),
                    assistant("Summary."),
                ]),
                true,
            )
            .unwrap();
        let messages = body["messages"].as_array().unwrap();
        let first = &messages[1]["content"];
        assert_eq!(first[0]["type"], "thinking");
        assert_eq!(first[0]["thinking"], "Need the file.");
        assert_eq!(first[0]["signature"], "EqQBCg==");
        assert_eq!(first[1]["type"], "text");
        assert_eq!(first[2]["type"], "tool_use");
        assert_eq!(first[2]["id"], "toolu_01");
        assert_eq!(first[2]["input"]["path"], "a.rs");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_01");
        assert_eq!(messages[2]["content"][0]["content"], "contents");
        assert!(messages[2]["content"][0].get("is_error").is_none());
        let second = &messages[3]["content"];
        assert_eq!(second[0]["signature"], "EqQBCg==second");
        assert_eq!(second[1]["text"], "Summary.");
    }

    #[test]
    fn a_missing_tool_result_is_repaired() {
        let body = codec()
            .encode_body(
                &request(vec![
                    user_text("do it"),
                    call("toolu_01", "read", "not-json"),
                ]),
                true,
            )
            .unwrap();
        let result = &body["messages"][2]["content"][0];
        assert_eq!(result["type"], "tool_result");
        assert_eq!(result["is_error"], true);
        assert_eq!(result["content"], NO_RESULT);
        assert_eq!(
            body["messages"][1]["content"][0]["input"],
            serde_json::json!({})
        );
    }

    struct Scripted {
        status: u16,
        content_type: &'static str,
        body: String,
        declared_len: Option<usize>,
    }

    async fn serve(steps: Vec<Scripted>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
        let recorded = std::sync::Arc::clone(&requests);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            for step in steps {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 65536];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                let len = step.declared_len.unwrap_or(step.body.len());
                let response = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {ct}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
                    status = step.status,
                    ct = step.content_type,
                    body = step.body,
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        (format!("http://{address}/v1"), requests)
    }

    fn sse(events: &[Value]) -> String {
        events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect()
    }

    fn ok_sse() -> String {
        sse(&[
            serde_json::json!({"type":"message_start","message":{"id":"msg_01","model":"claude-sonnet-5-5","usage":{"input_tokens":100,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":1}}}),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Need the file."}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCg=="}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            serde_json::json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Reading."}}),
            serde_json::json!({"type":"content_block_stop","index":1}),
            serde_json::json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01","name":"read","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
            serde_json::json!({"type":"content_block_stop","index":2}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":30,"cache_read_input_tokens":40,"output_tokens_details":{"thinking_tokens":8}}}),
            serde_json::json!({"type":"message_stop"}),
        ])
    }

    fn text_sse(text: &str) -> String {
        sse(&[
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        ])
    }

    fn header_has(raw: &str, name: &str, value: &str) -> bool {
        raw.lines().any(|line| {
            let Some((header, got)) = line.split_once(':') else {
                return false;
            };
            header.eq_ignore_ascii_case(name) && got.trim() == value
        })
    }

    #[tokio::test]
    async fn stream_round_trip_keeps_the_signature_and_sends_the_version_header() {
        let (endpoint, requests) = serve(vec![Scripted {
            status: 200,
            content_type: "text/event-stream",
            body: ok_sse(),
            declared_len: None,
        }])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let collector = std::sync::Arc::clone(&seen);
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| collector.lock().unwrap().push(event)));
        let items = codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                on_event,
                &CancellationToken::new(),
            )
            .await
            .expect("stream");
        let raw = requests.lock().unwrap()[0].clone();
        assert!(
            header_has(&raw, "anthropic-version", ANTHROPIC_VERSION),
            "{raw}"
        );
        assert!(
            header_has(&raw, "anthropic-beta", THINKING_BINDING_BETA),
            "{raw}"
        );
        assert!(header_has(&raw, "authorization", "Bearer sk-test"), "{raw}");
        assert!(header_has(&raw, "x-opencode-session", "ses_1"), "{raw}");
        assert!(raw.contains("POST /v1/messages"), "{raw}");
        match &items[0] {
            Item::Reasoning(reasoning) => {
                assert_eq!(reasoning.encrypted_content.as_deref(), Some("EqQBCg=="));
            }
            other => panic!("expected reasoning, got {other:?}"),
        }
        match &items[2] {
            Item::FunctionCall(call) => {
                assert_eq!(call.call_id, "toolu_01");
                assert_eq!(call.arguments, "{}");
            }
            other => panic!("expected tool, got {other:?}"),
        }
        let events = seen.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ResponseStreamEvent::ResponseCompleted(completed)
                if completed.response.usage.as_ref().is_some_and(|usage| {
                    usage.input_tokens == 140 && usage.input_tokens_details.cached_tokens == 40
                })
        )));
    }

    #[tokio::test]
    async fn empty_stream_retries_until_a_terminal_arrives() {
        let (endpoint, requests) = serve(vec![
            Scripted {
                status: 200,
                content_type: "text/event-stream",
                body: String::new(),
                declared_len: None,
            },
            Scripted {
                status: 200,
                content_type: "text/event-stream",
                body: ok_sse(),
                declared_len: None,
            },
        ])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        let items = codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("retried empty stream");
        assert_eq!(requests.lock().unwrap().len(), 2);
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn disconnect_after_items_keeps_the_partial() {
        let (endpoint, requests) = serve(vec![Scripted {
            status: 200,
            content_type: "text/event-stream",
            body: text_sse("recover me"),
            declared_len: Some(4096),
        }])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        let error = codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("truncated");
        let LitecodeError::LlmStreamInterrupted { partial, .. } = error else {
            panic!("expected interrupted stream, got {error:?}");
        };
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(crate::types::item_text_preview(&partial[0]), "recover me");
    }

    #[tokio::test]
    async fn stream_error_before_items_retries_and_after_items_preserves() {
        let (endpoint, requests) = serve(vec![
            Scripted {
                status: 200,
                content_type: "text/event-stream",
                body: sse(&[serde_json::json!({"type":"error","error":{"type":"api_error","message":"boom"}})]),
                declared_len: None,
            },
            Scripted {
                status: 200,
                content_type: "text/event-stream",
                body: ok_sse(),
                declared_len: None,
            },
        ])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect("error before items retries");
        assert_eq!(requests.lock().unwrap().len(), 2);

        let (endpoint, requests) = serve(vec![Scripted {
            status: 200,
            content_type: "text/event-stream",
            body: format!(
                "{}{}",
                text_sse("partial"),
                sse(&[serde_json::json!({"type":"error","error":{"type":"api_error","message":"late"}})])
            ),
            declared_len: None,
        }])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        let error = codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                None,
                &CancellationToken::new(),
            )
            .await
            .expect_err("error after items");
        let LitecodeError::LlmStreamInterrupted { message, partial } = error else {
            panic!("expected partial, got {error:?}");
        };
        assert!(message.contains("api_error"), "{message}");
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(crate::types::item_text_preview(&partial[0]), "partial");
    }

    #[tokio::test]
    async fn cancel_after_a_delta_seals_incomplete() {
        let (endpoint, _) = serve(vec![Scripted {
            status: 200,
            content_type: "text/event-stream",
            body: text_sse("hello partial"),
            declared_len: None,
        }])
        .await;
        let codec = codec_at(&endpoint, REASONING);
        let cancel = CancellationToken::new();
        let cancel_in_callback = cancel.clone();
        let on_event: Option<Box<dyn FnMut(StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| {
                if matches!(event, ResponseStreamEvent::ResponseOutputTextDelta(_)) {
                    cancel_in_callback.cancel();
                }
            }));
        let items = codec
            .complete_with_stream_events(
                &request(vec![user_text("hi")]),
                "sk-test",
                on_event,
                &cancel,
            )
            .await
            .expect("cancel seals");
        match &items[0] {
            Item::Message(MessageItem::Output(message)) => {
                assert_eq!(message.status, OutputStatus::Incomplete);
                assert_eq!(crate::types::item_text_preview(&items[0]), "hello partial");
            }
            other => panic!("expected message, got {other:?}"),
        }
    }
}
