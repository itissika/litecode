//! Resolve workspace citation targets without returning file contents.
//!
//! Each path goes through [`Sandbox::resolve`]. A miss, a directory, or an
//! escape is `exists: false` for that item; one bad ref does not fail the batch.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SandboxError;
use super::service::{MAX_FILE_SIZE, WorkspaceService};
use crate::engines::code_search::{LexicalQuery, lexical_search_with_preset};
use crate::workspace::filter::FilterPreset;

/// Hard cap for one `POST /api/workspace/citations` body.
pub const MAX_CITATION_BATCH: usize = 64;

const MAX_SYMBOL_LEN: usize = 256;

#[derive(Debug, Deserialize)]
pub struct CitationQuery {
    pub path: String,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub symbol: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CitationHit {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

impl CitationHit {
    fn miss() -> Self {
        Self {
            exists: false,
            path: None,
            line: None,
        }
    }
}

enum LineLook {
    Present,
    Absent,
    /// Byte cap hit before the target line was confirmed. Ignore the line.
    Capped,
}

pub fn resolve_citations(workspace: &WorkspaceService, refs: &[CitationQuery]) -> Vec<CitationHit> {
    let sandbox = workspace.sandbox();
    refs.iter()
        .map(|query| resolve_one(sandbox, query))
        .collect()
}

fn resolve_one(sandbox: &super::Sandbox, query: &CitationQuery) -> CitationHit {
    let raw = query.path.trim();
    if raw.is_empty() {
        return CitationHit::miss();
    }
    let abs = match sandbox.resolve(raw) {
        Ok(path) => path,
        Err(SandboxError::Escape | SandboxError::Invalid(_) | SandboxError::NotFound(_)) => {
            return CitationHit::miss();
        }
        Err(SandboxError::Io(_)) => return CitationHit::miss(),
    };
    if !abs.is_file() {
        return CitationHit::miss();
    }
    let Ok(rel) = sandbox.rel_path(&abs) else {
        return CitationHit::miss();
    };
    if rel.is_empty() {
        return CitationHit::miss();
    }

    let line = if let Some(target) = query.line.filter(|n| *n >= 1) {
        match look_up_line(&abs, target, MAX_FILE_SIZE) {
            Ok(LineLook::Present) => Some(target),
            _ => None,
        }
    } else {
        symbol_line(sandbox.root(), &abs, query.symbol.as_deref())
    };

    CitationHit {
        exists: true,
        path: Some(rel),
        line,
    }
}

/// Count newlines until `target` (1-based) or `max_bytes`. Does not retain bytes.
fn look_up_line(path: &Path, target: u32, max_bytes: u64) -> std::io::Result<LineLook> {
    if target == 0 {
        return Ok(LineLook::Absent);
    }
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = [0u8; 8192];
    let mut consumed: u64 = 0;
    let mut newlines: u32 = 0;
    let mut any = false;
    let mut last_nl = false;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            if !any {
                return Ok(LineLook::Absent);
            }
            // Match `str::lines`: a trailing newline does not add an extra line.
            let total = if last_nl {
                newlines
            } else {
                newlines.saturating_add(1)
            };
            return Ok(if target <= total {
                LineLook::Present
            } else {
                LineLook::Absent
            });
        }
        for &byte in &buf[..n] {
            any = true;
            consumed += 1;
            if consumed > max_bytes {
                return Ok(LineLook::Capped);
            }
            if byte == b'\n' {
                newlines = newlines.saturating_add(1);
                last_nl = true;
                if newlines == target {
                    return Ok(LineLook::Present);
                }
            } else {
                last_nl = false;
            }
        }
    }
}

