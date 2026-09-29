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

/// The private root, when a public directory is the one actually read.
pub fn ignored(workspace: &Path) -> Option<&'static str> {
    let public = workspace.join(PUBLIC_ROOT).is_dir();
    let private = workspace.join(PRIVATE_ROOT).is_dir();
    if public && private {
        Some(PRIVATE_ROOT)
    } else {
        None
    }
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
