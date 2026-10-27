//! Deterministic projection from tool contracts to a code-mode interface page.
//!
//! A code bundle does not maintain a second, hand-written description. Each
//! tool owns its semantic description, input schema, and `data` schema. This
//! module only renders those facts into TypeScript declarations embedded in the
//! CodeTool description.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::tool::spec::CodeFunctionSpec;

const PRELUDE: &str = r#"Open `run_code` when one program can finish the work: explore with reads/searches, make the minimal edits, then verify with the real check. A short program is fine; do not wrap a single peek or edit in a long ritual.
Explore first; write only after the target is known. `await` when a later call needs an earlier value. Use `Promise.all` only when every call must succeed; use `Promise.allSettled` when one failure must not stop the batch.
`return` a short result: what changed, what ran, and the few facts that matter, not raw dumps. The run header and logs are diagnostics; `result` is your output. Oversized result text is truncated with a note at the start.
`code` is plain JavaScript. The TypeScript declarations below are documentation only; do not paste type annotations into `code`. Call bundled tools only through `tools.name(args)`; a code suite keeps its namespace even with one member: `tools.suite.member(args)`. One argument object each. Other LiteCode tools may remain directly callable outside this SDK.
Every resolved call has boolean `ok`: `false` is a domain failure with `reason`/`message`, so one bad item need not end a batch. Unknown tools and extra JavaScript arguments throw `TypeError`; invalid tool arguments resolve `ok: false` `invalid_args`, and unknown fields are never silently dropped. Cancellation, the `run_code` budget, and hard limits end the program and cannot be caught.
`libs` are deterministic pure-JS helpers (no filesystem, network, or process). `fs.ls` / `fs.stat` / `fs.exists` are synchronous, read-only workspace probes; mutate files only through declared tools. The sandbox has no `setTimeout` / `setInterval`.
Unhandled Promise rejections do not end the program; a `[warn]` notes them after settle.

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

type FsFailure = { ok: false; reason: "not_found" | "permission" | "io"; message: string };

/** Read-only workspace probes. Synchronous; paths are workspace-relative. */
declare const fs: {
  /** One level, sorted by name, at most 500 entries (message says when cut). */
  ls(path: string): { ok: true; entries: Array<{ name: string; type: "file" | "dir" | "symlink" | "other"; size: number }>; message?: string } | FsFailure;
  /** mtime is ms since the Unix epoch. */
  stat(path: string): { ok: true; type: "file" | "dir" | "symlink" | "other"; size: number; mtime: number } | FsFailure;
  exists(path: string): boolean;
};

/** Any tool call resolves this when the tool did not get the job done. */
type ToolFailure = {
  ok: false;
  reason: "not_found" | "no_match" | "invalid_args" | "empty" | "permission" | "io" | "conflict" | "stale";
  message: string;
};

"#;

/// SDK type prefix for `name`. Collisions of this prefix are admission failures.
pub(crate) fn code_type_base(name: &str) -> String {
    pascal_case(name)
}

/// One code contract can be rendered. Returns the SDK type prefix.
pub fn validate_code_contract(spec: &CodeFunctionSpec) -> Result<String, String> {
    let name = spec.name.as_str();
    if name.is_empty()
        || name.chars().any(|ch| ch.is_control())
        || name.split('.').any(str::is_empty)
    {
        return Err(format!("tool '{name}' has an unsafe name"));
    }
    if spec.description.trim().is_empty() {
        return Err(format!("tool '{name}' has an empty description"));
    }
    if !input_schema_complete(&spec.input_schema) {
        return Err(format!(
            "tool '{name}' input schema must be a complete object schema"
        ));
    }
    if !spec
        .output_schema
        .as_object()
        .is_some_and(|schema| !schema.is_empty())
    {
        return Err(format!(
            "tool '{name}' data schema must be a non-empty object"
        ));
    }
    parse_schema(&spec.input_schema, &format!("{name}.input"))?;
    parse_schema(&spec.output_schema, &format!("{name}.data"))?;
    Ok(code_type_base(name))
}

