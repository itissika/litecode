//! Physically isolated snapshot-repo paths (outside the workspace tree).
//!
//! Layout: `{root}/{workspace_id}/` where `root` is `LITECODE_SNAPSHOTS_DIR`
//! or `~/.litecode/snapshots`. Never under `<workspace>/.litecode/`.
//!
//! Prefer the persisted workspace identity (`config::workspace_identity`) as
//! `workspace_id`. Path-derived [`workspace_snapshot_id`] remains for migration
//! and tests that have not ensured identity yet.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Env override for the snapshots root (workspace_id still appended).
pub const SNAPSHOTS_ROOT_ENV: &str = "LITECODE_SNAPSHOTS_DIR";

/// Env override for Litecode's host data home (registry, default snapshots parent).
pub const LITECODE_HOME_ENV: &str = "LITECODE_HOME";

/// Resolve the per-workspace bare snapshot git directory from a stable id.
pub fn snapshots_dir_for_id(workspace_id: &str) -> PathBuf {
    snapshots_root().join(workspace_id)
}

/// Resolve snapshots dir using path-derived legacy id (tests / pre-identity).
pub fn snapshots_dir_for_workspace(workspace_root: &Path) -> PathBuf {
    snapshots_dir_for_id(&workspace_snapshot_id(workspace_root))
}

/// Snapshots root: env override, else `{litecode_data_home}/.litecode/snapshots`.
pub fn snapshots_root() -> PathBuf {
    if let Ok(dir) = std::env::var(SNAPSHOTS_ROOT_ENV) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    litecode_data_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".litecode")
        .join("snapshots")
}

/// Host data home: `LITECODE_HOME`, else `HOME` / `USERPROFILE`.
pub fn litecode_data_home() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(LITECODE_HOME_ENV) {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    litecode_home_from_env()
}

/// Legacy path-derived id: `{sanitized_basename}_{sha256(canonical)[0..16 hex]}`.
///
/// Used when bootstrapping identity for workspaces that lack `workspace.json`,
/// so existing snapshot directories stay linked after upgrade.
pub fn workspace_snapshot_id(workspace: &Path) -> String {
    let canonical = crate::config::path::canon_abs_lossy(workspace);
    let normalized = canonical
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();

    let basename = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("workspace");
    let safe: String = basename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(32)
        .collect();
    let safe = if safe.is_empty() {
        "workspace".to_string()
    } else {
        safe
    };
    format!("{safe}_{hex}")
}

/// True if `path` is equal to or strictly inside `ancestor` (LAP compare).
pub fn path_is_under(path: &Path, ancestor: &Path) -> bool {
    crate::config::path::is_under(path, ancestor)
}

/// Snapshot dir must not live inside the workspace (or its `.git`).
pub fn assert_snapshots_isolated(workspace: &Path, snapshots_dir: &Path) -> Result<(), String> {
    if path_is_under(snapshots_dir, workspace) {
        return Err(format!(
            "snapshot dir {} must not be inside workspace {}",
            snapshots_dir.display(),
            workspace.display()
        ));
    }
    let project_git = workspace.join(".git");
    if project_git.exists() && path_is_under(snapshots_dir, &project_git) {
        return Err(format!(
            "snapshot dir {} must not be inside project .git {}",
            snapshots_dir.display(),
            project_git.display()
        ));
    }
    Ok(())
}

/// Remove legacy in-workspace snapshot repos (never migrate; old layout was unsafe).
pub fn purge_legacy_in_workspace_snapshots(workspace_root: &Path) -> std::io::Result<bool> {
    let legacy = workspace_root.join(".litecode").join("snapshots");
    if !legacy.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&legacy)?;
    tracing::warn!(
        path = %legacy.display(),
        "purged legacy in-workspace snapshot repo (now stored under ~/.litecode/snapshots)"
    );
    Ok(true)
}

