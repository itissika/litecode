//! Symbol existence is the worktree. Drift is commit history.
//!
//! The baseline is the last commit that touched the knowledge note, not the
//! moment the citation was written. Uncommitted edits do not count as drift.
//! No git, or a note that was never committed, leaves drift unreported.

use std::collections::HashMap;
use std::path::Path;

use crate::engines::code_search::{ScopeMatch, find_scope, lines_slice};
use crate::workspace::{self, GitError, PathCommit};

use super::mentions::is_workspace_file_ref;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolCheck {
    Present,
    Missing,
    Ambiguous,
    Drifted { commits: Vec<PathCommit> },
}

pub struct SymbolCache {
    shown: HashMap<(String, String), Option<String>>,
    bases: HashMap<String, CachedBase>,
}

enum CachedBase {
    Off,
    Uncommitted,
    Commit(String),
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            shown: HashMap::new(),
            bases: HashMap::new(),
        }
    }

    /// Worktree lookup, then drift against `drift_base` when that path is set.
    pub fn check(
        &mut self,
        workspace: &Path,
        file: &str,
        chain: &str,
        drift_base: Option<&str>,
    ) -> SymbolCheck {
        let Some(content) = read_worktree(workspace, file) else {
            return SymbolCheck::Missing;
        };
        match find_scope(file, &content, chain) {
            ScopeMatch::Missing => return SymbolCheck::Missing,
            ScopeMatch::Ambiguous(_) => return SymbolCheck::Ambiguous,
            ScopeMatch::Unique(_) => {}
        }
        let Some(base_path) = drift_base.filter(|path| !path.is_empty()) else {
            return SymbolCheck::Present;
        };
        match self.drift(workspace, file, chain, base_path) {
            Some(commits) => SymbolCheck::Drifted { commits },
            None => SymbolCheck::Present,
        }
    }

    /// `Some` when the symbol body at HEAD differs from the note's baseline commit.
    /// `None` when drift is off, unknown, or unchanged.
    pub fn drift(
        &mut self,
        workspace: &Path,
        file: &str,
        chain: &str,
        drift_base: &str,
    ) -> Option<Vec<PathCommit>> {
        let base = self.baseline(workspace, drift_base)?;
        let then = self.blob(workspace, &base, file)?;
        let now = self.blob(workspace, "HEAD", file)?;
        let then_body = scope_body(file, &then, chain)?;
        let now_body = scope_body(file, &now, chain)?;
        if then_body == now_body {
            return None;
        }
        let commits = workspace::log_between(workspace, &base, file, 3).unwrap_or_default();
        Some(commits)
    }

    fn baseline(&mut self, workspace: &Path, drift_base: &str) -> Option<String> {
        if let Some(cached) = self.bases.get(drift_base) {
            return match cached {
                CachedBase::Commit(hash) => Some(hash.clone()),
                CachedBase::Off | CachedBase::Uncommitted => None,
            };
        }
        let cached = match workspace::last_commit_touching(workspace, drift_base) {
            Ok(Some(hash)) => CachedBase::Commit(hash),
            Ok(None) => CachedBase::Uncommitted,
            Err(GitError::GitMissing | GitError::NotARepo) => CachedBase::Off,
            Err(_) => CachedBase::Off,
        };
        let hash = match &cached {
            CachedBase::Commit(hash) => Some(hash.clone()),
            CachedBase::Off | CachedBase::Uncommitted => None,
        };
        self.bases.insert(drift_base.to_string(), cached);
        hash
    }

    fn blob(&mut self, workspace: &Path, rev: &str, file: &str) -> Option<String> {
        let key = (rev.to_string(), file.replace('\\', "/"));
        if let Some(cached) = self.shown.get(&key) {
            return cached.clone();
        }
        let loaded = match workspace::show_at(workspace, rev, file) {
            Ok(text) => text,
            Err(_) => None,
        };
        self.shown.insert(key, loaded.clone());
        loaded
    }
}

impl Default for SymbolCache {
    fn default() -> Self {
        Self::new()
    }
}

fn read_worktree(workspace: &Path, file: &str) -> Option<String> {
    if !is_workspace_file_ref(file) {
        return None;
    }
    let path = workspace.join(file.replace('\\', "/"));
    std::fs::read_to_string(path).ok()
}

fn scope_body(file: &str, content: &str, chain: &str) -> Option<String> {
    let ScopeMatch::Unique(entry) = find_scope(file, content, chain) else {
        return None;
    };
    Some(normalize_body(&lines_slice(
        content,
        entry.start_line,
        entry.end_line,
    )))
}

fn normalize_body(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    use crate::config::git_install::find_git_exe;

    fn git(dir: &Path, args: &[&str]) {
        let git = find_git_exe().expect("git");
        let status = Command::new(git)
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git");
        assert!(status.success(), "{args:?}");
    }

    fn init(dir: &Path) {
        git(dir, &["-c", "init.defaultBranch=main", "init"]);
        for (key, val) in [
            ("user.email", "test@litecode.local"),
            ("user.name", "Litecode Test"),
            ("commit.gpgsign", "false"),
        ] {
            git(dir, &["config", key, val]);
        }
    }

    fn write_note_and_fn(dir: &Path, body: &str) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join(".litecode/knowledge")).unwrap();
        fs::write(dir.join("src/a.rs"), body).unwrap();
        fs::write(dir.join(".litecode/knowledge/note.md"), "note\n").unwrap();
    }

    #[test]
    fn drift_follows_commits_and_ignores_the_worktree() {
        if find_git_exe().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        write_note_and_fn(dir.path(), "fn alpha() {\n    let x = 1;\n}\n");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-m", "note"]);

        let mut cache = SymbolCache::new();
        let chain = "fn alpha";
        let base = ".litecode/knowledge/note.md";
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain, Some(base)),
            SymbolCheck::Present
        );

        fs::write(
            dir.path().join("src/a.rs"),
            "fn alpha() {\n    let x = 2;\n}\n",
        )
        .unwrap();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain, Some(base)),
            SymbolCheck::Present,
            "uncommitted edit is not drift"
        );

        git(dir.path(), &["add", "src/a.rs"]);
        git(dir.path(), &["commit", "-m", "edit alpha"]);
        let mut cache = SymbolCache::new();
        match cache.check(dir.path(), "src/a.rs", chain, Some(base)) {
            SymbolCheck::Drifted { commits } => {
                assert_eq!(commits[0].subject, "edit alpha");
            }
            other => panic!("expected drift, got {other:?}"),
        }

        fs::write(dir.path().join("src/a.rs"), "fn beta() {}\n").unwrap();
        git(dir.path(), &["add", "src/a.rs"]);
        git(dir.path(), &["commit", "-m", "drop alpha"]);
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain, Some(base)),
            SymbolCheck::Missing
        );

        let bare = tempfile::tempdir().unwrap();
        fs::create_dir_all(bare.path().join("src")).unwrap();
        fs::write(bare.path().join("src/a.rs"), "fn alpha() {}\n").unwrap();
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(bare.path(), "src/a.rs", chain, Some(base)),
            SymbolCheck::Present,
            "no repository means no drift"
        );
    }
}
