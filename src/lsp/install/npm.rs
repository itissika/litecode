//! npm install --prefix pipeline for installing Node.js-based LSP servers.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::lsp::paths::lsp_dir;
use crate::types::{LitecodeError, Result};

const NPM_NOT_FOUND: &str = "npm was not found. Install Node.js and npm, and ensure npm is on PATH";

/// Resolve the npm launcher. Windows must use `npm.cmd` / `npm.exe` (bare `npm`
/// is not a PE image). Invoke that path directly — do **not** wrap `.cmd` in
/// `cmd /C` via `ls_program_and_args`; Rust `Command` re-quotes and the spawn fails.
pub fn resolve_npm_shim() -> Option<PathBuf> {
    for dir in nodejs_search_dirs() {
        #[cfg(windows)]
        {
            let cmd = dir.join("npm.cmd");
            if cmd.is_file() {
                return Some(cmd);
            }
            let exe = dir.join("npm.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
        let unix = dir.join("npm");
        if unix.is_file() {
            return Some(unix);
        }
    }
    None
}

/// Program + argv for `std`/`tokio` `Command`. Always the shim path + npm args.
pub fn npm_program_and_args(npm_args: &[String]) -> Option<(PathBuf, Vec<String>)> {
    let shim = resolve_npm_shim()?;
    Some((shim, npm_args.to_vec()))
}

fn nodejs_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path_var) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path_var));
    }
    dirs.extend(extra_nodejs_dirs());
    dirs
}

fn extra_nodejs_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(symlink) = std::env::var("NVM_SYMLINK") {
            let trimmed = symlink.trim();
            if !trimmed.is_empty() {
                dirs.push(PathBuf::from(trimmed));
            }
        }
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(root) = std::env::var(key) {
                dirs.push(PathBuf::from(root).join("nodejs"));
            }
        }
    }
    #[cfg(unix)]
    {
        if let Ok(bin) = std::env::var("NVM_BIN") {
            let trimmed = bin.trim();
            if !trimmed.is_empty() {
                dirs.push(PathBuf::from(trimmed));
            }
        }
        let nvm_dir = std::env::var_os("NVM_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".nvm")));
        if let Some(nvm_dir) = nvm_dir {
            dirs.push(nvm_dir.join("current").join("bin"));
            if let Ok(alias) = std::fs::read_to_string(nvm_dir.join("alias").join("default")) {
                let ver = alias.trim();
                if !ver.is_empty() {
                    dirs.push(nvm_dir.join("versions").join("node").join(ver).join("bin"));
                }
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            dirs.push(home.join(".volta").join("bin"));
            dirs.push(
                home.join(".local")
                    .join("share")
                    .join("fnm")
                    .join("aliases")
                    .join("default")
                    .join("bin"),
            );
        }
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    }
    dirs
}

fn npm_command_tokio(npm_args: &[String]) -> Option<Command> {
    let (program, args) = npm_program_and_args(npm_args)?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    Some(cmd)
}

/// Install npm packages into `lsp_dir()/<server_id>/`.
/// The last package in `packages` is treated as the language server for version metadata.
pub async fn npm_install(server_id: &str, packages: &[(&str, &str)]) -> Result<()> {
    let dest_dir = lsp_dir()?.join(server_id);
    let Some((server_package, requested_version)) = packages.last().copied() else {
        return Err(LitecodeError::Config(format!(
            "{server_id}: npm install requires at least one package"
        )));
    };

    // Check if already installed with the correct version.
    if dest_dir.join(".meta").exists() && dest_dir.join("node_modules").is_dir() {
        if let Ok(meta) = read_meta(&dest_dir) {
            let installed_ver = meta.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if !installed_ver.is_empty() && installed_ver != "latest" {
                if requested_version == "latest" || installed_ver == requested_version {
                    if crate::lsp::deps::verify_managed_server(server_id).is_ok() {
                        tracing::info!(
                            server_id,
                            installed_ver,
                            "npm package already installed, skipping"
                        );
                        return Ok(());
                    }
                    tracing::warn!(
                        server_id,
                        "npm metadata exists but executable verification failed; reinstalling"
                    );
                }
                tracing::info!(
                    server_id,
                    old = installed_ver,
                    new = requested_version,
                    "version changed, reinstalling"
                );
            }
        }
    }

    let Some(mut version_cmd) = npm_command_tokio(&["--version".into()]) else {
        return Err(LitecodeError::Config(NPM_NOT_FOUND.into()));
    };
    let version_status = version_cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .await;
    match version_status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            return Err(LitecodeError::Config(format!(
                "npm --version failed (status {status}); executable: {}",
                resolve_npm_shim()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "npm".into())
            )));
        }
        Err(e) => {
            return Err(LitecodeError::Config(format!(
                "failed to start npm ({e}). {NPM_NOT_FOUND}"
            )));
        }
    }

    let parent = dest_dir.parent().unwrap_or(Path::new("."));
    let staging_dir = parent.join(format!(".{server_id}.staging-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging_dir).map_err(|e| {
        LitecodeError::Config(format!("create staging dir {}: {e}", staging_dir.display()))
    })?;

    let specs: Vec<String> = packages
        .iter()
        .map(|(name, version)| format!("{name}@{version}"))
        .collect();
    let mut install_args = vec![
        "install".into(),
        "--prefix".into(),
        staging_dir.to_string_lossy().into_owned(),
    ];
    install_args.extend(specs);
    let Some(mut cmd) = npm_command_tokio(&install_args) else {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(LitecodeError::Config(NPM_NOT_FOUND.into()));
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = cmd.output().await.map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging_dir);
        if e.kind() == std::io::ErrorKind::NotFound {
            LitecodeError::Config(NPM_NOT_FOUND.into())
        } else {
            LitecodeError::Config(format!("npm install failed (network error): {e}"))
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(LitecodeError::Config(format!(
            "npm install failed: {stderr}"
        )));
    }

    let actual_version = read_package_version(&staging_dir, server_package)
        .unwrap_or_else(|| requested_version.to_string());

    crate::lsp::deps::verify_managed_server_at(server_id, &staging_dir).map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging_dir);
        LitecodeError::Config(format!(
            "npm installed {server_id}, but its executable failed verification: {e}"
        ))
    })?;
    write_managed_meta(&staging_dir, &actual_version)?;
    super::replace_install_dir(&staging_dir, &dest_dir)?;
    super::github::write_manifest_entry(server_id, &actual_version)?;

    tracing::info!(server_id, version = %actual_version, "npm install complete");
    Ok(())
}

