//! Deterministic projection from tool contracts to a code-mode interface page.
//!
//! A code bundle does not maintain a second, hand-written description. Each
//! tool owns its semantic description, input schema, and `data` schema. This
//! module only renders those facts into TypeScript declarations embedded in the
//! CodeTool description.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::context_pipeline::Context;
use crate::tool::trait_::Tool;
use crate::types::ToolErrorKind;

const PRELUDE: &str = r#"`code` is plain JavaScript. The TypeScript declarations below are documentation only; do not copy type annotations into `code`.
The bundled tools declared below are reachable only through the generated `tools` object inside `code`; they are not separately callable. Other LiteCode tools are outside this SDK and may remain directly callable according to the registry. `libs` contains deterministic pure-JavaScript analysis helpers and exposes no filesystem, network, or process API. Standard JavaScript built-ins are available.
Expected tool failures resolve as `ToolResult` values with `status: "error"`; `status: "warning"` is a successful result with data. Runtime or protocol failures reject. Return a JSON-serializable value.
A binding may receive an optional `options.timeout_ms`; it is host execution metadata, not part of the tool arguments.

```ts
"#;

const LIBS_SDK: &str = r#"
type CsvOptions = {
  /** Field separator; one character. Default ",". */
  delimiter?: string;
  /** Quote character; one character. Default is a double quote. */
  quote?: string;
  /** Trim fields while parsing. Default false. */
  trim?: boolean;
};

type HistogramBin = {
  start: number;
  end: number;
  count: number;
};

type DiffLine = {
  kind: "equal" | "insert" | "delete";
  text: string;
};

declare const libs: {
  stats: {
    sum(values: number[]): number;
    mean(values: number[]): number;
    median(values: number[]): number;
    variance(values: number[], sample?: boolean): number;
    stddev(values: number[], sample?: boolean): number;
    quantile(values: number[], q: number): number;
    histogram(values: number[], bins?: number): HistogramBin[];
    min(values: number[]): number;
    max(values: number[]): number;
  };
  csv: {
    parse(text: string, options?: CsvOptions): string[][];
    format(rows: Array<Array<unknown>>, options?: CsvOptions): string;
  };
  text: {
    lines(text: string): string[];
    words(text: string): string[];
    count(text: string, needle: string): number;
    normalizeNewlines(text: string): string;
    truncate(text: string, maxChars: number, suffix?: string): string;
    diffLines(left: string, right: string): DiffLine[];
  };
  data: {
    groupBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, T[]>;
    countBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, number>;
    unique<T>(values: T[]): T[];
    sortBy<T>(rows: T[], key: string | ((row: T) => unknown), direction?: "asc" | "desc"): T[];
  };
};

"#;

/// Render the complete model-facing description for a bounded code tool.
///
/// Tools are sorted by name so both the generated text and prompt cache remain
/// deterministic. Every bundled tool must have a non-empty description and a
/// `data_schema`; otherwise the bundle is invalid and rendering fails.
pub fn render_code_tool_description(
    tools: &[Arc<dyn Tool>],
    ctx: &Context,
) -> Result<String, String> {
    let mut ordered: Vec<&Arc<dyn Tool>> = tools.iter().collect();
    ordered.sort_by(|left, right| left.name().cmp(right.name()));

    let mut seen_names = HashSet::new();
    let mut seen_types = HashSet::new();
    let mut declarations = Vec::with_capacity(ordered.len());

    for tool in ordered {
        let name = tool.name();
        if !seen_names.insert(name.to_string()) {
            return Err(format!("duplicate tool name '{name}' in code bundle"));
        }

        let description = tool.description(ctx);
        if description.trim().is_empty() {
            return Err(format!("tool '{name}' has an empty description"));
        }
        let data_schema = tool
            .data_schema()
            .ok_or_else(|| format!("tool '{name}' has no data schema"))?;
        let input_schema = tool.schema();
        if !input_schema.is_object() {
            return Err(format!("tool '{name}' input schema must be an object"));
        }
        if !data_schema
            .as_object()
            .is_some_and(|schema| !schema.is_empty())
        {
            return Err(format!(
                "tool '{name}' data schema must be a non-empty object"
            ));
        }

        let type_base = pascal_case(name);
        let input_type = format!("{type_base}Input");
        let data_type = format!("{type_base}Data");
        for type_name in [&input_type, &data_type] {
            if !seen_types.insert(type_name.clone()) {
                return Err(format!(
                    "tool '{name}' generates duplicate TypeScript type '{type_name}'"
                ));
            }
        }

        let input = parse_schema(&input_schema, &format!("{name}.input"))?;
        let data = parse_schema(&data_schema, &format!("{name}.data"))?;

        declarations.push(ToolDeclaration {
            name: name.to_string(),
            description,
            input_type,
            data_type,
            input,
            data,
            input_doc: input_schema
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
            data_doc: data_schema
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }

    let error_kinds = ToolErrorKind::ALL
        .iter()
        .map(|kind| {
            serde_json::to_string(kind).map_err(|error| format!("serialize ToolErrorKind: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?
        .join(" | ");

    let mut out = String::new();
    out.push_str(PRELUDE);
    out.push_str(&format!(
        "type ToolErrorKind = {error_kinds};\n\
         type ToolError = {{\n  kind: ToolErrorKind;\n  message: string;\n  retryable: boolean;\n}};\n\
         type ToolResult<T> =\n  | {{ status: \"ok\" | \"warning\"; data: T; error: null }}\n  | {{ status: \"error\"; data: null; error: ToolError }};\n\n\
         type ToolCallOptions = {{\n  /** Host-enforced per-call budget; the tool may clamp or reject it. */\n  timeout_ms?: number;\n}};\n\n"
    ));
    out.push_str(LIBS_SDK);

    for declaration in &declarations {
        render_type_alias(
            &declaration.input_type,
            &declaration.input,
            declaration.input_doc.as_deref(),
            &mut out,
        );
        out.push('\n');
        render_type_alias(
            &declaration.data_type,
            &declaration.data,
            declaration.data_doc.as_deref(),
            &mut out,
        );
        out.push('\n');
    }

    out.push_str("declare const tools: {\n");
    for declaration in &declarations {
        out.push_str(&render_comment(&declaration.description, 2));
        if is_ts_identifier(&declaration.name) {
            out.push_str(&format!(
                "  {}(args: {}, options?: ToolCallOptions): Promise<ToolResult<{}>>;\n",
                declaration.name, declaration.input_type, declaration.data_type
            ));
        } else {
            let key = serde_json::to_string(&declaration.name)
                .map_err(|error| format!("serialize tool name: {error}"))?;
            out.push_str(&format!(
                "  {key}: (args: {}, options?: ToolCallOptions) => Promise<ToolResult<{}>>;\n",
                declaration.input_type, declaration.data_type
            ));
        }
    }
    out.push_str("};\n```\n");

    Ok(out)
}

