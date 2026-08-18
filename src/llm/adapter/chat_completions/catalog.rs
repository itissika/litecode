use serde_json::Value;

use crate::types::{LitecodeError, Result};

pub(crate) fn normalize_endpoint(endpoint: String) -> String {
    endpoint.trim().trim_end_matches('/').to_string()
}

pub(crate) fn chat_post_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/chat/completions")
}

pub(crate) fn models_get_url(base: &str) -> String {
    format!("{}/models", base.trim_end_matches('/'))
}

pub(crate) fn parse_model_catalog(body: &str) -> Result<Vec<String>> {
    let value: Value = serde_json::from_str(body)
        .map_err(|e| LitecodeError::Llm(format!("model catalog is not JSON: {e}")))?;
    let Some(data) = value.get("data").and_then(Value::as_array) else {
        return Err(LitecodeError::Llm(
            "model catalog missing data array".into(),
        ));
    };
    Ok(data
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str))
        .map(|s| s.to_string())
        .collect())
}
