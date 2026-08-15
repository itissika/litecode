//! Locate Git for Windows installation paths (shared by shell and agent path resolution).

use std::path::PathBuf;
use std::process::Command;

/// Locate Git Bash's `bash.exe`. Returns `None` when Git for Windows is not installed.
#[cfg(windows)]
pub fn find_git_bash() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"));
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(not(windows))]
pub fn find_git_bash() -> Option<PathBuf> {
    None
}

/// Git for Windows install root (`...\Git`), derived from `bash.exe` at `...\Git\bin\bash.exe`.
#[cfg(windows)]
pub fn find_git_root() -> Option<PathBuf> {
    find_git_bash().and_then(|bash| {
        bash.parent()
            .and_then(|bin| bin.parent())
            .map(PathBuf::from)
    })
}

#[cfg(not(windows))]
pub fn find_git_root() -> Option<PathBuf> {
    None
}

/// Resolve a usable `git` executable for snapshot CLI (PATH, then Git for Windows).
pub fn find_git_exe() -> Option<PathBuf> {
    if Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        return Some(PathBuf::from("git"));
    }
    #[cfg(windows)]
    {
        if let Some(root) = find_git_root() {
            for rel in ["cmd\\git.exe", "bin\\git.exe"] {
                let p = root.join(rel);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}
