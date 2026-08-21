use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{Item, ResponseStreamEvent};
use crate::types::{LitecodeError, Result, StreamEvents};

use super::super::responses_sse::{
    SseLineReader, check_event_stream_content_type, sse_data_payload,
};
use super::super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::super::transport_error;
use super::decode::items_from_chat_message;
use super::stream::ChatSynth;

pub(crate) fn wrap_upstream(
    error_prefix: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> LitecodeError {
    LitecodeError::Llm(format!("{error_prefix}. HTTP {status}: {body}"))
}

pub(crate) async fn complete_from_response(
    resp: reqwest::Response,
    error_prefix: &str,
) -> Result<Vec<Item>> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(wrap_upstream(error_prefix, status, &text));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| transport_error("reading chat-completions response", &e))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| {
        LitecodeError::Llm(format!("{error_prefix}. not Chat JSON: {e}; body={text}"))
    })?;
    let message = value
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| LitecodeError::Llm(format!("{error_prefix}. missing choices[0].message")))?;
    Ok(items_from_chat_message(&message))
}

pub(crate) async fn stream_from_response<'a>(
    resp: reqwest::Response,
    model: &str,
    error_prefix: &str,
    mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
    cancel: &CancellationToken,
) -> Result<Vec<Item>> {
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(wrap_upstream(error_prefix, status, &text));
    }
    let resp = check_event_stream_content_type(resp).await?;

    let mut terminal_items: Option<Vec<Item>> = None;
    let mut reader = SseLineReader::new();
    let mut stream = resp.bytes_stream();
    let mut gate = StreamContractGate::new();
    let mut acc = StreamItemAccumulator::new();
    let mut synth = ChatSynth::new();
    let mut cancelled = cancel.is_cancelled();

    let mut forward_all = |events: Vec<ResponseStreamEvent>| -> Result<Option<Vec<Item>>> {
        let mut last = None;
        for event in events {
            if let Some(items) = forward_stream_event(&mut gate, &mut acc, event, &mut on_event)? {
                last = Some(items);
            }
        }
        Ok(last)
    };

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
                    transport_error("reading chat-completions event stream", &e)
                })?;
                for line in reader.feed(&chunk)? {
                    let Some(data) = sse_data_payload(&line) else {
                        continue;
                    };
                    if data.trim() == "[DONE]" {
                        continue;
                    }
                    let value: Value = serde_json::from_str(data).map_err(|e| {
                        LitecodeError::Llm(format!(
                            "{error_prefix}. Chat SSE JSON: {e}; payload={data}"
                        ))
                    })?;
                    let mut events = Vec::new();
                    synth.ingest_chunk(&value, &mut events);
                    if let Some(items) = forward_all(events)? {
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
        if let Some(line) = reader.finish()?
            && let Some(data) = sse_data_payload(&line)
            && data.trim() != "[DONE]"
        {
            let value: Value = serde_json::from_str(data).map_err(|e| {
                LitecodeError::Llm(format!(
                    "{error_prefix}. Chat SSE JSON: {e}; payload={data}"
                ))
            })?;
            let mut events = Vec::new();
            synth.ingest_chunk(&value, &mut events);
            if let Some(items) = forward_all(events)? {
                terminal_items = Some(items);
            }
        }
        if terminal_items.is_none()
            && let Some(items) = forward_all(synth.finish_events(model)?)?
        {
            terminal_items = Some(items);
        }
    }

    resolve_stream_outcome(terminal_items, &acc, cancelled)
}
