//! Workspace-relative path helpers shared by watchers and discovery walks.

use std::io;
use std::path::{Path, PathBuf};

use crate::config::path::{canon_abs, canon_abs_lossy, strip_verbatim};

/// Cached LAP root for repeated relative-path resolution (walk / search hot path).
#[derive(Debug, Clone)]
pub struct RelPathCtx {
    root_lap: PathBuf,
}

impl RelPathCtx {
    /// Canon root once. Fails if `root` cannot be canonicalized.
    pub fn new(root: &Path) -> io::Result<Self> {
        Ok(Self {
            root_lap: canon_abs(root)?,
        })
    }

    /// Best-effort LAP root when canonicalize may fail.
    pub fn new_lossy(root: &Path) -> Self {
        Self {
            root_lap: canon_abs_lossy(root),
        }
    }

    pub fn root_lap(&self) -> &Path {
        &self.root_lap
    }

    /// Map `path` to a `/`-separated relative path under the cached root, or `None`
    /// if outside. Canonicalizes `path` once (no `exists()` pre-check).
    pub fn rel(&self, path: &Path) -> Option<String> {
        let canon_path = match canon_abs(path) {
            Ok(p) => p,
            Err(_) => canon_abs_lossy(path),
        };
        if !canon_path.starts_with(&self.root_lap) {
            return None;
        }
        let rel = canon_path.strip_prefix(&self.root_lap).ok()?;
        Some(rel.to_string_lossy().replace('\\', "/"))
    }
}

/// Map an absolute (or exist/non-exist) path to a `/`-separated relative path
/// under `root`, or `None` if outside the workspace.
///
/// Both sides are compared in LAP form. Prefer [`RelPathCtx`] on hot paths.
pub fn rel_path_under(root: &Path, path: &Path) -> Option<String> {
    RelPathCtx::new(root).ok()?.rel(path)
}

/// Walk-filter relative path: `strip_prefix(walk_root)` without canonicalize.
///
/// Used for exclude / include shape matching only — not a sandbox trust boundary.
/// Strips Windows verbatim prefixes and ASCII-lowercases before compare (same
/// idea as [`crate::config::path::is_under`]). Returns `None` when `path` is not
/// under `walk_root` by string prefix.
pub fn cheap_rel_under(walk_root: &Path, path: &Path) -> Option<String> {
    // Fast path: same PathBuf form as WalkBuilder root (no allocation).
    if let Ok(rel) = path.strip_prefix(walk_root) {
        return Some(normalize_rel(rel));
    }

    let walk_root = strip_verbatim(walk_root);
    let path = strip_verbatim(path);

    if let Ok(rel) = path.strip_prefix(&walk_root) {
        return Some(normalize_rel(&rel));
    }

    #[cfg(windows)]
    {
        return cheap_rel_under_windows(&walk_root, &path);
    }
    #[cfg(not(windows))]
    {
        let _ = (walk_root, path);
        None
    }
}

fn normalize_rel(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(windows)]
fn cheap_rel_under_windows(walk_root: &Path, path: &Path) -> Option<String> {
    let root_norm = walk_root.to_string_lossy().replace('\\', "/");
    let path_norm = path.to_string_lossy().replace('\\', "/");
    let root_trim = root_norm.trim_end_matches('/');
    let root_l = root_trim.to_ascii_lowercase();
    let path_l = path_norm.to_ascii_lowercase();
    if path_l == root_l {
        return Some(String::new());
    }
    let prefix = format!("{root_l}/");
    if !path_l.starts_with(&prefix) {
        return None;
    }
    // ASCII case fold preserves length for drive-letter paths.
    let rel = path_norm.get(root_trim.len()..)?.trim_start_matches('/');
    Some(rel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rel_path_ctx_matches_rel_path_under() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let file = root.join("src").join("a.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn x() {}\n").unwrap();

        let ctx = RelPathCtx::new(root).unwrap();
        assert_eq!(ctx.rel(&file).as_deref(), Some("src/a.rs"));
        assert_eq!(rel_path_under(root, &file).as_deref(), Some("src/a.rs"));
        assert_eq!(ctx.rel(&file), rel_path_under(root, &file));
    }

    #[test]
    fn rel_path_ctx_rejects_outside() {
        let dir = TempDir::new().unwrap();
        let ctx = RelPathCtx::new(dir.path()).unwrap();
        let outside = TempDir::new().unwrap();
        let other = outside.path().join("x.rs");
        std::fs::write(&other, "x").unwrap();
        assert_eq!(ctx.rel(&other), None);
    }

    #[test]
    fn cheap_rel_under_strips_without_canon() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let file = root.join("nested").join("b.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "x").unwrap();

        let rel = cheap_rel_under(root, &file).expect("cheap rel");
        assert_eq!(rel, "nested/b.txt");
    }

    #[test]
    fn cheap_rel_under_rejects_outside() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let other = outside.path().join("x.rs");
        std::fs::write(&other, "x").unwrap();
        assert_eq!(cheap_rel_under(dir.path(), &other), None);
    }

    #[cfg(windows)]
    #[test]
    fn cheap_rel_under_strips_verbatim_prefix() {
        let dir = TempDir::new().unwrap();
        let root = canon_abs(dir.path()).unwrap();
        let file = root.join("a.txt");
        std::fs::write(&file, "x").unwrap();
        let raw = PathBuf::from(format!(r"\\?\{}", file.display()));
        assert!(
            raw.to_string_lossy().starts_with(r"\\?\"),
            "expected verbatim form"
        );
        let rel = cheap_rel_under(&root, &raw).expect("verbatim cheap rel");
        assert_eq!(rel, "a.txt");
    }
}