/// Locate `node` / `node.exe` on PATH and well-known Node install dirs.
pub fn resolve_node_binary() -> Option<PathBuf> {
    for dir in nodejs_search_dirs() {
        let unix = dir.join("node");
        if unix.is_file() {
            return Some(unix);
        }
        #[cfg(windows)]
        {
            let exe = dir.join("node.exe");
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// Read the installed version from node_modules/<package>/package.json.
fn read_package_version(dest_dir: &std::path::Path, package_name: &str) -> Option<String> {
    let pkg_json = dest_dir
        .join("node_modules")
        .join(package_name)
        .join("package.json");
    let data = std::fs::read_to_string(&pkg_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    parsed.get("version")?.as_str().map(|v| v.to_string())
}

// ---------------------------------------------------------------------------
// .meta helpers (same format as github.rs)
// ---------------------------------------------------------------------------

fn read_meta(dest_dir: &std::path::Path) -> Result<serde_json::Value> {
    let meta_path = dest_dir.join(".meta");
    let data = std::fs::read_to_string(&meta_path)
        .map_err(|e| LitecodeError::Config(format!("read .meta {}: {e}", meta_path.display())))?;
    serde_json::from_str(&data)
        .map_err(|e| LitecodeError::Config(format!("parse .meta {}: {e}", meta_path.display())))
}

pub(crate) fn managed_version(dest_dir: &std::path::Path) -> Option<String> {
    read_meta(dest_dir)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

pub(crate) fn write_managed_meta(dest_dir: &std::path::Path, version: &str) -> Result<()> {
    let meta = serde_json::json!({
        "version": version,
        "digest": "",
        "installed_at": chrono::Utc::now().to_rfc3339(),
    });
    let meta_path = dest_dir.join(".meta");
    let json = serde_json::to_string_pretty(&meta)?;
    super::github::atomic_write(&meta_path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_is_invoked_as_the_shim_not_via_cmd_exe() {
        let Some((program, args)) = npm_program_and_args(&["--version".into()]) else {
            return;
        };
        let name = program
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(
            name.eq_ignore_ascii_case("npm")
                || name.eq_ignore_ascii_case("npm.cmd")
                || name.eq_ignore_ascii_case("npm.exe"),
            "unexpected npm launcher {}",
            program.display()
        );
        assert!(
            !name.eq_ignore_ascii_case("cmd") && !name.eq_ignore_ascii_case("cmd.exe"),
            "npm must not be wrapped in cmd.exe (Rust Command re-quoting breaks it)"
        );
        assert_eq!(args, vec!["--version".to_string()]);
    }

    #[test]
    fn resolved_npm_reports_version() {
        let Some((program, args)) = npm_program_and_args(&["--version".into()]) else {
            return;
        };
        let status = std::process::Command::new(program)
            .args(args)
            .status()
            .expect("spawn npm --version");
        assert!(status.success(), "npm --version failed: {status}");
    }

    #[cfg(windows)]
    #[test]
    fn extra_nodejs_dirs_include_program_files() {
        let previous = std::env::var_os("ProgramFiles");
        unsafe { std::env::set_var("ProgramFiles", r"C:\Program Files") };
        let dirs = extra_nodejs_dirs();
        match previous {
            Some(value) => unsafe { std::env::set_var("ProgramFiles", value) },
            None => unsafe { std::env::remove_var("ProgramFiles") },
        }
        assert!(
            dirs.iter()
                .any(|d| d.ends_with(Path::new(r"Program Files\nodejs"))),
            "{dirs:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn extra_nodejs_dirs_include_usr_bin_and_nvm() {
        let previous_nvm = std::env::var_os("NVM_DIR");
        unsafe { std::env::set_var("NVM_DIR", "/home/me/.nvm") };
        let dirs = extra_nodejs_dirs();
        match previous_nvm {
            Some(value) => unsafe { std::env::set_var("NVM_DIR", value) },
            None => unsafe { std::env::remove_var("NVM_DIR") },
        }
        assert!(dirs.iter().any(|d| d == Path::new("/usr/bin")), "{dirs:?}");
        assert!(
            dirs.iter().any(|d| d == Path::new("/usr/local/bin")),
            "{dirs:?}"
        );
        assert!(
            dirs.iter()
                .any(|d| d == Path::new("/home/me/.nvm/current/bin")),
            "{dirs:?}"
        );
    }
}
