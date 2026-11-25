//! Which directory holds the knowledge nodes.
//!
//! `knowledge/` at the workspace root wins when it exists. Otherwise
//! `.litecode/knowledge`. A missing tree is not created here.

use std::path::Path;

pub const PRIVATE_ROOT: &str = ".litecode/knowledge";
pub const PUBLIC_ROOT: &str = "knowledge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeRoot {
    /// Workspace-relative directory, forward slashes.
    Present(String),
    Missing,
}

pub fn locate(workspace: &Path) -> KnowledgeRoot {
    if workspace.join(PUBLIC_ROOT).is_dir() {
        KnowledgeRoot::Present(PUBLIC_ROOT.to_string())
    } else if workspace.join(PRIVATE_ROOT).is_dir() {
        KnowledgeRoot::Present(PRIVATE_ROOT.to_string())
    } else {
        KnowledgeRoot::Missing
    }
}

pub fn join(root: &str, rel: &str) -> String {
    let rel = rel.trim_matches('/');
    if rel.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{rel}")
    }
}

/// Canonical logical knowledge path (`knowledge/<rel>`), independent of whether
/// the corpus currently lives on disk under `knowledge/` or `.litecode/knowledge/`.
pub fn logical_path(rel: &str) -> String {
    join(PUBLIC_ROOT, rel)
}

/// Strip the active knowledge prefix from a normalized workspace path, returning
/// the path relative to the knowledge root.
///
/// Always recognizes the logical prefix `knowledge/<rel>`. When the active root
/// is private (`.litecode/knowledge`), also recognizes `.litecode/knowledge/<rel>`.
/// When the public root (`knowledge`) is active, `.litecode/knowledge/...` is never
/// folded into the corpus.
pub fn strip_corpus_prefix<'a>(norm_path: &'a str, active_root: Option<&str>) -> Option<&'a str> {
    let active = active_root?;
    if let Some(rest) = norm_path.strip_prefix(&format!("{PUBLIC_ROOT}/"))
        && !rest.is_empty()
    {
        return Some(rest);
    }
    if active == PRIVATE_ROOT
        && let Some(rest) = norm_path.strip_prefix(&format!("{PRIVATE_ROOT}/"))
        && !rest.is_empty()
    {
        return Some(rest);
    }
    None
}

/// Map a logical `knowledge/<rest>` path to `.litecode/knowledge/<rest>` when the
/// active corpus root is private. Public paths and non-knowledge paths stay unchanged.
pub fn resolve_disk_path(norm_path: &str, active_root: Option<&str>) -> String {
    if active_root == Some(PRIVATE_ROOT) {
        if norm_path == PUBLIC_ROOT {
            return PRIVATE_ROOT.to_string();
        }
        if let Some(rest) = norm_path.strip_prefix(&format!("{PUBLIC_ROOT}/")) {
            return join(PRIVATE_ROOT, rest);
        }
    }
    norm_path.to_string()
}
