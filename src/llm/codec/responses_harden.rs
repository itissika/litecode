//! Named response repairs, applied before authority deserialization.
//!
//! Two layers:
//!
//! 1. **Shell fill** - every Responses payload is completed with the fields the
//!    authority types require but a vendor may omit on early events (output,
//!    status, reasoning summary, function-call arguments, annotations,
//!    `sequence_number`). Filling an absent required field is safe for every
//!    vendor, so it is a codec invariant rather than catalog data. Tool-call
//!    text (`arguments`, argument `delta`) is also coerced to a string when a
//!    gateway sends the parsed JSON value or null: the authority field is a
//!    string, and rejecting the event aborts the turn.
//! 2. **`usage_patch`** - vendor-specific repairs, one named value per
//!    implemented algorithm.

use serde_json::{Map, Value};

use crate::provider_catalog::UsagePatch;

/// Complete one Responses SSE payload in place.
pub(crate) fn harden(value: &mut Value, patch: UsagePatch) {
    fill_shells(value);
    match patch {
        UsagePatch::None => {}
        UsagePatch::FillEmptyTokenDetails => fill_token_details(value),
        UsagePatch::MapMaxEffortToXhigh => {
            fill_token_details(value);
            fill_total_tokens(value);
            normalize_max_effort(value);
        }
    }
}

// ── shell fill ───────────────────────────────────────────────────────────────

/// Vendors omit authority-required members on early events. Derive the event
/// context from `type`, falling back to a top-level `status`.
fn fill_shells(value: &mut Value) {
    let hint = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| match value.get("status").and_then(Value::as_str) {
            Some("completed") => Some("response.completed".into()),
            Some("incomplete") => Some("response.incomplete".into()),
            _ => None,
        });
    fill_shell_value(value, hint.as_deref());
}

fn fill_shell_value(value: &mut Value, event_type: Option<&str>) {
    match value {
        Value::Object(map) => {
            let local = map.get("type").and_then(Value::as_str).map(str::to_owned);
            let event = local.as_deref().or(event_type);
            fill_typed_object(map, event_type);
            if let Some(Value::Object(response)) = map.get_mut("response") {
                fill_response_object(response, event);
            }
            if map.get("object").and_then(Value::as_str) == Some("response") {
                fill_response_object(map, event);
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    fill_shell_value(child, event);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                fill_shell_value(child, event_type);
            }
        }
        _ => {}
    }
}

fn ensure_array(map: &mut Map<String, Value>, key: &str) {
    if matches!(map.get(key), None | Some(Value::Null)) {
        map.insert(key.to_string(), Value::Array(Vec::new()));
    }
}

fn ensure_string(map: &mut Map<String, Value>, key: &str, default: impl Into<String>) {
    if matches!(map.get(key), None | Some(Value::Null)) {
        map.insert(key.to_string(), Value::String(default.into()));
    }
}

fn ensure_u64(map: &mut Map<String, Value>, key: &str, default: u64) {
    if matches!(map.get(key), None | Some(Value::Null)) {
        map.insert(key.to_string(), Value::from(default));
    }
}

fn fill_typed_object(map: &mut Map<String, Value>, stream_event: Option<&str>) {
    let ty = map.get("type").and_then(Value::as_str).map(str::to_owned);
    if ty
        .as_deref()
        .is_some_and(|t| t.starts_with("response.") || t == "error")
    {
        ensure_u64(map, "sequence_number", 0);
    }
    match ty.as_deref() {
        Some("reasoning") => {
            ensure_array(map, "summary");
        }
        Some("function_call") => {
            coerce_json_text(map, "arguments");
            ensure_string(map, "arguments", "");
            if matches!(map.get("call_id"), None | Some(Value::Null)) {
                let fallback = map
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                map.insert("call_id".into(), Value::String(fallback));
            }
            ensure_string(map, "name", "");
        }
        Some("response.function_call_arguments.delta") => {
            coerce_json_text(map, "delta");
            ensure_string(map, "delta", "");
        }
        Some("response.function_call_arguments.done") => {
            coerce_json_text(map, "arguments");
            ensure_string(map, "arguments", "");
        }
        Some("message") => {
            ensure_array(map, "content");
            ensure_string(map, "role", "assistant");
            let status = match stream_event {
                Some("response.completed") => "completed",
                Some("response.incomplete") => "incomplete",
                _ => "in_progress",
            };
            ensure_string(map, "status", status);
        }
        Some("summary_text") => {
            ensure_string(map, "text", "");
        }
        Some("output_text") => {
            ensure_string(map, "text", "");
            ensure_array(map, "annotations");
        }
        _ => {}
    }
}

