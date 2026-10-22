//! Adapter for gopls — the official Go language server.

use std::path::Path;

use super::super::{DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct GoplsAdapter;

impl LspAdapter for GoplsAdapter {
    fn server_id(&self) -> &'static str {
        "gopls"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::Go
    }

    fn download_info(&self) -> DownloadInfo {
        DownloadInfo::unused()
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        let exe_name = if cfg!(windows) { "gopls.exe" } else { "gopls" };
        LanguageServerBinary {
            path: server_dir.join(exe_name),
            arguments: vec!["-mode=stdio".to_string()],
            env: None,
        }
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(GoplsAdapter)
}

#[cfg(test)]
mod tests {
    

}
