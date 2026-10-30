//! Custom-tool declaration helpers shared by runtime admission and settings.
//!
//! Built-in code tools do not use this module. Their public contract lives in
//! `sdk/` and the generated tool contract.

use serde_json::Value;

use crate::config::schema::{CallSurface, CustomToolDefinition};

/// Code names are canonical: a code suite member is `suite.member` even when
/// the suite has one member. Direct suite members stay flat.
pub fn custom_function_name(def: &CustomToolDefinition) -> String {
    match (def.effective_surface(), def.suite.as_deref().map(str::trim)) {
        (CallSurface::Code, Some(suite)) if !suite.is_empty() => {
            format!("{suite}.{}", def.name)
        }
        _ => def.name.clone(),
    }
}

/// Input shape of a custom function. Direct tools keep the background knob;
/// code calls are synchronous and cannot start a job.
pub(crate) fn custom_input_schema(config: &CustomToolDefinition) -> Value {
    let mut schema = config.to_json_schema();
    if config.effective_surface() == CallSurface::Direct
        && let Some(obj) = schema.as_object_mut()
    {
        let props = obj
            .entry("properties")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut();
        if let Some(props) = props {
            props.insert(
                "run_in_background".into(),
                serde_json::json!({
                    "type": "boolean",
                    "description": "When true, start as a background job and return job_id immediately; result is delivered later via CustomToolSettled mailbox (default false)."
                }),
            );
        }
    }
    schema
}

pub(crate) fn custom_description(config: &CustomToolDefinition) -> String {
    let trimmed = config.description.trim();
    if trimmed.is_empty() {
        format!("External tool: {}", config.name)
    } else {
        trimmed.to_string()
    }
}
