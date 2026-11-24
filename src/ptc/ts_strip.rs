//! Erasable TypeScript stripper for code-mode programs.
//!
//! Parses wrapped source with `tree-sitter-typescript` and replaces type-only
//! syntax spans with spaces (preserving `\r` and `\n`) so line/column numbers
//! stay 1:1 aligned with the caller's source. Non-erasable TypeScript
//! constructs (`enum`, non-ambient `namespace`/`module`, and constructor
//! parameter properties) are rejected before QuickJS runs.

use tree_sitter::{Node, Parser};

use super::{Failure, FailureKind, SourceLocation};

const WRAPPER_PREFIX: &str = "async function __litecode_ts__() {\n";
const WRAPPER_SUFFIX: &str = "\n}\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TsStripError {
    pub message: String,
    pub location: Option<SourceLocation>,
}

impl TsStripError {
    pub(crate) fn into_failure(self) -> Failure {
        let mut failure = Failure::new(FailureKind::Syntax, self.message, None, None);
        failure.location = self.location;
        failure
    }
}

/// Strip erasable TypeScript syntax from a code-mode function body, preserving
/// line numbers and columns.
pub(crate) fn strip(source: &str) -> Result<String, TsStripError> {
    let wrapped = format!("{WRAPPER_PREFIX}{source}{WRAPPER_SUFFIX}");
    let prefix_len = WRAPPER_PREFIX.len();
    let source_end = prefix_len + source.len();

    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    parser.set_language(&language).map_err(|_| TsStripError {
        message: "failed to initialize TypeScript parser".into(),
        location: None,
    })?;
    let Some(tree) = parser.parse(&wrapped, None) else {
        return Err(TsStripError {
            message: "failed to parse TypeScript source".into(),
            location: None,
        });
    };

    let mut out = source.as_bytes().to_vec();
    visit(tree.root_node(), &mut out, prefix_len, source_end, false)?;

    // `out` was initialized from valid UTF-8 and every erased span aligns with
    // tree-sitter token boundaries where non-newline bytes are replaced by ASCII space.
    String::from_utf8(out).map_err(|_| TsStripError {
        message: "internal error: stripped TypeScript output is not valid UTF-8".into(),
        location: None,
    })
}

fn location_of(node: Node<'_>, prefix_len: usize, source_end: usize) -> Option<SourceLocation> {
    if node.end_byte() <= prefix_len || node.start_byte() >= source_end {
        return None;
    }
    let pos = node.start_position();
    let line = u32::try_from(pos.row).ok()?.max(1);
    let column = u32::try_from(pos.column).ok()? + 1;
    Some(SourceLocation { line, column })
}

fn strip_err(
    message: impl Into<String>,
    node: Node<'_>,
    prefix_len: usize,
    source_end: usize,
) -> TsStripError {
    TsStripError {
        message: message.into(),
        location: location_of(node, prefix_len, source_end),
    }
}

fn erase_range(
    out: &mut [u8],
    abs_start: usize,
    abs_end: usize,
    prefix_len: usize,
    source_end: usize,
) {
    let start = abs_start.clamp(prefix_len, source_end) - prefix_len;
    let end = abs_end.clamp(prefix_len, source_end) - prefix_len;
    if start >= end {
        return;
    }
    for byte in &mut out[start..end] {
        if !matches!(*byte, b'\n' | b'\r') {
            *byte = b' ';
        }
    }
}

fn erase_node(out: &mut [u8], node: Node<'_>, prefix_len: usize, source_end: usize) {
    erase_range(out, node.start_byte(), node.end_byte(), prefix_len, source_end);
}

fn is_this_parameter(node: Node<'_>) -> bool {
    if !matches!(node.kind(), "required_parameter" | "optional_parameter") {
        return false;
    }
    if let Some(pattern) = node.child_by_field_name("pattern") {
        return pattern.kind() == "this";
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "this")
}

fn has_parameter_property_modifier(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        matches!(
            child.kind(),
            "accessibility_modifier" | "override_modifier" | "readonly"
        )
    })
}

fn is_function_boundary(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "function_expression"
            | "arrow_function"
            | "method_definition"
            | "generator_function"
            | "generator_function_declaration"
            | "class_static_block"
    )
}

fn return_has_expression(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() != "comment")
}

