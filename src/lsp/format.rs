//! Agent-facing formatting of LSP JSON results.

use serde_json::Value;

use crate::lsp::uri::uri_to_path;

pub(crate) fn format_action_result(action: &str, result: &Value) -> String {
    match action {
        "goToDefinition" | "findReferences" | "goToImplementation" => format_locations(result),
        "hover" => format_hover(result),
        "diagnostics" => format_diagnostics(result),
        "documentSymbol" | "workspaceSymbol" => format_symbols(result),
        "prepareCallHierarchy" | "incomingCalls" | "outgoingCalls" => format_call_hierarchy(result),
        _ => result.to_string(),
    }
}

fn format_locations(result: &Value) -> String {
    let locations = extract_locations(result);
    if locations.is_empty() {
        // Only reached when the language server index is known-settled.
        return "No locations found (language server ready; no definition/reference at this position)."
            .to_string();
    }
    locations
        .iter()
        .map(|(path, line, ch)| format!("{path}:{line}:{ch}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn extract_locations(result: &Value) -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    fn parse_one(val: &Value) -> Option<(String, u64, u64)> {
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
        let path = uri_to_path(uri)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| uri.to_string());
        Some((path, line, character))
    }
    if let Some(arr) = result.as_array() {
        for item in arr {
            if let Some(loc) = parse_one(item) {
                out.push(loc);
            }
        }
    } else if let Some(loc) = parse_one(result) {
        out.push(loc);
    }
    out
}

fn format_hover(result: &Value) -> String {
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
    let ch = start
        .and_then(|s| s["character"].as_u64())
        .map(|c| c + 1)
        .unwrap_or(0);
    format!("{sev}: {message} ({line}:{ch})")
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
        out.push(format!("{indent}{kind} {name} (L{line})"));
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

fn symbol_kind_name(kind: u64) -> &'static str {
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

fn format_call_hierarchy(result: &Value) -> String {
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
        let path = uri_to_path(uri)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| uri.to_string());
        let line = node
            .get("range")
            .or_else(|| node.get("selectionRange"))
            .and_then(|r| r.get("start"))
            .and_then(|s| s["line"].as_u64())
            .map(|l| l + 1)
            .unwrap_or(0);
        if path.is_empty() {
            lines.push(format!("{name} (L{line})"));
        } else {
            lines.push(format!("{name} — {path}:{line}"));
        }
    }
    lines.join("\n")
}
