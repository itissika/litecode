//! Adapter for typescript-language-server — TypeScript / JavaScript LSP via npm.

use std::path::Path;

use super::super::npm::resolve_node_binary;
use super::super::{DownloadInfo, InstallKind, LanguageServerBinary, LspAdapter};

pub struct TypeScriptAdapter;

pub(crate) const SERVER_ENTRY: &str = "node_modules/typescript-language-server/lib/cli.mjs";

impl LspAdapter for TypeScriptAdapter {
    fn server_id(&self) -> &'static str {
        "typescript-language-server"
    }

    fn install_kind(&self) -> InstallKind {
        InstallKind::Npm
    }

    fn download_info(&self) -> DownloadInfo {
        DownloadInfo::unused()
    }

    fn npm_packages(&self) -> &'static [(&'static str, &'static str)] {
        // npm `typescript@latest` is 7.x and no longer ships `tsserver` (verified
        // 2026-08-14: 7.0.2 bin is only `tsc`). TLS 5.3.0 still needs tsserver;
        // pin to 6.0.3 which publishes `bin/tsserver`.
        &[
            ("typescript", "6.0.3"),
            ("typescript-language-server", "latest"),
        ]
    }

    fn binary_info(&self, server_dir: &Path) -> LanguageServerBinary {
        let node = resolve_node_binary().unwrap_or_else(|| Path::new("node").to_path_buf());
        let entry = server_dir.join(SERVER_ENTRY);
        LanguageServerBinary {
            path: node,
            arguments: vec![entry.to_string_lossy().into_owned(), "--stdio".to_string()],
            env: None,
        }
    }
}

pub fn adapter() -> Box<dyn LspAdapter> {
    Box::new(TypeScriptAdapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_typescript_below_7() {
        let pkgs = adapter().npm_packages();
        assert_eq!(pkgs[0], ("typescript", "6.0.3"));
        assert_eq!(pkgs[1], ("typescript-language-server", "latest"));
    }
}
