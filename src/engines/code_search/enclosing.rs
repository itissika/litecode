//! Enclosing-scope breadcrumbs + syntax-ancestor snippets for agent `grep`.
//!
//! Pipeline: ripgrep finds `(path, line)` → this module annotates with AST parents
//! and (when possible) expands the snippet to the tightest larger syntax node
//! (Zed-aligned, capped at [`MAX_ANCESTOR_LINES`]).
//! Unsupported language, parse failure, or missing name → empty / `None` (caller
//! falls back to line-only heading and ±2 context).
//!
//! Language set (TIOBE Jul 2026 top code langs with useful AST scopes + C#):
//! Python, C, C++, Java, C#, JavaScript/TypeScript, Rust, plus Go.

use std::ops::Range;

use tree_sitter::{Node, Parser, Point, TreeCursor};

/// Cap on ancestor snippet length (Zed `MAX_ANCESTOR_LINES`).
pub const MAX_ANCESTOR_LINES: u32 = 6;

/// Expanded snippet window from a syntax ancestor (1-based inclusive lines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AncestorSnippet {
    pub start_line: u32,
    pub end_line: u32,
    /// Lines in the uncapped ancestor past [`Self::end_line`] (0 if fully shown).
    pub remaining_lines: u32,
}

/// One hop in the enclosing chain (outer → inner order after reverse).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeSegment {
    /// Short kind label: `fn`, `impl`, `struct`, `class`, `mod`, `method`, …
    pub kind: String,
    /// Symbol name when recoverable; otherwise empty (segment dropped).
    pub name: String,
}

impl ScopeSegment {
    pub fn display(&self) -> String {
        if self.name.is_empty() {
            self.kind.clone()
        } else {
            format!("{} {}", self.kind, self.name)
        }
    }
}

/// Human-readable chain, e.g. `impl Foo › fn save`. `None` if no scopes.
pub fn format_breadcrumb(segments: &[ScopeSegment]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }
    Some(
        segments
            .iter()
            .map(ScopeSegment::display)
            .collect::<Vec<_>>()
            .join(" › "),
    )
}

/// Resolve enclosing scopes for a 1-based line. Never errors — returns `[]` on fallback.
pub fn enclosing_scopes(path: &str, content: &str, line_1based: u32) -> Vec<ScopeSegment> {
    let Some(tree) = parse_tree(path, content) else {
        return Vec::new();
    };
    if line_1based == 0 {
        return Vec::new();
    }

    let row = line_1based.saturating_sub(1) as usize;
    let line_count = content.lines().count();
    if row >= line_count {
        return Vec::new();
    }

    // Point at start of the target line (column 0).
    let point = Point::new(row, 0);
    let root = tree.root_node();
    let mut node = root
        .descendant_for_point_range(point, point)
        .unwrap_or(root);
    // Prefer a non-root node when possible.
    if node.id() == root.id() {
        // Try mid-line if the line has content.
        if let Some(line) = content.lines().nth(row) {
            let col = line.len().saturating_sub(1).min(line.len());
            let mid = Point::new(row, col);
            if let Some(n) = root.descendant_for_point_range(mid, mid) {
                node = n;
            }
        }
    }

    let mut chain = Vec::new();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if let Some(seg) = scope_segment(n, content) {
            chain.push(seg);
        }
        cur = n.parent();
    }
    chain.reverse(); // outer → inner
    chain
}

/// Zed-style syntax ancestor snippet for a match's full line span.
///
/// Finds the tightest AST node strictly larger than the match lines, caps the
/// window at [`MAX_ANCESTOR_LINES`] from the node start, and returns `None` when
/// the capped window would not cover the match (caller falls back to ±2).
pub fn syntax_ancestor_snippet(
    path: &str,
    content: &str,
    match_start_line: u32,
    match_end_line: u32,
) -> Option<AncestorSnippet> {
    if match_start_line == 0 || match_end_line < match_start_line || content.is_empty() {
        return None;
    }
    let tree = parse_tree(path, content)?;
    let query = line_byte_range(content, match_start_line, match_end_line)?;
    if query.start >= query.end {
        return None;
    }

    let root = tree.root_node();
    let mut cursor = root.walk();
    if !goto_node_enclosing_range(&mut cursor, &query, true) {
        return None;
    }
    let node = cursor.node();
    // Root-of-file ancestors are rarely useful as snippets.
    if node.id() == root.id() {
        return None;
    }

    let full_start = node.start_position().row as u32;
    let mut full_end = node.end_position().row as u32;
    // tree-sitter often parks end at column 0 of the following line.
    if node.end_position().column == 0 && full_end > full_start {
        full_end -= 1;
    }

    let capped_end = full_end.min(full_start.saturating_add(MAX_ANCESTOR_LINES));
    let match_start_0 = match_start_line - 1;
    let match_end_0 = match_end_line - 1;
    // Capped window must still cover the match (else ±2 fallback).
    if capped_end < match_end_0 || full_start > match_start_0 {
        return None;
    }

    Some(AncestorSnippet {
        start_line: full_start + 1,
        end_line: capped_end + 1,
        remaining_lines: full_end.saturating_sub(capped_end),
    })
}

