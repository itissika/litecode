//! Workspace root resolution, InitWorkspace, and runtime path accessors.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use crate::session::snapshot;
use crate::session::snapshot_paths::purge_legacy_in_workspace_snapshots;
use crate::types::{LitecodeError, Result};

use serde::{Deserialize, Serialize};

use super::global_db::tools::is_workspace_optional;
use super::resolved::{WorkspacePaths, WorkspaceState};
use super::schema::ToolReadiness;

const CLAUDE_MD_SHELL: &str =
    "# Litecode workspace contract\n\n<!-- Add project instructions here -->\n";
thread_local! {
    static RUNTIME_PATHS: RefCell<Option<WorkspacePaths>> = const { RefCell::new(None) };
}

const WORKSPACE_ENGINES_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEnginesFile {
    pub version: u32,
    #[serde(default)]
    pub lsp: WorkspaceLspState,
    #[serde(default)]
    pub retrieval: WorkspaceRetrievalState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceLspState {
    #[serde(default)]
    pub desired: bool,
    #[serde(default)]
    pub servers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceRetrievalState {
    #[serde(default)]
    pub desired: bool,
}

impl Default for WorkspaceEnginesFile {
    fn default() -> Self {
        Self {
            version: WORKSPACE_ENGINES_VERSION,
            lsp: WorkspaceLspState::default(),
            retrieval: WorkspaceRetrievalState::default(),
        }
    }
}

pub fn workspace_engines_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("engines.json")
}

pub fn read_workspace_engines(workspace_root: &Path) -> Result<WorkspaceEnginesFile> {
    let path = workspace_engines_path(workspace_root);
    if !path.exists() {
        return Ok(WorkspaceEnginesFile::default());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))
}

pub fn write_workspace_engines(workspace_root: &Path, file: &WorkspaceEnginesFile) -> Result<()> {
    let path = workspace_engines_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let mut file = file.clone();
    file.version = WORKSPACE_ENGINES_VERSION;
    let body =
        serde_json::to_string_pretty(&file).map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

pub fn workspace_engine_desired(workspace_root: &Path, id: &str) -> bool {
    read_workspace_engines(workspace_root)
        .map(|file| match id {
            "lsp" => file.lsp.desired && !file.lsp.servers.is_empty(),
            "code_search" => file.retrieval.desired,
            _ => false,
        })
        .unwrap_or(false)
}

pub fn set_workspace_engine_desired(workspace_root: &Path, id: &str, desired: bool) -> Result<()> {
    let mut file = read_workspace_engines(workspace_root)?;
    match id {
        "lsp" => {
            if desired && file.lsp.servers.is_empty() {
                return Err(LitecodeError::Config(
                    "lsp engine requires at least one language server".into(),
                ));
            }
            file.lsp.desired = desired;
        }
        "code_search" => file.retrieval.desired = desired,
        _ => {
            return Err(LitecodeError::Config(format!(
                "unknown workspace engine '{id}'"
            )));
        }
    }
    write_workspace_engines(workspace_root, &file)
}

/// Workspace-scoped readiness derived from `.litecode/engines.json`.
pub fn workspace_readiness_from_engines(
    workspace_root: &Path,
) -> std::collections::HashMap<String, ToolReadiness> {
    let file = read_workspace_engines(workspace_root).unwrap_or_default();
    let mut out = std::collections::HashMap::new();
    if file.retrieval.desired && is_workspace_optional("code_search") {
        out.insert("code_search".into(), ToolReadiness::Ready);
    }
    if file.lsp.desired && !file.lsp.servers.is_empty() && is_workspace_optional("lsp") {
        out.insert("lsp".into(), ToolReadiness::Ready);
    }
    out
}

/// Configured LSP server ids from engines.json.
pub fn lsp_servers_from_engines(workspace_root: &Path) -> Vec<String> {
    read_workspace_engines(workspace_root)
        .map(|file| file.lsp.servers)
        .unwrap_or_default()
}

/// Persist retrieval engine desired-on and ensure index shell exists.
pub fn enable_code_search_engine(workspace_root: &Path) -> Result<()> {
    crate::engines::code_search::init_workspace_index(workspace_root)?;
    set_workspace_engine_desired(workspace_root, "code_search", true)
}

/// Persist LSP engine init: desired on + configured server ids.
pub fn write_lsp_init(workspace_root: &Path, servers: Vec<String>) -> Result<()> {
    if servers.is_empty() {
        return Err(LitecodeError::Config(
            "lsp init requires at least one language server".into(),
        ));
    }
    let mut engines = read_workspace_engines(workspace_root)?;
    engines.lsp = WorkspaceLspState {
        desired: true,
        servers,
    };
    write_workspace_engines(workspace_root, &engines)
}

/// Clear the enabled LSP server list and force desired off.
///
/// Distinct from stop: stop keeps `servers` so Start can reuse them; clearing
/// is what the UI needs when the last language-server card is turned Off.
pub fn clear_lsp_servers(workspace_root: &Path) -> Result<()> {
    let mut engines = read_workspace_engines(workspace_root)?;
    engines.lsp = WorkspaceLspState {
        desired: false,
        servers: vec![],
    };
    write_workspace_engines(workspace_root, &engines)
}

/// Merge workspace-scoped tool readiness from disk into a workspace state clone.
pub fn workspace_with_disk_readiness(workspace: &WorkspaceState) -> WorkspaceState {
    let mut workspace = workspace.clone();
    workspace.workspace_tool_readiness =
        workspace_readiness_from_engines(&workspace.workspace_root);
    workspace
}

/// Resolve workspace root: explicit `--workspace` or absolute canonical cwd.
pub fn resolve_workspace_root(override_path: Option<&Path>) -> Result<PathBuf> {
    let raw = match override_path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().map_err(|e| LitecodeError::Config(e.to_string()))?,
    };
    canonicalize_workspace_root(raw)
}

