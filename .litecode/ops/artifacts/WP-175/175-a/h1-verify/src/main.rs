//! Read-only replay of ResponsesCodec::parse_stream_event semantics for WP-175-a.
//! Mirrors: keepalive skip → responses_harden::harden(MapMaxEffortToXhigh) → ResponseStreamEvent deserialize.

use async_openai::types::responses::ResponseStreamEvent;
use serde_json::Value;

fn usage_patch_harden(value: &mut Value) {
    fill_shells(value);
    fill_token_details(value);
    fill_total_tokens(value);
    normalize_max_effort(value);
}

fn fill_shells(value: &mut Value) {
    walk(value, &mut |map| {
        if map.get("type").and_then(Value::as_str) == Some("reasoning") {
            map.entry("summary")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
        if map.get("type").and_then(Value::as_str) == Some("message") {
            map.entry("content")
                .or_insert_with(|| Value::Array(Vec::new()));
            map.entry("role")
                .or_insert_with(|| Value::String("assistant".into()));
        }
        if let Some(response) = map.get_mut("response").and_then(Value::as_object_mut) {
            response
                .entry("output")
                .or_insert_with(|| Value::Array(Vec::new()));
        }
    });
}

fn fill_token_details(value: &mut Value) {
    walk(value, &mut |map| {
        if let Some(details) = map.get_mut("input_tokens_details").and_then(Value::as_object_mut)
        {
            details
                .entry("cached_tokens")
                .or_insert_with(|| Value::from(0u64));
        }
        if let Some(details) = map.get_mut("output_tokens_details").and_then(Value::as_object_mut)
        {
            details
                .entry("reasoning_tokens")
                .or_insert_with(|| Value::from(0u64));
        }
    });
}

fn fill_total_tokens(value: &mut Value) {
    walk(value, &mut |map| {
        if map.contains_key("input_tokens") && map.contains_key("output_tokens") {
            let input = map.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
            let output = map.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
            map.entry("total_tokens")
                .or_insert_with(|| Value::from(input.saturating_add(output)));
        }
    });
}

fn normalize_max_effort(value: &mut Value) {
    walk(value, &mut |map| {
        if matches!(map.get("effort"), Some(Value::String(s)) if s == "max") {
            map.insert("effort".into(), Value::String("xhigh".into()));
        }
    });
}

fn walk(value: &mut Value, visit: &mut impl FnMut(&mut serde_json::Map<String, Value>)) {
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

fn parse_like_codec(data: &str) -> Result<Option<ResponseStreamEvent>, String> {
    let mut value: Value =
        serde_json::from_str(data).map_err(|e| format!("JSON: {e}; payload={data}"))?;
    if value.get("type").and_then(Value::as_str) == Some("keepalive") {
        return Ok(None);
    }
    usage_patch_harden(&mut value);
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| format!("ResponseStreamEvent: {e}; payload={data}"))
}

fn main() {
    let cases = [
        ("keepalive", r#"{"type":"keepalive","sequence_number":1}"#),
        (
            "unknown_type",
            r#"{"type":"response.vendor_telemetry","sequence_number":99}"#,
        ),
        (
            "completed_usage_no_details",
            include_str!("../cases/completed_usage_no_details.json"),
        ),
        (
            "completed_usage_with_details",
            include_str!("../cases/completed_usage_with_details.json"),
        ),
    ];

    for (name, payload) in cases {
        match parse_like_codec(payload) {
            Ok(None) => println!("{name}: OK (skipped keepalive)"),
            Ok(Some(_)) => println!("{name}: OK (parsed event)"),
            Err(e) => println!("{name}: ERR {e}"),
        }
    }
}
