//! Platform default shell for TerminalHub sessions and agent exec.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

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
        let program = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        ShellSpec {
            program,
            args: Vec::new(),
        }
    }
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

    #[test]
    fn default_shell_is_platform_native() {
        let s = default_shell();
        #[cfg(windows)]
        {
            let name = s.program.to_string_lossy().to_ascii_lowercase();
            assert!(
                name.contains("bash") || name.contains("powershell"),
                "got {:?}",
                s.program
            );
        }
        #[cfg(not(windows))]
        {
            assert!(!s.program.as_os_str().is_empty());
        }
    }

    #[test]
    fn shell_command_embeds_payload() {
        let s = shell_command("echo hi");
        let joined: Vec<String> = s
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            joined.iter().any(|a| a.contains("echo hi")),
            "args={joined:?}"
        );
    }
}
