//! npm install --prefix pipeline for installing Node.js-based LSP servers.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::lsp::paths::lsp_dir;
use crate::types::{LitecodeError, Result};

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

    let npm_ok = Command::new("npm")
        .arg("--version")
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
    let mut cmd = Command::new("npm");
    cmd.arg("install")
        .arg("--prefix")
        .arg(&staging_dir)
        .args(&specs)
        .stdout(std::process::Stdio::piped())
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

/// Locate `node` / `node.exe` on PATH.
pub fn resolve_node_binary() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
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
