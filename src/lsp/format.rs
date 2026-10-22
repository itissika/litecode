//! Agent-facing formatting of LSP JSON results.

use std::path::Path;

use serde_json::Value;

use crate::lsp::uri::uri_to_path;

/// WP-206 ⑧: agent-facing path for an LSP `uri`, consistent with read/grep/glob.
///
/// Inside one of `roots` → workspace-relative (`/`-separated, via
/// [`crate::workspace::filter::cheap_rel_under`]); outside (or no roots) →
/// absolute. Pure string-prefix work: symlinks are **not** resolved, so a
/// path reported through a symlink keeps the form the server sent. The first
/// root that matches wins (callers pass the canonical root first, then the
/// as-configured root).
pub(crate) fn display_path_for_uri(uri: &str, roots: &[&Path]) -> String {
    match uri_to_path(uri) {
        Some(p) => display_path(&p, roots),
        None => uri.to_string(),
    }
}

pub(crate) fn display_path(path: &Path, roots: &[&Path]) -> String {
    for root in roots {
        if root.as_os_str().is_empty() {
            continue;
        }
        if let Some(rel) = crate::workspace::filter::cheap_rel_under(root, path) {
            let rel = rel.trim_start_matches('/');
            return if rel.is_empty() { ".".to_string() } else { rel.to_string() };
        }
    }
    path.to_string_lossy().into_owned()
}

pub(crate) fn format_action_result(action: &str, result: &Value, roots: &[&Path]) -> String {
    match action {
        "goToDefinition" | "findReferences" | "goToImplementation" => {
            format_locations(result, roots)
        }
        "hover" => format_hover(result),
        "diagnostics" => format_diagnostics(result),
        "documentSymbol" | "workspaceSymbol" => format_symbols(result),
        "prepareCallHierarchy" | "incomingCalls" | "outgoingCalls" => {
            format_call_hierarchy(result, roots)
        }
        _ => result.to_string(),
    }
}

fn format_locations(result: &Value, roots: &[&Path]) -> String {
    let locations: Vec<(String, u64, u64)> = extract_location_items(result, roots)
        .into_iter()
        .map(|loc| (loc.path, loc.line, loc.character))
        .collect();
    if locations.is_empty() {
        // Only reached when the language server index is known-settled.
        return "No locations found (language server ready; no definition/reference at this position)."
            .to_string();
    }
    locations
        .iter()
        .map(|(path, line, _ch)| crate::tool::format_path_lines(path, *line as u32, *line as u32))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One location for Code-surface LSP tools (1-based line/character).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocationItem {
    pub path: String,
    pub line: u64,
    pub character: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_character: Option<u64>,
}

/// Symbol row for document_symbol / workspace_symbol (1-based).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub line: u64,
    pub character: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_character: Option<u64>,
    /// Required for workspace_symbol; omitted for document_symbol when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// One diagnostic item (1-based). `path` is the queried file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiagnosticItem {
    pub severity: String,
    pub path: String,
    pub line: u64,
    pub character: u64,
    pub message: String,
}

pub(crate) const MAX_LOCATIONS: usize = 100;
pub(crate) const MAX_DIAGNOSTIC_ITEMS: usize = 100;
pub(crate) const MAX_SYMBOLS: usize = 200;

/// Absolute paths (internal emptiness / routing checks; not agent-facing).
pub(crate) fn extract_locations(result: &Value) -> Vec<(String, u64, u64)> {
    extract_location_items(result, &[])
        .into_iter()
        .map(|loc| (loc.path, loc.line, loc.character))
        .collect()
}

