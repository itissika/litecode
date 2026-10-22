//! Anthropic usage → Responses usage.
//!
//! Anthropic's `input_tokens` excludes cache reads and cache writes. The
//! runtime treats `input_tokens` as the full prompt, so the three disjoint
//! counters are added back together. `message_delta.usage` is a cumulative
//! snapshot: a present field replaces the previous value, and a missing or
//! null field keeps it.

use serde_json::Value;

fn json_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    value
        .as_u64()
        .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
        .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
}

fn fit_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[derive(Debug, Default, Clone)]
pub(super) struct MessagesUsage {
    input: Option<u64>,
    cache_creation: Option<u64>,
    cache_read: Option<u64>,
    output: Option<u64>,
    thinking: Option<u64>,
}

impl MessagesUsage {
    pub(super) fn observe(&mut self, usage: &Value) {
        let set = |slot: &mut Option<u64>, key: &str| {
            if let Some(n) = json_u64(usage.get(key)) {
                *slot = Some(n);
            }
        };
        set(&mut self.input, "input_tokens");
        set(&mut self.cache_creation, "cache_creation_input_tokens");
        set(&mut self.cache_read, "cache_read_input_tokens");
        set(&mut self.output, "output_tokens");
        if let Some(n) = json_u64(usage.pointer("/output_tokens_details/thinking_tokens")) {
            self.thinking = Some(n);
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.input.is_none()
            && self.cache_creation.is_none()
            && self.cache_read.is_none()
            && self.output.is_none()
            && self.thinking.is_none()
    }

    /// Responses-shaped usage. `cached_tokens` is the cache read, not the write.
    pub(super) fn to_responses(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let input = self
            .input
            .unwrap_or(0)
            .saturating_add(self.cache_creation.unwrap_or(0))
            .saturating_add(self.cache_read.unwrap_or(0));
        let output = self.output.unwrap_or(0);
        Some(serde_json::json!({
            "input_tokens": fit_u32(input),
            "output_tokens": fit_u32(output),
            "total_tokens": fit_u32(input.saturating_add(output)),
            "input_tokens_details": { "cached_tokens": fit_u32(self.cache_read.unwrap_or(0)) },
            "output_tokens_details": { "reasoning_tokens": fit_u32(self.thinking.unwrap_or(0)) },
        }))
    }
}

#[cfg(test)]
mod tests {
    

}
