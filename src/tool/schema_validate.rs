//! Pipeline-level JSON Schema subset validation for tool arguments.
//!
//! No coercion. Semantic rules stay in [`crate::tool::Tool::validate_input`].

use serde_json::{Map, Value};

use crate::tool::trait_::Tool;

const PREVIEW_CHARS: usize = 40;

/// Parse tool `arguments` JSON. Invalid JSON is an error, never `Null`.
pub fn parse_tool_arguments(arguments: &str) -> Result<Value, String> {
    serde_json::from_str(arguments).map_err(|e| format!("arguments were not valid JSON ({e})"))
}

/// `invalid input for '{tool}': {detail}`
pub fn invalid_input_for(tool: &str, detail: impl AsRef<str>) -> String {
    format!("invalid input for '{tool}': {}", detail.as_ref())
}

pub fn missing_parameter(path: &str) -> String {
    format!("missing required parameter '{path}'")
}

pub fn must_be_nonempty_string(path: &str) -> String {
    format!("parameter '{path}' must be a non-empty string")
}

pub fn must_be(path: &str, constraint: &str) -> String {
    format!("parameter '{path}' must be {constraint}")
}

pub fn must_be_one_of(path: &str, allowed: &[&str], got: &str) -> String {
    let allowed: Vec<Value> = allowed.iter().map(|s| Value::String((*s).into())).collect();
    enum_error(path, &allowed, &Value::String(got.into()))
}

pub fn expected_type(path: &str, expected: &str, value: &Value) -> String {
    type_error(path, expected, value)
}

/// Top-level string field: missing vs wrong JSON type.
pub fn require_string<'a>(input: &'a Value, path: &str) -> Result<&'a str, String> {
    match input.get(path) {
        None => Err(missing_parameter(path)),
        Some(value) => require_string_value(value, path),
    }
}

pub fn require_string_value<'a>(value: &'a Value, path: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| type_error(path, "string", value))
}

pub fn require_nonempty_string<'a>(input: &'a Value, path: &str) -> Result<&'a str, String> {
    let s = require_string(input, path)?;
    if s.is_empty() {
        Err(must_be_nonempty_string(path))
    } else {
        Ok(s)
    }
}

pub fn require_nonempty_string_trimmed<'a>(
    input: &'a Value,
    path: &str,
) -> Result<&'a str, String> {
    let s = require_string(input, path)?;
    if s.trim().is_empty() {
        Err(must_be_nonempty_string(path))
    } else {
        Ok(s)
    }
}

/// Schema subset check, then the tool's semantic `validate_input`.
pub fn check_tool_input(tool: &dyn Tool, input: &Value) -> Result<(), String> {
    schema_validate(&tool.schema(), input)?;
    tool.validate_input(input)
}

/// Top-level keys present on `input` but absent from `schema.properties`.
///
/// Extra keys are allowed (the call still runs); callers may attach a Warning.
/// Unconstrained schemas and `additionalProperties: true` yield no names.
pub fn unknown_top_level_properties(schema: &Value, input: &Value) -> Vec<String> {
    if is_unconstrained(schema) {
        return Vec::new();
    }
    let Some(schema_obj) = schema.as_object() else {
        return Vec::new();
    };
    if schema_obj.get("additionalProperties") == Some(&Value::Bool(true)) {
        return Vec::new();
    }
    let Some(properties) = schema_obj.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let Some(map) = input.as_object() else {
        return Vec::new();
    };
    let mut extra: Vec<String> = map
        .keys()
        .filter(|key| !properties.contains_key(*key))
        .cloned()
        .collect();
    extra.sort();
    extra
}

/// Validate `input` against a JSON Schema subset from `tool.schema()`.
pub fn schema_validate(schema: &Value, input: &Value) -> Result<(), String> {
    if is_unconstrained(schema) {
        return Ok(());
    }
    validate_node(schema, input, "")
}

fn is_unconstrained(schema: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return true;
    };
    if obj.is_empty() {
        return true;
    }
    !obj.contains_key("type")
        && !obj.contains_key("enum")
        && !obj.contains_key("properties")
        && !obj.contains_key("items")
        && !obj.contains_key("required")
}

fn validate_node(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(schema_obj) = schema.as_object() else {
        return Ok(());
    };
    if is_unconstrained(schema) {
        return Ok(());
    }

    if let Some(enum_vals) = schema_obj.get("enum").and_then(Value::as_array)
        && !enum_vals.iter().any(|allowed| allowed == value)
    {
        return Err(enum_error(path, enum_vals, value));
    }

    if let Some(type_spec) = schema_obj.get("type") {
        match_type(type_spec, schema_obj, value, path)?;
    } else {
        if (schema_obj.contains_key("properties") || schema_obj.contains_key("required"))
            && let Value::Object(map) = value
        {
            validate_object(schema_obj, map, path)?;
        }
        if schema_obj.contains_key("items")
            && let Value::Array(items) = value
        {
            validate_array_items(schema_obj, items, path)?;
        }
    }
    Ok(())
}