/// Extract inclusive 1-based line text from source (no trailing newline on last line).
pub fn lines_slice(content: &str, start_line: u32, end_line: u32) -> String {
    content
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let n = i as u32 + 1;
            if n >= start_line && n <= end_line {
                Some(line)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_tree(path: &str, content: &str) -> Option<tree_sitter::Tree> {
    if content.is_empty() {
        return None;
    }
    let lang = language_for_path(path)?;
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;
    parser.parse(content, None)
}

fn line_byte_range(content: &str, start_1: u32, end_1: u32) -> Option<Range<usize>> {
    let mut start_byte = None;
    let mut end_byte = None;
    let mut offset = 0usize;
    for (i, line) in content.split_inclusive('\n').enumerate() {
        let line_no = i as u32 + 1;
        if line_no == start_1 {
            start_byte = Some(offset);
        }
        if line_no == end_1 {
            end_byte = Some(offset + line.len());
            break;
        }
        offset += line.len();
    }
    Some(start_byte?..end_byte?)
}

/// Port of Zed `Buffer::goto_node_enclosing_range` (byte ranges, `require_larger`).
fn goto_node_enclosing_range(
    cursor: &mut TreeCursor<'_>,
    query_range: &Range<usize>,
    require_larger: bool,
) -> bool {
    let mut ascending = false;
    loop {
        let mut range = cursor.node().byte_range();
        if query_range.is_empty() {
            if range.start > query_range.start {
                cursor.goto_previous_sibling();
                range = cursor.node().byte_range();
            }
        } else if range.end == query_range.start {
            cursor.goto_next_sibling();
            range = cursor.node().byte_range();
        }

        let encloses = range_contains_inclusive(&range, query_range)
            && (!require_larger || range.len() > query_range.len());
        if !encloses {
            ascending = true;
            if !cursor.goto_parent() {
                return false;
            }
            continue;
        } else if ascending {
            return true;
        }

        if cursor
            .goto_first_child_for_byte(query_range.start)
            .is_none()
        {
            return true;
        }
    }
}

fn range_contains_inclusive(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && outer.end >= inner.end
}

fn language_for_path(path: &str) -> Option<tree_sitter::Language> {
    let ext = path.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "js" | "jsx" | "mjs" | "cjs" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "cs" => Some(tree_sitter_c_sharp::LANGUAGE.into()),
        _ => None,
    }
}

fn scope_segment(node: Node<'_>, content: &str) -> Option<ScopeSegment> {
    let kind = node.kind();
    let (label, name) = match kind {
        // Rust
        "function_item" => ("fn", field_name(node, content, &["name"])),
        "impl_item" => (
            "impl",
            field_name(node, content, &["type"]).or_else(|| first_type_name(node, content)),
        ),
        "struct_item" => ("struct", field_name(node, content, &["name"])),
        "enum_item" => ("enum", field_name(node, content, &["name"])),
        "trait_item" => ("trait", field_name(node, content, &["name"])),
        "mod_item" => ("mod", field_name(node, content, &["name"])),
        "macro_definition" => ("macro", field_name(node, content, &["name"])),
        // Python (+ C/C++ share `function_definition` node kind)
        "function_definition" => (
            "fn",
            field_name(node, content, &["name"]).or_else(|| c_like_function_name(node, content)),
        ),
        "class_definition" => ("class", field_name(node, content, &["name"])),
        // Go / JS / TS / Java / C#
        "function_declaration" | "generator_function_declaration" => {
            ("fn", field_name(node, content, &["name"]))
        }
        "method_declaration" => ("method", field_name(node, content, &["name"])),
        "type_declaration" => ("type", go_type_spec_name(node, content)),
        "class_declaration" | "abstract_class_declaration" => {
            ("class", field_name(node, content, &["name"]))
        }
        "method_definition" => ("method", field_name(node, content, &["name"])),
        "interface_declaration" => ("interface", field_name(node, content, &["name"])),
        "enum_declaration" => ("enum", field_name(node, content, &["name"])),
        "constructor_declaration" => ("ctor", field_name(node, content, &["name"])),
        "record_declaration" => ("record", field_name(node, content, &["name"])),
        "struct_declaration" => ("struct", field_name(node, content, &["name"])),
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            ("namespace", field_name(node, content, &["name"]))
        }
        // C / C++
        "struct_specifier" => ("struct", field_name(node, content, &["name"])),
        "class_specifier" => ("class", field_name(node, content, &["name"])),
        "namespace_definition" => ("namespace", field_name(node, content, &["name"])),
        "union_specifier" => ("union", field_name(node, content, &["name"])),
        "enum_specifier" => ("enum", field_name(node, content, &["name"])),
        _ => return None,
    };
    let name = name?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(ScopeSegment {
        kind: label.to_string(),
        name,
    })
}

