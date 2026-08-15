//! Adapter for clangd — the official C/C++ language server based on LLVM.

use std::path::Path;

use super::super::{ArchiveFormat, DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct ClangdAdapter;

impl LspAdapter for ClangdAdapter {
    fn server_id(&self) -> &'static str {
        "clangd"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::Github
    }

    fn download_info(&self) -> DownloadInfo {
        let asset_pattern = detect_asset_pattern();
        DownloadInfo {
            repo: "clangd/clangd".to_string(),
            asset_pattern,
            exact_asset: false,
            format: ArchiveFormat::Zip,
            unpack_as: None,
        }
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        let exe_name = if cfg!(windows) {
            "clangd.exe"
        } else {
            "clangd"
        };
        LanguageServerBinary {
            path: server_dir.join("bin").join(exe_name),
            arguments: vec!["--background-index".to_string(), "-j=4".to_string()],
            env: None,
        }
    }

    fn auto_installable(&self) -> bool {
        false
    }

    fn install_hint(&self) -> Option<&'static str> {
        Some(
            "clangd 需要较新的系统库（Linux 上需要 glibc 2.18+）。\
             推荐通过系统包管理器安装，例如: sudo apt install clangd。\
             安装后 litecode 会自动发现系统路径中的 clangd。",
        )
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(ClangdAdapter)
}

fn detect_asset_pattern() -> String {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-22",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", _) => "darwin-apple",
        ("windows", _) => "windows",
        _ => {
            tracing::warn!(
                os = std::env::consts::OS,
                arch = std::env::consts::ARCH,
                "unknown platform for clangd, using linux fallback"
            );
            "linux-22"
        }
    };
    format!("clangd-{platform}")
}