/// The authority tool-call fields are strings of JSON. A gateway that already
/// parsed the value, or that sends null, is the same payload.
fn coerce_json_text(map: &mut Map<String, Value>, key: &str) {
    let text = match map.get(key) {
        None | Some(Value::String(_)) => return,
        Some(Value::Null) => String::new(),
        Some(other) => other.to_string(),
    };
    map.insert(key.to_string(), Value::String(text));
}

fn fill_response_object(map: &mut Map<String, Value>, event_type: Option<&str>) {
    ensure_array(map, "output");
    if matches!(map.get("status"), None | Some(Value::Null)) {
        let status = match event_type {
            Some("response.completed") => "completed",
            Some("response.incomplete") => "incomplete",
            Some("response.failed") => "failed",
            Some("response.cancelled") => "cancelled",
            _ => "in_progress",
        };
        map.insert("status".into(), Value::String(status.into()));
    }
}

// ── named usage patches ──────────────────────────────────────────────────────

/// Fill missing members of an existing `*_tokens_details` object.
fn fill_token_details(value: &mut Value) {
    walk(value, &mut |map| {
        if let Some(Value::Object(details)) = map.get_mut("input_tokens_details") {
            ensure_u64(details, "cached_tokens", 0);
        }
        if let Some(Value::Object(details)) = map.get_mut("output_tokens_details") {
            ensure_u64(details, "reasoning_tokens", 0);
        }
    });
}

/// Derive `total_tokens` when the vendor sends only the two halves.
fn fill_total_tokens(value: &mut Value) {
    walk(value, &mut |map| {
        if map.contains_key("input_tokens") && map.contains_key("output_tokens") {
            let input = map.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            let output = map
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            ensure_u64(map, "total_tokens", input.saturating_add(output));
        }
    });
}

/// The authority reasoning-effort enum has no `max`, but the catalog sends that
/// literal for the top tier of several vendors, so the response echo of it must
/// still be readable.
fn normalize_max_effort(value: &mut Value) {
    walk(value, &mut |map| {
        if matches!(map.get("effort"), Some(Value::String(s)) if s == "max") {
            map.insert("effort".into(), Value::String("xhigh".into()));
        }
    });
}

