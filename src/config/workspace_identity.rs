//! Stable workspace identity: local `.litecode/workspace.json` + host registry.
//!
//! `workspace_id` associates a project tree with host-global data (snapshots).
//! It is not a connection credential, host key, or Agent session id.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::session::snapshot_paths::{litecode_data_home, workspace_snapshot_id};
use crate::types::{LitecodeError, Result};

const WORKSPACE_IDENTITY_VERSION: u32 = 1;
const REGISTRY_VERSION: u32 = 1;
const IDENTITY_OVERRIDE_ENV: &str = "LITECODE_WORKSPACE_IDENTITY";

/// Serializes the host-wide `workspace-registry.json` read-modify-write.
///
/// Multiple workspaces can be initialized concurrently in one process (tool
/// warmup, parallel tests); without this the registry's read-modify-write is a
/// data race (last-writer-wins drops entries / transient file-not-found on a
/// concurrent tmp+rename). The registry is process-global and small, so a
/// single coarse mutex is the correct cost for correctness.
static REGISTRY_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkspaceIdentityFile {
    version: u32,
    workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct WorkspaceRegistryFile {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, RegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RegistryEntry {
    /// LAP-normalized absolute workspace root.
    root: String,
}

/// Path to `.litecode/workspace.json`.
pub fn workspace_identity_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("workspace.json")
}

/// Host registry path (`~/.litecode/workspace-registry.json` or env home).
pub fn workspace_registry_path() -> PathBuf {
    litecode_data_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".litecode")
        .join("workspace-registry.json")
}

/// Read local `workspace_id` if the identity file exists.
pub fn peek_workspace_id(workspace_root: &Path) -> Option<String> {
    read_local_identity(workspace_root)
        .ok()
        .flatten()
        .map(|f| f.workspace_id)
}

/// Ensure a stable `workspace_id` for this workspace root (move/copy rules).
///
/// Returns the id that must be used for host-global paths (snapshots, etc.).
pub fn ensure_workspace_identity(workspace_root: &Path) -> Result<String> {
    let _guard = REGISTRY_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    ensure_workspace_identity_locked(workspace_root)
}