fn visit(
    node: Node<'_>,
    out: &mut [u8],
    prefix_len: usize,
    source_end: usize,
    in_nested_function: bool,
) -> Result<(), TsStripError> {
    if node.end_byte() <= prefix_len || node.start_byte() >= source_end {
        return Ok(());
    }

    let in_user_source = node.start_byte() >= prefix_len && node.end_byte() <= source_end;
    let child_in_nested = in_nested_function || (in_user_source && is_function_boundary(node.kind()));
    match node.kind() {
        "return_statement" if in_user_source && !in_nested_function => {
            if return_has_expression(node) {
                return Err(strip_err(
                    "top-level `return <value>` is not supported; call `inspect(value)` to emit output (bare `return;` is allowed for early exit)",
                    node,
                    prefix_len,
                    source_end,
                ));
            }
        }
        "enum_declaration" if in_user_source => {
            return Err(strip_err(
                "erasable TypeScript subset does not support `enum` declarations; use a const object or union type",
                node,
                prefix_len,
                source_end,
            ));
        }
        "internal_module" if in_user_source => {
            return Err(strip_err(
                "erasable TypeScript subset does not support `namespace` declarations",
                node,
                prefix_len,
                source_end,
            ));
        }
        "required_parameter" | "optional_parameter" if in_user_source => {
            if has_parameter_property_modifier(node) {
                return Err(strip_err(
                    "erasable TypeScript subset does not support constructor parameter properties (`public`/`private`/`protected`/`readonly`)",
                    node,
                    prefix_len,
                    source_end,
                ));
            }
            if node.kind() == "optional_parameter" {
                let value_start = node
                    .child_by_field_name("value")
                    .map(|value| value.start_byte())
                    .unwrap_or(usize::MAX);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "?" && child.end_byte() <= value_start {
                        erase_node(out, child, prefix_len, source_end);
                    }
                }
            }
        }
        "type_alias_declaration"
        | "interface_declaration"
        | "ambient_declaration"
        | "function_signature"
        | "abstract_method_signature"
        | "method_signature"
        | "index_signature"
        | "type_annotation"
        | "adding_type_annotation"
        | "opting_type_annotation"
        | "omitting_type_annotation"
        | "type_predicate_annotation"
        | "asserts_annotation"
        | "type_parameters"
        | "type_arguments"
        | "implements_clause"
        | "accessibility_modifier"
        | "override_modifier"
            if in_user_source =>
        {
            erase_node(out, node, prefix_len, source_end);
            return Ok(());
        }
        "abstract_class_declaration" if in_user_source => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "abstract" {
                    erase_node(out, child, prefix_len, source_end);
                }
            }
        }
        "public_field_definition" if in_user_source => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            if children
                .iter()
                .any(|child| matches!(child.kind(), "declare" | "abstract"))
            {
                erase_node(out, node, prefix_len, source_end);
                return Ok(());
            }
            let value_start = node
                .child_by_field_name("value")
                .map(|value| value.start_byte())
                .unwrap_or(usize::MAX);
            for child in children {
                if child.kind() == "readonly"
                    || (child.kind() == "!" && child.end_byte() <= value_start)
                {
                    erase_node(out, child, prefix_len, source_end);
                }
            }
        }
        "variable_declarator" if in_user_source => {
            let value_start = node
                .child_by_field_name("value")
                .map(|value| value.start_byte())
                .unwrap_or(usize::MAX);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "!" && child.end_byte() <= value_start {
                    erase_node(out, child, prefix_len, source_end);
                }
            }
        }
        "formal_parameters" if in_user_source => {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            let mut i = 0;
            while i < children.len() {
                let child = children[i];
                if is_this_parameter(child) {
                    erase_node(out, child, prefix_len, source_end);
                    if i + 1 < children.len() && children[i + 1].kind() == "," {
                        erase_node(out, children[i + 1], prefix_len, source_end);
                        i += 2;
                        continue;
                    }
                    if i > 0 && children[i - 1].kind() == "," {
                        erase_node(out, children[i - 1], prefix_len, source_end);
                    }
                    i += 1;
                    continue;
                }
                visit(child, out, prefix_len, source_end, child_in_nested)?;
                i += 1;
            }
            return Ok(());
        }
        "as_expression" | "satisfies_expression" | "non_null_expression" if in_user_source => {
            if let Some(expr) = node.named_child(0) {
                visit(expr, out, prefix_len, source_end, child_in_nested)?;
                erase_range(out, expr.end_byte(), node.end_byte(), prefix_len, source_end);
            }
            return Ok(());
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, out, prefix_len, source_end, child_in_nested)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::strip;

    #[test]
    fn strips_annotations_interfaces_types_and_preserves_lines() {
        let src = concat!(
            "interface Item {\n",
            "  path: string;\n",
            "  count?: number;\n",
            "}\n",
            "type MaybeItem = Item | null;\n",
            "const items: Item[] = [];\n",
            "const name = (items[0] as Item | undefined)?.path ?? \"none\";\n",
            "const forced = name!;\n",
            "const cfg = { ok: true } satisfies Record<string, boolean>;\n",
            "function pick<T>(xs: T[], fallback?: T): T | undefined {\n",
            "  return xs[0] ?? fallback;\n",
            "}\n",
            "const arrow = async <T extends { id: number }>(v: T): Promise<number> => v.id;\n",
            "inspect({ count: items.length, forced, cfg, v: await arrow({ id: 7 }), p: pick([1, 2]) });\n",
        );
        let out = strip(src).expect("strip succeeds");
        assert_eq!(out.lines().count(), src.lines().count());
        assert!(!out.contains("interface Item"));
        assert!(!out.contains("MaybeItem"));
        assert!(!out.contains("satisfies"));
        assert!(!out.contains(" as "));
        assert!(out.contains("const items         = [];"));
        assert!(out.contains("const cfg = { ok: true }"));
        assert!(out.contains("function pick   (xs     , fallback    )"));
        assert!(out.contains("const arrow = async"));
        assert!(out.contains("=> v.id;"));
    }

    #[test]
    fn strips_this_parameter_type_assertions_and_definite_assignments() {
        let src = concat!(
            "function method(this: { n: number }, delta?: number): number {\n",
            "  return this.n + (delta ?? 0);\n",
            "}\n",
            "let assigned!: string;\n",
            "assigned = <string>\"ok\";\n",
            "const arr = [1, 2] as const;\n",
            "inspect([method.call({ n: 5 }, 3), assigned, arr.length]);\n",
        );
        let out = strip(src).expect("strip succeeds");
        assert!(!out.contains("this:"));
        assert!(!out.contains("!:"));
        assert!(!out.contains("<string>"));
        assert!(!out.contains("as const"));
        let normalized = out.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(normalized.contains("function method( delta )"), "{out}");
        assert!(normalized.contains("let assigned ;"), "{out}");
    }

    #[test]
    fn strips_abstract_classes_implements_and_readonly_fields() {
        let src = concat!(
            "interface Greeter { greet(): string; }\n",
            "abstract class Base<T> implements Greeter {\n",
            "  declare ghost: string;\n",
            "  public readonly label: string = \"hi\";\n",
            "  abstract greet(): string;\n",
            "}\n",
            "class Child extends Base<number> {\n",
            "  public override greet(): string { return this.label; }\n",
            "}\n",
            "inspect(new Child().greet());\n",
        );
        let out = strip(src).expect("strip succeeds");
        assert!(!out.contains("implements Greeter"));
        assert!(!out.contains("ghost"));
        assert!(!out.contains("abstract "));
        assert!(out.contains("         class Base"));
        assert!(out.contains("class Child extends Base"));
        assert!(out.contains("inspect(new Child().greet());"));
    }

    #[test]
    fn preserves_plain_js_properties_and_ternary_operators() {
        let src = concat!(
            "const obj = { as: 1, satisfies: 2, type: \"ok\", interface: 3 };\n",
            "const v = obj.as > 0 ? obj.satisfies : obj.interface;\n",
            "inspect(`${obj.type}:${v}`);\n",
        );
        let out = strip(src).expect("strip succeeds");
        assert_eq!(out, src);
    }

    #[test]
    fn rejects_top_level_return_with_value_while_allowing_bare_and_nested_returns() {
        let err = strip("const x = 1;\nreturn x;\n").expect_err("top-level return value must fail");
        assert!(err.message.contains("inspect(value)"), "{}", err.message);
        assert_eq!(err.location.map(|l| l.line), Some(2));

        let branch_err =
            strip("if (true) {\n  return { ok: true };\n}\n").expect_err("branch return must fail");
        assert!(branch_err.message.contains("inspect(value)"), "{}", branch_err.message);
        assert_eq!(branch_err.location.map(|l| l.line), Some(2));

        let try_err = strip("try {\n  return await tools.read({});\n} catch (e) {}\n")
            .expect_err("try return must fail");
        assert_eq!(try_err.location.map(|l| l.line), Some(2));

        // Bare top-level return (with or without comments) and nested function returns stay valid.
        let ok = strip(concat!(
            "function helper(n: number): number { return n + 1; }\n",
            "const arrow = (x: number) => { return x * 2; };\n",
            "if (helper(1) === 2) { inspect(arrow(3)); return /* early exit */; }\n",
            "return;\n",
        ));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn rejects_non_erasable_enum_namespace_and_parameter_properties() {
        let enum_err = strip("enum Color {\n  Red,\n}\n").expect_err("enum must fail");
        assert!(enum_err.message.contains("enum"));
        assert_eq!(enum_err.location.map(|l| l.line), Some(1));

        let ns_err = strip("namespace N { export const x = 1; }\n").expect_err("namespace must fail");
        assert!(ns_err.message.contains("namespace"));

        let prop_err = strip("class A { constructor(public x: number) {} }\n")
            .expect_err("parameter property must fail");
        assert!(prop_err.message.contains("parameter properties"));

        let ro_err = strip("class A { constructor(readonly x: number) {} }\n")
            .expect_err("readonly parameter property must fail");
        assert!(ro_err.message.contains("parameter properties"));
    }
}
