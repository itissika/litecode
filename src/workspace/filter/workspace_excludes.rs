//! Workspace-owned exclude lists. Seeded from VS Code defaults on first open;
//! Settings writes `.litecode/excludes.json` and activates the process cache.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::types::{LitecodeError, Result};

use super::defaults::{FILES_EXCLUDE, SEARCH_EXCLUDE, WATCHER_EXCLUDE};

const WORKSPACE_EXCLUDES_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceExcludesFile {
    pub version: u32,
    #[serde(default = "default_files_exclude")]
    pub files_exclude: Vec<String>,
    #[serde(default = "default_search_exclude")]
    pub search_exclude: Vec<String>,
    #[serde(default = "default_watcher_exclude")]
    pub watcher_exclude: Vec<String>,
    #[serde(default = "default_git_ignore")]
    pub git_ignore: bool,
}

fn default_git_ignore() -> bool {
    true
}

fn default_files_exclude() -> Vec<String> {
    static_globs(FILES_EXCLUDE)
}

fn default_search_exclude() -> Vec<String> {
    static_globs(SEARCH_EXCLUDE)
}

fn default_watcher_exclude() -> Vec<String> {
    static_globs(WATCHER_EXCLUDE)
}

fn static_globs(globs: &[&str]) -> Vec<String> {
    globs.iter().map(|s| (*s).to_string()).collect()
}

impl WorkspaceExcludesFile {
    pub fn builtin_defaults() -> Self {
        Self {
            version: WORKSPACE_EXCLUDES_VERSION,
            files_exclude: default_files_exclude(),
            search_exclude: default_search_exclude(),
            watcher_exclude: default_watcher_exclude(),
            git_ignore: true,
        }
    }

    pub fn lists_view(&self) -> WorkspaceExcludesLists {
        WorkspaceExcludesLists {
            files_exclude: self.files_exclude.clone(),
            search_exclude: self.search_exclude.clone(),
            watcher_exclude: self.watcher_exclude.clone(),
            git_ignore: self.git_ignore,
        }
    }

    fn normalize(mut self) -> Self {
        self.version = WORKSPACE_EXCLUDES_VERSION;
        self.files_exclude = normalize_globs(self.files_exclude);
        self.search_exclude = normalize_globs(self.search_exclude);
        self.watcher_exclude = normalize_globs(self.watcher_exclude);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceExcludesLists {
    pub files_exclude: Vec<String>,
    pub search_exclude: Vec<String>,
    pub watcher_exclude: Vec<String>,
    pub git_ignore: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceExcludesView {
    pub files_exclude: Vec<String>,
    pub search_exclude: Vec<String>,
    pub watcher_exclude: Vec<String>,
    pub git_ignore: bool,
    pub defaults: WorkspaceExcludesLists,
}

impl WorkspaceExcludesView {
    pub fn from_file(file: &WorkspaceExcludesFile) -> Self {
        let lists = file.lists_view();
        Self {
            files_exclude: lists.files_exclude,
            search_exclude: lists.search_exclude,
            watcher_exclude: lists.watcher_exclude,
            git_ignore: lists.git_ignore,
            defaults: WorkspaceExcludesFile::builtin_defaults().lists_view(),
        }
    }
}

fn normalize_globs(globs: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for g in globs {
        let t = g.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if seen.insert(t.to_string()) {
            out.push(t.to_string());
        }
    }
    out
}

fn cache() -> &'static RwLock<WorkspaceExcludesFile> {
    static CACHE: OnceLock<RwLock<WorkspaceExcludesFile>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(WorkspaceExcludesFile::builtin_defaults()))
}

/// Live lists used by walk / path matchers. Defaults until a workspace activates.
pub fn active_workspace_excludes() -> WorkspaceExcludesFile {
    cache()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn activate_workspace_excludes(file: WorkspaceExcludesFile) {
    *cache().write().unwrap_or_else(|e| e.into_inner()) = file.normalize();
}

pub fn workspace_excludes_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("excludes.json")
}

pub fn read_workspace_excludes(workspace_root: &Path) -> Result<WorkspaceExcludesFile> {
    let path = workspace_excludes_path(workspace_root);
    if !path.exists() {
        return Ok(WorkspaceExcludesFile::builtin_defaults());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| LitecodeError::Config(e.to_string()))?;
    let file: WorkspaceExcludesFile = serde_json::from_str(&content)
        .map_err(|e| LitecodeError::Config(format!("parse {}: {e}", path.display())))?;
    Ok(file.normalize())
}

/// Write the file without touching the process cache (unit tests).
pub fn persist_workspace_excludes(
    workspace_root: &Path,
    file: &WorkspaceExcludesFile,
) -> Result<()> {
    let path = workspace_excludes_path(workspace_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    let body = serde_json::to_string_pretty(&file.clone().normalize())
        .map_err(|e| LitecodeError::Config(e.to_string()))?;
    std::fs::write(&path, body).map_err(|e| LitecodeError::Config(e.to_string()))
}

/// Persist and activate (Settings PUT / workspace boot).
pub fn write_workspace_excludes(
    workspace_root: &Path,
    file: WorkspaceExcludesFile,
) -> Result<WorkspaceExcludesFile> {
    let file = file.normalize();
    persist_workspace_excludes(workspace_root, &file)?;
    activate_workspace_excludes(file.clone());
    Ok(file)
}

/// Create `.litecode/excludes.json` from builtins when missing, then activate.
pub fn ensure_workspace_excludes(workspace_root: &Path) -> Result<WorkspaceExcludesFile> {
    let path = workspace_excludes_path(workspace_root);
    if !path.exists() {
        persist_workspace_excludes(workspace_root, &WorkspaceExcludesFile::builtin_defaults())?;
    }
    let file = read_workspace_excludes(workspace_root)?;
    activate_workspace_excludes(file.clone());
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_match_static_tables() {
        let d = WorkspaceExcludesFile::builtin_defaults();
        assert_eq!(d.files_exclude, default_files_exclude());
        assert_eq!(d.search_exclude, default_search_exclude());
        assert!(d.git_ignore);
        assert!(d.search_exclude.iter().any(|g| g.contains("node_modules")));
    }

    #[test]
    fn persist_roundtrip_does_not_require_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = WorkspaceExcludesFile::builtin_defaults();
        file.search_exclude.retain(|g| !g.contains("node_modules"));
        file.search_exclude.push("**/vendor".into());
        persist_workspace_excludes(dir.path(), &file).unwrap();
        let read = read_workspace_excludes(dir.path()).unwrap();
        assert!(!read.search_exclude.iter().any(|g| g.contains("node_modules")));
        assert!(read.search_exclude.iter().any(|g| g == "**/vendor"));
    }

    #[test]
    fn normalize_drops_blanks_comments_dupes() {
        let file = WorkspaceExcludesFile {
            version: 0,
            files_exclude: vec![
                "  **/.git  ".into(),
                "".into(),
                "# comment".into(),
                "**/.git".into(),
            ],
            search_exclude: vec![],
            watcher_exclude: vec![],
            git_ignore: false,
        }
        .normalize();
        assert_eq!(file.files_exclude, vec!["**/.git".to_string()]);
        assert_eq!(file.version, WORKSPACE_EXCLUDES_VERSION);
        assert!(!file.git_ignore);
    }

    #[test]
    fn ensure_seeds_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let seeded = ensure_workspace_excludes(dir.path()).unwrap();
        assert!(workspace_excludes_path(dir.path()).is_file());
        assert_eq!(seeded.search_exclude, default_search_exclude());
    }
}
