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

const PRELUDE: &str = r#"`code` is plain JavaScript. The TypeScript declarations below are documentation only; do not copy type annotations into `code`.
The bundled tools declared below are reachable only through the generated `tools` object inside `code`; they are not separately callable. Other LiteCode tools are outside this SDK and may remain directly callable according to the registry. `libs` contains deterministic pure-JavaScript analysis helpers and exposes no filesystem, network, or process API. Standard JavaScript built-ins are available.
A call resolves to the tool's data. A domain failure rejects `ToolError`. Its `code` is one of `invalid_input`, `not_found`, `invalid_regex`, `stale`, `stale_cursor`, `conflict`, `permission_denied`, `timeout`, `io`, `unsupported`, `internal`. A protocol failure (an extra argument, an illegal value, nesting that is too deep, an integer above 2^53-1, or the in-flight cap) throws `TypeError`. Cancellation, the `run_code` budget, the output cap, and memory or stack limits end the program and cannot be caught.
Permission denial is a catchable `ToolError` whose `code` is `permission_denied`.
`tools.edit` resolves `EditResult{ok,path,message}`. `tools.apply_edits` resolves `{results: EditResult[]}`. Miss, multiple match, missing path, and permission problems set `ok: false` with the reason in `message`; they do not reject `ToolError`. Branch on `ok` or filter `results`.
Use `Promise.allSettled` for a batch. One uncaught rejection ends the program and cancels the other in-flight calls.
LSP errors from edit or write are not in the resolved data. They appear only in this run's outer text.
A read of session lines already in the context window resolves empty `lines` with an explanation in `message` only (no `reason` field). Grep/glob empty results and hit-the-limit guidance likewise live only in `message`. Continue a long read with a larger `start_line`; do not pass a cursor or `page.next`.
The model sees about 64KB of this run. Return a summary, not a raw bulk of data.
Call each binding as `tools["name"](args)`. When `name` is an identifier, `tools.name(args)` is the same function. Each binding takes one argument object. Tool timeouts are internal. The only budget the program sets is `run_code`'s `timeout_ms`.
Return a JSON-serializable value: `undefined` is not JSON, so use `null` or omit the property.

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

/// Context used only to ask a tool for its description. Code-surface text must
/// not pick up the caller's cwd, permission, or depth.
fn description_context() -> Context {
    Context {
        cwd: std::path::PathBuf::new(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(std::path::Path::new(".")),
        agents_md: None,
        claude_md: None,
    }
}

/// SDK type prefix for `name`. Collisions of this prefix are admission failures.
pub(crate) fn code_type_base(name: &str) -> String {
    pascal_case(name)
}

/// One code tool's contract can be rendered. Returns the SDK type prefix.
pub fn validate_code_contract(tool: &dyn Tool) -> Result<String, String> {
    let name = tool.name();
    if name.is_empty() || name.chars().any(|ch| ch.is_control()) {
        return Err(format!("tool '{name}' has an unsafe name"));
    }
    let description = tool.description(&description_context());
    if description.trim().is_empty() {
        return Err(format!("tool '{name}' has an empty description"));
    }
    let input_schema = tool.schema();
    if !input_schema_complete(&input_schema) {
        return Err(format!(
            "tool '{name}' input schema must be a complete object schema"
        ));
    }
    let data_schema = tool
        .data_schema()
        .ok_or_else(|| format!("tool '{name}' has no data schema"))?;
    if !data_schema
        .as_object()
        .is_some_and(|schema| !schema.is_empty())
    {
        return Err(format!(
            "tool '{name}' data schema must be a non-empty object"
        ));
    }
    parse_schema(&input_schema, &format!("{name}.input"))?;
    parse_schema(&data_schema, &format!("{name}.data"))?;
    Ok(code_type_base(name))
}

fn input_schema_complete(schema: &Value) -> bool {
    let Some(object) = schema.as_object() else {
        return false;
    };
    if object.is_empty() {
        return false;
    }
    let type_ok = match object.get("type") {
        Some(Value::String(kind)) => kind == "object",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind.as_str() == Some("object")),
        _ => false,
    };
    if !type_ok {
        return false;
    }
    match object.get("properties") {
        None => true,
        Some(Value::Object(_)) => true,
        Some(_) => false,
    }
}

