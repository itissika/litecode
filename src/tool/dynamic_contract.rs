//! Runtime-defined custom code tools.
//!
//! Built-in code tools have hand-authored TS declarations in `sdk/`. Custom
//! tools are defined at runtime, so this module is the only schema-driven SDK
//! fallback and must stay isolated from the built-in contract.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::config::schema::{CallSurface, CustomToolDefinition};
use crate::tool::DynamicCodeSpec;
use crate::tool::code_contract::SdkToolFragment;
use crate::tool::custom_spec::custom_input_schema;

pub struct DynamicTool {
    pub fragment: SdkToolFragment,
    pub types: BTreeMap<String, String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

const ALLOWED_SCHEMA_KEYS: &[&str] = &[
    "type",
    "description",
    "title",
    "default",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "minItems",
    "maxItems",
    "uniqueItems",
    "enum",
    "const",
    "properties",
    "required",
    "items",
    "additionalProperties",
    "oneOf",
    "anyOf",
];

fn schema_type_names(schema: &Map<String, Value>) -> Result<Vec<String>, String> {
    match schema.get("type") {
        None => Ok(Vec::new()),
        Some(Value::String(kind)) => Ok(vec![kind.clone()]),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .map(|kind| {
                kind.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "schema type array must contain only strings".to_string())
            })
            .collect(),
        Some(_) => Err("schema type must be a string or string array".into()),
    }
}

fn validate_supported_schema(schema: &Value, path: &str) -> Result<(), String> {
    let Some(object) = schema.as_object() else {
        return Err(format!("{path}: schema must be an object"));
    };
    for key in object.keys() {
        if !ALLOWED_SCHEMA_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "{path}: unsupported schema keyword '{key}' (no $ref/allOf/not/if)"
            ));
        }
    }
    for kind in schema_type_names(object)? {
        if !matches!(
            kind.as_str(),
            "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
        ) {
            return Err(format!("{path}: unsupported schema type '{kind}'"));
        }
    }
    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return Err(format!("{path}: required must be an array"));
        };
        if required.iter().any(|item| !item.is_string()) {
            return Err(format!("{path}: required must contain only strings"));
        }
    }
    if let Some(properties) = object.get("properties") {
        let Some(properties) = properties.as_object() else {
            return Err(format!("{path}: properties must be an object"));
        };
        for (name, child) in properties {
            validate_supported_schema(child, &format!("{path}.{name}"))?;
        }
    }
    if let Some(items) = object.get("items") {
        validate_supported_schema(items, &format!("{path}[]"))?;
    }
    if let Some(additional) = object.get("additionalProperties")
        && !additional.is_boolean()
    {
        return Err(format!(
            "{path}: additionalProperties must be boolean in a custom code schema"
        ));
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = object.get(key) {
            let Some(variants) = variants.as_array() else {
                return Err(format!("{path}: {key} must be an array"));
            };
            if variants.is_empty() {
                return Err(format!("{path}: {key} must not be empty"));
            }
            for (index, variant) in variants.iter().enumerate() {
                validate_supported_schema(variant, &format!("{path}.{key}[{index}]"))?;
            }
        }
    }
    Ok(())
}

fn root_is_object(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    match object.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("object")),
        None => object.contains_key("properties"),
        _ => false,
    }
}

fn strip_ok_field(mut schema: Value) -> Value {
    if let Value::Object(object) = &mut schema {
        if let Some(Value::Object(properties)) = object.get_mut("properties") {
            properties.remove("ok");
        }
        if let Some(Value::Array(required)) = object.get_mut("required") {
            required.retain(|item| item.as_str() != Some("ok"));
        }
    }
    schema
}

fn ensure_ok_field(schema: Value) -> Value {
    let Value::Object(mut object) = schema else {
        return schema;
    };
    let Value::Object(properties) = object
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()))
    else {
        return Value::Object(object);
    };
    properties.insert(
        "ok".into(),
        serde_json::json!({
            "type": "boolean",
            "const": true,
            "description": "The host adds ok:true to a successful result."
        }),
    );
    let required = object
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(required) = required
        && !required.iter().any(|item| item.as_str() == Some("ok"))
    {
        required.insert(0, Value::String("ok".into()));
    }
    Value::Object(object)
}

/// Settings/admission-time validation for a custom code tool.
pub fn validate_custom_code_contract(
    def: &CustomToolDefinition,
    surface: CallSurface,
) -> Result<(), String> {
    if surface != CallSurface::Code {
        return Ok(());
    }
    let input = custom_input_schema(def);
    let output = def
        .output_schema
        .clone()
        .ok_or_else(|| "code custom tool must declare output_schema".to_string())?;
    validate_supported_schema(&input, "input")?;
    validate_supported_schema(&output, "output")?;
    if !root_is_object(&input) {
        return Err("custom code input schema must describe an object".into());
    }
    if !root_is_object(&output) {
        return Err("custom code output schema must describe an object".into());
    }
    Ok(())
}

fn pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            upper = true;
        }
    }
    if out.is_empty() { "Tool".into() } else { out }
}

