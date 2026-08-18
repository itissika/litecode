//! Human IDE Source Control: git CLI against the **workspace** repository.
//!
//! Never points `GIT_DIR` at Litecode snapshot repos. Snapshot tracking stays
//! in `session::snapshot` and must not be called from this module.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};

use serde::Serialize;

use crate::config::git_install::find_git_exe;
use crate::session::snapshot_paths::{snapshots_dir_for_workspace, snapshots_root};
use crate::workspace::sandbox::{Sandbox, SandboxError};

const DEFAULT_LOG_LIMIT: usize = 50;
const MAX_LOG_LIMIT: usize = 200;

static MUTATE_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn mutate_lock(workspace: &Path) -> Arc<Mutex<()>> {
    let key = crate::config::path::canon_abs_lossy(workspace);
    let mut map = MUTATE_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git executable not found")]
    GitMissing,
    #[error("not a git repository")]
    NotARepo,
    #[error("{0}")]
    Command(String),
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error("path not allowed: {0}")]
    PathNotAllowed(String),
    #[error("commit message is required")]
    EmptyMessage,
    #[error("nothing to commit")]
    NothingToCommit,
    #[error("select at least one path")]
    EmptyPaths,
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitFile {
    pub path: String,
    /// Index or worktree letter (`M`, `A`, `D`, `R`, `C`, `U`, `?`).
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub untracked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitStatus {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub upstream_ahead: u32,
    pub upstream_behind: u32,
    pub staged: Vec<GitFile>,
    pub changes: Vec<GitFile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitCommit {
    pub sha: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitLog {
    pub is_repo: bool,
    pub commits: Vec<GitCommit>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GitMutation {
    pub ok: bool,
}

fn require_git() -> Result<PathBuf, GitError> {
    find_git_exe().ok_or(GitError::GitMissing)
}

/// Spawn git in `workspace` with snapshot-safe env (no inherited GIT_DIR).
fn git_command(workspace: &Path) -> Result<Command, GitError> {
    let git = require_git()?;
    let mut cmd = Command::new(git);
    cmd.current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    sanitize_git_env(&mut cmd);
    Ok(cmd)
}

fn sanitize_git_env(cmd: &mut Command) {
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_PREFIX",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
}

fn run_git(workspace: &Path, args: &[&str]) -> Result<(i32, Vec<u8>, String), GitError> {
    let output = git_command(workspace)?
        .args(args)
        .output()
        .map_err(GitError::Io)?;
    let code = output.status.code().unwrap_or(1);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((code, output.stdout, stderr))
}

fn run_git_ok(workspace: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
    let (code, stdout, stderr) = run_git(workspace, args)?;
    if code != 0 {
        return Err(GitError::Command(if stderr.is_empty() {
            format!("git {} failed (exit {code})", args.join(" "))
        } else {
            stderr
        }));
    }
    Ok(stdout)
}

fn stdout_text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// True when `git_dir` is the Litecode snapshot repo (must never be used here).
pub fn git_dir_is_snapshot(workspace: &Path, git_dir: &Path) -> bool {
    let snap = snapshots_dir_for_workspace(workspace);
    let git_dir = crate::config::path::canon_abs_lossy(git_dir);
    let snap = crate::config::path::canon_abs_lossy(&snap);
    git_dir == snap
        || crate::config::path::is_under(&git_dir, &snap)
        || crate::config::path::is_under(&git_dir, &snapshots_root())
}

fn is_git_repo(workspace: &Path) -> bool {
    match run_git(workspace, &["rev-parse", "--is-inside-work-tree"]) {
        Ok((0, stdout, _)) => stdout_text(stdout).eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn assert_not_snapshot_repo(workspace: &Path) -> Result<(), GitError> {
    let Ok(stdout) = run_git_ok(
        workspace,
        &["rev-parse", "--path-format=absolute", "--git-dir"],
    ) else {
        return Ok(());
    };
    let git_dir = PathBuf::from(stdout_text(stdout));
    if git_dir_is_snapshot(workspace, &git_dir) {
        return Err(GitError::Command(
            "refusing to use Litecode snapshot git directory".into(),
        ));
    }
    Ok(())
}

fn workspace_rel(sandbox: &Sandbox, requested: &str) -> Result<String, GitError> {
    let trimmed = requested.trim().trim_start_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Err(GitError::PathNotAllowed(requested.into()));
    }
    let abs = sandbox.resolve(trimmed)?;
    let rel = sandbox.rel_path(&abs)?;
    if rel.is_empty() || rel == "." {
        return Err(GitError::PathNotAllowed(requested.into()));
    }
    Ok(rel.replace('\\', "/"))
}

fn resolve_paths(sandbox: &Sandbox, paths: &[String]) -> Result<Vec<String>, GitError> {
    if paths.is_empty() {
        return Err(GitError::EmptyPaths);
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        out.push(workspace_rel(sandbox, p)?);
    }
    Ok(out)
}

fn parse_status_z(raw: &[u8]) -> (Vec<GitFile>, Vec<GitFile>) {
    let parts: Vec<&[u8]> = raw.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    let mut staged = Vec::new();
    let mut changes = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let rec = parts[i];
        i += 1;
        if rec.len() < 4 {
            continue;
        }
        let x = rec[0] as char;
        let y = rec[1] as char;
        if x == '!' {
            continue;
        }
        let path = String::from_utf8_lossy(&rec[3..])
            .replace('\\', "/")
            .to_string();
        let rename = matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C');
        let orig_path = if rename && i < parts.len() {
            let orig = String::from_utf8_lossy(parts[i])
                .replace('\\', "/")
                .to_string();
            i += 1;
            Some(orig)
        } else {
            None
        };
        let untracked = x == '?' && y == '?';
        if untracked {
            changes.push(GitFile {
                path,
                status: "?".into(),
                orig_path: None,
                untracked: true,
            });
            continue;
        }
        if x != ' ' && x != '?' {
            staged.push(GitFile {
                path: path.clone(),
                status: x.to_string(),
                orig_path: orig_path.clone(),
                untracked: false,
            });
        }
        if y != ' ' && y != '?' {
            changes.push(GitFile {
                path,
                status: y.to_string(),
                orig_path,
                untracked: false,
            });
        }
    }
    (staged, changes)
}

fn branch_name(workspace: &Path) -> Option<String> {
    if let Ok(stdout) = run_git_ok(workspace, &["symbolic-ref", "--short", "HEAD"]) {
        let name = stdout_text(stdout);
        if !name.is_empty() {
            return Some(name);
        }
    }
    if let Ok(stdout) = run_git_ok(workspace, &["branch", "--show-current"]) {
        let name = stdout_text(stdout);
        if !name.is_empty() {
            return Some(name);
        }
    }
    let Ok(stdout) = run_git_ok(workspace, &["rev-parse", "--abbrev-ref", "HEAD"]) else {
        return None;
    };
    let name = stdout_text(stdout);
    if name.is_empty() || name == "HEAD" {
        None
    } else {
        Some(name)
    }
}

fn upstream_counts(workspace: &Path) -> (u32, u32) {
    let Ok(stdout) = run_git_ok(
        workspace,
        &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
    ) else {
        return (0, 0);
    };
    let text = stdout_text(stdout);
    let mut bits = text.split_whitespace();
    let behind = bits.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = bits.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

pub fn status(workspace: &Path) -> Result<GitStatus, GitError> {
    if !is_git_repo(workspace) {
        return Ok(GitStatus {
            is_repo: false,
            branch: None,
            upstream_ahead: 0,
            upstream_behind: 0,
            staged: Vec::new(),
            changes: Vec::new(),
        });
    }
    assert_not_snapshot_repo(workspace)?;
    let stdout = run_git_ok(
        workspace,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=dirty",
        ],
    )?;
    let (staged, changes) = parse_status_z(&stdout);
    let (upstream_ahead, upstream_behind) = upstream_counts(workspace);
    Ok(GitStatus {
        is_repo: true,
        branch: branch_name(workspace),
        upstream_ahead,
        upstream_behind,
        staged,
        changes,
    })
}

pub fn log(workspace: &Path, limit: Option<usize>) -> Result<GitLog, GitError> {
    if !is_git_repo(workspace) {
        return Ok(GitLog {
            is_repo: false,
            commits: Vec::new(),
        });
    }
    assert_not_snapshot_repo(workspace)?;
    let n = limit.unwrap_or(DEFAULT_LOG_LIMIT).clamp(1, MAX_LOG_LIMIT);
    let n_s = n.to_string();
    let stdout = match run_git_ok(
        workspace,
        &[
            "log",
            "-n",
            &n_s,
            "--format=%H%x00%s%x00%an%x00%aI%x00%b%x1e",
        ],
    ) {
        Ok(s) => s,
        Err(GitError::Command(msg)) if msg.contains("does not have any commits") => {
            return Ok(GitLog {
                is_repo: true,
                commits: Vec::new(),
            });
        }
        Err(e) => return Err(e),
    };
    Ok(GitLog {
        is_repo: true,
        commits: parse_log(&stdout),
    })
}

fn parse_log(raw: &[u8]) -> Vec<GitCommit> {
    let text = String::from_utf8_lossy(raw);
    let mut commits = Vec::new();
    for rec in text.split('\u{1e}') {
        let rec = rec.trim_matches(['\n', '\r', '\0']);
        if rec.is_empty() {
            continue;
        }
        let mut fields = rec.split('\0');
        let Some(sha) = fields.next() else { continue };
        if sha.is_empty() {
            continue;
        }
        let subject = fields.next().unwrap_or("").to_string();
        let author = fields.next().unwrap_or("").to_string();
        let date = fields.next().unwrap_or("").to_string();
        let body = fields.next().unwrap_or("").trim().to_string();
        commits.push(GitCommit {
            sha: sha.to_string(),
            subject,
            author,
            date,
            body,
        });
    }
    commits
}

fn with_mutate<T>(
    workspace: &Path,
    f: impl FnOnce() -> Result<T, GitError>,
) -> Result<T, GitError> {
    let lock = mutate_lock(workspace);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

fn require_repo(workspace: &Path) -> Result<(), GitError> {
    if !is_git_repo(workspace) {
        return Err(GitError::NotARepo);
    }
    assert_not_snapshot_repo(workspace)
}

fn git_with_paths<'a>(base: &'a [&'a str], paths: &'a [String]) -> Vec<&'a str> {
    let mut args = Vec::with_capacity(base.len() + 1 + paths.len());
    args.extend_from_slice(base);
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    args
}

pub fn stage(sandbox: &Sandbox, paths: &[String]) -> Result<GitMutation, GitError> {
    let workspace = sandbox.root();
    let rels = resolve_paths(sandbox, paths)?;
    with_mutate(workspace, || {
        require_repo(workspace)?;
        let cmd = git_with_paths(&["add", "-A"], &rels);
        run_git_ok(workspace, &cmd)?;
        Ok(GitMutation { ok: true })
    })
}

pub fn unstage(sandbox: &Sandbox, paths: &[String]) -> Result<GitMutation, GitError> {
    let workspace = sandbox.root();
    let rels = resolve_paths(sandbox, paths)?;
    with_mutate(workspace, || {
        require_repo(workspace)?;
        let cmd = git_with_paths(&["restore", "--staged"], &rels);
        match run_git_ok(workspace, &cmd) {
            Ok(_) => {}
            Err(_) => {
                let cmd = git_with_paths(&["rm", "-f", "--cached"], &rels);
                run_git_ok(workspace, &cmd)?;
            }
        }
        Ok(GitMutation { ok: true })
    })
}

pub fn restore(sandbox: &Sandbox, paths: &[String]) -> Result<GitMutation, GitError> {
    let workspace = sandbox.root();
    let rels = resolve_paths(sandbox, paths)?;
    with_mutate(workspace, || {
        require_repo(workspace)?;
        let st = status(workspace)?;
        let is_untracked = |p: &str| {
            st.changes
                .iter()
                .any(|f| f.untracked && (f.path == p || f.path.starts_with(&format!("{p}/"))))
        };
        let is_new_in_index = |p: &str| {
            st.staged.iter().any(|f| f.path == p && f.status == "A")
                && !st.changes.iter().any(|f| f.path == p && !f.untracked)
        };
        let untracked: Vec<String> = rels.iter().filter(|p| is_untracked(p)).cloned().collect();
        let new_in_index: Vec<String> = rels
            .iter()
            .filter(|p| !is_untracked(p) && is_new_in_index(p))
            .cloned()
            .collect();
        let tracked: Vec<String> = rels
            .iter()
            .filter(|p| !is_untracked(p) && !is_new_in_index(p))
            .cloned()
            .collect();
        if !tracked.is_empty() {
            let cmd = git_with_paths(
                &["restore", "--source=HEAD", "--worktree", "--staged"],
                &tracked,
            );
            run_git_ok(workspace, &cmd)?;
        }
        if !new_in_index.is_empty() {
            let cmd = git_with_paths(&["restore", "--staged"], &new_in_index);
            if run_git_ok(workspace, &cmd).is_err() {
                let cmd = git_with_paths(&["rm", "-f", "--cached"], &new_in_index);
                run_git_ok(workspace, &cmd)?;
            }
            let cmd = git_with_paths(&["clean", "-f", "-d"], &new_in_index);
            run_git_ok(workspace, &cmd)?;
        }
        if !untracked.is_empty() {
            let cmd = git_with_paths(&["clean", "-f", "-d"], &untracked);
            run_git_ok(workspace, &cmd)?;
        }
        Ok(GitMutation { ok: true })
    })
}

pub fn commit(sandbox: &Sandbox, message: &str) -> Result<GitMutation, GitError> {
    let workspace = sandbox.root();
    let message = message.trim();
    if message.is_empty() {
        return Err(GitError::EmptyMessage);
    }
    with_mutate(workspace, || {
        require_repo(workspace)?;
        let st = status(workspace)?;
        if st.staged.is_empty() {
            return Err(GitError::NothingToCommit);
        }
        run_git_ok(workspace, &["commit", "-m", message])?;
        Ok(GitMutation { ok: true })
    })
}

pub fn pull(workspace: &Path) -> Result<GitMutation, GitError> {
    with_mutate(workspace, || {
        require_repo(workspace)?;
        run_git_ok(workspace, &["pull"])?;
        Ok(GitMutation { ok: true })
    })
}

pub fn push(workspace: &Path) -> Result<GitMutation, GitError> {
    with_mutate(workspace, || {
        require_repo(workspace)?;
        run_git_ok(workspace, &["push"])?;
        Ok(GitMutation { ok: true })
    })
}

pub fn git_error_status(err: &GitError) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    match err {
        GitError::NotARepo | GitError::GitMissing => StatusCode::BAD_REQUEST,
        GitError::EmptyMessage | GitError::NothingToCommit | GitError::EmptyPaths => {
            StatusCode::BAD_REQUEST
        }
        GitError::Sandbox(SandboxError::Escape) | GitError::PathNotAllowed(_) => {
            StatusCode::FORBIDDEN
        }
        GitError::Sandbox(SandboxError::Invalid(_)) => StatusCode::BAD_REQUEST,
        GitError::Sandbox(SandboxError::NotFound(_)) => StatusCode::NOT_FOUND,
        GitError::Command(_) => StatusCode::BAD_REQUEST,
        GitError::Io(_) | GitError::Sandbox(SandboxError::Io(_)) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command as StdCommand;

    fn git_available() -> bool {
        find_git_exe().is_some()
    }

    fn init_repo(dir: &Path) {
        let git = find_git_exe().expect("git");
        let status = StdCommand::new(&git)
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir)
            .status()
            .expect("git init");
        assert!(status.success());
        let _ = StdCommand::new(&git)
            .args(["symbolic-ref", "HEAD", "refs/heads/main"])
            .current_dir(dir)
            .status();
        let cfg = |key: &str, val: &str| {
            let ok = StdCommand::new(&git)
                .args(["config", key, val])
                .current_dir(dir)
                .status()
                .unwrap()
                .success();
            assert!(ok);
        };
        cfg("user.email", "test@litecode.local");
        cfg("user.name", "Litecode Test");
        cfg("commit.gpgsign", "false");
    }

    fn sandbox_at(dir: &Path) -> Sandbox {
        Sandbox::new(dir.to_path_buf()).expect("sandbox")
    }

    #[test]
    fn parse_status_splits_staged_and_changes() {
        let raw = b"M  staged.txt\0 M unstaged.txt\0MM both.txt\0?? new.txt\0";
        let (staged, changes) = parse_status_z(raw);
        assert_eq!(
            staged.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["staged.txt", "both.txt"]
        );
        assert_eq!(
            changes.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["unstaged.txt", "both.txt", "new.txt"]
        );
        assert!(changes.iter().any(|f| f.untracked && f.path == "new.txt"));
    }

    #[test]
    fn parse_status_rename_consumes_orig_path() {
        let raw = b"R  dest.txt\0src.txt\0";
        let (staged, changes) = parse_status_z(raw);
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].path, "dest.txt");
        assert_eq!(staged[0].orig_path.as_deref(), Some("src.txt"));
        assert!(changes.is_empty());
    }

    #[test]
    fn parse_log_records() {
        let raw = b"abc123\0fix foo\0Ada\02026-01-01T00:00:00Z\0longer body\n\x1edef456\0add bar\0Bob\02026-01-02T00:00:00Z\0\x1e";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].subject, "fix foo");
        assert_eq!(commits[0].body, "longer body");
        assert_eq!(commits[1].author, "Bob");
    }

    #[test]
    fn not_a_repo_status_is_empty() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let st = status(dir.path()).expect("status");
        assert!(!st.is_repo);
        assert!(st.staged.is_empty());
    }

    #[test]
    fn stage_commit_log_and_restore() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let sb = sandbox_at(dir.path());

        stage(&sb, &["a.txt".into()]).unwrap();
        let st = status(dir.path()).unwrap();
        assert!(st.is_repo);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert!(st.staged.iter().any(|f| f.path == "a.txt"));

        commit(&sb, "add a").unwrap();
        let lg = log(dir.path(), Some(10)).unwrap();
        assert_eq!(lg.commits.len(), 1);
        assert_eq!(lg.commits[0].subject, "add a");

        fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        fs::write(dir.path().join("b.txt"), "new\n").unwrap();
        let st = status(dir.path()).unwrap();
        assert!(st.changes.iter().any(|f| f.path == "a.txt"));
        assert!(st.changes.iter().any(|f| f.untracked && f.path == "b.txt"));

        restore(&sb, &["a.txt".into(), "b.txt".into()]).unwrap();
        let st = status(dir.path()).unwrap();
        assert!(st.changes.is_empty());
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "one\n"
        );
        assert!(!dir.path().join("b.txt").exists());
    }

    #[test]
    fn unstage_moves_file_back_to_changes() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let sb = sandbox_at(dir.path());
        stage(&sb, &["a.txt".into()]).unwrap();
        unstage(&sb, &["a.txt".into()]).unwrap();
        let st = status(dir.path()).unwrap();
        assert!(st.staged.is_empty());
        assert!(st.changes.iter().any(|f| f.untracked && f.path == "a.txt"));
    }

    #[test]
    fn path_escape_is_rejected() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let sb = sandbox_at(dir.path());
        let err = stage(&sb, &["../secret.txt".into()]).unwrap_err();
        assert!(matches!(
            err,
            GitError::Sandbox(SandboxError::Escape) | GitError::PathNotAllowed(_)
        ));
        let err = restore(&sb, &[".".into()]).unwrap_err();
        assert!(matches!(err, GitError::PathNotAllowed(_)));
    }

    #[test]
    fn empty_commit_message_rejected() {
        if !git_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        let sb = sandbox_at(dir.path());
        let err = commit(&sb, "   ").unwrap_err();
        assert!(matches!(err, GitError::EmptyMessage));
    }

    #[test]
    fn inherited_git_dir_env_does_not_select_snapshot_repo() {
        if !git_available() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        init_repo(workspace.path());
        fs::write(workspace.path().join("a.txt"), "one\n").unwrap();
        let sb = sandbox_at(workspace.path());
        stage(&sb, &["a.txt".into()]).unwrap();
        commit(&sb, "add a").unwrap();

        let snapshots = tempfile::tempdir().unwrap();
        let fake_snap = snapshots.path().join("snap");
        fs::create_dir_all(&fake_snap).unwrap();
        let git = find_git_exe().unwrap();
        assert!(
            StdCommand::new(&git)
                .args(["init", "--bare"])
                .arg(&fake_snap)
                .status()
                .unwrap()
                .success()
        );

        let prev_git_dir = std::env::var_os("GIT_DIR");
        let prev_snap = std::env::var_os(crate::session::snapshot_paths::SNAPSHOTS_ROOT_ENV);
        unsafe {
            std::env::set_var("GIT_DIR", &fake_snap);
            std::env::set_var(
                crate::session::snapshot_paths::SNAPSHOTS_ROOT_ENV,
                snapshots.path(),
            );
        }
        let result = status(workspace.path());
        unsafe {
            match prev_git_dir {
                Some(v) => std::env::set_var("GIT_DIR", v),
                None => std::env::remove_var("GIT_DIR"),
            }
            match prev_snap {
                Some(v) => std::env::set_var(crate::session::snapshot_paths::SNAPSHOTS_ROOT_ENV, v),
                None => std::env::remove_var(crate::session::snapshot_paths::SNAPSHOTS_ROOT_ENV),
            }
        }
        let st = result.expect("status with poisoned GIT_DIR");
        assert!(st.is_repo);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert!(st.staged.is_empty());
        assert!(st.changes.is_empty());
    }

    #[test]
    fn git_dir_is_snapshot_detects_shadow_repo() {
        let workspace = PathBuf::from("/tmp/litecode-git-ui-ws");
        let snap = snapshots_dir_for_workspace(&workspace);
        assert!(git_dir_is_snapshot(&workspace, &snap));
        assert!(!git_dir_is_snapshot(&workspace, &workspace.join(".git")));
    }
}
