use serde_json::Value;

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
}

/// Chat Completions usage → Responses `usage` (what the ctx ring reads).
pub(crate) fn chat_usage_to_responses(usage: &Value) -> Option<Value> {
    let prompt =
        json_u64(usage.get("prompt_tokens")).or_else(|| json_u64(usage.get("input_tokens")))?;
    let completion = json_u64(usage.get("completion_tokens"))
        .or_else(|| json_u64(usage.get("output_tokens")))
        .unwrap_or(0);
    let cached = json_u64(usage.pointer("/prompt_tokens_details/cached_tokens"))
        .or_else(|| json_u64(usage.pointer("/input_tokens_details/cached_tokens")))
        .or_else(|| json_u64(usage.get("cached_tokens")))
        .or_else(|| json_u64(usage.get("prompt_cache_hit_tokens")))
        .unwrap_or(0);
    let reasoning = json_u64(usage.pointer("/completion_tokens_details/reasoning_tokens"))
        .or_else(|| json_u64(usage.pointer("/output_tokens_details/reasoning_tokens")))
        .unwrap_or(0);
    Some(serde_json::json!({
        "input_tokens": prompt,
        "output_tokens": completion,
        "total_tokens": prompt.saturating_add(completion),
        "input_tokens_details": { "cached_tokens": cached },
        "output_tokens_details": { "reasoning_tokens": reasoning },
    }))
}