/// Canonicalize a workspace root path (creates the directory when missing).
///
/// Result is LAP ([`crate::config::path::canon_abs`]).
pub fn canonicalize_workspace_root(path: PathBuf) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        let cwd = std::env::current_dir().map_err(|e| LitecodeError::Config(e.to_string()))?;
        cwd.join(path)
    };
    if !absolute.exists() {
        std::fs::create_dir_all(&absolute).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    crate::config::path::canon_abs(&absolute)
        .map_err(|e| LitecodeError::Config(format!("canonicalize workspace root: {e}")))
}

/// Create workspace contract shell and `.litecode/` runtime layout.
pub fn init_workspace(workspace_root: &Path) -> Result<()> {
    let claude_md = workspace_root.join("CLAUDE.md");
    if !claude_md.exists() {
        std::fs::write(&claude_md, CLAUDE_MD_SHELL)
            .map_err(|e| LitecodeError::Config(e.to_string()))?;
    }

    let litecode_dir = workspace_root.join(".litecode");
    std::fs::create_dir_all(litecode_dir.join("logs"))
        .map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::create_dir_all(litecode_dir.join("plan"))
        .map_err(|e| LitecodeError::Config(e.to_string()))?;

    // File-revert snapshots live under ~/.litecode/snapshots — never in-tree.
    if let Err(e) = purge_legacy_in_workspace_snapshots(workspace_root) {
        tracing::warn!(error = %e, "failed to purge legacy in-workspace snapshots");
    }

    let sessions_db = litecode_dir.join("sessions.db");
    if !sessions_db.exists() {
        std::fs::File::create(&sessions_db).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }

    // Persist / reconcile stable workspace_id (move vs copy).
    super::workspace_identity::ensure_workspace_identity(workspace_root)?;

    Ok(())
}

pub fn read_contract(workspace_root: &Path) -> String {
    match read_contract_result(workspace_root) {
        ContractRead::Found(content) => content,
        ContractRead::Missing | ContractRead::IoError(_) => String::new(),
    }
}

/// Outcome of reading `CLAUDE.md` — distinguishes missing file from IO failure (G16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractRead {
    Found(String),
    Missing,
    IoError(std::io::ErrorKind),
}

pub fn read_contract_result(workspace_root: &Path) -> ContractRead {
    let path = workspace_root.join("CLAUDE.md");
    match std::fs::read_to_string(&path) {
        Ok(content) => ContractRead::Found(content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ContractRead::Missing,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to read workspace contract"
            );
            ContractRead::IoError(e.kind())
        }
    }
}

