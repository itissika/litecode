//! Named scopes in one workspace file.
//!
//! Tree-sitter walks the file. Nothing here talks to a language server.
//! A symbol query also reports whether that symbol's body on disk differs
//! from HEAD. An unknown check stays `drift: None`.

use serde::{Deserialize, Serialize};

use super::service::WorkspaceService;
use crate::engines::code_search::{ScopeEntry, lines_slice, list_scopes, scope_at};

pub const MAX_SYMBOL_BATCH: usize = 64;

const SUMMARY_LINES: usize = 2;

#[derive(Debug, Clone, Serialize)]
pub struct ListedSymbol {
    pub chain: String,
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolAtHit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl SymbolAtHit {
    fn none() -> Self {
        Self {
            chain: None,
            kind: None,
            name: None,
            start_line: None,
            end_line: None,
            label: None,
        }
    }

    fn from_entry(entry: ScopeEntry) -> Self {
        let label = leaf_label(&entry.chain);
        Self {
            chain: Some(entry.chain),
            kind: Some(entry.kind),
            name: Some(entry.name),
            start_line: Some(entry.start_line),
            end_line: Some(entry.end_line),
            label: Some(label),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SymbolRefQuery {
    pub file: String,
    #[serde(default)]
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolDrift {
    pub drifted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolRefHit {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    /// Same status code the knowledge snapshot uses for this citation.
    pub status: &'static str,
    pub file_exists: bool,
    pub symbol_exists: bool,
    pub ambiguous: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<SymbolDrift>,
}

pub fn list_file_symbols(path: &str, content: &str) -> Vec<ListedSymbol> {
    list_scopes(path, content)
        .into_iter()
        .map(|entry| {
            let summary = scope_summary(content, entry.start_line);
            ListedSymbol {
                chain: entry.chain,
                kind: entry.kind,
                name: entry.name,
                start_line: entry.start_line,
                end_line: entry.end_line,
                summary,
            }
        })
        .collect()
}

pub fn locate_symbol(path: &str, content: &str, start: u32, end: u32) -> SymbolAtHit {
    scope_at(path, content, start, end)
        .map(SymbolAtHit::from_entry)
        .unwrap_or_else(SymbolAtHit::none)
}

/// Resolve `/symbol-refs` queries through the knowledge index engine, so the
/// editor sees exactly what the knowledge snapshot says (one read policy,
/// exact names, cached parses, git2 HEAD; `docs/knowledge-references.md`).
pub fn resolve_symbol_refs(
    workspace: &WorkspaceService,
    refs: &[SymbolRefQuery],
) -> Vec<SymbolRefHit> {
    let root = workspace.sandbox().root().to_path_buf();
    let queries: Vec<(String, Option<String>)> = refs
        .iter()
        .map(|query| (query.file.trim().to_string(), query.symbol.clone()))
        .collect();
    let results = crate::knowledge::index::query_symbols(&root, &queries);
    refs.iter()
        .zip(results)
        .map(|(query, result)| SymbolRefHit {
            file: query.file.trim().to_string(),
            symbol: query
                .symbol
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string),
            status: result.status,
            file_exists: result.file_exists,
            symbol_exists: result.symbol_exists,
            ambiguous: result.ambiguous,
            start_line: result.start_line,
            end_line: result.end_line,
            drift: result.drifted.map(|drifted| SymbolDrift { drifted }),
        })
        .collect()
}

/// Signature line plus the next non-empty line.
pub fn scope_summary(content: &str, start_line: u32) -> String {
    if start_line == 0 {
        return String::new();
    }
    let window = lines_slice(content, start_line, start_line.saturating_add(12));
    let mut kept = Vec::new();
    for line in window.lines() {
        if kept.is_empty() {
            kept.push(line.trim().to_string());
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        kept.push(line.trim().to_string());
        if kept.len() == SUMMARY_LINES {
            break;
        }
    }
    kept.join("\n")
}

fn leaf_label(chain: &str) -> String {
    chain.rsplit(" › ").next().unwrap_or(chain).to_string()
}

#[cfg(test)]
mod tests {
    

}
