//! npm install --prefix pipeline for installing Node.js-based LSP servers.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::lsp::paths::lsp_dir;
use crate::types::{LitecodeError, Result};

/// Windows Node shims are `npm.cmd`, not a PE `npm.exe`. `CreateProcess("npm")`
/// therefore fails even when Node is installed (same trap as `ls_program_and_args`).
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

/// Program + argv that can be passed to `Command` (wraps `.cmd` via `cmd /C`).
pub fn npm_program_and_args(npm_args: &[String]) -> Option<(PathBuf, Vec<String>)> {
    let shim = resolve_npm_shim()?;
    Some(super::ls_program_and_args(&shim, npm_args))
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
    #[cfg(not(windows))]
    {
        Vec::new()
    }
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
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
        dirs
    }
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
        return Err(LitecodeError::Config(
            "未找到 npm 命令。请安装 Node.js 和 npm".into(),
        ));
    };
    let npm_ok = version_cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if !npm_ok {
        return Err(LitecodeError::Config(
            "未找到 npm 命令。请安装 Node.js 和 npm".into(),
        ));
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
        return Err(LitecodeError::Config(
            "未找到 npm 命令。请安装 Node.js 和 npm".into(),
        ));
    };
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = cmd.output().await.map_err(|e| {
        let _ = std::fs::remove_dir_all(&staging_dir);
        if e.kind() == std::io::ErrorKind::NotFound {
            LitecodeError::Config("未找到 npm 命令。请安装 Node.js 和 npm".into())
        } else {
            LitecodeError::Config(format!("npm install 失败（网络错误）。请检查网络连接: {e}"))
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(LitecodeError::Config(format!("npm install 失败: {stderr}")));
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

/// Locate `node` / `node.exe` on PATH (and Windows Node.js install dirs).
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

    #[cfg(windows)]
    #[test]
    fn cmd_shim_is_not_passed_to_create_process() {
        let (program, args) = super::super::ls_program_and_args(
            Path::new(r"C:\Program Files\nodejs\npm.cmd"),
            &["install".into(), "--prefix".into(), r"D:\tmp".into()],
        );
        assert_eq!(program, PathBuf::from("cmd"));
        assert_eq!(args[0], "/D");
        assert_eq!(args[1], "/S");
        assert_eq!(args[2], "/C");
        assert!(args[3].contains("npm.cmd"));
        assert!(args[3].contains("install"));
    }

    #[test]
    fn unix_npm_is_invoked_directly() {
        let (program, args) =
            super::super::ls_program_and_args(Path::new("/usr/bin/npm"), &["--version".into()]);
        assert_eq!(program, PathBuf::from("/usr/bin/npm"));
        assert_eq!(args, vec!["--version".to_string()]);
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
}