/// Load workspace layer: resolve root, init layout, read contract.
///
/// Process contract: one serve/CLI process owns one workspace for life.
/// The process cwd is left untouched (no global `chdir` side effect); all
/// downstream consumers resolve workspace paths through `active_paths()`,
/// which this function initializes explicitly below.
pub fn load_workspace_state(override_path: Option<&Path>) -> Result<WorkspaceState> {
    let workspace_root = resolve_workspace_root(override_path)?;
    init_workspace(&workspace_root)?;
    let workspace_id = super::workspace_identity::ensure_workspace_identity(&workspace_root)?;
    let paths = WorkspacePaths::for_workspace(&workspace_root, &workspace_id);
    // Explicitly initialize the runtime paths (RUNTIME_PATHS) so consumers do
    // not fall back to the process cwd, which no longer mirrors the workspace.
    set_runtime_paths(paths.clone());
    std::fs::create_dir_all(&paths.snapshots_dir)
        .map_err(|e| LitecodeError::Config(format!("create snapshots dir: {e}")))?;
    let workspace_tool_readiness = workspace_readiness_from_engines(&workspace_root);
    let state = WorkspaceState {
        workspace_root: workspace_root.clone(),
        workspace_id,
        contract: read_contract(&workspace_root),
        paths: paths.clone(),
        workspace_tool_readiness,
    };
    match snapshot::maintain_snapshots(&paths.snapshots_dir, &paths.sessions_db) {
        Ok(report) if report.orphans_removed > 0 || report.stale_removed > 0 => {
            tracing::info!(
                orphans = report.orphans_removed,
                stale = report.stale_removed,
                "snapshot maintenance"
            );
        }
        Err(e) => tracing::warn!(error = %e, "snapshot maintenance failed"),
        _ => {}
    }
    if let Err(e) = snapshot::warm_snapshot_repo(&workspace_root, &paths.snapshots_dir) {
        tracing::warn!(error = %e, "snapshot warm failed");
    }
    Ok(state)
}

#[cfg(test)]
mod engine_state_tests {
    use super::*;

    #[test]
    fn engines_json_is_sole_workspace_engine_truth() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_lsp_init(root, vec!["rust_analyzer".into()]).unwrap();

        assert!(workspace_engines_path(root).is_file());
        assert!(workspace_engine_desired(root, "lsp"));
        assert_eq!(
            lsp_servers_from_engines(root),
            vec!["rust_analyzer".to_string()]
        );
        assert_eq!(
            workspace_readiness_from_engines(root).get("lsp"),
            Some(&ToolReadiness::Ready)
        );
    }

    #[test]
    fn retrieval_desired_drives_readiness() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        set_workspace_engine_desired(root, "code_search", true).unwrap();
        assert!(workspace_engine_desired(root, "code_search"));
        assert_eq!(
            workspace_readiness_from_engines(root).get("code_search"),
            Some(&ToolReadiness::Ready)
        );
        set_workspace_engine_desired(root, "code_search", false).unwrap();
        assert!(!workspace_engine_desired(root, "code_search"));
        assert!(!workspace_readiness_from_engines(root).contains_key("code_search"));
    }

    #[test]
    fn lsp_desired_without_servers_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let err = set_workspace_engine_desired(root, "lsp", true).unwrap_err();
        assert!(err.to_string().contains("language server"));
    }

    #[test]
    fn stopping_lsp_preserves_configured_servers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_lsp_init(root, vec!["rust_analyzer".into(), "typescript".into()]).unwrap();

        set_workspace_engine_desired(root, "lsp", false).unwrap();

        assert!(!workspace_engine_desired(root, "lsp"));
        assert_eq!(
            lsp_servers_from_engines(root),
            vec!["rust_analyzer".to_string(), "typescript".to_string()]
        );
    }

    #[test]
    fn clearing_lsp_removes_servers_and_desired() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_lsp_init(root, vec!["rust_analyzer".into()]).unwrap();

        clear_lsp_servers(root).unwrap();

        assert!(!workspace_engine_desired(root, "lsp"));
        assert!(lsp_servers_from_engines(root).is_empty());
    }
}