fn field_name(node: Node<'_>, content: &str, fields: &[&str]) -> Option<String> {
    for f in fields {
        if let Some(child) = node.child_by_field_name(f) {
            let t = child.utf8_text(content.as_bytes()).ok()?.trim();
            if !t.is_empty() {
                return Some(simplify_type_text(t));
            }
        }
    }
    None
}

fn first_type_name(node: Node<'_>, content: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "type_identifier" | "identifier" | "generic_type" | "scoped_type_identifier"
        ) {
            let t = child.utf8_text(content.as_bytes()).ok()?.trim();
            if !t.is_empty() {
                return Some(simplify_type_text(t));
            }
        }
    }
    None
}

fn go_type_spec_name(node: Node<'_>, content: &str) -> Option<String> {
    // type_declaration → type_spec → name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec"
            && let Some(n) = field_name(child, content, &["name"])
        {
            return Some(n);
        }
    }
    field_name(node, content, &["name"])
}

/// C/C++ `function_definition` name lives under nested `declarator` / `function_declarator`.
fn c_like_function_name(node: Node<'_>, content: &str) -> Option<String> {
    let decl = node.child_by_field_name("declarator")?;
    deepest_identifier(decl, content)
}

fn deepest_identifier(node: Node<'_>, content: &str) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "field_identifier" | "qualified_identifier"
    ) {
        let t = node.utf8_text(content.as_bytes()).ok()?.trim();
        if !t.is_empty() {
            // `Foo::bar` → keep last segment for readability
            let leaf = t.rsplit("::").next().unwrap_or(t);
            return Some(leaf.to_string());
        }
    }
    if let Some(inner) = node.child_by_field_name("declarator") {
        return deepest_identifier(inner, content);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(n) = deepest_identifier(child, content) {
            return Some(n);
        }
    }
    None
}

