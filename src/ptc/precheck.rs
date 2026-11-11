//! Static check before a program runs.
//!
//! Report-only by design. Typo-class errors (a tool name, an argument, a result
//! field) are cheap to see in the source and expensive to discover at runtime,
//! where they cost the whole run. Every finding is a warning on the same
//! channel the run uses for survived contract errors: the program still runs,
//! and the model reads the list once at the end.
//!
//! The checker is deliberately shallow. It parses the program as a function
//! body, walks the tree once, and only judges what it can prove from an inline
//! literal or a single `const x = await tools…` binding. Anything else —
//! a computed key, a spread, a reassigned variable, a parse error — is silent:
//! a checker bug must never change what the program does.

use std::collections::BTreeSet;

use serde_json::Value;
use tree_sitter::Node;

use crate::tool::code_contract::{CodeSurface, CodeToolContract};

/// Findings for one program, in source order, deduped.
pub(crate) fn check(source: &str, surface: &CodeSurface) -> Vec<String> {
    let mut parser = tree_sitter::Parser::new();
    let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    // A program is a function body; wrapping it keeps `return` and top-level
    // `await` valid, so the parse matches what the engine will compile.
    let wrapped = format!("async function __litecode_precheck() {{\n{source}\n}}");
    let Some(tree) = parser.parse(&wrapped, None) else {
        return Vec::new();
    };
    if tree.root_node().has_error() {
        // The engine reports syntax errors; a second, worse guess adds noise.
        return Vec::new();
    }
    let bytes = wrapped.as_bytes();
    let mut found = Vec::new();
    visit_calls(tree.root_node(), bytes, surface, &mut found);
    visit_results(tree.root_node(), bytes, surface, &mut found);
    let mut seen = BTreeSet::new();
    found
        .into_iter()
        .filter(|line| seen.insert(line.clone()))
        .collect()
}