fn ensure_workspace_identity_locked(workspace_root: &Path) -> Result<String> {
    let root = crate::config::path::canon_abs(workspace_root).map_err(|e| {
        LitecodeError::Config(format!("canonicalize workspace root for identity: {e}"))
    })?;
    let root_key = root_key(&root);

    let litecode_dir = root.join(".litecode");
    std::fs::create_dir_all(&litecode_dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

    let mut local_id = match read_local_identity(&root)? {
        Some(file) if !file.workspace_id.trim().is_empty() => file.workspace_id,
        _ => {
            // First boot: adopt legacy path-derived id so existing snapshot dirs stay linked.
            let inherited = workspace_snapshot_id(&root);
            write_local_identity(&root, &inherited)?;
            inherited
        }
    };

    let mut registry = load_registry()?;
    let override_choice = identity_override();

    match registry.entries.get(&local_id).cloned() {
        None => {
            registry.entries.insert(
                local_id.clone(),
                RegistryEntry {
                    root: root_key.clone(),
                },
            );
            save_registry(&registry)?;
            Ok(local_id)
        }
        Some(entry) if paths_equivalent(&entry.root, &root_key) => Ok(local_id),
        Some(entry) => {
            let registered = PathBuf::from(&entry.root);
            let registered_exists = path_exists_as_dir(&registered);

            if !registered_exists {
                // Move: update registration to the new location.
                if matches!(override_choice, Some(IdentityChoice::Copy)) {
                    local_id = mint_copy_identity(&root, &mut registry)?;
                    return Ok(local_id);
                }
                registry.entries.insert(
                    local_id.clone(),
                    RegistryEntry {
                        root: root_key.clone(),
                    },
                );
                save_registry(&registry)?;
                tracing::info!(
                    workspace_id = %local_id,
                    from = %entry.root,
                    to = %root_key,
                    "workspace identity: treated as move"
                );
                return Ok(local_id);
            }

            // Registered path still exists and differs → copy (or explicit move).
            match override_choice {
                Some(IdentityChoice::Move) => {
                    registry.entries.insert(
                        local_id.clone(),
                        RegistryEntry {
                            root: root_key.clone(),
                        },
                    );
                    save_registry(&registry)?;
                    tracing::warn!(
                        workspace_id = %local_id,
                        from = %entry.root,
                        to = %root_key,
                        "workspace identity: forced move while original path still exists"
                    );
                    Ok(local_id)
                }
                Some(IdentityChoice::Copy) | None => {
                    // Default when original still exists: treat as copy.
                    if override_choice.is_none() {
                        tracing::info!(
                            workspace_id = %local_id,
                            registered = %entry.root,
                            current = %root_key,
                            "workspace identity: treated as copy (original path still exists)"
                        );
                    }
                    local_id = mint_copy_identity(&root, &mut registry)?;
                    Ok(local_id)
                }
            }
        }
    }
}

fn mint_copy_identity(root: &Path, registry: &mut WorkspaceRegistryFile) -> Result<String> {
    let new_id = uuid::Uuid::new_v4().to_string();
    write_local_identity(root, &new_id)?;
    registry.entries.insert(
        new_id.clone(),
        RegistryEntry {
            root: root_key(root),
        },
    );
    save_registry(registry)?;
    Ok(new_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdentityChoice {
    Move,
    Copy,
}

fn identity_override() -> Option<IdentityChoice> {
    let raw = std::env::var(IDENTITY_OVERRIDE_ENV).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "move" => Some(IdentityChoice::Move),
        "copy" => Some(IdentityChoice::Copy),
        other if other.is_empty() => None,
        other => {
            tracing::warn!(
                value = %other,
                env = IDENTITY_OVERRIDE_ENV,
                "unknown workspace identity override; ignoring"
            );
            None
        }
    }
}

fn read_local_identity(workspace_root: &Path) -> Result<Option<WorkspaceIdentityFile>> {
    let path = workspace_identity_path(workspace_root);
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let file: WorkspaceIdentityFile = serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    Ok(Some(file))
}

fn write_local_identity(workspace_root: &Path, workspace_id: &str) -> Result<()> {
    let path = workspace_identity_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let file = WorkspaceIdentityFile {
        version: WORKSPACE_IDENTITY_VERSION,
        workspace_id: workspace_id.to_string(),
    };
    let body =
        serde_json::to_string_pretty(&file).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

fn load_registry() -> Result<WorkspaceRegistryFile> {
    let path = workspace_registry_path();
    if !path.exists() {
        return Ok(WorkspaceRegistryFile {
            version: REGISTRY_VERSION,
            entries: BTreeMap::new(),
        });
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let content = content.trim();
    if content.is_empty() {
        return Ok(WorkspaceRegistryFile {
            version: REGISTRY_VERSION,
            entries: BTreeMap::new(),
        });
    }
    let mut file: WorkspaceRegistryFile = serde_json::from_str(content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    file.version = REGISTRY_VERSION;
    Ok(file)
}

fn save_registry(file: &WorkspaceRegistryFile) -> Result<()> {
    let path = workspace_registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let mut file = file.clone();
    file.version = REGISTRY_VERSION;
    let body =
        serde_json::to_string_pretty(&file).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        LitecodeError::Config(e.to_string())
    })
}

fn root_key(root: &Path) -> String {
    root.to_string_lossy().replace('\\', "/")
}

fn paths_equivalent(a: &str, b: &str) -> bool {
    let normalize = |s: &str| {
        let mut out = s.replace('\\', "/");
        while out.ends_with('/') && out.len() > 1 {
            out.pop();
        }
        #[cfg(windows)]
        {
            out = out.to_ascii_lowercase();
        }
        out
    };
    normalize(a) == normalize(b)
}

fn path_exists_as_dir(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_dir(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Isolate the shared home env for this test. Delegates to the single
    /// process-wide lock in `snapshot_paths::test_home` so this module cannot
    /// race other `--lib` tests (config::workspace, tools::todo) that also
    /// repoint `LITECODE_HOME`.
    fn isolated_home() -> crate::session::snapshot_paths::test_home::HomeGuard {
        crate::session::snapshot_paths::test_home::isolate_home()
    }

    #[test]
    fn first_boot_writes_identity_and_registry() {
        let _env = isolated_home();
        let ws = tempfile::tempdir().unwrap();
        let id = ensure_workspace_identity(ws.path()).unwrap();
        assert!(!id.is_empty());
        assert_eq!(peek_workspace_id(ws.path()).as_deref(), Some(id.as_str()));
        let again = ensure_workspace_identity(ws.path()).unwrap();
        assert_eq!(id, again);
    }

    #[test]
    fn move_keeps_id_when_old_path_gone() {
        let _env = isolated_home();

        let original = tempfile::tempdir().unwrap();
        let id = ensure_workspace_identity(original.path()).unwrap();
        let identity_body =
            std::fs::read_to_string(workspace_identity_path(original.path())).unwrap();

        let moved = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(moved.path().join(".litecode")).unwrap();
        std::fs::write(workspace_identity_path(moved.path()), &identity_body).unwrap();
        drop(original); // remove original path

        let moved_id = ensure_workspace_identity(moved.path()).unwrap();
        assert_eq!(id, moved_id);
    }

    #[test]
    fn copy_mints_new_id_when_original_still_exists() {
        let _env = isolated_home();

        let original = tempfile::tempdir().unwrap();
        let id = ensure_workspace_identity(original.path()).unwrap();
        let identity_body =
            std::fs::read_to_string(workspace_identity_path(original.path())).unwrap();

        let copy = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(copy.path().join(".litecode")).unwrap();
        std::fs::write(workspace_identity_path(copy.path()), &identity_body).unwrap();

        let copy_id = ensure_workspace_identity(copy.path()).unwrap();
        assert_ne!(id, copy_id);
        assert_eq!(
            peek_workspace_id(copy.path()).as_deref(),
            Some(copy_id.as_str())
        );
        // Original keeps its id.
        assert_eq!(ensure_workspace_identity(original.path()).unwrap(), id);
    }
}
