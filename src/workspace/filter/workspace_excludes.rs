//! Workspace-owned exclude lists. Seeded from VS Code defaults on first open;
//! Settings writes `.litecode/excludes.json` and activates the process cache.
//! Direct edits to that file are reloaded by the workspace watcher.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::types::{LitecodeError, Result};

use super::defaults::{CODE_INDEX_EXTENSIONS, FILES_EXCLUDE, SEARCH_EXCLUDE, WATCHER_EXCLUDE};

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
    /// Suffix allowlist for the code semantic index. Missing field → built-in
    /// source suffixes. Present `[]` → index no source files.
    #[serde(default = "default_code_index_extensions")]
    pub code_index_extensions: Vec<String>,
    #[serde(default = "default_git_ignore")]
    pub git_ignore: bool,
    /// Browse-only: honor `.gitignore` in the explorer tree, independent of
    /// [`Self::git_ignore`] which gates the search / index corpora.
    #[serde(default = "default_explorer_git_ignore")]
    pub explorer_git_ignore: bool,
}

fn default_git_ignore() -> bool {
    true
}

fn default_explorer_git_ignore() -> bool {
    false
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

fn default_code_index_extensions() -> Vec<String> {
    static_globs(CODE_INDEX_EXTENSIONS)
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
            code_index_extensions: default_code_index_extensions(),
            git_ignore: true,
            explorer_git_ignore: false,
        }
    }

    pub fn lists_view(&self) -> WorkspaceExcludesLists {
        WorkspaceExcludesLists {
            files_exclude: self.files_exclude.clone(),
            search_exclude: self.search_exclude.clone(),
            watcher_exclude: self.watcher_exclude.clone(),
            code_index_extensions: self.code_index_extensions.clone(),
            git_ignore: self.git_ignore,
            explorer_git_ignore: self.explorer_git_ignore,
        }
    }

    fn normalize(mut self) -> Self {
        self.version = WORKSPACE_EXCLUDES_VERSION;
        self.files_exclude = normalize_globs(self.files_exclude);
        self.search_exclude = normalize_globs(self.search_exclude);
        self.watcher_exclude = normalize_globs(self.watcher_exclude);
        self.code_index_extensions = normalize_extensions(self.code_index_extensions);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceExcludesLists {
    pub files_exclude: Vec<String>,
    pub search_exclude: Vec<String>,
    pub watcher_exclude: Vec<String>,
    /// Absent on an old settings client → built-in suffixes. `[]` is kept.
    #[serde(default = "default_code_index_extensions")]
    pub code_index_extensions: Vec<String>,
    pub git_ignore: bool,
    /// Browse-only gitignore switch; `#[serde(default)]` keeps PUT bodies from
    /// older clients valid.
    #[serde(default)]
    pub explorer_git_ignore: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceExcludesView {
    pub files_exclude: Vec<String>,
    pub search_exclude: Vec<String>,
    pub watcher_exclude: Vec<String>,
    pub code_index_extensions: Vec<String>,
    pub git_ignore: bool,
    pub explorer_git_ignore: bool,
    pub defaults: WorkspaceExcludesLists,
}

impl WorkspaceExcludesView {
    pub fn from_file(file: &WorkspaceExcludesFile) -> Self {
        let lists = file.lists_view();
        Self {
            files_exclude: lists.files_exclude,
            search_exclude: lists.search_exclude,
            watcher_exclude: lists.watcher_exclude,
            code_index_extensions: lists.code_index_extensions,
            git_ignore: lists.git_ignore,
            explorer_git_ignore: lists.explorer_git_ignore,
            defaults: WorkspaceExcludesFile::builtin_defaults().lists_view(),
        }
    }
}

/// Suffix tokens, not globs. `*.tsx`, `.RS`, and `types.d.ts` all become the
/// last segment (`tsx`, `rs`, `ts`). A trailing-slash directory form is not
/// folded into a directory glob — this list never matches directories.
fn normalize_extensions(exts: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in exts {
        let Some(token) = normalize_extension_token(&raw) else {
            continue;
        };
        if seen.insert(token.clone()) {
            out.push(token);
        }
    }
    out
}

fn normalize_extension_token(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let token = t.rsplit('.').next().unwrap_or(t);
    let token = token.trim_matches(|c: char| matches!(c, '*' | '/' | '\\' | ' '));
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(token.to_ascii_lowercase())
}

fn normalize_globs(globs: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for g in globs {
        let t = g.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // `dir/` is gitignore-speak and matches nothing here (rels have no
        // trailing slash). Fold it to `dir` so a copied ignore line works.
        let t = t.trim_end_matches('/');
        if t.is_empty() {
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
    cache().read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn activate_workspace_excludes(file: WorkspaceExcludesFile) {
    *cache().write().unwrap_or_else(|e| e.into_inner()) = file.normalize();
}

pub fn workspace_excludes_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".litecode").join("excludes.json")
}

/// Workspace-relative `/`-separated path of the excludes file.
pub const WORKSPACE_EXCLUDES_REL: &str = ".litecode/excludes.json";

pub fn is_workspace_excludes_rel(rel: &str) -> bool {
    rel.trim_start_matches("./") == WORKSPACE_EXCLUDES_REL
}

/// Paths whose change must rescan the semantic code index (excludes + gitignore).
pub fn path_triggers_code_index_sync(rel: &str) -> bool {
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    is_workspace_excludes_rel(&rel) || rel == ".gitignore" || rel.ends_with("/.gitignore")
}

/// Re-read `.litecode/excludes.json` into the process cache.
///
/// Missing file or parse/IO errors leave the current cache unchanged (do not
/// re-seed). Returns whether the cache was replaced.
pub fn reload_workspace_excludes_from_disk(workspace_root: &Path) -> bool {
    let path = workspace_excludes_path(workspace_root);
    if !path.exists() {
        tracing::warn!(
            path = %path.display(),
            "excludes.json missing; keeping active lists"
        );
        return false;
    }
    match read_workspace_excludes(workspace_root) {
        Ok(file) => {
            activate_workspace_excludes(file);
            true
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "excludes.json reload failed; keeping active lists"
            );
            false
        }
    }
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
    


}

#[cfg(test)]
pub(crate) fn lock_excludes_cache_for_test() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Activate `file` for the test body, then restore the previous cache.
/// Serializes tests that mutate the global excludes cache.
#[cfg(test)]
pub(crate) fn with_excludes_cache_for_test(file: WorkspaceExcludesFile, f: impl FnOnce()) {
    struct CacheRestore(WorkspaceExcludesFile);
    impl Drop for CacheRestore {
        fn drop(&mut self) {
            activate_workspace_excludes(self.0.clone());
        }
    }
    let _lock = lock_excludes_cache_for_test();
    let _guard = CacheRestore(active_workspace_excludes());
    activate_workspace_excludes(file);
    f();
}
