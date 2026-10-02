//! Symbol existence is the worktree. Drift is that file's HEAD against the worktree.
//!
//! The note only displays the result. It is not the baseline, and it does not
//! need to be committed or tracked. No git leaves drift unreported.

use std::collections::HashMap;
use std::path::Path;

use crate::engines::code_search::{ScopeMatch, find_scope, lines_slice};
use crate::workspace;

use super::mentions::is_workspace_file_ref;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolCheck {
    Present,
    Missing,
    Ambiguous,
    Drifted,
}

/// Whether a symbol's body differs between HEAD and the file on disk.
///
/// `Unknown` is not "unchanged": there is no git, or the worktree body could
/// not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    Changed,
    Unchanged,
    Unknown,
}

#[derive(Clone)]
enum HeadFile {
    Off,
    Absent,
    Text(String),
}

pub struct SymbolCache {
    head: HashMap<String, HeadFile>,
}

impl SymbolCache {
    pub fn new() -> Self {
        Self {
            head: HashMap::new(),
        }
    }

    /// Worktree lookup, then HEAD against that worktree body.
    pub fn check(&mut self, workspace: &Path, file: &str, chain: &str) -> SymbolCheck {
        let Some(content) = read_worktree(workspace, file) else {
            return SymbolCheck::Missing;
        };
        match find_scope(file, &content, chain) {
            ScopeMatch::Missing => return SymbolCheck::Missing,
            ScopeMatch::Ambiguous(_) => return SymbolCheck::Ambiguous,
            ScopeMatch::Unique(_) => {}
        }
        match self.drift(workspace, file, chain) {
            Drift::Changed => SymbolCheck::Drifted,
            Drift::Unchanged | Drift::Unknown => SymbolCheck::Present,
        }
    }

    /// Compare the symbol body at HEAD with the file on disk.
    pub fn drift(&mut self, workspace: &Path, file: &str, chain: &str) -> Drift {
        let Some(now) = read_worktree(workspace, file) else {
            return Drift::Unknown;
        };
        let Some(now_body) = scope_body(file, &now, chain) else {
            return Drift::Unknown;
        };
        match self.head_file(workspace, file) {
            HeadFile::Off => Drift::Unknown,
            HeadFile::Absent => Drift::Changed,
            HeadFile::Text(then) => match scope_body(file, &then, chain) {
                Some(then_body) if then_body == now_body => Drift::Unchanged,
                _ => Drift::Changed,
            },
        }
    }

    fn head_file(&mut self, workspace: &Path, file: &str) -> HeadFile {
        let key = file.replace('\\', "/");
        if let Some(cached) = self.head.get(&key) {
            return cached.clone();
        }
        let loaded = match workspace::show_at(workspace, "HEAD", &key) {
            Ok(Some(text)) => HeadFile::Text(text),
            Ok(None) => HeadFile::Absent,
            Err(_) => HeadFile::Off,
        };
        self.head.insert(key, loaded.clone());
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

    #[test]
    fn drift_compares_head_with_the_worktree() {
        if find_git_exe().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init(dir.path());
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join(".litecode/knowledge")).unwrap();
        fs::write(
            dir.path().join("src/a.rs"),
            "fn alpha() {\n    let x = 1;\n}\n",
        )
        .unwrap();
        git(dir.path(), &["add", "src/a.rs"]);
        git(dir.path(), &["commit", "-m", "alpha"]);
        fs::write(dir.path().join(".litecode/knowledge/note.md"), "note\n").unwrap();

        let mut cache = SymbolCache::new();
        let chain = "fn alpha";
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain),
            SymbolCheck::Present,
            "an untracked note still compares the file"
        );

        fs::write(
            dir.path().join("src/a.rs"),
            "fn alpha() {\n    let x = 2;\n}\n",
        )
        .unwrap();
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain),
            SymbolCheck::Drifted
        );

        git(dir.path(), &["add", ".litecode/knowledge/note.md"]);
        git(dir.path(), &["commit", "-m", "note only"]);
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain),
            SymbolCheck::Drifted,
            "committing the note leaves the file diff in place"
        );

        git(dir.path(), &["add", "src/a.rs"]);
        git(dir.path(), &["commit", "-m", "edit alpha"]);
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain),
            SymbolCheck::Present,
            "committing the file clears drift"
        );

        fs::write(
            dir.path().join("src/b.rs"),
            "fn beta() {\n    let y = 1;\n}\n",
        )
        .unwrap();
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/b.rs", "fn beta"),
            SymbolCheck::Drifted,
            "a file absent from HEAD differs from the worktree"
        );

        fs::write(dir.path().join("src/a.rs"), "fn beta() {}\n").unwrap();
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(dir.path(), "src/a.rs", chain),
            SymbolCheck::Missing
        );

        let bare = tempfile::tempdir().unwrap();
        fs::create_dir_all(bare.path().join("src")).unwrap();
        fs::write(bare.path().join("src/a.rs"), "fn alpha() {}\n").unwrap();
        let mut cache = SymbolCache::new();
        assert_eq!(
            cache.check(bare.path(), "src/a.rs", chain),
            SymbolCheck::Present,
            "no repository means no drift"
        );
    }
}