fn property_key(name: &str) -> String {
    let valid = name.chars().enumerate().all(|(index, ch)| {
        (ch.is_ascii_alphabetic() || ch == '_' || ch == '$')
            || (index > 0 && (ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'))
    });
    if valid {
        name.to_string()
    } else {
        serde_json::to_string(name).unwrap_or_else(|_| "\"invalid\"".into())
    }
}

fn literal_text(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(_) | Value::Object(_) => "unknown".into(),
    }
}

fn render_description(schema: &Map<String, Value>, indent: usize) -> String {
    let Some(description) = schema.get("description").and_then(Value::as_str) else {
        return String::new();
    };
    let description = description.trim();
    if description.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(indent);
    let mut out = format!("{pad}/**\n");
    for line in description.lines() {
        out.push_str(&format!("{pad} * {}\n", line.trim()));
    }
    out.push_str(&format!("{pad} */\n"));
    out
}

fn schema_to_ts(schema: &Value, indent: usize) -> String {
    let Some(object) = schema.as_object() else {
        return "unknown".into();
    };
    if let Some(value) = object.get("const") {
        return literal_text(value);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .map(literal_text)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = object.get(key).and_then(Value::as_array) {
            let rendered = variants
                .iter()
                .map(|variant| schema_to_ts(variant, indent))
                .collect::<Vec<_>>()
                .join(" | ");
            return format!("({rendered})");
        }
    }
    let kinds: Vec<&str> = match object.get("type") {
        Some(Value::String(kind)) => vec![kind.as_str()],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        None if object.contains_key("properties") => vec!["object"],
        None if object.contains_key("items") => vec!["array"],
        None => vec!["unknown"],
        Some(_) => vec!["unknown"],
    };
    let mut rendered = Vec::new();
    for kind in kinds {
        let part = match kind {
            "string" => "string".to_string(),
            "number" | "integer" => "number".to_string(),
            "boolean" => "boolean".to_string(),
            "null" => "null".to_string(),
            "array" => {
                let item = schema_to_ts(object.get("items").unwrap_or(&Value::Null), indent);
                format!("Array<{item}>")
            }
            "object" => render_object_type(object, indent),
            _ => "unknown".to_string(),
        };
        rendered.push(part);
    }
    if rendered.len() == 1 {
        rendered.remove(0)
    } else {
        format!("({})", rendered.join(" | "))
    }
}

fn render_object_type(object: &Map<String, Value>, indent: usize) -> String {
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: BTreeSet<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut out = String::from("{\n");
    let child_indent = indent + 2;
    let pad = " ".repeat(child_indent);
    let mut names: Vec<&String> = properties.keys().collect();
    names.sort();
    for name in names {
        let child = &properties[name];
        out.push_str(&render_description(
            child.as_object().unwrap_or(&Map::new()),
            child_indent,
        ));
        out.push_str(&pad);
        out.push_str(&property_key(name));
        if !required.contains(name.as_str()) {
            out.push('?');
        }
        out.push_str(": ");
        out.push_str(&schema_to_ts(child, child_indent));
        out.push_str(";\n");
    }
    if object.get("additionalProperties").and_then(Value::as_bool) == Some(true) {
        out.push_str(&pad);
        out.push_str("[key: string]: unknown;\n");
    }
    if out == "{\n" {
        out.push('}');
    } else {
        out.push_str(&" ".repeat(indent));
        out.push('}');
    }
    out
}

fn select_type_names(name: &str, reserved: &BTreeSet<String>) -> (String, String) {
    let base = format!("Custom{}", pascal_case(name));
    for suffix in 0..1000 {
        let stem = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}{suffix}")
        };
        let input = format!("{stem}Input");
        let data = format!("{stem}Data");
        if !reserved.contains(&input) && !reserved.contains(&data) {
            return (input, data);
        }
    }
    (
        format!("{base}FallbackInput"),
        format!("{base}FallbackData"),
    )
}

/// Build the isolated JS fragment and schemas for one runtime custom tool.
pub fn build_dynamic_tool(
    name: &str,
    spec: DynamicCodeSpec,
    reserved_types: &BTreeSet<String>,
) -> Result<DynamicTool, String> {
    validate_supported_schema(&spec.input_schema, "input")?;
    validate_supported_schema(&spec.output_schema, "output")?;
    if !root_is_object(&spec.input_schema) {
        return Err(format!(
            "custom code tool '{name}' input schema must describe an object"
        ));
    }
    if !root_is_object(&spec.output_schema) {
        return Err(format!(
            "custom code tool '{name}' output schema must describe an object"
        ));
    }
    let (input_type, data_type) = select_type_names(name, reserved_types);
    let payload_output_schema = strip_ok_field(spec.output_schema);
    let output_schema = ensure_ok_field(payload_output_schema.clone());
    let mut types = BTreeMap::new();
    types.insert(
        input_type.clone(),
        format!(
            "type {input_type} = {};",
            schema_to_ts(&spec.input_schema, 0)
        ),
    );
    types.insert(
        data_type.clone(),
        format!(
            "type {data_type} = {};",
            schema_to_ts(&payload_output_schema, 0)
        ),
    );
    let path: Vec<String> = name.split('.').map(str::to_string).collect();
    let method = path
        .last()
        .cloned()
        .ok_or_else(|| "custom code tool name is empty".to_string())?;
    let suite = if path.len() > 1 {
        path[0].clone()
    } else {
        "custom".into()
    };
    let fragment = SdkToolFragment {
        path,
        suite,
        doc: spec.description,
        signature: format!("{method}(args: {input_type}): Promise<ToolResult<{data_type}>>;"),
        type_deps: vec![input_type, data_type],
    };
    Ok(DynamicTool {
        fragment,
        types,
        input_schema: spec.input_schema,
        output_schema,
    })
}