/// A dotted code name opens a namespace. A function with the same full name as
/// that namespace cannot also exist, because a property cannot be both a
/// function and an object.
pub(crate) fn nested_name_collision<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut names: Vec<&str> = names.into_iter().collect();
    names.sort_unstable();
    for name in &names {
        if let Some(other) = names
            .iter()
            .find(|other| name.starts_with(&format!("{other}.")))
        {
            return Some(format!(
                "tool '{name}' nests under '{other}', but both are callable; rename one of them"
            ));
        }
    }
    None
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
pub fn render_code_tool_description(specs: &[CodeFunctionSpec]) -> Result<String, String> {
    let mut ordered: Vec<&CodeFunctionSpec> = specs.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    if let Some(reason) =
        nested_name_collision(ordered.iter().map(|spec| spec.name.as_str()))
    {
        return Err(reason);
    }

    let mut seen_names = HashSet::new();
    let mut seen_types = HashSet::new();
    let mut declarations = Vec::with_capacity(ordered.len());

    for spec in &ordered {
        let name = spec.name.as_str();
        if !seen_names.insert(name.to_string()) {
            return Err(format!("duplicate tool name '{name}' in code bundle"));
        }
        validate_code_contract(spec)?;
        let description = spec.description.clone();
        let data_schema = &spec.output_schema;
        let input_schema = &spec.input_schema;

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
        let data = with_ok_field(parse_schema(&data_schema, &format!("{name}.data"))?);

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

    // One body per distinct type shape. Several tools share a schema (run, wait
    // and kill all return the command data; the LSP operations share their path,
    // range and location shapes), while a type is named after its tool, so the
    // same body used to be printed once per tool. Emit it once and alias the
    // rest: same fields, same docs, fewer tokens.
    let mut bodies: Vec<String> = Vec::with_capacity(declarations.len() * 2);
    for declaration in &declarations {
        bodies.push(render_type_body(&declaration.input));
        bodies.push(render_type_body(&declaration.data));
    }
    let mut body_counts: HashMap<&str, usize> = HashMap::new();
    for body in &bodies {
        *body_counts.entry(body.as_str()).or_default() += 1;
    }
    let mut canonical_by_body: HashMap<String, String> = HashMap::new();
    for (index, declaration) in declarations.iter().enumerate() {
        let pairs = [
            (
                declaration.input_type.as_str(),
                bodies[index * 2].as_str(),
                declaration.input_doc.as_deref(),
            ),
            (
                declaration.data_type.as_str(),
                bodies[index * 2 + 1].as_str(),
                declaration.data_doc.as_deref(),
            ),
        ];
        for (name, body, doc) in pairs {
            let shared = body_counts.get(body).copied().unwrap_or(0) > 1;
            render_type_declaration(name, body, doc, shared, &mut canonical_by_body, &mut out);
            out.push('\n');
        }
    }

    out.push_str("declare const tools: {\n");
    let mut rendered_suites = HashSet::new();
    for declaration in &declarations {
        let Some((suite, _)) = declaration.name.split_once('.') else {
            render_binding(declaration, 2, &mut out)?;
            continue;
        };
        if !rendered_suites.insert(suite.to_string()) {
            continue;
        }
        let key = serde_json::to_string(suite)
            .map_err(|error| format!("serialize suite name: {error}"))?;
        out.push_str(&format!("  readonly [{key}]: {{\n"));
        for member_declaration in declarations
            .iter()
            .filter(|candidate| candidate.name.starts_with(&format!("{suite}.")))
        {
            render_binding(member_declaration, 4, &mut out)?;
        }
        out.push_str("  };\n");
    }
    out.push_str("};\n```\n");

    Ok(out)
}

fn render_binding(
    declaration: &ToolDeclaration,
    indent: usize,
    out: &mut String,
) -> Result<(), String> {
    let member = declaration
        .name
        .rsplit_once('.')
        .map_or(declaration.name.as_str(), |(_, member)| member);
    out.push_str(&render_comment(&declaration.description, indent));
    let key =
        serde_json::to_string(member).map_err(|error| format!("serialize tool name: {error}"))?;
    out.push_str(&format!(
        "{}readonly [{key}]: (args: {}) => Promise<{} | ToolFailure>;\n",
        " ".repeat(indent),
        declaration.input_type,
        declaration.data_type
    ));
    Ok(())
}

/// The host adds a leading `ok: true` to every resolved object that does not
/// carry its own `ok` (WP-210 G). Mirror that in the SDK.
fn with_ok_field(data: TsType) -> TsType {
    match data {
        TsType::Object(mut fields) if !fields.iter().any(|field| field.name == "ok") => {
            fields.insert(
                0,
                TsField {
                    name: "ok".to_string(),
                    required: true,
                    description: None,
                    ty: TsType::Literal("true".to_string()),
                },
            );
            TsType::Object(fields)
        }
        other => other,
    }
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

/// Canonical name for a body several tools share, so an alias reads
/// `type WaitData = CommandData;` instead of pointing at whichever tool
/// happened to render first.
const SHARED_TYPE_NAMES: &[(&str, &str)] = &[
    ("DiagnosticsInput", "PathInput"),
    ("DocumentSymbolInput", "PathInput"),
    ("DocumentSymbolData", "SymbolsData"),
    ("WorkspaceSymbolData", "SymbolsData"),
    ("FindReferencesInput", "RangeInput"),
    ("GoToDefinitionInput", "RangeInput"),
    ("GoToImplementationInput", "RangeInput"),
    ("HoverInput", "RangeInput"),
    ("FindReferencesData", "LocationsData"),
    ("GoToDefinitionData", "LocationsData"),
    ("GoToImplementationData", "LocationsData"),
    ("KillData", "CommandData"),
    ("RunData", "CommandData"),
    ("WaitData", "CommandData"),
];

fn shared_type_name(name: &str) -> Option<&'static str> {
    SHARED_TYPE_NAMES
        .iter()
        .find(|(tool_name, _)| *tool_name == name)
        .map(|(_, canonical)| *canonical)
}

fn render_doc(doc: Option<&str>, out: &mut String) {
    if let Some(doc) = doc.filter(|doc| !doc.trim().is_empty()) {
        out.push_str(&render_comment(doc, 0));
    }
}

/// Body of a rendered alias: everything after `type Name =`, newline included.
/// Two tools that share one schema produce the same string here, which is what
/// the dedupe compares.
fn render_type_body(ty: &TsType) -> String {
    let mut body = String::new();
    if matches!(ty, TsType::Union(_)) && !is_flat(ty) {
        body.push('\n');
        let TsType::Union(items) = ty else {
            unreachable!();
        };
        for (index, item) in items.iter().enumerate() {
            body.push_str("  | ");
            body.push_str(&render_type(item, 4));
            body.push_str(if index + 1 == items.len() {
                ";\n"
            } else {
                "\n"
            });
        }
    } else {
        body.push(' ');
        body.push_str(&render_type(ty, 0));
        body.push_str(";\n");
    }
    body
}

/// Emit one type: the body under its canonical name, then an alias for this
/// tool's own name when the canonical name differs.
fn render_type_declaration(
    name: &str,
    body: &str,
    doc: Option<&str>,
    shared: bool,
    canonical_by_body: &mut HashMap<String, String>,
    out: &mut String,
) {
    if let Some(canonical) = canonical_by_body.get(body) {
        if canonical != name {
            render_doc(doc, out);
            out.push_str(&format!("type {name} = {canonical};\n"));
        }
        return;
    }
    let canonical = if shared {
        shared_type_name(name).unwrap_or(name)
    } else {
        name
    };
    canonical_by_body.insert(body.to_string(), canonical.to_string());
    render_doc(doc, out);
    out.push_str(&format!("type {canonical} ={body}"));
    if canonical != name {
        out.push_str(&format!("type {name} = {canonical};\n"));
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
    use std::sync::Arc;

    use serde_json::json;

    use crate::context_pipeline::Context;
    use crate::tool::FunctionSpec;
    use crate::tool::project::project_tools;
    use crate::tool::trait_::Tool;

    struct NoDataTool;

    impl Tool for NoDataTool {
        fn name(&self) -> &str {
            "no_data"
        }

        fn surface(&self) -> crate::tool::ToolSurface {
            crate::tool::ToolSurface::Code
        }

        fn schema(&self) -> Value {
            json!({"type": "object"})
        }

        fn description(&self, _ctx: &Context) -> String {
            "Tool without a data contract.".to_string()
        }
    }



    fn spec(name: &str) -> CodeFunctionSpec {
        CodeFunctionSpec {
            name: name.to_string(),
            description: format!("{name} probe"),
            input_schema: json!({ "type": "object", "properties": {} }),
            output_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    #[test]
    fn code_suite_members_render_under_a_suite_object() {
        let text = render_code_tool_description(&[
            spec("deck.only"),
            spec("deck.other"),
            spec("alpha"),
        ])
        .expect("renders");
        assert!(text.contains("readonly [\"alpha\"]:"), "{text}");
        assert!(text.contains("readonly [\"deck\"]: {"), "{text}");
        assert!(text.contains("readonly [\"only\"]:"), "{text}");
        assert!(text.contains("readonly [\"other\"]:"), "{text}");
        assert!(!text.contains("[\"deck.only\"]"), "{text}");
    }

    #[test]
    fn a_tool_name_cannot_also_be_a_code_suite_namespace() {
        let error = render_code_tool_description(&[spec("deck"), spec("deck.only")])
            .expect_err("nested namespace collision");
        assert!(error.contains("nests under 'deck'"), "{error}");
    }

    #[test]
    fn rejects_tools_without_a_data_contract() {
        let error = FunctionSpec::from_tool(&NoDataTool).unwrap_err();
        assert_eq!(error, "tool 'no_data' has no data schema");
    }

    #[test]
    fn unregistered_lsp_tools_do_not_appear_in_the_description() {
        // WP-211: PRELUDE no longer hard-codes the seven LSP names. When those
        // tools are not admitted (desired=false / not passed), neither the
        // rendered SDK nor the projected whitelist may mention them.
        let tools: Vec<Arc<dyn Tool>> = crate::tool::registry::builtin_code_surface_tools()
            .into_iter()
            .filter(|tool| {
                !crate::tools::lsp::LSP_CODE_TOOL_NAMES
                    .iter()
                    .any(|name| *name == tool.name())
            })
            .collect();
        assert!(
            !tools.is_empty(),
            "fixture must keep non-LSP code-surface tools"
        );
        let specs: Vec<CodeFunctionSpec> = tools
            .iter()
            .filter_map(|tool| FunctionSpec::from_tool(tool.as_ref()).ok()?.into_code())
            .collect();
        let text = render_code_tool_description(&specs).expect("renders without LSP tools");
        let projected = project_tools(tools);
        for name in crate::tools::lsp::LSP_CODE_TOOL_NAMES {
            // Match bindings / types, not the English word "diagnostics" in PRELUDE.
            let binding = format!("[\"{name}\"]");
            let dotted = format!("tools.{name}");
            let type_base = code_type_base(name);
            assert!(
                !text.contains(&binding) && !text.contains(&dotted) && !text.contains(&type_base),
                "description must not bind unadmitted `{name}` (binding/type):\n{text}"
            );
            assert!(
                !projected.sdk.contains(&binding)
                    && !projected.sdk.contains(&dotted)
                    && !projected.sdk.contains(&type_base),
                "projected sdk must not bind unadmitted `{name}`"
            );
            assert!(
                !projected.whitelist.iter().any(|entry| entry == name),
                "projected whitelist must not contain unadmitted `{name}`: {:?}",
                projected.whitelist
            );
        }
    }

}
