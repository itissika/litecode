//! Named scopes in one workspace file.
//!
//! Tree-sitter walks the file. Nothing here talks to a language server.
//! Drift against git history is filled in by the knowledge check, not here.

use serde::{Deserialize, Serialize};

use super::service::{WorkspaceError, WorkspaceService};
use crate::engines::code_search::{
    ScopeEntry, ScopeMatch, find_scope, lines_slice, list_scopes, scope_at,
};

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
    /// Knowledge node path whose last commit is the drift baseline.
    /// Accepted now; the drift field stays empty until that check is wired.
    #[serde(default)]
    pub drift_base_of: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolDrift {
    pub drifted: bool,
    pub commits: Vec<SymbolDriftCommit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolDriftCommit {
    pub hash: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolRefHit {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
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

pub fn resolve_symbol_refs(
    workspace: &WorkspaceService,
    refs: &[SymbolRefQuery],
) -> Vec<SymbolRefHit> {
    let mut cache: std::collections::HashMap<String, FileText> = std::collections::HashMap::new();
    let mut drift = crate::knowledge::symbol_check::SymbolCache::new();
    refs.iter()
        .map(|query| resolve_one(workspace, query, &mut cache, &mut drift))
        .collect()
}

enum FileText {
    Missing,
    Unreadable,
    Ready(String),
}

fn resolve_one(
    workspace: &WorkspaceService,
    query: &SymbolRefQuery,
    cache: &mut std::collections::HashMap<String, FileText>,
    drift: &mut crate::knowledge::symbol_check::SymbolCache,
) -> SymbolRefHit {
    let file = query.file.trim().to_string();
    let symbol = query
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let mut hit = SymbolRefHit {
        file: file.clone(),
        symbol: symbol.clone(),
        file_exists: false,
        symbol_exists: false,
        ambiguous: false,
        start_line: None,
        end_line: None,
        drift: None,
    };
    if file.is_empty() {
        return hit;
    }
    let text = cache
        .entry(file.clone())
        .or_insert_with(|| load_text(workspace, &file));
    let FileText::Ready(content) = text else {
        hit.file_exists = matches!(text, FileText::Unreadable);
        return hit;
    };
    hit.file_exists = true;
    let Some(chain) = symbol else {
        return hit;
    };
    match find_scope(&file, content, &chain) {
        ScopeMatch::Unique(entry) => {
            hit.symbol_exists = true;
            hit.start_line = Some(entry.start_line);
            hit.end_line = Some(entry.end_line);
        }
        ScopeMatch::Ambiguous(_) => {
            hit.ambiguous = true;
        }
        ScopeMatch::Missing => {}
    }
    if hit.symbol_exists
        && let Some(base) = query
            .drift_base_of
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
    {
        let root = workspace.sandbox().root().to_path_buf();
        match drift.drift(&root, &file, &chain, base) {
            Some(commits) => {
                hit.drift = Some(SymbolDrift {
                    drifted: true,
                    commits: commits
                        .into_iter()
                        .map(|commit| SymbolDriftCommit {
                            hash: commit.hash,
                            subject: commit.subject,
                        })
                        .collect(),
                });
            }
            None => {
                hit.drift = Some(SymbolDrift {
                    drifted: false,
                    commits: Vec::new(),
                });
            }
        }
    }
    hit
}

fn load_text(workspace: &WorkspaceService, path: &str) -> FileText {
    match workspace.read_file(path) {
        Ok((_, content)) => FileText::Ready(content),
        Err(
            WorkspaceError::NotFound(_) | WorkspaceError::NotFile(_) | WorkspaceError::IsDir(_),
        ) => FileText::Missing,
        Err(WorkspaceError::Sandbox(_)) => FileText::Missing,
        Err(_) => FileText::Unreadable,
    }
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
    use super::*;

    #[test]
    fn lists_a_rust_method_with_a_two_line_summary() {
        let src = "impl Store {\n    pub fn save(&self) {\n        let _ = 1;\n    }\n}\n";
        let symbols = list_file_symbols("store.rs", src);
        let save = symbols
            .iter()
            .find(|entry| entry.chain == "impl Store › fn save")
            .expect("method");
        assert!(save.summary.contains("pub fn save"));
        assert!(save.summary.contains("let _ = 1"));
        let at = locate_symbol("store.rs", src, save.start_line, save.start_line);
        assert_eq!(at.chain.as_deref(), Some("impl Store › fn save"));
        assert_eq!(at.label.as_deref(), Some("fn save"));
    }

    #[test]
    fn batch_reports_missing_ambiguous_and_unique() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/a.rs"),
            "fn new() {}\nfn new() {}\nfn alpha() {}\n",
        )
        .unwrap();
        let workspace = WorkspaceService::new(dir.path().to_path_buf()).unwrap();
        let hits = resolve_symbol_refs(
            &workspace,
            &[
                SymbolRefQuery {
                    file: "src/missing.rs".into(),
                    symbol: Some("fn alpha".into()),
                    drift_base_of: None,
                },
                SymbolRefQuery {
                    file: "src/a.rs".into(),
                    symbol: Some("fn new".into()),
                    drift_base_of: None,
                },
                SymbolRefQuery {
                    file: "src/a.rs".into(),
                    symbol: Some("fn alpha".into()),
                    drift_base_of: Some(".litecode/knowledge/a.md".into()),
                },
            ],
        );
        assert!(!hits[0].file_exists);
        assert!(hits[1].ambiguous);
        assert!(!hits[1].symbol_exists);
        assert!(hits[2].symbol_exists);
        assert_eq!(hits[2].start_line, Some(3));
        assert!(hits[2].drift.is_none());
    }
}