pub(crate) fn extract_location_items(result: &Value, roots: &[&Path]) -> Vec<LocationItem> {
    let mut out = Vec::new();
    fn parse_one(val: &Value, roots: &[&Path]) -> Option<LocationItem> {
        let uri = val
            .get("uri")
            .or_else(|| val.get("targetUri"))
            .and_then(|u| u.as_str())?;
        let range = val
            .get("range")
            .or_else(|| val.get("targetRange"))
            .or_else(|| val.get("targetSelectionRange"))?;
        let start = range.get("start")?;
        let line = start["line"].as_u64()? + 1;
        let character = start["character"].as_u64()? + 1;
        let end = range.get("end");
        let end_line = end.and_then(|e| e["line"].as_u64()).map(|l| l + 1);
        let end_character = end.and_then(|e| e["character"].as_u64()).map(|c| c + 1);
        let path = display_path_for_uri(uri, roots);
        Some(LocationItem {
            path,
            line,
            character,
            end_line,
            end_character,
        })
    }
    if let Some(arr) = result.as_array() {
        for item in arr {
            if let Some(loc) = parse_one(item, roots) {
                out.push(loc);
            }
        }
    } else if let Some(loc) = parse_one(result, roots) {
        out.push(loc);
    }
    out
}

pub(crate) fn extract_hover_markdown(result: &Value) -> String {
    format_hover(result)
}

pub(crate) fn extract_diagnostic_items(result: &Value, file_path: &str) -> Vec<DiagnosticItem> {
    let Some(diags) = result.as_array() else {
        return Vec::new();
    };
    diags.iter()
        .filter_map(|diag| {
            let severity = match diag["severity"].as_u64().unwrap_or(0) {
                1 => "Error",
                2 => "Warning",
                3 => "Information",
                4 => "Hint",
                _ => "Unknown",
            };
            let message = diag["message"].as_str()?.to_string();
            let start = diag.get("range").and_then(|r| r.get("start"));
            let line = start.and_then(|s| s["line"].as_u64()).map(|l| l + 1).unwrap_or(1);
            let character = start
                .and_then(|s| s["character"].as_u64())
                .map(|c| c + 1)
                .unwrap_or(1);
            Some(DiagnosticItem {
                severity: severity.into(),
                path: file_path.to_string(),
                line,
                character,
                message,
            })
        })
        .collect()
}

pub(crate) fn extract_symbol_items(
    result: &Value,
    require_path: bool,
    roots: &[&Path],
) -> Vec<SymbolItem> {
    let mut out = Vec::new();
    fn walk(val: &Value, require_path: bool, roots: &[&Path], out: &mut Vec<SymbolItem>) {
        if let Some(arr) = val.as_array() {
            for item in arr {
                walk(item, require_path, roots, out);
            }
            return;
        }
        let Some(name) = val.get("name").and_then(|n| n.as_str()) else {
            return;
        };
        let kind = val
            .get("kind")
            .and_then(|k| k.as_u64())
            .map(symbol_kind_name)
            .unwrap_or("Symbol")
            .to_string();
        let loc = val
            .get("location")
            .or_else(|| val.get("selectionRange").map(|_| val));
        let range = loc
            .and_then(|l| l.get("range").or_else(|| l.get("selectionRange")))
            .or_else(|| val.get("selectionRange").or_else(|| val.get("range")));
        let start = range.and_then(|r| r.get("start"));
        let end = range.and_then(|r| r.get("end"));
        let line = start.and_then(|s| s["line"].as_u64()).map(|l| l + 1).unwrap_or(1);
        let character = start
            .and_then(|s| s["character"].as_u64())
            .map(|c| c + 1)
            .unwrap_or(1);
        let end_line = end.and_then(|e| e["line"].as_u64()).map(|l| l + 1);
        let end_character = end.and_then(|e| e["character"].as_u64()).map(|c| c + 1);
        let path = loc
            .and_then(|l| l.get("uri").and_then(|u| u.as_str()))
            .or_else(|| val.get("uri").and_then(|u| u.as_str()))
            .map(|uri| display_path_for_uri(uri, roots));
        if require_path && path.is_none() {
            // skip workspace rows without a path
        } else {
            out.push(SymbolItem {
                name: name.to_string(),
                kind,
                line,
                character,
                end_line,
                end_character,
                path,
            });
        }
        if let Some(children) = val.get("children") {
            walk(children, require_path, roots, out);
        }
    }
    walk(result, require_path, roots, &mut out);
    out
}