/// Workspace root derived from resolved `.litecode/` paths.
pub fn workspace_root_from_paths(paths: &WorkspacePaths) -> PathBuf {
    paths
        .sessions_db
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Install resolved workspace paths for tool execution on the current thread.
pub fn set_runtime_paths(paths: WorkspacePaths) {
    RUNTIME_PATHS.with(|slot| *slot.borrow_mut() = Some(paths));
}

/// Clear thread-local paths (tests / fallback exercise).
pub fn clear_runtime_paths() {
    RUNTIME_PATHS.with(|slot| *slot.borrow_mut() = None);
}

/// Active workspace paths (from `ResolvedConfig`); falls back to LAP cwd layout.
pub fn active_paths() -> WorkspacePaths {
    RUNTIME_PATHS.with(|slot| {
        if let Some(paths) = slot.borrow().clone() {
            return paths;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lap = crate::config::path::canon_abs_lossy(&cwd);
        WorkspacePaths::for_legacy_root(&lap)
    })
}

/// Tool default workspace root in LAP form (identity/compare safe).
pub fn workspace_root_lap() -> PathBuf {
    crate::config::path::canon_abs_lossy(workspace_root_from_paths(&active_paths()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate_litecode_home() -> crate::session::snapshot_paths::test_home::HomeGuard {
        crate::session::snapshot_paths::test_home::isolate_home()
    }

    #[test]
    fn active_paths_prefers_runtime_paths_over_cwd() {
        let workspace_dir = tempfile::tempdir().unwrap();
        let cwd_dir = tempfile::tempdir().unwrap();
        let paths = WorkspacePaths::for_workspace(workspace_dir.path(), "test-workspace-id");
        set_runtime_paths(paths.clone());
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(cwd_dir.path()).unwrap();

        let active = active_paths();
        assert_eq!(active.plan_dir, paths.plan_dir);
        assert_eq!(active.snapshots_dir, paths.snapshots_dir);
        assert_ne!(
            active.plan_dir,
            cwd_dir.path().join(".litecode").join("plan")
        );

        std::env::set_current_dir(prev).ok();
    }

    #[test]
    fn init_workspace_creates_layout_and_claude_shell() {
        let _home = isolate_litecode_home();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Plant legacy in-tree snapshots — must be purged.
        std::fs::create_dir_all(root.join(".litecode").join("snapshots").join(".git")).unwrap();
        init_workspace(root).unwrap();

        assert!(root.join("CLAUDE.md").is_file());
        let claude = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(claude.contains("Litecode workspace contract"));

        let litecode = root.join(".litecode");
        assert!(litecode.join("logs").is_dir());
        assert!(litecode.join("plan").is_dir());
        assert!(
            !litecode.join("snapshots").exists(),
            "in-workspace snapshots must be purged"
        );
        assert!(litecode.join("sessions.db").is_file());
        assert!(
            litecode.join("workspace.json").is_file(),
            "init_workspace must persist workspace identity"
        );
    }

    #[test]
    fn load_workspace_state_uses_external_snapshots_dir() {
        let _home = isolate_litecode_home();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("CLAUDE.md"), "# Contract\n").unwrap();
        let state = load_workspace_state(Some(root)).unwrap();
        assert!(
            !crate::session::snapshot_paths::path_is_under(
                &state.paths.snapshots_dir,
                &state.workspace_root
            ),
            "snapshots_dir={} must not live under workspace {}",
            state.paths.snapshots_dir.display(),
            state.workspace_root.display()
        );
        assert!(state.paths.snapshots_dir.is_dir());
        assert!(!state.workspace_id.is_empty());
        assert!(
            state
                .paths
                .sessions_db
                .ends_with(std::path::Path::new(".litecode").join("sessions.db"))
        );
    }

    #[test]
    fn init_workspace_preserves_existing_claude_md() {
        let _home = isolate_litecode_home();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("CLAUDE.md"), "# My project\n").unwrap();
        init_workspace(root).unwrap();
        let claude = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert_eq!(claude, "# My project\n");
    }

    #[test]
    fn read_contract_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_contract_result(dir.path()), ContractRead::Missing);
        assert!(read_contract(dir.path()).is_empty());
    }

    #[test]
    fn read_contract_reads_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "# body\n").unwrap();
        assert_eq!(
            read_contract_result(dir.path()),
            ContractRead::Found("# body\n".into())
        );
    }

    #[test]
    #[cfg(unix)]
    fn read_contract_io_error_is_distinct_from_missing() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(&path, "secret").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&path, perms).unwrap();

        let result = read_contract_result(dir.path());
        assert_ne!(result, ContractRead::Missing);
        assert!(matches!(result, ContractRead::IoError(_)));
        assert!(read_contract(dir.path()).is_empty());
    }

    #[test]
    fn load_workspace_state_reads_contract() {
        let _home = isolate_litecode_home();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("CLAUDE.md"), "# Contract body\n").unwrap();
        let state = load_workspace_state(Some(root)).unwrap();
        assert_eq!(state.contract, "# Contract body\n");
        assert!(
            state
                .paths
                .sessions_db
                .ends_with(std::path::Path::new(".litecode").join("sessions.db"))
        );
    }
}
