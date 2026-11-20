//! Runtime-defined custom code tools.
//!
//! Built-in code tools have hand-authored TS declarations in `sdk/`. Custom
//! tools are defined at runtime, so this module is the only schema-driven SDK
//! fallback and must stay isolated from the built-in contract.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::config::schema::CustomToolDefinition;
use crate::tool::DynamicCodeSpec;
use crate::tool::code_contract::SdkToolFragment;
use crate::tool::custom_spec::custom_input_schema;

pub struct DynamicTool {
    pub fragment: SdkToolFragment,
    pub types: BTreeMap<String, String>,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
}

const MAX_SCHEMA_DEPTH: usize = 6;
const MAX_SCHEMA_NODES: usize = 64;

struct LowerCtx<'a> {
    root: &'a Value,
    visiting: Vec<String>,
    remaining_nodes: usize,
}

impl<'a> LowerCtx<'a> {
    fn new(root: &'a Value) -> Self {
        Self {
            root,
            visiting: Vec::new(),
            remaining_nodes: MAX_SCHEMA_NODES,
        }
    }

    fn take_budget(&mut self) -> bool {
        if self.remaining_nodes == 0 {
            return false;
        }
        self.remaining_nodes -= 1;
        true
    }
}

fn resolve_local_ref<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    if reference == "#" {
        return Some(root);
    }
    let pointer = reference.strip_prefix("#/")?;
    let mut current = root;
    for raw_seg in pointer.split('/') {
        let seg = raw_seg.replace("~1", "/").replace("~0", "~");
        current = current.get(&seg)?;
    }
    Some(current)
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