fn visit_calls(node: Node<'_>, src: &[u8], surface: &CodeSurface, out: &mut Vec<String>) {
    if node.kind() == "call_expression" {
        if let Some(function) = node.child_by_field_name("function") {
            if let Some(path) = dotted_path(function, src) {
                if path.first().is_some_and(|root| root == "tools") && path.len() >= 2 {
                    check_call(&path, node, src, surface, out);
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_calls(child, src, surface, out);
    }
}

fn check_call(
    path: &[String],
    call: Node<'_>,
    src: &[u8],
    surface: &CodeSurface,
    out: &mut Vec<String>,
) {
    // A real tool is `tools.suite.member`. Longer paths are function
    // properties (`read.call`), which the namespace never guards.
    if !(2..=3).contains(&path.len()) {
        return;
    }
    let key = path[1..].join(".");
    let full = format!("tools.{key}");
    let Some(contract) = surface
        .bindings
        .get(&key)
        .and_then(|tool| surface.tools.get(tool))
    else {
        // Name what the caller could have meant: the suite's members when the
        // suite is real, the suites when it is not.
        let suite = &path[1];
        let members = members_of(surface, suite);
        let message = if members.is_empty() {
            let suites = suites_of(surface);
            format!("tool 'tools.{suite}' not found. available: {suites}")
        } else if path.len() == 2 {
            format!("'tools.{suite}' is a suite, not a tool; call one of: {members}")
        } else {
            format!("tool '{full}' not found. available: {members}")
        };
        out.push(message);
        return;
    };
    // Arguments are reported under the SDK path the schema message uses
    // ("file.read"), so the warning and the call's own failure read as one.
    check_arguments(call, src, &key, contract, out);
}

fn check_arguments(
    call: Node<'_>,
    src: &[u8],
    sdk_path: &str,
    contract: &CodeToolContract,
    out: &mut Vec<String>,
) {
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return;
    };
    let mut cursor = arguments.walk();
    let named: Vec<Node<'_>> = arguments.named_children(&mut cursor).collect();
    if named.len() != 1 {
        out.push(format!(
            "'{sdk_path}' accepts exactly one argument, got {}",
            named.len()
        ));
        return;
    }
    let object = named[0];
    if object.kind() != "object" {
        return;
    }
    let Some(properties) = contract.input_schema.get("properties").and_then(Value::as_object)
    else {
        return;
    };
    let mut keys = Vec::new();
    let mut object_cursor = object.walk();
    for item in object.named_children(&mut object_cursor) {
        let key = match item.kind() {
            "pair" => match item.child_by_field_name("key") {
                Some(key) if matches!(key.kind(), "property_identifier" | "string") => {
                    node_text(key, src)
                }
                // A computed key means the shape is not provable.
                _ => return,
            },
            "shorthand_property_identifier" => node_text(item, src),
            // Spread or a method: the shape is not provable.
            _ => return,
        };
        keys.push(key);
    }
    let mut known: Vec<&str> = properties.keys().map(String::as_str).collect();
    known.sort();
    for key in &keys {
        if !properties.contains_key(key) {
            out.push(format!(
                "invalid input for '{sdk_path}': unknown parameter '{key}'. parameters: {}",
                known.join(", ")
            ));
        }
    }
    if let Some(required) = contract.input_schema.get("required").and_then(Value::as_array) {
        for name in required.iter().filter_map(Value::as_str) {
            if !keys.iter().any(|key| key == name) {
                out.push(format!(
                    "invalid input for '{sdk_path}': missing required parameter '{name}'"
                ));
            }
        }
    }
}

fn visit_results(node: Node<'_>, src: &[u8], surface: &CodeSurface, out: &mut Vec<String>) {
    if node.kind() == "variable_declarator" {
        check_declarator(node, src, surface, out);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit_results(child, src, surface, out);
    }
}

/// `const x = await tools.s.m(…)` followed by `x.field`: only this shape, and
/// only when `x` is never reassigned.
fn check_declarator(node: Node<'_>, src: &[u8], surface: &CodeSurface, out: &mut Vec<String>) {
    let Some(name) = node.child_by_field_name("name").filter(|name| name.kind() == "identifier")
    else {
        return;
    };
    let Some(value) = node.child_by_field_name("value") else {
        return;
    };
    if value.kind() != "await_expression" {
        return;
    }
    let Some(inner) = value.named_child(0) else {
        return;
    };
    if inner.kind() != "call_expression" {
        return;
    }
    let Some(function) = inner.child_by_field_name("function") else {
        return;
    };
    let Some(path) = dotted_path(function, src) else {
        return;
    };
    if path.first().is_none_or(|root| root != "tools") || path.len() < 3 {
        return;
    }
    let key = path[1..].join(".");
    let Some(contract) = surface
        .bindings
        .get(&key)
        .and_then(|tool| surface.tools.get(tool))
    else {
        return;
    };
    let Some(properties) = contract.output_schema.get("properties").and_then(Value::as_object)
    else {
        return;
    };
    let variable = node_text(name, src);
    let Some(block) = enclosing_block(node) else {
        return;
    };
    if reassigned(block, &variable, src) {
        return;
    }
    let mut cursor = block.walk();
    for read in block.named_children(&mut cursor) {
        check_reads(read, &variable, &key, properties, src, out);
    }
}

fn check_reads(
    node: Node<'_>,
    variable: &str,
    key: &str,
    properties: &serde_json::Map<String, Value>,
    src: &[u8],
    out: &mut Vec<String>,
) {
    if node.kind() == "member_expression" {
        let object = node.child_by_field_name("object");
        let property = node.child_by_field_name("property");
        if let (Some(object), Some(property)) = (object, property) {
            if object.kind() == "identifier"
                && node_text(object, src) == variable
                && property.kind() == "property_identifier"
            {
                let field = node_text(property, src);
                // Every result may be a failure envelope, where these exist.
                let envelope = matches!(field.as_str(), "ok" | "reason" | "message");
                if !envelope && !properties.contains_key(&field) {
                    let mut known: Vec<&str> = properties.keys().map(String::as_str).collect();
                    known.sort();
                    out.push(format!(
                        "'tools.{key}' result has no field '{field}'. fields: {}",
                        known.join(", ")
                    ));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        check_reads(child, variable, key, properties, src, out);
    }
}

/// The `const` statement's containing block: the unit a one-level read lives in.
fn enclosing_block(node: Node<'_>) -> Option<Node<'_>> {
    let mut current = node.parent()?;
    loop {
        if current.kind() == "statement_block" {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn reassigned(block: Node<'_>, variable: &str, src: &[u8]) -> bool {
    if block.kind() == "identifier" && node_text(block, src) == variable {
        return block.parent().is_some_and(|parent| {
            matches!(parent.kind(), "assignment_expression" | "update_expression")
        });
    }
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .any(|child| reassigned(child, variable, src))
}

fn members_of(surface: &CodeSurface, suite: &str) -> String {
    let prefix = format!("{suite}.");
    let mut members: Vec<&str> = surface
        .bindings
        .keys()
        .filter_map(|key| key.strip_prefix(&prefix))
        .collect();
    members.sort();
    members.join(", ")
}

fn suites_of(surface: &CodeSurface) -> String {
    let mut suites: Vec<&str> = surface
        .bindings
        .keys()
        .filter_map(|key| key.split('.').next())
        .collect();
    suites.sort();
    suites.dedup();
    suites.join(", ")
}

fn dotted_path(node: Node<'_>, src: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![node_text(node, src)]),
        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;
            if property.kind() != "property_identifier" {
                return None;
            }
            let mut path = dotted_path(object, src)?;
            path.push(node_text(property, src));
            Some(path)
        }
        _ => None,
    }
}

fn node_text(node: Node<'_>, src: &[u8]) -> String {
    node.utf8_text(src).unwrap_or_default().to_string()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::code_contract::CodeToolContract;
    use serde_json::json;

    fn surface() -> CodeSurface {
        let input = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "limit": {"type": "number"}
            },
            "required": ["path"],
            "additionalProperties": false
        });
        let output = json!({
            "type": "object",
            "properties": {
                "ok": {"type": "boolean"},
                "path": {"type": "string"},
                "start_line": {"type": "number"},
                "lines": {"type": "array"}
            },
            "required": ["ok", "path"]
        });
        CodeSurface {
            sdk: String::new(),
            sdk_hash: String::new(),
            tools: [(
                "read".to_string(),
                CodeToolContract {
                    input_schema: input,
                    output_schema: output,
                },
            )]
            .into(),
            bindings: [("file.read".to_string(), "read".to_string())].into(),
        }
    }

    #[test]
    fn a_clean_program_has_no_findings() {
        let findings = check(
            r#"
            const r = await tools.file.read({ path: "a.txt", limit: 2 });
            return r.lines;
            "#,
            &surface(),
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_misspelled_tool_is_named_with_its_suite() {
        let findings = check(r#"await tools.file.readx({ path: "a" });"#, &surface());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("tools.file.readx"), "{findings:?}");
        assert!(findings[0].contains("available: read"), "{findings:?}");
    }

    #[test]
    fn a_misspelled_suite_lists_the_suites() {
        let findings = check(r#"await tools.fiel.read({ path: "a" });"#, &surface());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("tools.fiel"), "{findings:?}");
        assert!(findings[0].contains("available: file"), "{findings:?}");
    }

    #[test]
    fn an_unknown_parameter_names_the_real_ones() {
        let findings = check(r#"await tools.file.read({ path: "a", limt: 1 });"#, &surface());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].contains("unknown parameter 'limt'"),
            "{findings:?}"
        );
        assert!(findings[0].contains("limit"), "{findings:?}");
    }

    #[test]
    fn a_missing_required_parameter_is_named() {
        let findings = check(r#"await tools.file.read({ limit: 1 });"#, &surface());
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].contains("missing required parameter 'path'"),
            "{findings:?}"
        );
    }

    #[test]
    fn a_result_field_the_tool_does_not_have_is_named() {
        let findings = check(
            r#"
            const r = await tools.file.read({ path: "a" });
            return r.data;
            "#,
            &surface(),
        );
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("no field 'data'"), "{findings:?}");
        assert!(findings[0].contains("start_line"), "{findings:?}");
    }

    #[test]
    fn a_computed_key_or_spread_is_not_guessed() {
        let findings = check(
            r#"
            const args = { path: "a", limt: 1 };
            await tools.file.read(args);
            await tools.file.read({ ...args });
            "#,
            &surface(),
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_reassigned_result_is_not_checked() {
        let findings = check(
            r#"
            let r = await tools.file.read({ path: "a" });
            r = await tools.file.read({ path: "b" });
            return r.data;
            "#,
            &surface(),
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_function_property_is_not_a_tool_path() {
        let findings = check(
            r#"await tools.file.read.call({ path: "a" });"#,
            &surface(),
        );
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn a_broken_program_is_silent() {
        let findings = check("const = ;", &surface());
        assert!(findings.is_empty(), "{findings:?}");
    }
}
