use serde::Serialize;

use super::filter::{FilterPreset, configure_walk_under};
use super::sandbox::{Sandbox, SandboxError};
use ignore::WalkBuilder;

#[derive(Debug, Clone, Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error("not a directory: {0}")]
    NotDir(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("ignore walk error: {0}")]
    Walk(String),
}

pub fn list_tree(
    sandbox: &Sandbox,
    rel_path: &str,
    depth: usize,
) -> Result<Vec<TreeEntry>, TreeError> {
    let dir = sandbox.resolve(rel_path)?;
    if !dir.is_dir() {
        return Err(TreeError::NotDir(sandbox.rel_path(&dir)?));
    }

    if depth == 0 {
        return Ok(Vec::new());
    }

    let root = sandbox.root().to_path_buf();
    let mut entries = Vec::new();
    let max_depth = depth;

    let mut builder = WalkBuilder::new(&dir);
    configure_walk_under(&mut builder, &root, &dir, FilterPreset::Explorer);
    let walker = builder.max_depth(Some(max_depth)).build();

    for result in walker {
        let entry = result.map_err(|e| TreeError::Walk(e.to_string()))?;
        let path = entry.path();
        if path == dir {
            continue;
        }

        let rel = sandbox.rel_path(path)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        // Use the WalkDir entry's own file type (no second stat for is_dir).
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let size = if is_dir {
            None
        } else {
            entry.metadata().ok().map(|m| m.len())
        };

        entries.push(TreeEntry {
            name,
            path: rel,
            // NOTE: contract with the frontend is `kind: "file" | "dir"`
            // (see web/src/api/workspace.ts). Do not change this to
            // "directory"/"file" — the file tree keys off "dir" to tell
            // folders apart from files.
            kind: if is_dir { "dir".into() } else { "file".into() },
            size,
        });
    }

    entries.sort_by(|a, b| match (a.kind.as_str(), b.kind.as_str()) {
        ("dir", "file") => std::cmp::Ordering::Less,
        ("file", "dir") => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tree_reports_dir_and_file_kinds_and_sizes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "hello").unwrap();

        let sandbox = Sandbox::new(root.to_path_buf()).unwrap();
        let entries = list_tree(&sandbox, "", 2).unwrap();

        let by_name: std::collections::HashMap<&str, &TreeEntry> =
            entries.iter().map(|e| (e.name.as_str(), e)).collect();
        // A directory is reported as kind "dir" with no size.
        let src = by_name["src"];
        assert_eq!(src.kind, "dir");
        assert_eq!(src.size, None);
        // A file is reported as kind "file" with its byte size.
        let file = by_name["a.txt"];
        assert_eq!(file.kind, "file");
        assert_eq!(file.size, Some(5));
    }
}
