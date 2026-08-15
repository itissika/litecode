use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::path::{canon_abs, canon_abs_lossy};

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("path escapes project root")]
    Escape,
    #[error("invalid path: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Sandbox {
    /// Workspace root in LAP form.
    root: PathBuf,
}

impl Sandbox {
    pub fn new(root: PathBuf) -> Result<Self, SandboxError> {
        if !root.exists() {
            std::fs::create_dir_all(&root)?;
        }
        let root = canon_abs(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a workspace-relative path (empty string = project root).
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, SandboxError> {
        let rel = rel.trim().trim_start_matches('/');
        if rel.is_empty() {
            return Ok(self.root.clone());
        }
        // Human APIs share the same LAP/traversal/workspace-scope primitive as
        // Agent SAFE and LSP paths; only the policy entry differs.
        crate::workspace::resolve_human_relative(&self.root, rel).map_err(|error| match error {
            crate::workspace::ToolPathError::RelativeTraversal
            | crate::workspace::ToolPathError::OutsideWorkspace
            | crate::workspace::ToolPathError::UnixStyleOnWindows { .. } => SandboxError::Escape,
            crate::workspace::ToolPathError::Empty => {
                SandboxError::Invalid("path must be non-empty".into())
            }
            crate::workspace::ToolPathError::Io(error) => SandboxError::Io(error),
        })
    }

    /// Relative path from project root (posix-style forward slashes).
    pub fn rel_path(&self, abs: &Path) -> Result<String, SandboxError> {
        let abs = if abs.exists() {
            canon_abs(abs)?
        } else {
            canon_abs_lossy(abs)
        };
        if !abs.starts_with(&self.root) {
            return Err(SandboxError::Escape);
        }
        let rel = abs.strip_prefix(&self.root).unwrap_or(Path::new(""));
        let s = rel.to_string_lossy().replace('\\', "/");
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::path::canon_abs;

    #[test]
    fn rejects_parent_dir_escape() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        assert!(matches!(
            sandbox.resolve("../etc/passwd"),
            Err(SandboxError::Escape)
        ));
    }

    #[test]
    fn resolves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hi").unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        let resolved = sandbox.resolve("a.txt").unwrap();
        assert_eq!(resolved, canon_abs(&file).unwrap());
        assert!(resolved.starts_with(sandbox.root()));
    }

    #[test]
    fn root_is_lap_not_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox = Sandbox::new(dir.path().to_path_buf()).unwrap();
        let s = sandbox.root().to_string_lossy();
        assert!(!s.starts_with(r"\\?\"), "sandbox root must be LAP: {s}");
        assert_eq!(sandbox.root(), canon_abs(dir.path()).unwrap());
    }
}