struct ToolDeclaration {
    name: String,
    description: String,
    input_type: String,
    data_type: String,
    input: TsType,
    data: TsType,
    input_doc: Option<String>,
    data_doc: Option<String>,
}

#[derive(Debug)]
enum TsType {
    Unknown,
    Keyword(&'static str),
    Literal(String),
    Array(Box<TsType>),
    Object(Vec<TsField>),
    Union(Vec<TsType>),
}

#[derive(Debug)]
struct TsField {
    name: String,
    required: bool,
    description: Option<String>,
    ty: TsType,
}

fn parse_schema(schema: &Value, path: &str) -> Result<TsType, String> {
    let Some(object) = schema.as_object() else {
        return Ok(TsType::Unknown);
    };

    if let Some(value) = object.get("const") {
        return Ok(TsType::Literal(json_literal(value, path)?));
    }

    if let Some(variants) = object
        .get("oneOf")
        .or_else(|| object.get("anyOf"))
        .and_then(Value::as_array)
    {
        let types = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| parse_schema(variant, &format!("{path}.variant[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(normalize_union(types));
    }

    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        let types = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                json_literal(value, &format!("{path}.enum[{index}]")).map(TsType::Literal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(normalize_union(types));
    }

    if let Some(type_spec) = object.get("type") {
        match type_spec {
            Value::String(kind) => return parse_typed(object, kind, path),
            Value::Array(kinds) => {
                let types = kinds
                    .iter()
                    .enumerate()
                    .filter_map(|(index, kind)| kind.as_str().map(|kind| (index, kind.to_string())))
                    .map(|(index, kind)| {
                        parse_typed(object, &kind, &format!("{path}.type[{index}]"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if types.is_empty() {
                    return Err(format!("{path}: type array contains no strings"));
                }
                return Ok(normalize_union(types));
            }
            _ => {
                return Err(format!("{path}: unsupported non-string `type`"));
            }
        }
    }

    if object.contains_key("properties") {
        return parse_object(object, path);
    }
    if object.contains_key("items") {
        return parse_array(object, path);
    }

    Ok(TsType::Unknown)
}

fn parse_typed(object: &Map<String, Value>, kind: &str, path: &str) -> Result<TsType, String> {
    match kind {
        "string" => Ok(TsType::Keyword("string")),
        "integer" | "number" => Ok(TsType::Keyword("number")),
        "boolean" => Ok(TsType::Keyword("boolean")),
        "null" => Ok(TsType::Keyword("null")),
        "object" => parse_object(object, path),
        "array" => parse_array(object, path),
        other => Err(format!("{path}: unsupported schema type '{other}'")),
    }
}

fn parse_object(object: &Map<String, Value>, path: &str) -> Result<TsType, String> {
    let required: HashSet<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();

    let mut fields = Vec::new();
    if let Some(properties) = object.get("properties") {
        let properties = properties
            .as_object()
            .ok_or_else(|| format!("{path}: `properties` must be an object"))?;
        for (name, schema) in properties {
            fields.push(TsField {
                name: name.clone(),
                required: required.contains(name.as_str()),
                description: schema
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                ty: parse_schema(schema, &format!("{path}.{name}"))?,
            });
        }
    }
    fields.sort_by(|left, right| {
        let left_kind = left.name != "kind";
        let right_kind = right.name != "kind";
        left_kind
            .cmp(&right_kind)
            .then(right.required.cmp(&left.required))
            .then(left.name.cmp(&right.name))
    });
    Ok(TsType::Object(fields))
}

fn parse_array(object: &Map<String, Value>, path: &str) -> Result<TsType, String> {
    let item_type = match object.get("items") {
        Some(Value::Array(items)) => {
            let types = items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_schema(item, &format!("{path}.items[{index}]")))
                .collect::<Result<Vec<_>, _>>()?;
            normalize_union(types)
        }
        Some(item) => parse_schema(item, &format!("{path}.items"))?,
        None => TsType::Unknown,
    };
    Ok(TsType::Array(Box::new(item_type)))
}

fn normalize_union(types: Vec<TsType>) -> TsType {
    match types.len() {
        0 => TsType::Unknown,
        1 => types.into_iter().next().unwrap_or(TsType::Unknown),
        _ => TsType::Union(types),
    }
}

fn json_literal(value: &Value, path: &str) -> Result<String, String> {
    match value {
        Value::String(value) => {
            serde_json::to_string(value).map_err(|error| format!("{path}: {error}"))
        }
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Null => Ok("null".to_string()),
        Value::Array(_) | Value::Object(_) => {
            Err(format!("{path}: const/enum values must be JSON primitives"))
        }
    }
}

fn render_type_alias(name: &str, ty: &TsType, doc: Option<&str>, out: &mut String) {
    if let Some(doc) = doc.filter(|doc| !doc.trim().is_empty()) {
        out.push_str(&render_comment(doc, 0));
    }
    if matches!(ty, TsType::Union(_)) && !is_flat(ty) {
        out.push_str(&format!("type {name} =\n"));
        let TsType::Union(items) = ty else {
            unreachable!();
        };
        for (index, item) in items.iter().enumerate() {
            out.push_str("  | ");
            out.push_str(&render_type(item, 4));
            out.push_str(if index + 1 == items.len() {
                ";\n"
            } else {
                "\n"
            });
        }
    } else {
        out.push_str(&format!("type {name} = "));
        out.push_str(&render_type(ty, 0));
        out.push_str(";\n");
    }
}

fn is_flat(ty: &TsType) -> bool {
    match ty {
        TsType::Unknown | TsType::Keyword(_) | TsType::Literal(_) => true,
        TsType::Array(item) => is_flat(item),
        TsType::Object(_) => false,
        TsType::Union(items) => items.iter().all(is_flat),
    }
}

fn render_type(ty: &TsType, indent: usize) -> String {
    if is_flat(ty) {
        return render_flat(ty);
    }
    match ty {
        TsType::Object(fields) => render_object(fields, indent),
        TsType::Array(item) => format!("Array<{}>", render_type(item, indent)),
        TsType::Union(items) => {
            let mut out = String::new();
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                }
                if index > 0 {
                    out.push_str("| ");
                }
                out.push_str(&render_type(item, indent + 2));
            }
            out
        }
        TsType::Unknown | TsType::Keyword(_) | TsType::Literal(_) => render_flat(ty),
    }
}

fn render_flat(ty: &TsType) -> String {
    match ty {
        TsType::Unknown => "unknown".to_string(),
        TsType::Keyword(keyword) => (*keyword).to_string(),
        TsType::Literal(literal) => literal.clone(),
        TsType::Array(item) => format!("Array<{}>", render_flat(item)),
        TsType::Union(items) => items
            .iter()
            .map(render_flat)
            .collect::<Vec<_>>()
            .join(" | "),
        TsType::Object(fields) => render_object(fields, 0),
    }
}

fn render_object(fields: &[TsField], indent: usize) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }
    let field_indent = indent + 2;
    let mut out = String::from("{\n");
    for field in fields {
        if let Some(description) = field
            .description
            .as_deref()
            .filter(|description| !description.trim().is_empty())
        {
            out.push_str(&render_comment(description, field_indent));
        }
        out.push_str(&" ".repeat(field_indent));
        out.push_str(&ts_property_name(&field.name));
        if !field.required {
            out.push('?');
        }
        out.push_str(": ");
        out.push_str(&render_type(&field.ty, field_indent));
        out.push_str(";\n");
    }
    out.push_str(&" ".repeat(indent));
    out.push('}');
    out
}

fn render_comment(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for line in text.trim().lines() {
        if line.is_empty() {
            out.push_str(&format!("{pad}//\n"));
        } else {
            out.push_str(&format!("{pad}// {line}\n"));
        }
    }
    out
}

fn ts_property_name(name: &str) -> String {
    if is_ts_identifier(name) {
        name.to_string()
    } else {
        serde_json::to_string(name).unwrap_or_else(|_| "\"<invalid-name>\"".to_string())
    }
}

fn is_ts_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || matches!(first, '_' | '$')) {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
}

fn pascal_case(name: &str) -> String {
    let mut out = String::new();
    let mut uppercase = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if uppercase {
                out.push(ch.to_ascii_uppercase());
                uppercase = false;
            } else {
                out.push(ch);
            }
        } else {
            uppercase = true;
        }
    }
    if out.is_empty() {
        out.push_str("Tool");
    }
    if out
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
    {
        out.insert(0, 'T');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolCallResult;
    use serde_json::json;

    struct FakeTool;

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "demo"
        }

        fn schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "mode": {
                        "type": "string",
                        "enum": ["fast", "safe"],
                        "description": "Execution mode."
                    },
                    "input": {
                        "type": "object",
                        "description": "Nested input.",
                        "properties": {
                            "value": {
                                "type": "integer",
                                "description": "A value."
                            }
                        },
                        "required": ["value"]
                    }
                },
                "required": ["mode"]
            })
        }

        fn data_schema(&self) -> Option<Value> {
            Some(json!({
                "oneOf": [
                    {
                        "type": "object",
                        "description": "A text result.",
                        "properties": {
                            "kind": {"const": "text"},
                            "value": {
                                "type": "string",
                                "description": "Result text."
                            }
                        },
                        "required": ["kind", "value"]
                    },
                    {
                        "type": "object",
                        "properties": {
                            "kind": {"const": "image"},
                            "bytes": {"type": "integer"}
                        },
                        "required": ["kind", "bytes"]
                    }
                ]
            }))
        }

        fn call_inner(&self, _input: Value) -> ToolCallResult {
            ToolCallResult::ok("")
        }

        fn description(&self, _ctx: &Context) -> String {
            "Demo tool used by the projection test. Glob text stays literal: **/*.rs.".to_string()
        }
    }

    struct NoDataTool;

    impl Tool for NoDataTool {
        fn name(&self) -> &str {
            "no_data"
        }

        fn schema(&self) -> Value {
            json!({"type": "object"})
        }

        fn description(&self, _ctx: &Context) -> String {
            "Tool without a data contract.".to_string()
        }
    }

    fn ctx() -> Context {
        Context {
            cwd: std::path::PathBuf::from("."),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(std::path::Path::new(
                ".",
            )),
            agents_md: None,
            claude_md: None,
        }
    }

    #[test]
    fn renders_contract_as_commented_typescript() {
        let page = render_code_tool_description(&[Arc::new(FakeTool)], &ctx()).unwrap();
        assert!(page.contains("do not copy type annotations"));
        assert!(page.contains("type DemoInput = {"));
        assert!(page.contains("  // Execution mode."));
        assert!(page.contains("mode: \"fast\" | \"safe\";"));
        assert!(page.contains("input?: {"));
        assert!(page.contains("type DemoData ="));
        assert!(page.contains("kind: \"text\";"));
        assert!(page.contains(
            "demo(args: DemoInput, options?: ToolCallOptions): Promise<ToolResult<DemoData>>;"
        ));
        assert!(page.contains("Glob text stays literal: **/*.rs."));
        assert!(page.contains("declare const libs:"));
        assert!(page.contains("output_limit"));
        assert!(page.contains("stats: {"));
        assert!(page.contains("csv: {"));
        assert!(page.contains("groupBy<T>"));
        assert!(!page.contains("tools.call"));
    }

    #[test]
    fn rejects_tools_without_a_data_contract() {
        let error = render_code_tool_description(&[Arc::new(NoDataTool)], &ctx()).unwrap_err();
        assert_eq!(error, "tool 'no_data' has no data schema");
    }
}