/// First whole-word hit in this file. Not a definition lookup.
fn symbol_line(root: &Path, file: &Path, symbol: Option<&str>) -> Option<u32> {
    let symbol = symbol?.trim();
    if symbol.is_empty() || symbol.len() > MAX_SYMBOL_LEN {
        return None;
    }
    let query = LexicalQuery {
        pattern: symbol.to_string(),
        root: root.to_path_buf(),
        path: Some(file.to_path_buf()),
        case_sensitive: true,
        whole_word: true,
        is_regex: false,
        include: None,
        exclude: None,
        multiline: false,
        max_matches: 1,
        before_context: 0,
        after_context: 0,
    };
    let outcome = lexical_search_with_preset(&query, FilterPreset::Search).ok()?;
    outcome.matches.first().map(|hit| hit.start_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(dir: &std::path::Path) -> std::sync::Arc<WorkspaceService> {
        WorkspaceService::new(dir.to_path_buf()).unwrap()
    }

    #[test]
    fn line_lookup_matches_lines_and_stops_at_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        assert!(matches!(
            look_up_line(&path, 2, 100).unwrap(),
            LineLook::Present
        ));
        assert!(matches!(
            look_up_line(&path, 3, 100).unwrap(),
            LineLook::Absent
        ));
        std::fs::write(&path, "").unwrap();
        assert!(matches!(
            look_up_line(&path, 1, 100).unwrap(),
            LineLook::Absent
        ));
        std::fs::write(&path, "hello").unwrap();
        assert!(matches!(
            look_up_line(&path, 1, 100).unwrap(),
            LineLook::Present
        ));
        std::fs::write(&path, "abcdefghij").unwrap();
        assert!(matches!(
            look_up_line(&path, 1, 4).unwrap(),
            LineLook::Capped
        ));
    }

    #[test]
    fn resolve_reports_exists_line_symbol_and_hides_escapes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lines.txt"), "alpha\nbeta\n").unwrap();
        std::fs::write(
            dir.path().join("sym.rs"),
            "let x = Session.user;\nfn unique_citation_symbol() {}\n",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let outside = dir.path().parent().unwrap().join("secret-citation.txt");
        std::fs::write(&outside, "SECRET_CITATION_BODY").unwrap();

        let ws = workspace(dir.path());
        let hits = resolve_citations(
            &ws,
            &[
                CitationQuery {
                    path: "lines.txt".into(),
                    line: Some(2),
                    symbol: Some("ignored".into()),
                },
                CitationQuery {
                    path: "lines.txt".into(),
                    line: Some(50),
                    symbol: None,
                },
                CitationQuery {
                    path: "sym.rs".into(),
                    line: None,
                    symbol: Some("unique_citation_symbol".into()),
                },
                CitationQuery {
                    path: "sym.rs".into(),
                    line: None,
                    symbol: Some("Session.user".into()),
                },
                CitationQuery {
                    path: "sym.rs".into(),
                    line: None,
                    symbol: Some("missing_symbol".into()),
                },
                CitationQuery {
                    path: "nope.ts".into(),
                    line: None,
                    symbol: None,
                },
                CitationQuery {
                    path: "../secret-citation.txt".into(),
                    line: None,
                    symbol: None,
                },
                CitationQuery {
                    path: "subdir".into(),
                    line: None,
                    symbol: None,
                },
            ],
        );

        assert!(hits[0].exists);
        assert_eq!(hits[0].path.as_deref(), Some("lines.txt"));
        assert_eq!(hits[0].line, Some(2));
        assert!(hits[1].exists);
        assert_eq!(hits[1].line, None);
        assert_eq!(hits[2].line, Some(2));
        assert_eq!(hits[3].line, Some(1));
        assert!(hits[4].exists);
        assert_eq!(hits[4].line, None);
        assert!(!hits[5].exists);
        assert!(!hits[6].exists);
        assert!(hits[6].path.is_none());
        assert!(!hits[7].exists);

        let json = serde_json::to_string(&hits).unwrap();
        assert!(!json.contains("SECRET_CITATION_BODY"));
        assert!(!json.contains("alpha"));
        let _ = std::fs::remove_file(&outside);
    }
}