pub(crate) fn format_hover(result: &Value) -> String {
    if result.is_null() {
        return "No hover information available".to_string();
    }
    let formatted = match result.get("contents") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(obj)) => obj
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| {
                v.as_str().map(|s| s.to_string()).or_else(|| {
                    v.get("value")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => "No hover information available".to_string(),
    };
    if formatted.trim().is_empty() {
        "No hover information available".to_string()
    } else {
        formatted
    }
}

fn format_diagnostics(result: &Value) -> String {
    let Some(diags) = result.as_array() else {
        return "No diagnostics".to_string();
    };
    if diags.is_empty() {
        return "No diagnostics".to_string();
    }
    diags
        .iter()
        .take(100)
        .map(format_one_diagnostic)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Only Error (severity=1) diagnostics, capped; `None` when nothing actionable.
pub(crate) fn format_error_diagnostics_block(result: &Value) -> Option<String> {
    const MAX_PER_FILE: usize = 20;
    let diags = result.as_array()?;
    let errors: Vec<&Value> = diags
        .iter()
        .filter(|d| d["severity"].as_u64() == Some(1))
        .collect();
    if errors.is_empty() {
        return None;
    }
    let limited = &errors[..errors.len().min(MAX_PER_FILE)];
    let mut lines: Vec<String> = limited.iter().map(|d| format_one_diagnostic(d)).collect();
    let more = errors.len().saturating_sub(MAX_PER_FILE);
    if more > 0 {
        lines.push(format!("... and {more} more"));
    }
    Some(lines.join("\n"))
}

fn format_one_diagnostic(diag: &Value) -> String {
    let sev = match diag["severity"].as_u64().unwrap_or(0) {
        1 => "Error",
        2 => "Warning",
        3 => "Information",
        4 => "Hint",
        _ => "Unknown",
    };
    let message = diag["message"].as_str().unwrap_or("unknown");
    let start = diag.get("range").and_then(|r| r.get("start"));
    let line = start
        .and_then(|s| s["line"].as_u64())
        .map(|l| l + 1)
        .unwrap_or(0);
    let loc = if line == 0 {
        String::new()
    } else {
        format!(
            " ({})",
            crate::tool::format_line_label(line as u32, line as u32)
        )
    };
    format!("{sev}: {message}{loc}")
}

fn format_symbols(result: &Value) -> String {
    let mut lines = Vec::new();
    fn walk(val: &Value, depth: usize, out: &mut Vec<String>) {
        let indent = "  ".repeat(depth);
        if let Some(arr) = val.as_array() {
            for item in arr {
                walk(item, depth, out);
            }
            return;
        }
        let Some(name) = val.get("name").and_then(|n| n.as_str()) else {
            return;
        };
        let kind = val
            .get("kind")
            .and_then(|k| k.as_u64())
            .map(symbol_kind_name)
            .unwrap_or("Symbol");
        let loc = val
            .get("location")
            .or_else(|| val.get("selectionRange").map(|_| val));
        let range = loc
            .and_then(|l| l.get("range").or_else(|| l.get("selectionRange")))
            .or_else(|| val.get("selectionRange").or_else(|| val.get("range")));
        let line = range
            .and_then(|r| r.get("start"))
            .and_then(|s| s["line"].as_u64())
            .map(|l| l + 1)
            .unwrap_or(0);
        out.push(format!(
            "{indent}{kind} {name} ({})",
            crate::tool::format_line_label(line as u32, line as u32)
        ));
        if let Some(children) = val.get("children") {
            walk(children, depth + 1, out);
        }
    }
    walk(result, 0, &mut lines);
    if lines.is_empty() {
        "No symbols found".into()
    } else {
        lines.join("\n")
    }
}

pub(crate) fn symbol_kind_name(kind: u64) -> &'static str {
    // LSP SymbolKind
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        5 => "Class",
        6 => "Method",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        23 => "Struct",
        _ => "Symbol",
    }
}

