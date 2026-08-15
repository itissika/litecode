//! Classify raw agent path strings before LAP join / Safe-All checks.
//!
//! Owns only: absolute vs relative classification, and Windows Unix-style
//! (Git Bash / MSYS) normalization. Policy, warnings, and schema hints live
//! elsewhere — see [`super::tool_path`] and [`crate::permission::sensitive`].

use std::path::{Component, Path, PathBuf};

use crate::config::git_install::find_git_root;
use crate::config::path::strip_verbatim;

/// Resolved path kind after classification (before workspace join / LAP).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathCandidate {
    /// Platform-native absolute path (or mapped from Unix-style on Windows).
    Absolute(PathBuf),
    /// Workspace-relative path (no `..`).
    Relative(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifyError {
    Empty,
    RelativeTraversal,
    UnixStyleOnWindows { raw: String },
}

/// Classify a trimmed agent path string into absolute vs relative, with Windows
/// Unix-style normalization.
pub fn classify_agent_path(raw: &str) -> Result<PathCandidate, ClassifyError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ClassifyError::Empty);
    }

    let input = Path::new(raw);

    if input.is_absolute() {
        return Ok(PathCandidate::Absolute(strip_verbatim(input)));
    }

    #[cfg(windows)]
    if is_unix_style_on_windows(raw) {
        match resolve_unix_style_windows(raw) {
            Some(mapped) => return Ok(PathCandidate::Absolute(mapped)),
            None => {
                return Err(ClassifyError::UnixStyleOnWindows {
                    raw: raw.to_string(),
                });
            }
        }
    }

    for component in input.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ClassifyError::RelativeTraversal);
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    Ok(PathCandidate::Relative(input.to_path_buf()))
}

/// True when `raw` looks like a Unix absolute path on Windows (`/etc/...`, not `//` UNC).
#[cfg(windows)]
fn is_unix_style_on_windows(raw: &str) -> bool {
    raw.starts_with('/') && !raw.starts_with("//")
}

/// Map Git Bash / MSYS-style paths to Windows paths.
#[cfg(windows)]
fn resolve_unix_style_windows(raw: &str) -> Option<PathBuf> {
    if let Some(drive_path) = msys_drive_path(raw) {
        return Some(drive_path);
    }

    if let Some(tmp_path) = msys_tmp_path(raw) {
        return Some(tmp_path);
    }

    let git_root = find_git_root()?;
    let relative = raw.trim_start_matches('/');
    if relative.is_empty() {
        return Some(git_root);
    }
    Some(git_root.join(relative))
}

/// `/tmp` and `/tmp/...` → process temp directory (Git Bash convention).
#[cfg(windows)]
fn msys_tmp_path(raw: &str) -> Option<PathBuf> {
    if raw == "/tmp" {
        return Some(std::env::temp_dir());
    }
    if let Some(rest) = raw.strip_prefix("/tmp/") {
        return Some(std::env::temp_dir().join(rest));
    }
    None
}

/// `/c/Users/foo` → `C:\Users\foo`; `/c` → `C:\`.
#[cfg(windows)]
fn msys_drive_path(raw: &str) -> Option<PathBuf> {
    if !raw.starts_with('/') {
        return None;
    }
    let rest = &raw[1..];
    if rest.is_empty() {
        return None;
    }

    let (drive, path_rest): (&str, String) = if let Some(slash) = rest.find('/') {
        (&rest[..slash], rest[slash + 1..].to_string())
    } else {
        (rest, String::new())
    };

    if drive.len() != 1 {
        return None;
    }
    let ch = drive.chars().next()?;
    if !ch.is_ascii_alphabetic() {
        return None;
    }

    let drive_root = format!("{}:\\", ch.to_ascii_uppercase());
    if path_rest.is_empty() {
        Some(PathBuf::from(drive_root))
    } else {
        let win = format!("{}{}", drive_root, path_rest.replace('/', "\\"));
        Some(PathBuf::from(win))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_traversal_is_rejected() {
        assert!(matches!(
            classify_agent_path("../outside"),
            Err(ClassifyError::RelativeTraversal)
        ));
    }

    #[test]
    fn relative_path_ok() {
        assert!(matches!(
            classify_agent_path("src/main.rs"),
            Ok(PathCandidate::Relative(_))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn unix_style_on_windows_not_traversal() {
        let result = classify_agent_path("/etc/passwd");
        match result {
            Ok(PathCandidate::Absolute(_)) => {}
            Err(ClassifyError::UnixStyleOnWindows { .. }) => {}
            Err(ClassifyError::RelativeTraversal) => {
                panic!("must not misclassify unix-style path as relative traversal");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn msys_tmp_mapping() {
        let mapped = msys_tmp_path("/tmp/litecode_tool_eval").unwrap();
        assert!(mapped.ends_with("litecode_tool_eval"));
        assert!(mapped.starts_with(std::env::temp_dir()));
    }

    #[cfg(windows)]
    #[test]
    fn msys_drive_mapping() {
        let mapped = msys_drive_path("/c/Users/foo").unwrap();
        assert_eq!(mapped, PathBuf::from(r"C:\Users\foo"));
        let mapped = msys_drive_path("/d").unwrap();
        assert_eq!(mapped, PathBuf::from(r"D:\"));
    }

    #[cfg(windows)]
    #[test]
    fn git_etc_passwd_maps_when_installed() {
        if find_git_root().is_none() {
            return;
        }
        let candidate = classify_agent_path("/etc/passwd").unwrap();
        assert!(matches!(candidate, PathCandidate::Absolute(_)));
        let abs = match candidate {
            PathCandidate::Absolute(p) => p,
            PathCandidate::Relative(_) => panic!("expected absolute"),
        };
        assert!(abs.to_string_lossy().contains("etc"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_absolute_is_native() {
        assert!(matches!(
            classify_agent_path("/etc/passwd"),
            Ok(PathCandidate::Absolute(_))
        ));
    }
}