fn walk(value: &mut Value, visit: &mut impl FnMut(&mut Map<String, Value>)) {
    match value {
        Value::Object(map) => {
            visit(map);
            for child in map.values_mut() {
                walk(child, visit);
            }
        }
        Value::Array(items) => {
            for child in items {
                walk(child, visit);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deepseek_patch_also_fills_total_and_effort() {
        let mut value = json!({
            "usage": { "input_tokens": 2, "output_tokens": 1 },
            "output": [{ "type": "reasoning", "effort": "max" }]
        });
        harden(&mut value, UsagePatch::MapMaxEffortToXhigh);
        assert_eq!(value["usage"]["total_tokens"], 3);
        assert_eq!(value["output"][0]["effort"], "xhigh");
        assert_eq!(value["output"][0]["summary"], json!([]));
    }

    #[test]
    fn tool_call_object_arguments_become_a_json_string() {
        let mut added = json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "bash",
                "arguments": { "command": "ls" }
            }
        });
        harden(&mut added, UsagePatch::None);
        assert_eq!(added["sequence_number"], 0);
        assert_eq!(added["item"]["arguments"], "{\"command\":\"ls\"}");

        let mut delta = json!({
            "type": "response.function_call_arguments.delta",
            "item_id": "fc_1",
            "output_index": 1,
            "delta": { "command": "ls" }
        });
        harden(&mut delta, UsagePatch::None);
        assert_eq!(delta["delta"], "{\"command\":\"ls\"}");

        let mut done = json!({
            "type": "response.function_call_arguments.done",
            "item_id": "fc_1",
            "output_index": 1,
            "arguments": null
        });
        harden(&mut done, UsagePatch::None);
        assert_eq!(done["arguments"], "");
        assert_eq!(done["sequence_number"], 0);
    }

    #[test]
    fn tool_call_string_arguments_stay_verbatim() {
        let mut event = json!({
            "type": "response.function_call_arguments.delta",
            "sequence_number": 4,
            "item_id": "fc_1",
            "output_index": 1,
            "delta": "{\"command\":"
        });
        harden(&mut event, UsagePatch::None);
        assert_eq!(event["delta"], "{\"command\":");
        assert_eq!(event["sequence_number"], 4);
    }

    #[test]
    fn response_created_with_null_output_deserializes_to_stream_event() {
        let mut event = json!({
            "type": "response.created",
            "sequence_number": null,
            "response": {
                "id": "resp_1",
                "object": "response",
                "created_at": 1,
                "model": "m",
                "status": null,
                "output": null
            }
        });
        harden(&mut event, UsagePatch::None);
        assert_eq!(event["sequence_number"], 0);
        assert_eq!(event["response"]["output"], json!([]));
        assert_eq!(event["response"]["status"], "in_progress");
        let parsed: crate::authority::responses::ResponseStreamEvent =
            serde_json::from_value(event).expect("response.created with null output must parse");
        match parsed {
            crate::authority::responses::ResponseStreamEvent::ResponseCreated(ev) => {
                assert!(ev.response.output.is_empty());
            }
            other => panic!("expected ResponseCreated, got {other:?}"),
        }
    }

    #[test]
    fn null_required_shell_fields_are_filled_with_defaults() {
        let mut event = json!({
            "type": "response.completed",
            "response": {
                "id": "resp_2",
                "object": "response",
                "status": null,
                "output": [
                    { "type": "reasoning", "summary": null },
                    {
                        "type": "message",
                        "role": null,
                        "status": null,
                        "content": [{ "type": "output_text", "text": null, "annotations": null }]
                    },
                    { "type": "function_call", "id": "fc_9", "call_id": null, "name": null, "arguments": null }
                ],
                "usage": {
                    "input_tokens": 5,
                    "output_tokens": 7,
                    "total_tokens": null,
                    "input_tokens_details": { "cached_tokens": null },
                    "output_tokens_details": { "reasoning_tokens": null }
                }
            }
        });
        harden(&mut event, UsagePatch::MapMaxEffortToXhigh);
        assert_eq!(event["response"]["status"], "completed");
        assert_eq!(event["response"]["output"][0]["summary"], json!([]));
        assert_eq!(event["response"]["output"][1]["role"], "assistant");
        assert_eq!(event["response"]["output"][1]["status"], "completed");
        assert_eq!(event["response"]["output"][1]["content"][0]["text"], "");
        assert_eq!(event["response"]["output"][1]["content"][0]["annotations"], json!([]));
        assert_eq!(event["response"]["output"][2]["call_id"], "fc_9");
        assert_eq!(event["response"]["output"][2]["name"], "");
        assert_eq!(event["response"]["output"][2]["arguments"], "");
        assert_eq!(event["response"]["usage"]["total_tokens"], 12);
        assert_eq!(event["response"]["usage"]["input_tokens_details"]["cached_tokens"], 0);
        assert_eq!(event["response"]["usage"]["output_tokens_details"]["reasoning_tokens"], 0);
    }
}