fn match_type(
    type_spec: &Value,
    schema_obj: &Map<String, Value>,
    value: &Value,
    path: &str,
) -> Result<(), String> {
    let types = collect_types(type_spec);
    if types.is_empty() {
        return Ok(());
    }
    if types.iter().any(|t| type_matches(t, value)) {
        if types.contains(&"object")
            && let Value::Object(map) = value
        {
            validate_object(schema_obj, map, path)?;
        }
        if types.contains(&"array")
            && let Value::Array(items) = value
        {
            validate_array_items(schema_obj, items, path)?;
        }
        return Ok(());
    }
    Err(type_error(path, &expected_label(&types), value))
}

fn collect_types(type_spec: &Value) -> Vec<&str> {
    match type_spec {
        Value::String(s) => vec![s.as_str()],
        Value::Array(arr) => arr.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "integer" => is_json_integer(value),
        "number" => value.is_number(),
        _ => true,
    }
}

fn is_json_integer(value: &Value) -> bool {
    let Value::Number(n) = value else {
        return false;
    };
    if n.is_i64() || n.is_u64() {
        return true;
    }
    n.as_f64()
        .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
}

fn validate_object(
    schema_obj: &Map<String, Value>,
    map: &Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    if let Some(required) = schema_obj.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !map.contains_key(key) {
                return Err(missing_parameter(&child_path(path, key)));
            }
        }
    }
    let Some(properties) = schema_obj.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (key, prop_schema) in properties {
        let Some(child) = map.get(key) else {
            continue;
        };
        validate_node(prop_schema, child, &child_path(path, key))?;
    }
    Ok(())
}

fn validate_array_items(
    schema_obj: &Map<String, Value>,
    items: &[Value],
    path: &str,
) -> Result<(), String> {
    let Some(items_schema) = schema_obj.get("items") else {
        return Ok(());
    };
    match items_schema {
        Value::Array(tuple) => {
            for (i, (schema, value)) in tuple.iter().zip(items.iter()).enumerate() {
                validate_node(schema, value, &index_path(path, i))?;
            }
        }
        schema => {
            for (i, value) in items.iter().enumerate() {
                validate_node(schema, value, &index_path(path, i))?;
            }
        }
    }
    Ok(())
}