fn simplify_type_text(t: &str) -> String {
    // `impl Foo<T> for Bar` field may be long; keep first identifier-ish token run.
    let trimmed = t.split_whitespace().next().unwrap_or(t);
    trimmed
        .trim_matches(|c: char| c == '{' || c == '}')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_of(src: &str, needle: &str) -> u32 {
        src.lines()
            .enumerate()
            .find(|(_, l)| l.contains(needle))
            .map(|(i, _)| i as u32 + 1)
            .unwrap_or_else(|| panic!("needle not found: {needle}"))
    }

    #[test]
    fn rust_nested_impl_method_breadcrumb() {
        let src = r#"
pub struct Store;

impl Store {
    pub fn save(&self, path: &str) {
        let x = path.len();
        let _ = x;
    }
}
"#;
        let segs = enclosing_scopes("store.rs", src, line_of(src, "path.len()"));
        assert_eq!(format_breadcrumb(&segs).unwrap(), "impl Store › fn save");
    }

    #[test]
    fn rust_free_function() {
        let src = "fn alpha() {\n    let hit = 1;\n}\n";
        let segs = enclosing_scopes("a.rs", src, 2);
        assert_eq!(format_breadcrumb(&segs).unwrap(), "fn alpha");
    }

    #[test]
    fn python_class_method() {
        let src = r#"
class Repo:
    def index(self):
        return 42
"#;
        let crumb =
            format_breadcrumb(&enclosing_scopes("r.py", src, line_of(src, "return 42"))).unwrap();
        assert_eq!(crumb, "class Repo › fn index");
    }

    #[test]
    fn typescript_class_method() {
        let src = r#"
export class Engine {
  search(q: string): number {
    return q.length;
  }
}
"#;
        let crumb =
            format_breadcrumb(&enclosing_scopes("e.ts", src, line_of(src, "q.length"))).unwrap();
        assert_eq!(crumb, "class Engine › method search");
    }

    #[test]
    fn go_method_on_type() {
        let src = r#"
package p

type Client struct{}

func (c *Client) Ping() {
	x := 1
	_ = x
}
"#;
        let crumb =
            format_breadcrumb(&enclosing_scopes("c.go", src, line_of(src, "x := 1"))).unwrap();
        assert_eq!(crumb, "method Ping");
    }

    #[test]
    fn csharp_namespace_class_method() {
        let src = r#"
namespace App.Core {
  public class Store {
    public void Save(string path) {
      var n = path.Length;
    }
  }
}
"#;
        let crumb = format_breadcrumb(&enclosing_scopes(
            "Store.cs",
            src,
            line_of(src, "path.Length"),
        ))
        .unwrap();
        assert_eq!(crumb, "namespace App.Core › class Store › method Save");
    }

    #[test]
    fn java_class_method() {
        let src = r#"
public class Engine {
  public int search(String q) {
    return q.length();
  }
}
"#;
        let crumb = format_breadcrumb(&enclosing_scopes(
            "Engine.java",
            src,
            line_of(src, "q.length()"),
        ))
        .unwrap();
        assert_eq!(crumb, "class Engine › method search");
    }

    #[test]
    fn c_function() {
        let src = r#"
int save(const char *path) {
  int n = 0;
  return n;
}
"#;
        let crumb =
            format_breadcrumb(&enclosing_scopes("save.c", src, line_of(src, "int n = 0"))).unwrap();
        assert_eq!(crumb, "fn save");
    }

    #[test]
    fn cpp_namespace_class_method() {
        let src = r#"
namespace app {
class Store {
 public:
  void save(const char* path) {
    int n = 0;
    (void)n;
  }
};
}
"#;
        let crumb = format_breadcrumb(&enclosing_scopes(
            "store.cpp",
            src,
            line_of(src, "int n = 0"),
        ))
        .unwrap();
        assert!(
            crumb.contains("Store") && crumb.contains("save"),
            "expected class/method in {crumb}"
        );
        assert!(
            crumb.contains("namespace") || crumb.starts_with("class"),
            "{crumb}"
        );
    }

    #[test]
    fn unsupported_extension_falls_back_empty() {
        let segs = enclosing_scopes("notes.md", "# title\nhit\n", 2);
        assert!(segs.is_empty());
        assert!(format_breadcrumb(&segs).is_none());
    }

    #[test]
    fn out_of_range_line_falls_back_empty() {
        let segs = enclosing_scopes("a.rs", "fn main() {}\n", 99);
        assert!(segs.is_empty());
    }

    #[test]
    fn top_level_line_outside_fn_may_be_empty_or_mod_only() {
        // Use-item / blank: no function parent — empty is correct fallback.
        let src = "use std::io;\n\nfn main() {}\n";
        let segs = enclosing_scopes("a.rs", src, 1);
        assert!(
            segs.is_empty() || segs.iter().all(|s| s.kind != "fn"),
            "use line should not claim a fn: {segs:?}"
        );
    }

    #[test]
    fn ancestor_expands_if_block() {
        let src = "\
fn method_with_block() {
    let condition = true;
    if condition {
        println!(\"Inside if block\");
    }
}
";
        let hit = line_of(src, "Inside if block");
        let snip = syntax_ancestor_snippet("t.rs", src, hit, hit).expect("ancestor");
        let text = lines_slice(src, snip.start_line, snip.end_line);
        assert!(
            text.contains("if condition") && text.contains("Inside if block"),
            "got snippet L{}-{}:\n{text}",
            snip.start_line,
            snip.end_line
        );
        assert_eq!(snip.remaining_lines, 0);
    }

    #[test]
    fn ancestor_caps_long_function_and_reports_remaining() {
        let mut body = String::from("fn long_function() {\n");
        for i in 1..=12 {
            body.push_str(&format!("    println!(\"Line {i}\");\n"));
        }
        body.push_str("}\n");
        let hit = line_of(&body, "Line 5");
        let snip = syntax_ancestor_snippet("t.rs", &body, hit, hit).expect("ancestor");
        assert_eq!(snip.start_line, 1, "should start at fn");
        assert!(
            snip.end_line - snip.start_line + 1 <= MAX_ANCESTOR_LINES + 1,
            "capped window too large: {snip:?}"
        );
        assert!(
            snip.remaining_lines > 0,
            "long fn should report remaining, got {snip:?}"
        );
        let text = lines_slice(&body, snip.start_line, snip.end_line);
        assert!(text.contains("fn long_function"), "{text}");
        assert!(text.contains("Line 5"), "{text}");
        assert!(
            !text.contains("Line 12"),
            "cap should drop the tail: {text}"
        );
    }

    #[test]
    fn ancestor_near_end_falls_back_none() {
        let mut body = String::from("fn long_function() {\n");
        for i in 1..=12 {
            body.push_str(&format!("    println!(\"Line {i}\");\n"));
        }
        body.push_str("}\n");
        let hit = line_of(&body, "Line 12");
        // Cap from fn start cannot cover Line 12 → None (±2 fallback at call site).
        assert!(syntax_ancestor_snippet("t.rs", &body, hit, hit).is_none());
    }

    #[test]
    fn ancestor_unsupported_lang_is_none() {
        assert!(syntax_ancestor_snippet("a.txt", "hello\nworld\n", 1, 1).is_none());
    }
}
