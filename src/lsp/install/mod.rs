//! LSP server installation management — adapter trait, download types, and registry.

pub mod adapters;
pub mod github;
pub mod npm;

#[cfg(test)]
pub mod github_tests;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use crate::lsp::paths::lsp_dir;
use crate::types::{LitecodeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Github,
    Npm,
    Go,
    DotnetTool,
}

#[derive(Debug, Clone)]
pub struct DownloadInfo {
    pub repo: String,
    pub asset_pattern: String,
    /// When true, `asset_pattern` is the full GitHub asset file name.
    pub exact_asset: bool,
    pub format: ArchiveFormat,
    /// After unpack, place/rename the executable to this file name in `server_dir`.
    pub unpack_as: Option<String>,
}

impl DownloadInfo {
    pub fn unused() -> Self {
        Self {
            repo: String::new(),
            asset_pattern: String::new(),
            exact_asset: false,
            format: ArchiveFormat::Raw,
            unpack_as: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Gz,
    TarGz,
    Zip,
    Raw,
}

#[derive(Debug, Clone)]
pub struct LanguageServerBinary {
    pub path: PathBuf,
    pub arguments: Vec<String>,
    pub env: Option<HashMap<String, String>>,
}

pub type DownloadProgress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

static INSTALL_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

pub(crate) fn install_lock() -> &'static tokio::sync::Mutex<()> {
    INSTALL_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Windows `.cmd` shims cannot be passed to `CreateProcess`. Return `cmd /C`.
pub fn ls_program_and_args(path: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    #[cfg(windows)]
    {
        let is_cmd = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"));
        if is_cmd {
            let mut inner = format!("\"{}\"", path.display());
            for arg in args {
                inner.push(' ');
                if arg.contains(char::is_whitespace) {
                    inner.push('"');
                    inner.push_str(arg);
                    inner.push('"');
                } else {
                    inner.push_str(arg);
                }
            }
            return (
                PathBuf::from("cmd"),
                vec![
                    "/D".into(),
                    "/S".into(),
                    "/C".into(),
                    format!("\"{inner}\""),
                ],
            );
        }
    }
    (path.to_path_buf(), args.to_vec())
}

/// Replace a verified staging directory without deleting a previously working
/// install first. This matters on Windows where a rename can fail while an
/// LSP process still has the old executable open.
pub(crate) fn replace_install_dir(staging: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().unwrap_or(Path::new("."));
    let name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("language-server");
    let backup = parent.join(format!(".{name}.backup-{}", uuid::Uuid::new_v4()));
    let had_previous = dest.exists();

    if had_previous {
        std::fs::rename(dest, &backup).map_err(|e| {
            LitecodeError::Config(format!(
                "move existing install {} aside before replacement: {e}",
                dest.display()
            ))
        })?;
    }
    if let Err(e) = std::fs::rename(staging, dest) {
        if had_previous {
            let _ = std::fs::rename(&backup, dest);
        }
        return Err(LitecodeError::Config(format!(
            "activate staged install {} -> {}: {e}",
            staging.display(),
            dest.display()
        )));
    }
    if had_previous {
        let _ = std::fs::remove_dir_all(backup);
    }
    Ok(())
}

pub trait LspAdapter: Send + Sync {
    fn server_id(&self) -> &'static str;
    fn install_kind(&self) -> InstallKind;
    fn download_info(&self) -> DownloadInfo;
    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary;
    /// Binary used for `--version` after install. Defaults to [`Self::binary_info`].
    /// Override when the LSP entrypoint does not accept `--version` (e.g. pyright-langserver).
    fn verify_binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        self.binary_info(server_dir)
    }
    /// Extra files that must exist for a managed install to be considered complete.
    fn extra_managed_files(&self, _server_dir: &Path) -> Vec<PathBuf> {
        Vec::new()
    }
    /// npm packages to install as `name@version` pairs. Last package is the server.
    fn npm_packages(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// Whether this server can be auto-installed by litecode.
    /// Servers that require system packages (e.g. glibc for clangd) return false.
    fn auto_installable(&self) -> bool {
        true
    }
    /// Optional hint shown when auto-install is not available.
    fn install_hint(&self) -> Option<&'static str> {
        None
    }
}

/// Registry of all known LSP adapters.
pub fn adapters() -> Vec<Box<dyn LspAdapter>> {
    vec![
        adapters::rust_analyzer::adapter(),
        adapters::gopls::adapter(),
        adapters::clangd::adapter(),
        adapters::csharp::adapter(),
        adapters::typescript::adapter(),
        adapters::pyright::adapter(),
    ]
}

/// Install a language server by id into the managed lsp directory.
pub async fn install_server_to_lsp_dir(
    server_id: &str,
    progress: Option<DownloadProgress>,
) -> Result<()> {
    let _install_guard = install_lock().lock().await;
    let adps = adapters();
    let adapter = adps
        .iter()
        .find(|a| a.server_id() == server_id)
        .ok_or_else(|| LitecodeError::Config(format!("unknown language server id: {server_id}")))?;

    if !adapter.auto_installable() {
        let hint = adapter.install_hint().unwrap_or("install it via your system package manager");
        return Err(LitecodeError::Config(format!(
            "{server_id} does not support automatic install. {hint}"
        )));
    }

    let dest_dir = lsp_dir()?.join(server_id);

    // A verified managed binary is an offline-safe install. Do not replace
    // it merely because the user pressed Install again.
    if crate::lsp::deps::verify_managed_server(server_id).is_ok() {
        if github::installed_version(server_id)
            .ok()
            .flatten()
            .is_none()
            && let Some(version) = npm::managed_version(&dest_dir)
        {
            github::write_manifest_entry(server_id, &version)?;
        }
        tracing::info!(
            server_id,
            "verified managed LSP already available, skipping download"
        );
        return Ok(());
    }

    match adapter.install_kind() {
        InstallKind::Npm => {
            let packages = adapter.npm_packages();
            if packages.is_empty() {
                return Err(LitecodeError::Config(format!(
                    "{server_id} is npm-installable but declared no packages"
                )));
            }
            npm::npm_install(server_id, packages).await
        }
        InstallKind::Go => install_gopls(server_id, &dest_dir).await,
        InstallKind::DotnetTool => install_csharp_ls(server_id, &dest_dir).await,
        InstallKind::Github => {
            let info = adapter.download_info();
            github::download_from_github(&info, "latest", &dest_dir, progress).await
        }
    }
}

async fn install_gopls(server_id: &str, dest_dir: &Path) -> Result<()> {
    let go_ok = tokio::process::Command::new("go")
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if !go_ok {
        return Err(LitecodeError::Config(
            "go was not found. Install Go, ensure it is on PATH, then retry gopls install.".into(),
        ));
    }

    let parent = dest_dir.parent().unwrap_or(Path::new("."));
    let staging = parent.join(format!(".{server_id}.staging-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging)?;
    let gobin = staging.to_string_lossy().to_string();
    let output = match tokio::process::Command::new("go")
        .args(["install", "golang.org/x/tools/gopls@latest"])
        .env("GOBIN", &gobin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(LitecodeError::Config(format!(
                "failed to run go install: {e}"
            )));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(LitecodeError::Config(format!(
            "install gopls failed: {stderr}"
        )));
    }
    if let Err(e) = crate::lsp::deps::verify_managed_server_at(server_id, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(LitecodeError::Config(format!(
            "installed gopls, but executable verification failed: {e}"
        )));
    }
    npm::write_managed_meta(&staging, "latest")?;
    replace_install_dir(&staging, dest_dir)?;
    github::write_manifest_entry(server_id, "latest")?;
    Ok(())
}

async fn install_csharp_ls(server_id: &str, dest_dir: &Path) -> Result<()> {
    crate::lsp::deps::ensure_dotnet_sdk()?;

    let parent = dest_dir.parent().unwrap_or(Path::new("."));
    let staging = parent.join(format!(".{server_id}.staging-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging)?;
    let tool_path = staging.to_string_lossy().to_string();
    let output = match tokio::process::Command::new("dotnet")
        .args(["tool", "install", "csharp-ls", "--tool-path", &tool_path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(LitecodeError::Config(format!(
                "failed to run dotnet tool install: {e}"
            )));
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(LitecodeError::Config(format!(
            "install csharp-ls failed: {stderr}"
        )));
    }
    if let Err(e) = crate::lsp::deps::verify_managed_server_at(server_id, &staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(LitecodeError::Config(format!(
            "installed csharp-ls, but executable verification failed: {e}"
        )));
    }
    npm::write_managed_meta(&staging, "latest")?;
    replace_install_dir(&staging, dest_dir)?;
    github::write_manifest_entry(server_id, "latest")?;
    Ok(())
}

/// Check whether a language server has been installed (present in manifest.json).
pub fn check_installed(server_id: &str) -> Result<bool> {
    match github::installed_version(server_id) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => {
            // Distinguish "not found" (no manifest or dir) from real errors.
            let msg = e.to_string();
            if msg.contains("cannot determine home directory") || msg.contains("manifest") {
                tracing::warn!(server_id, error = %e, "check_installed error, assuming not installed");
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}
