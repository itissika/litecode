//! Adapter for pyright — Python type checker / LSP via npm.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::super::npm::resolve_node_binary;
use super::super::{DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct PyrightAdapter;

pub(crate) const SERVER_ENTRY: &str = "node_modules/pyright/langserver.index.js";
pub(crate) const CLI_ENTRY: &str = "node_modules/pyright/index.js";
pub(crate) const DIST_ENTRY: &str = "node_modules/pyright/dist/pyright-langserver.js";

impl LspAdapter for PyrightAdapter {
    fn server_id(&self) -> &'static str {
        "pyright-langserver"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::Npm
    }

    fn download_info(&self) -> DownloadInfo {
        DownloadInfo::unused()
    }

    fn npm_packages(&self) -> &'static [(&'static str, &'static str)] {
        &[("pyright", "latest")]
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        let node = resolve_node_binary().unwrap_or_else(|| Path::new("node").to_path_buf());
        let mut env = HashMap::new();
        if let Ok(python_path) = std::env::var("LITECODE_PYTHON_PATH") {
            env.insert("LITECODE_PYTHON_PATH".to_string(), python_path);
        }
        let entry = server_dir.join(SERVER_ENTRY);
        LanguageServerBinary {
            path: node,
            arguments: vec![entry.to_string_lossy().into_owned(), "--stdio".to_string()],
            env: Some(env),
        }
    }

    fn verify_binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        // pyright-langserver has no --version; it only speaks LSP over --stdio.
        // The CLI entry (`index.js --version`) is the package health check.
        let node = resolve_node_binary().unwrap_or_else(|| Path::new("node").to_path_buf());
        LanguageServerBinary {
            path: node,
            arguments: vec![server_dir.join(CLI_ENTRY).to_string_lossy().into_owned()],
            env: None,
        }
    }

    fn extra_managed_files(&self, server_dir: &Path) -> Vec<PathBuf> {
        vec![server_dir.join(DIST_ENTRY)]
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(PyrightAdapter)
}

#[cfg(test)]
mod tests {
    

}
