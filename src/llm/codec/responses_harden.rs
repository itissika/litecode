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

fn fill_typed_object(map: &mut Map<String, Value>, stream_event: Option<&str>) {
    let ty = map.get("type").and_then(Value::as_str).map(str::to_owned);
    if ty
        .as_deref()
        .is_some_and(|t| t.starts_with("response.") || t == "error")
    {
        map.entry("sequence_number")
            .or_insert_with(|| Value::from(0u64));
    }
    match ty.as_deref() {
        Some("reasoning") => {
            map.entry("summary")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        Some("function_call") => {
            coerce_json_text(map, "arguments");
            map.entry("arguments")
                .or_insert_with(|| Value::String(String::new()));
            if !map.contains_key("call_id") {
                let fallback = map
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                map.insert("call_id".into(), Value::String(fallback));
            }
            map.entry("name")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("response.function_call_arguments.delta") => {
            coerce_json_text(map, "delta");
            map.entry("delta")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("response.function_call_arguments.done") => {
            coerce_json_text(map, "arguments");
            map.entry("arguments")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("message") => {
            map.entry("content")
                .or_insert_with(|| Value::Array(Vec::new()));
            map.entry("role")
                .or_insert_with(|| Value::String("assistant".into()));
            let status = match stream_event {
                Some("response.completed") => "completed",
                Some("response.incomplete") => "incomplete",
                _ => "in_progress",
            };
            map.entry("status")
                .or_insert_with(|| Value::String(status.into()));
        }
        Some("summary_text") => {
            map.entry("text")
                .or_insert_with(|| Value::String(String::new()));
        }
        Some("output_text") => {
            map.entry("text")
                .or_insert_with(|| Value::String(String::new()));
            map.entry("annotations")
                .or_insert_with(|| Value::Array(Vec::new()));
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
    map.entry("output")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !map.contains_key("status") {
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
            details
                .entry("cached_tokens")
                .or_insert_with(|| Value::from(0u64));
        }
        if let Some(Value::Object(details)) = map.get_mut("output_tokens_details") {
            details
                .entry("reasoning_tokens")
                .or_insert_with(|| Value::from(0u64));
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
            map.entry("total_tokens")
                .or_insert_with(|| Value::from(input.saturating_add(output)));
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
    fn shell_fill_marks_completed_messages() {
        let mut event = json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "output": [{ "type": "message", "id": "m", "content": [] }]
            }
        });
        harden(&mut event, UsagePatch::None);
        assert_eq!(event["response"]["output"][0]["status"], "completed");
        assert_eq!(event["response"]["output"][0]["role"], "assistant");
    }

    #[test]
    fn fill_token_details_only_touches_existing_objects() {
        let mut value = json!({
            "usage": {
                "input_tokens": 2,
                "output_tokens": 1,
                "input_tokens_details": {}
            }
        });
        harden(&mut value, UsagePatch::FillEmptyTokenDetails);
        assert_eq!(value["usage"]["input_tokens_details"]["cached_tokens"], 0);
        assert!(value["usage"].get("output_tokens_details").is_none());
        assert!(value["usage"].get("total_tokens").is_none());
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
}
