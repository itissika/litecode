//! Agent tool path semantics layered on top of LAP.
//!
//! Relative paths always name files beneath the active workspace.  ALL mode
//! additionally accepts explicit absolute paths outside that workspace; SAFE
//! mode does not.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::path::{canon_abs, canon_abs_lossy, is_under, strip_verbatim};
use crate::config::workspace::workspace_root_lap;
use crate::workspace::agent_path::{ClassifyError, PathCandidate, classify_agent_path};

/// Schema / hint text for built-in file tools (`read`, `write`, `edit`, `glob`).
pub const AGENT_FILE_PATH_HINT: &str = "Prefer a workspace-relative path; absolute paths outside the workspace are allowed only under All permission. On Windows use C:\\...; Unix-style /paths map under Git for Windows when installed.";

fn map_classify_error(error: ClassifyError) -> ToolPathError {
    match error {
        ClassifyError::Empty => ToolPathError::Empty,
        ClassifyError::RelativeTraversal => ToolPathError::RelativeTraversal,
        ClassifyError::UnixStyleOnWindows { raw } => ToolPathError::UnixStyleOnWindows { raw },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPathMode {
    All,
    Safe,
}

#[derive(Debug, Error)]
pub enum ToolPathError {
    #[error("path must be non-empty")]
    Empty,
    #[error("relative path must stay under the workspace (cannot use '..')")]
    RelativeTraversal,
    #[error("SAFE mode only permits paths under the workspace")]
    OutsideWorkspace,
    #[error(
        "path '{raw}' is a Unix-style path and is not valid as a Windows path; use C:\\... or a workspace-relative path"
    )]
    UnixStyleOnWindows { raw: String },
    #[error("resolve path: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve a built-in tool path into LAP without relying on process CWD.
pub fn resolve_tool_path(raw: &str, mode: ToolPathMode) -> Result<PathBuf, ToolPathError> {
    resolve_tool_path_from(&workspace_root_lap(), raw, mode)
}

/// Resolve a human workspace API path. Human APIs never accept an absolute
/// path, preventing an accidental transition into Agent ALL semantics.
pub fn resolve_human_relative(workspace_root: &Path, raw: &str) -> Result<PathBuf, ToolPathError> {
    if Path::new(raw.trim()).is_absolute() {
        return Err(ToolPathError::OutsideWorkspace);
    }
    resolve_tool_path_from(workspace_root, raw, ToolPathMode::Safe)
}

/// Resolve a built-in Agent file-tool path under its selected permission mode.
pub fn resolve_agent(
    workspace_root: &Path,
    raw: &str,
    mode: ToolPathMode,
) -> Result<PathBuf, ToolPathError> {
    resolve_tool_path_from(workspace_root, raw, mode)
}

/// LSP is intentionally never an ALL consumer.
pub fn resolve_lsp_workspace(workspace_root: &Path, raw: &str) -> Result<PathBuf, ToolPathError> {
    resolve_tool_path_from(workspace_root, raw, ToolPathMode::Safe)
}

/// Variant with an explicit root for services and tests.
pub fn resolve_tool_path_from(
    workspace_root: &Path,
    raw: &str,
    mode: ToolPathMode,
) -> Result<PathBuf, ToolPathError> {
    let root = canon_abs_lossy(workspace_root);
    let candidate = match classify_agent_path(raw) {
        Ok(c) => c,
        Err(e) => return Err(map_classify_error(e)),
    };
    let candidate = match candidate {
        PathCandidate::Absolute(path) => strip_verbatim(&path),
        PathCandidate::Relative(rel) => root.join(rel),
    };

    let resolved = if candidate.exists() {
        canon_abs(&candidate)?
    } else if mode == ToolPathMode::Safe {
        // 1.5: the target does not exist yet, but one of its ancestors may be a
        // symlink pointing outside the workspace. Resolve through the deepest
        // existing ancestor (canonical, symlink-aware) instead of a lexical
        // fallback; `is_under` below then rejects any escape. Only the Safe
        // (workspace-bounded) path needs this guard — All mode permits external
        // writes by design, so a symlink to outside is not an escape there.
        crate::config::path::canon_join_nonexistent(&root, &candidate)
            .map_err(|_| ToolPathError::OutsideWorkspace)?
    } else {
        canon_abs_lossy(&candidate)
    };

    if mode == ToolPathMode::Safe && !is_under(&resolved, &root) {
        return Err(ToolPathError::OutsideWorkspace);
    }
    Ok(resolved)
}

/// True when `resolved` is not under `workspace_root` (LAP comparison).
pub fn is_resolved_outside_workspace(workspace_root: &Path, resolved: &Path) -> bool {
    let root = canon_abs_lossy(workspace_root);
    !is_under(resolved, &root)
}

/// Whether a raw agent path names a location outside the workspace.
///
/// Single source for permission matchers and tooling. Unresolvable inputs
/// (empty, `..` traversal, unmapped Unix-style on Windows) count as outside
/// so SAFE policy can deny them uniformly.
pub fn raw_path_outside_workspace(workspace_root: &Path, raw: &str) -> bool {
    match resolve_agent(workspace_root, raw, ToolPathMode::All) {
        Ok(resolved) => is_resolved_outside_workspace(workspace_root, &resolved),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_path_uses_workspace_root() {
        let root = tempfile::tempdir().unwrap();
        let path = resolve_tool_path_from(root.path(), "src/main.rs", ToolPathMode::All).unwrap();
        assert!(path.starts_with(crate::config::path::canon_abs(root.path()).unwrap()));
    }

    #[test]
    fn relative_traversal_is_rejected_in_all_modes() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            resolve_tool_path_from(root.path(), "../outside", ToolPathMode::All),
            Err(ToolPathError::RelativeTraversal)
        ));
    }

    #[test]
    fn all_allows_and_safe_rejects_external_absolute_path() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let raw = external.path().to_string_lossy();
        assert!(resolve_tool_path_from(root.path(), &raw, ToolPathMode::All).is_ok());
        assert!(matches!(
            resolve_tool_path_from(root.path(), &raw, ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
    }

    #[test]
    fn human_relative_rejects_absolute_paths() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let raw = external.path().to_string_lossy();
        assert!(matches!(
            resolve_human_relative(root.path(), &raw),
            Err(ToolPathError::OutsideWorkspace)
        ));
        assert!(resolve_human_relative(root.path(), "src/main.rs").is_ok());
    }

    #[test]
    fn agent_and_lsp_share_safe_scope() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::NamedTempFile::new().unwrap();
        let raw = external.path().to_string_lossy();
        assert!(matches!(
            resolve_agent(root.path(), &raw, ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
        assert!(matches!(
            resolve_lsp_workspace(root.path(), &raw),
            Err(ToolPathError::OutsideWorkspace)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn unix_style_passwd_not_relative_traversal() {
        let root = tempfile::tempdir().unwrap();
        let result = resolve_tool_path_from(root.path(), "/etc/passwd", ToolPathMode::All);
        match result {
            Ok(path) => {
                let s = path.to_string_lossy();
                assert!(s.contains("etc"), "mapped path should include etc: {s}");
            }
            Err(ToolPathError::UnixStyleOnWindows { .. }) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_absolute_outside_workspace_safe_rejected() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            resolve_tool_path_from(root.path(), "/etc/passwd", ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
        assert!(resolve_tool_path_from(root.path(), "/etc/passwd", ToolPathMode::All).is_ok());
    }

    #[test]
    fn raw_path_outside_workspace_matches_resolved_boundary() {
        let root = tempfile::tempdir().unwrap();
        assert!(!raw_path_outside_workspace(root.path(), "src/main.rs"));
        let external = tempfile::NamedTempFile::new().unwrap();
        let raw = external.path().to_string_lossy();
        assert!(raw_path_outside_workspace(root.path(), &raw));
        assert!(raw_path_outside_workspace(root.path(), "../outside"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dir_escape_rejected_for_new_file() {
        // 1.5: a symlinked directory inside the workspace pointing outside it.
        // Writing a NEW file through the link must be rejected — the target does
        // not exist, so a lexical fallback would pass `is_under` and the write
        // would land outside the workspace.
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(matches!(
            resolve_tool_path_from(root.path(), "link/newfile.txt", ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_subdir_escape_rejected_for_new_file() {
        // Same escape via a nested symlinked subdirectory.
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        let link = root.path().join("sub").join("link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(matches!(
            resolve_tool_path_from(root.path(), "sub/link/deep/new.txt", ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn symlinked_dir_escape_rejected_for_new_file_windows() {
        // Same 1.5 escape on Windows. Directory symlinks require Developer Mode
        // or an elevated shell; skip cleanly where creation is not permitted so
        // the test never flakes on a locked-down host.
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = root.path().join("link");
        // Creating a directory symlink requires Developer Mode or an elevated
        // shell; where it is not permitted (e.g. ERROR_PRIVILEGE_NOT_HELD,
        // os error 1314) the test skips cleanly instead of flaking.
        if let Err(e) = std::os::windows::fs::symlink_dir(outside.path(), &link) {
            eprintln!("skipping Windows symlink escape test: symlink creation not permitted ({e})");
            return;
        }
        assert!(matches!(
            resolve_tool_path_from(root.path(), "link/newfile.txt", ToolPathMode::Safe),
            Err(ToolPathError::OutsideWorkspace)
        ));
    }
}
