use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{Item, ResponseStreamEvent};
use crate::types::{LitecodeError, Result, StreamEvents};

use super::super::interrupted_stream_error;
use super::super::responses_sse::{
    SseLineReader, check_event_stream_content_type, sse_data_payload,
};
use super::super::stream_contract::{
    StreamContractGate, StreamItemAccumulator, forward_stream_event, resolve_stream_outcome,
};
use super::stream::ChatSynth;

pub(crate) fn wrap_upstream(
    error_prefix: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> LitecodeError {
    LitecodeError::Llm(format!("{error_prefix}. HTTP {status}: {body}"))
}

fn forward_all<'a>(
    events: Vec<ResponseStreamEvent>,
    gate: &mut StreamContractGate,
    acc: &mut StreamItemAccumulator,
    on_event: &mut Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
) -> Result<Option<Vec<Item>>> {
    let mut last = None;
    for event in events {
        if let Some(items) = forward_stream_event(gate, acc, event, on_event)? {
            last = Some(items);
        }
    }
    Ok(last)
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
                    interrupted_stream_error(
                        "reading chat-completions event stream",
                        &e,
                        &acc,
                    )
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
                    if let Some(items) =
                        forward_all(events, &mut gate, &mut acc, &mut on_event)?
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
            if let Some(items) = forward_all(events, &mut gate, &mut acc, &mut on_event)? {
                terminal_items = Some(items);
            }
        }
        if terminal_items.is_none()
            && let Some(items) = forward_all(
                synth.finish_events(model)?,
                &mut gate,
                &mut acc,
                &mut on_event,
            )?
        {
            terminal_items = Some(items);
        }
    }

    resolve_stream_outcome(terminal_items, &acc, cancelled)
}