fn format_call_hierarchy(result: &Value, roots: &[&Path]) -> String {
    if result.is_null() {
        return "No call hierarchy results".into();
    }
    let items = if let Some(arr) = result.as_array() {
        arr.clone()
    } else {
        vec![result.clone()]
    };
    if items.is_empty() {
        return "No call hierarchy results".into();
    }
    let mut lines = Vec::new();
    for item in items {
        // CallHierarchyItem or { from/to: CallHierarchyItem, fromRanges }
        let node = item.get("from").or_else(|| item.get("to")).unwrap_or(&item);
        let name = node
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("(unknown)");
        let uri = node.get("uri").and_then(|u| u.as_str()).unwrap_or("");
        let path = if uri.is_empty() {
            String::new()
        } else {
            display_path_for_uri(uri, roots)
        };
        let line = node
            .get("range")
            .or_else(|| node.get("selectionRange"))
            .and_then(|r| r.get("start"))
            .and_then(|s| s["line"].as_u64())
            .map(|l| l + 1)
            .unwrap_or(0);
        if path.is_empty() {
            lines.push(format!(
                "{name} ({})",
                crate::tool::format_line_label(line as u32, line as u32)
            ));
        } else {
            lines.push(format!(
                "{name} — {}",
                crate::tool::format_path_lines(&path, line as u32, line as u32)
            ));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::uri::file_to_uri;
    use std::path::PathBuf;

    fn loc(path: &Path) -> Value {
        serde_json::json!([{
            "uri": file_to_uri(path),
            "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 9 } }
        }])
    }

    #[test]
    fn location_inside_workspace_is_relative() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let file = root.join("src").join("lib.rs");
        let items = extract_location_items(&loc(&file), &[root.as_path()]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "src/lib.rs");
        assert_eq!(items[0].line, 3);
    }

    #[test]
    fn location_outside_workspace_stays_absolute() {
        let ws = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("dep.rs");
        let items = extract_location_items(&loc(&file), &[ws.path()]);
        assert_eq!(items.len(), 1);
        let got = PathBuf::from(&items[0].path);
        assert!(got.is_absolute(), "outside path must stay absolute: {}", items[0].path);
        assert!(items[0].path.ends_with("dep.rs"));
    }

    #[test]
    fn display_path_does_not_resolve_symlinks() {
        // Pure prefix strip: a symlinked root is matched textually, never canonicalized.
        let root = Path::new("/ws/link");
        assert_eq!(display_path(Path::new("/ws/link/a/b.rs"), &[root]), "a/b.rs");
        // Same file reached via the symlink target is outside by string → absolute.
        assert_eq!(
            display_path(Path::new("/ws/real/a/b.rs"), &[root]),
            "/ws/real/a/b.rs"
        );
        // No roots → absolute (internal callers).
        assert_eq!(display_path(Path::new("/ws/link/a.rs"), &[]), "/ws/link/a.rs");
    }

    #[test]
    fn display_path_tries_roots_in_order() {
        let canon = Path::new("/real/ws");
        let configured = Path::new("/home/u/ws");
        assert_eq!(
            display_path(Path::new("/home/u/ws/x.rs"), &[canon, configured]),
            "x.rs"
        );
    }

    #[test]
    fn workspace_symbol_paths_relative_inside_absolute_outside() {
        let ws = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let inside = ws.path().join("m.rs");
        let outside = other.path().join("o.rs");
        let raw = serde_json::json!([
            { "name": "A", "kind": 12, "location": { "uri": file_to_uri(&inside),
              "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } } } },
            { "name": "B", "kind": 12, "location": { "uri": file_to_uri(&outside),
              "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 1 } } } }
        ]);
        let syms = extract_symbol_items(&raw, true, &[ws.path()]);
        assert_eq!(syms[0].path.as_deref(), Some("m.rs"));
        let b = syms[1].path.as_deref().unwrap();
        assert!(Path::new(b).is_absolute() && b.ends_with("o.rs"), "got {b}");
    }

    #[test]
    fn format_locations_text_uses_relative_path() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("src").join("lib.rs");
        let text = format_action_result("goToDefinition", &loc(&file), &[tmp.path()]);
        assert!(text.contains("src/lib.rs"), "got {text}");
        assert!(!text.contains(&tmp.path().to_string_lossy().into_owned()), "got {text}");
    }
}
