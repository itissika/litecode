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

/// One named scope in a file, outer chain first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEntry {
    /// Human-readable breadcrumb (`impl Store › fn save`), kept for grep snippets.
    pub chain: String,
    /// Space-free canonical dot-separated symbol anchor (`Store.save`, `resolve`, `Store.Greet.hi`).
    pub anchor: String,
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
}

/// Lookup of one symbol anchor inside a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeMatch {
    Unique(ScopeEntry),
    Missing,
    Ambiguous(Vec<ScopeEntry>),
}

/// Every named scope in the file, document order. Unsupported languages yield `[]`.
pub fn list_scopes(path: &str, content: &str) -> Vec<ScopeEntry> {
    let Some(tree) = parse_tree(path, content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = Vec::new();
    walk_scopes(tree.root_node(), content, &mut stack, &mut out);
    out
}

/// Normalize a symbol anchor query: trim and treat `::` as `.`.
pub fn normalize_symbol_query(raw: &str) -> String {
    raw.trim().replace("::", ".")
}

/// Find `anchor` (`resolve`, `Store.save`, `Store::save`, `Store.Greet.hi`).
/// More than one hit is ambiguous.
pub fn find_scope(path: &str, content: &str, anchor: &str) -> ScopeMatch {
    let anchor = normalize_symbol_query(anchor);
    if anchor.is_empty() {
        return ScopeMatch::Missing;
    }
    match_scopes(&list_scopes(path, content), &anchor)
}

/// [`find_scope`] over a scope list that was already parsed, so one parse can
/// answer many anchors.
pub fn match_scopes(scopes: &[ScopeEntry], query: &str) -> ScopeMatch {
    let norm = normalize_symbol_query(query);
    if norm.is_empty() || norm.chars().any(char::is_whitespace) {
        return ScopeMatch::Missing;
    }
    let q_segs: Vec<&str> = norm.split('.').collect();
    if q_segs.iter().any(|s| s.is_empty()) {
        return ScopeMatch::Missing;
    }

    // Pass 1: exact match on canonical anchor segments.
    let exact: Vec<ScopeEntry> = scopes
        .iter()
        .filter(|entry| {
            let parts: Vec<&str> = entry.anchor.split('.').collect();
            parts == q_segs
        })
        .cloned()
        .collect();
    if !exact.is_empty() {
        return scope_hits(prefer_definitions(exact));
    }

    // Pass 2: segment-boundary suffix match on canonical anchor or trait-impl aliases.
    let relaxed: Vec<ScopeEntry> = scopes
        .iter()
        .filter(|entry| entry_matches_relaxed(entry, &q_segs))
        .cloned()
        .collect();
    scope_hits(prefer_definitions(relaxed))
}

/// True when the file extension has a tree-sitter grammar, i.e. symbol
/// citations into this file can resolve at all.
pub fn supports_scopes(path: &str) -> bool {
    language_for_path(path).is_some()
}

fn scope_hits(hits: Vec<ScopeEntry>) -> ScopeMatch {
    match hits.len() {
        0 => ScopeMatch::Missing,
        1 => ScopeMatch::Unique(hits.into_iter().next().expect("one hit")),
        _ => ScopeMatch::Ambiguous(hits),
    }
}

/// When a Rust file defines `struct Store` and `impl Store`, `#Store` resolves
/// to the type definition rather than colliding with the `impl` container block.
fn prefer_definitions(hits: Vec<ScopeEntry>) -> Vec<ScopeEntry> {
    if hits.len() > 1 && hits.iter().any(|e| e.kind != "impl") {
        hits.into_iter().filter(|e| e.kind != "impl").collect()
    } else {
        hits
    }
}

fn entry_matches_relaxed(entry: &ScopeEntry, q_segs: &[&str]) -> bool {
    let primary: Vec<&str> = entry.anchor.split('.').collect();
    if primary.len() > q_segs.len() && primary[primary.len() - q_segs.len()..] == *q_segs {
        return true;
    }
    for alias in trait_impl_aliases(&entry.chain) {
        let parts: Vec<&str> = alias.iter().map(String::as_str).collect();
        if parts == q_segs
            || (parts.len() > q_segs.len() && parts[parts.len() - q_segs.len()..] == *q_segs)
        {
            return true;
        }
    }
    false
}

/// Alternative dot-paths for `impl Trait for Type` scopes:
/// - `[Type, ...rest]` (e.g. `Store.hi` when unique on `Store`)
/// - `[Trait, ...rest]` (e.g. `Greet.hi`)
/// - `[Trait, Type, ...rest]` (e.g. `Greet.Store.hi`)
fn trait_impl_aliases(chain: &str) -> Vec<Vec<String>> {
    let segs: Vec<&str> = chain.split(" › ").collect();
    let mut variants: Vec<Vec<String>> = vec![Vec::new()];
    let mut saw_trait_impl = false;
    for seg in segs {
        if let Some(body) = seg.strip_prefix("impl ")
            && let Some((tr, ty)) = body.rsplit_once(" for ")
        {
            let ty_id = clean_anchor_ident(ty);
            let tr_id = clean_anchor_ident(tr);
            if !ty_id.is_empty() && !tr_id.is_empty() {
                saw_trait_impl = true;
                let mut next = Vec::with_capacity(variants.len() * 3);
                for base in &variants {
                    let mut by_ty = base.clone();
                    by_ty.push(ty_id.clone());
                    next.push(by_ty);
                    let mut by_tr = base.clone();
                    by_tr.push(tr_id.clone());
                    next.push(by_tr);
                    let mut tr_ty = base.clone();
                    tr_ty.push(tr_id.clone());
                    tr_ty.push(ty_id.clone());
                    next.push(tr_ty);
                }
                variants = next;
                continue;
            }
        }
        let name = seg.split_once(' ').map(|(_, n)| n).unwrap_or(seg);
        let id = clean_anchor_ident(name);
        if !id.is_empty() {
            for base in &mut variants {
                base.push(id.clone());
            }
        }
    }
    if saw_trait_impl { variants } else { Vec::new() }
}

/// Strip generics (`<...>`), module qualifiers (`a::B` → `B`), and stray spaces/punctuation
/// to form a single clean identifier segment.
fn clean_anchor_ident(raw: &str) -> String {
    let trimmed = raw.trim();
    let no_generics = trimmed.split('<').next().unwrap_or(trimmed).trim();
    let last_scoped = no_generics.rsplit("::").next().unwrap_or(no_generics).trim();
    let token = last_scoped.split_whitespace().next().unwrap_or(last_scoped);
    token
        .trim_matches(|c: char| matches!(c, '&' | '*' | '(' | ')' | '{' | '}' | '[' | ']'))
        .to_string()
}

fn format_anchor(stack: &[ScopeSegment]) -> String {
    let mut parts = Vec::with_capacity(stack.len() + 1);
    for seg in stack {
        if seg.kind == "impl"
            && let Some((tr, ty)) = seg.name.rsplit_once(" for ")
        {
            let ty_id = clean_anchor_ident(ty);
            let tr_id = clean_anchor_ident(tr);
            if !ty_id.is_empty() {
                parts.push(ty_id);
            }
            if !tr_id.is_empty() {
                parts.push(tr_id);
            }
            continue;
        }
        let id = clean_anchor_ident(&seg.name);
        if !id.is_empty() {
            parts.push(id);
        }
    }
    parts.join(".")
}

/// Tightest named scope that fully contains the inclusive 1-based line range.
/// A selection that spans two siblings and only fits their parent returns the parent.
/// A selection with no containing named scope returns `None`.
pub fn scope_at(path: &str, content: &str, start_line: u32, end_line: u32) -> Option<ScopeEntry> {
    if start_line == 0 || end_line < start_line {
        return None;
    }
    list_scopes(path, content)
        .into_iter()
        .filter(|entry| entry.start_line <= start_line && entry.end_line >= end_line)
        .min_by_key(|entry| {
            (
                entry.end_line.saturating_sub(entry.start_line),
                usize::MAX - entry.chain.len(),
            )
        })
}

fn walk_scopes(
    node: Node<'_>,
    content: &str,
    stack: &mut Vec<ScopeSegment>,
    out: &mut Vec<ScopeEntry>,
) {
    let pushed = if let Some(segment) = scope_segment(node, content) {
        stack.push(segment);
        if let Some(chain) = format_breadcrumb(stack) {
            let anchor = format_anchor(stack);
            let leaf = stack.last().expect("just pushed");
            let (start_line, end_line) = node_line_span(node);
            if !anchor.is_empty() {
                out.push(ScopeEntry {
                    chain,
                    anchor,
                    kind: leaf.kind.clone(),
                    name: leaf.name.clone(),
                    start_line,
                    end_line,
                });
            }
        }
        true
    } else {
        false
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_scopes(child, content, stack, out);
    }
    if pushed {
        stack.pop();
    }
}

fn node_line_span(node: Node<'_>) -> (u32, u32) {
    let start = node.start_position().row as u32 + 1;
    let mut end = node.end_position().row as u32 + 1;
    if node.end_position().column == 0 && end > start {
        end -= 1;
    }
    (start, end.max(start))
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
        "impl_item" => ("impl", impl_scope_name(node, content)),
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

/// `impl Store` stays `Store`. `impl Display for Store` keeps the trait.
fn impl_scope_name(node: Node<'_>, content: &str) -> Option<String> {
    let ty = raw_field(node, content, "type").or_else(|| first_type_name(node, content))?;
    if let Some(tr) = raw_field(node, content, "trait") {
        Some(format!("{tr} for {ty}"))
    } else {
        Some(ty)
    }
}

fn raw_field(node: Node<'_>, content: &str, field: &str) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    let text = child.utf8_text(content.as_bytes()).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.trim_end_matches('{').trim();
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed.to_string())
    }
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
mod tests {}