fn child_path(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn index_path(path: &str, i: usize) -> String {
    if path.is_empty() {
        format!("[{i}]")
    } else {
        format!("{path}[{i}]")
    }
}

fn type_error(path: &str, expected: &str, value: &Value) -> String {
    let got = got_clause(value);
    if path.is_empty() {
        format!("expected {expected}, got {got}")
    } else {
        format!("parameter '{path}' expected {expected}, got {got}")
    }
}

fn enum_error(path: &str, allowed: &[Value], value: &Value) -> String {
    let names = allowed
        .iter()
        .map(enum_token)
        .collect::<Vec<_>>()
        .join(", ");
    let got = got_clause(value);
    if path.is_empty() {
        format!("expected one of {names}, got {got}")
    } else {
        format!("parameter '{path}' expected one of {names}, got {got}")
    }
}

fn enum_token(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn expected_label(types: &[&str]) -> String {
    if types.len() == 1 {
        types[0].to_string()
    } else {
        types.join(" or ")
    }
}

fn got_clause(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("boolean {b}"),
        Value::Number(n) => {
            let kind = if is_json_integer(value) {
                "integer"
            } else {
                "number"
            };
            format!("{kind} {n}")
        }
        Value::String(s) => format!("string {}", preview_string(s)),
        Value::Array(_) => format!("array {}", truncate_json(value)),
        Value::Object(_) => format!("object {}", truncate_json(value)),
    }
}

fn preview_string(s: &str) -> String {
    truncate_json(&Value::String(s.to_string()))
}

fn truncate_json(value: &Value) -> String {
    let raw = value.to_string();
    if raw.chars().count() <= PREVIEW_CHARS {
        return raw;
    }
    let kept: String = raw.chars().take(PREVIEW_CHARS).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn err(schema: Value, input: Value) -> String {
        schema_validate(&schema, &input).unwrap_err()
    }

    #[test]
    fn empty_schema_accepts_anything() {
        assert!(schema_validate(&json!({}), &json!("x")).is_ok());
        assert!(schema_validate(&json!({}), &json!({"a": 1})).is_ok());
    }

    #[test]
    fn extra_keys_allowed() {
        let schema = json!({
            "type": "object",
            "properties": {"pattern": {"type": "string"}},
            "required": ["pattern"]
        });
        assert!(schema_validate(&schema, &json!({"pattern": "*.rs", "extra": true})).is_ok());
        assert_eq!(
            unknown_top_level_properties(&schema, &json!({"pattern": "*.rs", "extra": true})),
            vec!["extra".to_string()]
        );
        assert!(unknown_top_level_properties(&schema, &json!({"pattern": "*.rs"})).is_empty());
    }

    #[test]
    fn missing_required() {
        let schema = json!({
            "type": "object",
            "properties": {"pattern": {"type": "string"}},
            "required": ["pattern"]
        });
        assert_eq!(
            err(schema, json!({})),
            "missing required parameter 'pattern'"
        );
    }

    #[test]
    fn replace_all_string_is_type_error() {
        let schema = json!({
            "type": "object",
            "properties": {"replace_all": {"type": "boolean"}}
        });
        assert_eq!(
            err(schema, json!({"replace_all": "true"})),
            "parameter 'replace_all' expected boolean, got string \"true\""
        );
    }

    #[test]
    fn offset_string_is_type_error() {
        let schema = json!({
            "type": "object",
            "properties": {"offset": {"type": "integer"}}
        });
        assert_eq!(
            err(schema, json!({"offset": "12"})),
            "parameter 'offset' expected integer, got string \"12\""
        );
    }

    #[test]
    fn timeout_fraction_is_not_integer() {
        let schema = json!({
            "type": "object",
            "properties": {"timeout": {"type": "integer"}}
        });
        assert_eq!(
            err(schema, json!({"timeout": 1.5})),
            "parameter 'timeout' expected integer, got number 1.5"
        );
    }

    #[test]
    fn integer_accepted_as_number() {
        let schema = json!({
            "type": "object",
            "properties": {"n": {"type": "number"}}
        });
        assert!(schema_validate(&schema, &json!({"n": 3})).is_ok());
    }

    #[test]
    fn todos_status_enum() {
        let schema = json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": {"type": "string"},
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["content", "status"]
                    }
                }
            },
            "required": ["todos"]
        });
        assert_eq!(
            err(
                schema,
                json!({"todos": [{"content": "x", "status": "done"}]})
            ),
            "parameter 'todos[0].status' expected one of pending, in_progress, completed, got string \"done\""
        );
    }

    #[test]
    fn array_items_recursion_ok() {
        let schema = json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"content": {"type": "string"}},
                        "required": ["content"]
                    }
                }
            },
            "required": ["todos"]
        });
        assert!(
            schema_validate(
                &schema,
                &json!({"todos": [{"content": "a"}, {"content": "b"}]})
            )
            .is_ok()
        );
    }

    #[test]
    fn invalid_json_string() {
        let e = parse_tool_arguments("{not json").unwrap_err();
        assert!(e.starts_with("arguments were not valid JSON"), "{e}");
    }

    #[test]
    fn ref_only_property_accepts_any() {
        let schema = json!({
            "type": "object",
            "properties": {
                "payload": {"$ref": "#/definitions/Anything"}
            }
        });
        assert!(schema_validate(&schema, &json!({"payload": 1})).is_ok());
        assert!(schema_validate(&schema, &json!({"payload": {"a": true}})).is_ok());
    }

    #[test]
    fn string_or_null_union() {
        let schema = json!({
            "type": "object",
            "properties": {"note": {"type": ["string", "null"]}}
        });
        assert!(schema_validate(&schema, &json!({"note": null})).is_ok());
        assert!(schema_validate(&schema, &json!({"note": "hi"})).is_ok());
        assert_eq!(
            err(schema, json!({"note": 1})),
            "parameter 'note' expected string or null, got integer 1"
        );
    }

    #[test]
    fn null_vs_string_is_type_error_not_missing() {
        let schema = json!({
            "type": "object",
            "properties": {"file_path": {"type": "string"}},
            "required": ["file_path"]
        });
        assert_eq!(
            err(schema, json!({"file_path": null})),
            "parameter 'file_path' expected string, got null"
        );
    }

    #[test]
    fn root_not_object() {
        let schema = json!({
            "type": "object",
            "properties": {"pattern": {"type": "string"}}
        });
        assert_eq!(
            err(schema, json!("glob")),
            "expected object, got string \"glob\""
        );
    }

    #[test]
    fn invalid_input_for_prefix() {
        assert_eq!(
            invalid_input_for(
                "edit",
                "parameter 'replace_all' expected boolean, got string \"true\""
            ),
            "invalid input for 'edit': parameter 'replace_all' expected boolean, got string \"true\""
        );
    }

    #[test]
    fn empty_string_is_not_called_missing() {
        assert_eq!(
            require_nonempty_string(&json!({"file_path": ""}), "file_path").unwrap_err(),
            "parameter 'file_path' must be a non-empty string"
        );
        assert_eq!(
            require_string(&json!({}), "file_path").unwrap_err(),
            "missing required parameter 'file_path'"
        );
    }
}
