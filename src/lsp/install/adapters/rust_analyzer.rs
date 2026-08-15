//! Adapter for rust-analyzer — the official Rust language server.

use std::path::Path;

use super::super::{ArchiveFormat, DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct RustAnalyzerAdapter;

impl LspAdapter for RustAnalyzerAdapter {
    fn server_id(&self) -> &'static str {
        "rust-analyzer"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::Github
    }

    fn download_info(&self) -> DownloadInfo {
        let (asset_pattern, format, unpack_as) = github_asset_spec();
        DownloadInfo {
            repo: "rust-lang/rust-analyzer".to_string(),
            asset_pattern,
            exact_asset: true,
            format,
            unpack_as: Some(unpack_as),
        }
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        LanguageServerBinary {
            path: server_dir.join(binary_file_name()),
            arguments: vec![],
            env: None,
        }
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(RustAnalyzerAdapter)
}

pub(crate) fn binary_file_name() -> &'static str {
    if cfg!(windows) {
        "rust-analyzer.exe"
    } else {
        "rust-analyzer"
    }
}

pub(crate) fn github_asset_spec() -> (String, ArchiveFormat, String) {
    let triple = target_triple();
    let (ext, format) = if cfg!(windows) {
        ("zip", ArchiveFormat::Zip)
    } else {
        ("gz", ArchiveFormat::Gz)
    };
    (
        format!("rust-analyzer-{triple}.{ext}"),
        format,
        binary_file_name().to_string(),
    )
}

fn target_triple() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => format!("x86_64-unknown-linux-{}", linux_libc()),
        ("linux", "aarch64") => format!("aarch64-unknown-linux-{}", linux_libc()),
        ("macos", "x86_64") => "x86_64-apple-darwin".to_string(),
        ("macos", "aarch64") => "aarch64-apple-darwin".to_string(),
        ("windows", "x86_64") => "x86_64-pc-windows-msvc".to_string(),
        ("windows", "aarch64") => "aarch64-pc-windows-msvc".to_string(),
        (os, arch) => format!("unsupported-{os}-{arch}"),
    }
}

#[cfg(not(target_os = "linux"))]
fn linux_libc() -> &'static str {
    "gnu"
}

#[cfg(target_os = "linux")]
fn linux_libc() -> &'static str {
    if ldd_reports_musl() || lib_dir_has_musl() {
        "musl"
    } else {
        "gnu"
    }
}

#[cfg(target_os = "linux")]
fn ldd_reports_musl() -> bool {
    use std::process::Command;
    let output = Command::new("ldd").arg("--version").output();
    let Ok(output) = output else {
        return false;
    };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.contains("musl")
}

#[cfg(target_os = "linux")]
fn lib_dir_has_musl() -> bool {
    let Ok(entries) = std::fs::read_dir("/lib") else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().starts_with("ld-musl-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::install::ArchiveFormat;

    #[test]
    fn github_asset_name_is_exact_filename() {
        let (name, format, unpack) = github_asset_spec();
        assert!(name.starts_with("rust-analyzer-"), "{name}");
        assert!(!name.contains("unsupported-"), "unknown platform: {name}");
        if cfg!(windows) {
            assert!(name.ends_with(".zip"), "{name}");
            assert_eq!(format, ArchiveFormat::Zip);
            assert_eq!(unpack, "rust-analyzer.exe");
            assert!(
                name.contains("pc-windows-msvc"),
                "windows asset should be msvc: {name}"
            );
        } else {
            assert!(name.ends_with(".gz"), "{name}");
            assert_eq!(format, ArchiveFormat::Gz);
            assert_eq!(unpack, "rust-analyzer");
        }
    }

    #[test]
    fn binary_info_points_at_unpacked_name() {
        let dir = Path::new("/tmp/rust-analyzer");
        let binary = adapter().binary_info(dir);
        assert_eq!(binary.path, dir.join(binary_file_name()));
        assert!(binary.arguments.is_empty());
    }
}
