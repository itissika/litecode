use serde_json::Value;

use crate::types::Item;

/// Tool schema exposed to the model (product-level; wire encoding is adapter-private).
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Product-kernel model request — authority `Item` input only; no chat `messages[]`.
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub model: String,
    pub instructions: String,
    pub input: Vec<Item>,
    pub tools: Vec<ToolDef>,
    pub max_output_tokens: u32,
    pub temperature: f64,
    pub reasoning_effort: Option<String>,
    pub thinking_mode: Option<String>,
    pub json_output: bool,
}