fn litecode_home_from_env() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .or_else(|| {
            std::env::var("USERPROFILE").ok().or_else(|| {
                let home = std::env::var("HOMEDRIVE").unwrap_or_default();
                let path = std::env::var("HOMEPATH").unwrap_or_default();
                let combined = format!("{home}{path}");
                if combined.is_empty() {
                    None
                } else {
                    Some(combined)
                }
            })
        })
        .map(PathBuf::from)
}

/// Shared test-only home isolation.
///
/// `init_workspace` / `ensure_workspace_identity` read the host registry via
/// the `LITECODE_HOME` / `LITECODE_SNAPSHOTS_DIR` env vars. Several `--lib` test
/// modules (config::workspace, config::workspace_identity, tools::todo) call
/// them, and each used its own lock (or none) while mutating the SAME env vars —
/// racing one another under parallel `cargo test`. A single process-wide lock
/// held for the whole isolation scope makes home env mutations mutually
/// exclusive, so no test reads a transient home another test is about to delete.
#[cfg(test)]
pub(crate) mod test_home {
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    /// The single process-wide lock guarding home/snapshots env mutation.
    ///
    /// Every `--lib` test that repoints `LITECODE_HOME` / `LITECODE_SNAPSHOTS_DIR`
    /// must acquire THIS lock (directly or via [`isolate_home`]) so parallel
    /// tests never read a transient home another test is about to delete.
    pub(crate) fn env_lock() -> &'static Mutex<()> {
        &LOCK
    }

    /// Guard that restores the previous home env and releases the shared lock.
    pub(crate) struct HomeGuard {
        home: Option<OsString>,
        snapshots: Option<OsString>,
        _dir: tempfile::TempDir,
        _lock: MutexGuard<'static, ()>,
    }

    /// Point the home env at a fresh tempdir for the lifetime of the returned
    /// guard. Use one guard per test, held for the whole test body.
    pub(crate) fn isolate_home() -> HomeGuard {
        let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        let home = std::env::var_os(super::LITECODE_HOME_ENV);
        let snapshots = std::env::var_os(super::SNAPSHOTS_ROOT_ENV);
        unsafe {
            std::env::set_var(super::LITECODE_HOME_ENV, dir.path().as_os_str());
            std::env::set_var(super::SNAPSHOTS_ROOT_ENV, dir.path().join("snapshots"));
        }
        HomeGuard {
            home,
            snapshots,
            _dir: dir,
            _lock,
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.home {
                    Some(v) => std::env::set_var(super::LITECODE_HOME_ENV, v),
                    None => std::env::remove_var(super::LITECODE_HOME_ENV),
                }
                match &self.snapshots {
                    Some(v) => std::env::set_var(super::SNAPSHOTS_ROOT_ENV, v),
                    None => std::env::remove_var(super::SNAPSHOTS_ROOT_ENV),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_id_is_stable_for_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let a = workspace_snapshot_id(dir.path());
        let b = workspace_snapshot_id(dir.path());
        assert_eq!(a, b);
        assert!(a.contains('_'));
    }

    #[test]
    fn path_is_under_detects_nested() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("a").join("b");
        std::fs::create_dir_all(&child).unwrap();
        assert!(path_is_under(&child, dir.path()));
        assert!(!path_is_under(dir.path(), &child));
    }

    #[test]
    fn assert_rejects_in_workspace_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join(".litecode").join("snapshots");
        std::fs::create_dir_all(&bad).unwrap();
        assert!(assert_snapshots_isolated(dir.path(), &bad).is_err());
    }

    #[test]
    fn purge_legacy_removes_dir() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(".litecode").join("snapshots");
        std::fs::create_dir_all(legacy.join(".git")).unwrap();
        assert!(purge_legacy_in_workspace_snapshots(dir.path()).unwrap());
        assert!(!legacy.exists());
        assert!(!purge_legacy_in_workspace_snapshots(dir.path()).unwrap());
    }
}