/// Render the complete model-facing description for a bounded code tool.
///
/// Tools are sorted by name so both the generated text and prompt cache remain
/// deterministic. The text is a pure function of the tools' own contracts.
pub fn render_code_tool_description(tools: &[Arc<dyn Tool>]) -> Result<String, String> {
    let ctx = description_context();
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
        validate_code_contract(tool.as_ref())?;
        let description = tool.description(&ctx);
        let data_schema = tool
            .data_schema()
            .ok_or_else(|| format!("tool '{name}' has no data schema"))?;
        let input_schema = tool.schema();

        let type_base = code_type_base(name);
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

    let mut out = String::new();
    out.push_str(PRELUDE);
    out.push_str(
        "declare class ToolError extends Error {\n  readonly name: \"ToolError\";\n  readonly code: string;\n  readonly details?: unknown;\n  readonly retryable?: true;\n  readonly tool: string;\n  readonly seq: number;\n}\n\n\
         /** Whole JSON number. The host rejects fractional values. */\n\
         type Integer = number;\n\n",
    );
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
        let key = serde_json::to_string(&declaration.name)
            .map_err(|error| format!("serialize tool name: {error}"))?;
        out.push_str(&format!(
            "  readonly [{key}]: (args: {}) => Promise<{}>;\n",
            declaration.input_type, declaration.data_type
        ));
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
        return Err(format!("{path}: schema must be an object"));
    };

    for key in [
        "$ref",
        "allOf",
        "not",
        "patternProperties",
        "dependencies",
        "if",
        "unevaluatedProperties",
    ] {
        if object.contains_key(key) {
            return Err(format!("{path}: unsupported schema construct '{key}'"));
        }
    }
    if let Some(extra) = object.get("additionalProperties")
        && !extra.is_boolean()
    {
        return Err(format!("{path}: unsupported additionalProperties schema"));
    }

    if let Some(value) = object.get("const") {
        return Ok(TsType::Literal(json_literal(value, path)?));
    }

    if let Some(variants) = object
        .get("oneOf")
        .or_else(|| object.get("anyOf"))
        .and_then(Value::as_array)
    {
        if variants.is_empty() {
            return Err(format!("{path}: empty union"));
        }
        let types = variants
            .iter()
            .enumerate()
            .map(|(index, variant)| parse_schema(variant, &format!("{path}.variant[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;
        return normalize_union(types, path);
    }

    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if values.is_empty() {
            return Err(format!("{path}: empty enum"));
        }
        let types = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                json_literal(value, &format!("{path}.enum[{index}]")).map(TsType::Literal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return normalize_union(types, path);
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
                return normalize_union(types, path);
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

    Err(format!("{path}: schema cannot be rendered"))
}

fn parse_typed(object: &Map<String, Value>, kind: &str, path: &str) -> Result<TsType, String> {
    match kind {
        "string" => Ok(TsType::Keyword("string")),
        "integer" => Ok(TsType::Keyword("Integer")),
        "number" => Ok(TsType::Keyword("number")),
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
            if items.is_empty() {
                return Err(format!("{path}: empty tuple items"));
            }
            let types = items
                .iter()
                .enumerate()
                .map(|(index, item)| parse_schema(item, &format!("{path}.items[{index}]")))
                .collect::<Result<Vec<_>, _>>()?;
            normalize_union(types, path)?
        }
        Some(item) => parse_schema(item, &format!("{path}.items"))?,
        None => return Err(format!("{path}: array schema has no items")),
    };
    Ok(TsType::Array(Box::new(item_type)))
}

fn normalize_union(types: Vec<TsType>, path: &str) -> Result<TsType, String> {
    match types.len() {
        0 => Err(format!("{path}: empty union")),
        1 => Ok(types.into_iter().next().unwrap_or(TsType::Keyword("never"))),
        _ => Ok(TsType::Union(types)),
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
        TsType::Keyword(_) | TsType::Literal(_) => true,
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
        TsType::Keyword(_) | TsType::Literal(_) => render_flat(ty),
    }
}

fn render_flat(ty: &TsType) -> String {
    match ty {
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
                    "ratio": {
                        "type": "number",
                        "description": "A fractional ratio."
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

    struct TimedTool;

    impl Tool for TimedTool {
        fn name(&self) -> &str {
            "timed"
        }

        fn schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }

        fn data_schema(&self) -> Option<Value> {
            Some(
                json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]}),
            )
        }

        fn description(&self, _ctx: &Context) -> String {
            "Tool with a declared timeout.".into()
        }

        fn timeout(&self) -> Option<std::time::Duration> {
            Some(std::time::Duration::from_secs(5))
        }
    }

    struct RefTool;

    impl Tool for RefTool {
        fn name(&self) -> &str {
            "ref_tool"
        }

        fn schema(&self) -> Value {
            json!({"type": "object", "properties": {"item": {"$ref": "#/defs/Item"}}})
        }

        fn data_schema(&self) -> Option<Value> {
            Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}))
        }

        fn description(&self, _ctx: &Context) -> String {
            "Tool with an unresolved ref.".into()
        }
    }

    #[test]
    fn renders_contract_as_commented_typescript() {
        let page = render_code_tool_description(&[Arc::new(FakeTool)]).unwrap();
        assert!(page.contains("do not copy type annotations"));
        assert!(page.contains("tools[\"name\"]"));
        assert!(page.contains("type Integer = number;"));
        assert!(page.contains("type DemoInput = {"));
        assert!(page.contains("  // Execution mode."));
        assert!(page.contains("mode: \"fast\" | \"safe\";"));
        assert!(page.contains("ratio?: number;"));
        assert!(page.contains("value: Integer;"));
        assert!(page.contains("input?: {"));
        assert!(page.contains("type DemoData ="));
        assert!(page.contains("kind: \"text\";"));
        assert!(page.contains("bytes: Integer;"));
        assert!(
            page.contains(
                "readonly [\"demo\"]: (args: DemoInput) => Promise<DemoData>;"
            )
        );
        assert!(!page.contains("ToolCallOptions"));
        assert!(page.contains("Glob text stays literal: **/*.rs."));
        assert!(page.contains("declare const libs:"));
        assert!(page.contains("stats: {"));
        assert!(page.contains("csv: {"));
        assert!(page.contains("groupBy<T>"));
        assert!(!page.contains("tools.call"));
        assert!(
            !page
                .replace("details?: unknown", "")
                .contains(": unknown"),
            "schema projection must not fall back to unknown"
        );
    }

    #[test]
    fn declared_timeout_does_not_add_a_per_call_options_argument() {
        let page =
            render_code_tool_description(&[Arc::new(TimedTool), Arc::new(FakeTool)]).unwrap();
        assert!(!page.contains("ToolCallOptions"));
        for line in page.lines().filter(|line| line.contains("readonly [")) {
            assert!(!line.contains("options"), "{line}");
        }
        assert!(page.contains(
            "readonly [\"timed\"]: (args: TimedInput) => Promise<TimedData>;"
        ));
        assert!(page.contains(
            "readonly [\"demo\"]: (args: DemoInput) => Promise<DemoData>;"
        ));
        assert!(page.contains("declare class ToolError"));
        assert!(!page.contains("ToolResult"));
        assert!(!page.contains("status: \"partial\""));
        assert!(!page.contains("retryable: boolean"));
        assert!(page.contains("`undefined` is not JSON"));
        let timed = page.find("[\"timed\"]").unwrap();
        let demo = page.find("[\"demo\"]").unwrap();
        assert!(demo < timed, "bindings are sorted by name");
    }

    #[test]
    fn rejects_tools_without_a_data_contract() {
        let error = render_code_tool_description(&[Arc::new(NoDataTool)]).unwrap_err();
        assert_eq!(error, "tool 'no_data' has no data schema");
    }

    #[test]
    fn unresolved_ref_fails_admission() {
        let error = render_code_tool_description(&[Arc::new(RefTool)]).unwrap_err();
        assert!(error.contains("$ref"), "{error}");
        assert!(!error.contains("unknown"));
    }
}