/// Settings/admission-time validation for a custom code tool.
pub fn validate_custom_code_contract(def: &CustomToolDefinition) -> Result<(), String> {
    let input = custom_input_schema(def);
    if !root_is_object(&input) {
        return Err("custom tool input schema must describe an object".into());
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
    let valid = !name.is_empty()
        && name.chars().enumerate().all(|(index, ch)| {
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
    let mut ctx = LowerCtx::new(schema);
    schema_to_ts_inner(schema, indent, 0, &mut ctx)
}

fn schema_to_ts_inner(
    schema: &Value,
    indent: usize,
    depth: usize,
    ctx: &mut LowerCtx<'_>,
) -> String {
    if depth >= MAX_SCHEMA_DEPTH || !ctx.take_budget() {
        return "unknown".into();
    }
    let Some(object) = schema.as_object() else {
        return "unknown".into();
    };
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        if ctx.visiting.iter().any(|seen| seen == reference) || ctx.visiting.len() >= MAX_SCHEMA_DEPTH
        {
            return "unknown".into();
        }
        let Some(target) = resolve_local_ref(ctx.root, reference) else {
            return "unknown".into();
        };
        ctx.visiting.push(reference.to_string());
        let lowered = schema_to_ts_inner(target, indent, depth + 1, ctx);
        ctx.visiting.pop();
        return lowered;
    }
    if let Some(value) = object.get("const") {
        return literal_text(value);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if values.is_empty() {
            return "unknown".into();
        }
        return values
            .iter()
            .map(literal_text)
            .collect::<Vec<_>>()
            .join(" | ");
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = object.get(key).and_then(Value::as_array)
            && !variants.is_empty()
        {
            let rendered = variants
                .iter()
                .map(|variant| schema_to_ts_inner(variant, indent, depth + 1, ctx))
                .collect::<Vec<_>>()
                .join(" | ");
            return format!("({rendered})");
        }
    }
    if let Some(variants) = object.get("allOf").and_then(Value::as_array)
        && !variants.is_empty()
    {
        let parts: Vec<String> = variants
            .iter()
            .map(|variant| schema_to_ts_inner(variant, indent, depth + 1, ctx))
            .filter(|part| part != "unknown")
            .collect();
        return match parts.len() {
            0 => "unknown".into(),
            1 => parts.into_iter().next().unwrap(),
            _ => format!("({})", parts.join(" & ")),
        };
    }
    let kinds: Vec<&str> = match object.get("type") {
        Some(Value::String(kind)) => vec![kind.as_str()],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        None if object.contains_key("properties") || object.contains_key("additionalProperties") => {
            vec!["object"]
        }
        None if object.contains_key("items") => vec!["array"],
        None | Some(_) => vec!["unknown"],
    };
    let mut rendered = Vec::new();
    for kind in kinds {
        let part = match kind {
            "string" => "string".to_string(),
            "number" | "integer" => "number".to_string(),
            "boolean" => "boolean".to_string(),
            "null" => "null".to_string(),
            "array" => {
                let item = schema_to_ts_inner(
                    object.get("items").unwrap_or(&Value::Null),
                    indent,
                    depth + 1,
                    ctx,
                );
                format!("Array<{item}>")
            }
            "object" => render_object_type(object, indent, depth, ctx),
            _ => "unknown".to_string(),
        };
        rendered.push(part);
    }
    if rendered.is_empty() {
        "unknown".into()
    } else if rendered.len() == 1 {
        rendered.remove(0)
    } else {
        format!("({})", rendered.join(" | "))
    }
}

fn render_object_type(
    object: &Map<String, Value>,
    indent: usize,
    depth: usize,
    ctx: &mut LowerCtx<'_>,
) -> String {
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let additional = object.get("additionalProperties");
    if properties.is_empty() {
        return match additional {
            Some(Value::Bool(false)) => "Record<string, never>".to_string(),
            Some(schema @ Value::Object(_)) => {
                let val_ts = schema_to_ts_inner(schema, indent, depth + 1, ctx);
                format!("Record<string, {val_ts}>")
            }
            _ => "Record<string, unknown>".to_string(),
        };
    }
    if depth + 1 >= MAX_SCHEMA_DEPTH {
        return "Record<string, unknown>".to_string();
    }
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
    let mut truncated = false;
    for name in names {
        if ctx.remaining_nodes == 0 {
            truncated = true;
            break;
        }
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
        out.push_str(&schema_to_ts_inner(child, child_indent, depth + 1, ctx));
        out.push_str(";\n");
    }
    if truncated || additional == Some(&Value::Bool(true)) {
        out.push_str(&pad);
        out.push_str("[key: string]: unknown;\n");
    } else if let Some(schema @ Value::Object(_)) = additional {
        let val_ts = schema_to_ts_inner(schema, child_indent, depth + 1, ctx);
        out.push_str(&pad);
        out.push_str(&format!("[key: string]: {val_ts};\n"));
    }
    out.push_str(&" ".repeat(indent));
    out.push('}');
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

/// Build the isolated JS fragment and schemas for one runtime tool.
pub fn build_dynamic_tool(
    name: &str,
    spec: DynamicCodeSpec,
    reserved_types: &BTreeSet<String>,
) -> Result<DynamicTool, String> {
    let path: Vec<String> = name.split('.').map(str::to_string).collect();
    let method = path
        .last()
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "dynamic code tool name is empty".to_string())?;
    let suite = if path.len() > 1 {
        path[0].clone()
    } else {
        "custom".into()
    };

    let (input_type, data_type) = select_type_names(name, reserved_types);
    let mut types = BTreeMap::new();
    let lowered_input = if root_is_object(&spec.input_schema) {
        schema_to_ts(&spec.input_schema, 0)
    } else {
        "Record<string, unknown>".into()
    };
    types.insert(input_type.clone(), format!("type {input_type} = {lowered_input};"));

    let mut type_deps = vec![input_type.clone()];
    let return_type = if let Some(output_schema) = &spec.output_schema {
        let lowered_output = schema_to_ts(output_schema, 0);
        types.insert(
            data_type.clone(),
            format!("type {data_type} = {lowered_output};"),
        );
        type_deps.push(data_type.clone());
        data_type
    } else {
        "unknown".to_string()
    };

    let member = property_key(&method);
    let fragment = SdkToolFragment {
        path,
        suite,
        doc: spec.description,
        signature: format!("{member}(args: {input_type}): Promise<{return_type}>;"),
        type_deps,
    };
    Ok(DynamicTool {
        fragment,
        types,
        input_schema: spec.input_schema,
        output_schema: spec.output_schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(output_schema: Option<Value>) -> DynamicCodeSpec {
        DynamicCodeSpec {
            description: "echo".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false
            }),
            output_schema,
        }
    }

    #[test]
    fn custom_code_signature_supports_unconstrained_and_typed_outputs() {
        let untyped = build_dynamic_tool("echo", spec(None), &BTreeSet::new()).expect("untyped tool");
        assert_eq!(
            untyped.fragment.signature,
            "echo(args: CustomEchoInput): Promise<unknown>;"
        );
        assert!(untyped.output_schema.is_none());

        let output = serde_json::json!({
            "type": "object",
            "properties": {"files": {"type": "integer"}},
            "required": ["files"],
            "additionalProperties": false
        });
        let tool = build_dynamic_tool("echo", spec(Some(output.clone())), &BTreeSet::new())
            .expect("dynamic tool");
        assert_eq!(
            tool.fragment.signature,
            "echo(args: CustomEchoInput): Promise<CustomEchoData>;"
        );
        assert!(!tool.fragment.signature.contains("ToolResult"));
        assert_eq!(tool.output_schema, Some(output));
        assert!(tool.types["CustomEchoData"].contains("files"));
    }

    #[test]
    fn schema_to_ts_tolerates_refs_allof_and_cycles() {
        let mut s = spec(None);
        s.input_schema = serde_json::json!({
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "next": { "$ref": "#/$defs/Node" }
                    },
                    "required": ["id"]
                }
            },
            "type": "object",
            "properties": {
                "root": { "$ref": "#/$defs/Node" },
                "combined": {
                    "allOf": [
                        { "type": "object", "properties": { "a": { "type": "number" } }, "required": ["a"] },
                        { "type": "object", "properties": { "b": { "type": "boolean" } } }
                    ]
                }
            },
            "required": ["root"]
        });
        let tool = build_dynamic_tool("mcp_graph.walk", s, &BTreeSet::new()).expect("tolerant");
        let ts = &tool.types["CustomMcpGraphWalkInput"];
        assert!(ts.contains("id: string;"), "{ts}");
        assert!(ts.contains("a: number;"), "{ts}");
        assert!(ts.contains("b?: boolean;"), "{ts}");
    }
}
