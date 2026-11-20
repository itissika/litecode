//! Custom-tool declaration helpers shared by runtime admission and settings.
//!
//! Built-in code tools do not use this module. Their public contract lives in
//! `sdk/` and the generated tool contract.

use serde_json::Value;

use crate::config::schema::CustomToolDefinition;

/// Code names are canonical: a custom suite member is `suite.member`.
pub fn custom_function_name(def: &CustomToolDefinition) -> String {
    match def.suite.as_deref().map(str::trim) {
        Some(suite) if !suite.is_empty() => format!("{suite}.{}", def.name),
        _ => def.name.clone(),
    }
}

/// Input shape of a custom function inside `run_code`.
pub(crate) fn custom_input_schema(config: &CustomToolDefinition) -> Value {
    config.to_json_schema()
}

pub(crate) fn custom_description(config: &CustomToolDefinition) -> String {
    let trimmed = config.description.trim();
    if trimmed.is_empty() {
        format!("External tool: {}", config.name)
    } else {
        trimmed.to_string()
    }
}
