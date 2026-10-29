//! Resolve the program a launch declares. LiteCode finds the executable; it
//! never changes the command or its arguments.

use std::path::{Path, PathBuf};

/// Resolve one command to an executable path.
///
/// Absolute paths are used as written; a path with a separator is relative to
/// the working directory; a bare name is looked up on PATH (with PATHEXT on
/// Windows) and then in the working directory. Nothing is rewritten: a
/// declared command runs exactly as written.
pub fn resolve_executable(command: &str, cwd: &Path) -> Result<PathBuf, String> {
    if command.trim().is_empty() {
        return Err("command is empty".into());
    }
    let path = Path::new(command);
    if path.is_absolute() {
        return existing(path.to_path_buf(), command, cwd);
    }
    if command.contains('/') || command.contains('\\') {
        return existing(cwd.join(path), command, cwd);
    }
    if let Some(found) = search_path(command) {
        return Ok(found);
    }
    existing(cwd.join(path), command, cwd)
}

fn existing(path: PathBuf, command: &str, cwd: &Path) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "command not found: {command} (cwd: {})",
            cwd.display()
        ))
    }
}

fn search_path(command: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let extensions = executable_extensions();
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for extension in &extensions {
            let candidate = dir.join(format!("{command}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let mut out: Vec<String> = raw
        .split(';')
        .filter(|part| !part.trim().is_empty())
        .map(|part| part.trim().to_ascii_lowercase())
        .collect();
    if !out.iter().any(|ext| ext.is_empty()) {
        out.push(String::new());
    }
    out
}

#[cfg(not(windows))]
fn executable_extensions() -> Vec<String> {
    vec![String::new()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_paths_are_used_as_written() {
        let exe = std::env::current_exe().expect("current exe");
        assert_eq!(
            resolve_executable(&exe.to_string_lossy(), Path::new(".")).expect("resolve"),
            exe
        );
    }

    #[test]
    fn relative_paths_resolve_under_the_working_directory() {
        let dir = std::env::temp_dir().join(format!("litecode-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let name = if cfg!(windows) { "tool.cmd" } else { "tool.sh" };
        let file = dir.join(name);
        std::fs::write(&file, b"").expect("write tool");
        let resolved = resolve_executable(&format!(".{}{}", std::path::MAIN_SEPARATOR, name), &dir)
            .expect("resolve relative");
        assert_eq!(resolved, file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_command_is_an_error() {
        let err = resolve_executable("litecode-no-such-command-xyz", Path::new("."))
            .expect_err("missing command");
        assert!(err.contains("litecode-no-such-command-xyz"), "{err}");
    }

    #[test]
    fn npx_keeps_its_own_name() {
        let Some(found) = search_path("npx") else {
            return;
        };
        let name = found
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert_eq!(name, "npx", "npx must resolve to npx, not npm");
    }
}
