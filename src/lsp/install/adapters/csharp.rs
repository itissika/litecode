//! Adapter for csharp-ls — the C# language server.

use std::path::Path;

use super::super::{DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct CSharpAdapter;

impl LspAdapter for CSharpAdapter {
    fn server_id(&self) -> &'static str {
        "csharp-ls"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::DotnetTool
    }

    fn download_info(&self) -> DownloadInfo {
        DownloadInfo::unused()
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        let exe_name = if cfg!(windows) {
            "csharp-ls.exe"
        } else {
            "csharp-ls"
        };
        LanguageServerBinary {
            path: server_dir.join(exe_name),
            arguments: vec![],
            env: None,
        }
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(CSharpAdapter)
}

#[cfg(test)]
mod tests {}
