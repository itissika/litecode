//! Litecode Absolute Path (LAP): the single product form for compare / wire / URI.
//!
//! Contract:
//! 1. Absolute path after `canonicalize` (symlinks / `.` / `..` resolved).
//! 2. Windows verbatim prefixes stripped immediately (`\\?\C:\…` → `C:\…`,
//!    `\\?\UNC\host\share` → `\\host\share`).
//! 3. Windows ASCII drive letters uppercased (`c:\` → `C:\`).
//!
//! **Zero-bypass rule:** `std::fs::canonicalize` / `.canonicalize()` may appear
//! **only** in this module. All callers — product compare/wire **and** binary /
//! model / install-dir probes — must use [`canon_abs`], [`canon_abs_lossy`],
//! [`os_probe_abs`], [`strip_verbatim`], or [`is_under`]. Probe results are
//! still LAP (never raw verbatim).
//!
//! Capability (whether I/O must stay under the workspace) is intentionally
//! out of scope — LAP is identity/compare shape only.

use std::io;
use std::path::{Path, PathBuf};

/// Strip Windows `canonicalize` verbatim prefixes and normalize drive letter case.
///
/// No-op on Unix paths and already-stripped Windows paths.
pub fn strip_verbatim(path: impl AsRef<Path>) -> PathBuf {
    let p = path.as_ref();
    let s = p.to_string_lossy();
    let stripped = if let Some(rest) = s.strip_prefix(r"\\?\") {
        if let Some(unc) = rest.strip_prefix("UNC\\") {
            // `\\?\UNC\host\share` → `\\host\share`
            format!(r"\\{unc}")
        } else {
            rest.to_string()
        }
    } else {
        s.into_owned()
    };
    uppercase_drive(PathBuf::from(stripped))
}

/// Uppercase an ASCII Windows drive letter (`c:\foo` → `C:\foo`). No-op otherwise.
fn uppercase_drive(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let mut out = String::with_capacity(s.len());
        out.push((b[0] as char).to_ascii_uppercase());
        out.push_str(&s[1..]);
        return PathBuf::from(out);
    }
    path
}

/// Canonical absolute path in LAP form. Path must exist (or be creatable as a dir
/// only when callers create it first — this fn does not mkdir).
pub fn canon_abs(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    let absolute = make_absolute(path.as_ref())?;
    let canon = absolute.canonicalize()?;
    Ok(strip_verbatim(canon))
}

/// Probe an on-disk binary / model / install directory into LAP form.
///
/// Same implementation as [`canon_abs`]; the distinct name marks intent and
/// forbids callers from using bare `.canonicalize()` for probes.
pub fn os_probe_abs(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    canon_abs(path)
}

/// Best-effort LAP: canonicalize when possible, else strip-only on an absolute form.
pub fn canon_abs_lossy(path: impl AsRef<Path>) -> PathBuf {
    let absolute = make_absolute(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
    match absolute.canonicalize() {
        Ok(canon) => strip_verbatim(canon),
        Err(_) => strip_verbatim(absolute),
    }
}

fn make_absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Resolve a path that may not exist yet: canonicalize the nearest existing
/// ancestor (LAP), then join the remaining suffix. Used by Sandbox for new files.
///
/// `root` must already be LAP (e.g. from [`canon_abs`]). Escape checks use LAP
/// `starts_with` against `root`.
pub fn canon_join_nonexistent(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let mut parent = path.parent().unwrap_or(root);
    while !parent.exists() {
        parent = parent.parent().unwrap_or(root);
    }
    let canon_parent = canon_abs(parent)?;
    if !canon_parent.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path escapes project root",
        ));
    }
    let suffix = path.strip_prefix(parent).unwrap_or(path);
    let resolved = strip_verbatim(canon_parent.join(suffix));
    if !resolved.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path escapes project root",
        ));
    }
    Ok(resolved)
}

/// True if `path` equals or is strictly inside `ancestor` after both are LAP.
///
/// Containment is decided per path segment, never per character: a sibling
/// named `litecode-outside` is not under `litecode`. On Windows the same
/// location can be spelled in any case, so the comparison is case-folded
/// first and stays component-wise.
pub fn is_under(path: &Path, ancestor: &Path) -> bool {
    let path = canon_abs_lossy(path);
    let ancestor = canon_abs_lossy(ancestor);
    #[cfg(windows)]
    {
        let path = path.to_string_lossy().to_ascii_lowercase();
        let ancestor = ancestor.to_string_lossy().to_ascii_lowercase();
        Path::new(&path).starts_with(Path::new(&ancestor))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(&ancestor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    /// The reported defect: a sibling whose name merely starts with the root's
    /// name (`litecode-x` next to `litecode`) used to count as inside.
    #[test]
    fn is_under_compares_whole_segments() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("litecode");
        let sibling = tmp.path().join("litecode-x");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let touch = |path: std::path::PathBuf| {
            std::fs::write(&path, "x").unwrap();
            path
        };
        // A sibling with a shared character prefix is outside.
        assert!(!is_under(&touch(sibling.join("a.txt")), &root));
        // A real child is inside.
        assert!(is_under(&touch(root.join("sub").join("a.txt")), &root));
        // The root itself is inside, with and without a trailing separator.
        assert!(is_under(&root, &root));
        assert!(is_under(&root, &root.join("")));
    }

    /// Windows spells one location in any case; that must stay inside, and the
    /// segment rule must hold across spellings too.
    #[cfg(windows)]
    #[test]
    fn is_under_folds_case_on_windows() {
        let root = Path::new(r"C:\litecode");
        assert!(is_under(Path::new(r"C:\LITECODE"), root));
        assert!(is_under(Path::new(r"C:\LiteCode\sub\a.txt"), root));
        assert!(!is_under(Path::new(r"C:\LITECODE-X\a.txt"), root));
        // A real location, spelled in another case, is still inside.
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("litecode");
        std::fs::create_dir_all(real.join("sub")).unwrap();
        let upper = std::path::PathBuf::from(real.to_string_lossy().to_uppercase());
        assert!(is_under(&upper, &real));
    }
    #[test]
    fn strip_verbatim_drive_and_unc() {
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\E:\litecode")),
            PathBuf::from(r"E:\litecode")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\UNC\host\share\proj")),
            PathBuf::from(r"\\host\share\proj")
        );
        assert_eq!(
            strip_verbatim(Path::new("/home/proj")),
            PathBuf::from("/home/proj")
        );
    }

    #[test]
    fn strip_uppercases_drive_letter() {
        assert_eq!(
            strip_verbatim(Path::new(r"c:\litecode")),
            PathBuf::from(r"C:\litecode")
        );
        assert_eq!(
            strip_verbatim(Path::new(r"\\?\e:\litecode")),
            PathBuf::from(r"E:\litecode")
        );
    }

    #[cfg(windows)]
    #[test]
    fn is_under_is_case_insensitive_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let sub = root.join("SUB");
        std::fs::create_dir(&sub).unwrap();
        assert!(is_under(&sub, root));
        // Different-case spelling of the same location is still "under".
        assert!(is_under(&root.join("sub"), root));
        assert!(!is_under(root, &sub));
    }

    #[cfg(unix)]
    #[test]
    fn canon_abs_resolves_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let lap = canon_abs(&link).unwrap();
        assert_eq!(lap, canon_abs(&real).unwrap());
    }
}
