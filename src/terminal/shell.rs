//! Platform default shell for TerminalHub sessions and agent exec.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use crate::config::git_install::find_git_bash;

/// Program + args for an interactive login-style shell (no command).
#[derive(Debug, Clone)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// Default interactive shell for human PTY sessions.
pub fn default_shell() -> ShellSpec {
    #[cfg(windows)]
    {
        match find_git_bash() {
            Some(bash) => ShellSpec {
                program: bash,
                args: vec![OsString::from("-li")],
            },
            None => ShellSpec {
                program: PathBuf::from("powershell.exe"),
                args: vec![OsString::from("-NoLogo"), OsString::from("-NoProfile")],
            },
        }
    }
    #[cfg(not(windows))]
    {
        let env_shell = std::env::var_os("SHELL");
        let program = resolve_unix_shell_with(env_shell.as_deref(), |path| path.is_file());
        ShellSpec {
            program,
            args: Vec::new(),
        }
    }
}

#[allow(dead_code)]
fn resolve_unix_shell_with(
    env_shell: Option<&std::ffi::OsStr>,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    if let Some(raw) = env_shell.filter(|s| !s.is_empty()) {
        let candidate = PathBuf::from(raw);
        let is_plain_sh = candidate
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "sh");
        if !is_plain_sh && exists(&candidate) {
            return candidate;
        }
    }
    for bash in ["/bin/bash", "/usr/bin/bash"] {
        let candidate = PathBuf::from(bash);
        if exists(&candidate) {
            return candidate;
        }
    }
    if let Some(raw) = env_shell.filter(|s| !s.is_empty()) {
        let candidate = PathBuf::from(raw);
        if exists(&candidate) {
            return candidate;
        }
    }
    PathBuf::from("/bin/sh")
}

/// Shell invocation that runs a single command then exits (agent exec / bg task).
pub fn shell_command(command: &str) -> ShellSpec {
    #[cfg(windows)]
    {
        match find_git_bash() {
            Some(bash) => ShellSpec {
                program: bash,
                args: vec![OsString::from("-c"), OsString::from(command)],
            },
            None => ShellSpec {
                program: PathBuf::from("powershell.exe"),
                args: vec![
                    OsString::from("-NoLogo"),
                    OsString::from("-NoProfile"),
                    OsString::from("-NonInteractive"),
                    OsString::from("-Command"),
                    OsString::from(command),
                ],
            },
        }
    }
    #[cfg(not(windows))]
    {
        let mut spec = default_shell();
        // Prefer `-c` even when SHELL is zsh/bash/fish — all support it.
        spec.args = vec![OsString::from("-c"), OsString::from(command)];
        spec
    }
}

/// Display label for the shell program (`bash.exe` → `bash`), so a terminal
/// tab can name the shell it is running.
pub fn shell_label(spec: &ShellSpec) -> String {
    let stem = spec.program.file_stem().map(|stem| stem.to_string_lossy());
    match stem {
        Some(stem) if !stem.is_empty() => stem.to_ascii_lowercase(),
        _ => spec.program.to_string_lossy().to_ascii_lowercase(),
    }
}

/// Resolve working directory for a session (LAP workspace root when unset).
pub fn resolve_cwd(workdir: Option<&Path>) -> PathBuf {
    match workdir {
        Some(p) if !p.as_os_str().is_empty() => crate::config::path::canon_abs_lossy(p),
        _ => crate::config::workspace::workspace_root_lap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn resolve_unix_shell_prefers_bash_when_env_unset_or_sh() {
        let has_bash = |p: &Path| p == Path::new("/bin/bash") || p == Path::new("/bin/sh");
        assert_eq!(
            resolve_unix_shell_with(None, has_bash),
            PathBuf::from("/bin/bash")
        );
        assert_eq!(
            resolve_unix_shell_with(Some(OsStr::new("/bin/sh")), has_bash),
            PathBuf::from("/bin/bash")
        );
    }

    #[test]
    fn resolve_unix_shell_respects_custom_non_sh_shell() {
        let has_zsh = |p: &Path| p == Path::new("/usr/bin/zsh") || p == Path::new("/bin/bash");
        assert_eq!(
            resolve_unix_shell_with(Some(OsStr::new("/usr/bin/zsh")), has_zsh),
            PathBuf::from("/usr/bin/zsh")
        );
    }

    #[test]
    fn resolve_unix_shell_falls_back_to_sh_when_bash_missing() {
        let only_sh = |p: &Path| p == Path::new("/bin/sh");
        assert_eq!(
            resolve_unix_shell_with(None, only_sh),
            PathBuf::from("/bin/sh")
        );
        assert_eq!(
            resolve_unix_shell_with(Some(OsStr::new("/bin/sh")), only_sh),
            PathBuf::from("/bin/sh")
        );
    }
}
