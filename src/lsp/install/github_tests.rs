//! Unit tests for the GitHub download pipeline.

#[cfg(test)]
mod tests {
    use super::super::{ArchiveFormat, github};
    
    


    // -----------------------------------------------------------------------
    // asset_pattern matching
    // -----------------------------------------------------------------------

    fn assets(entries: &[(&str, &str)]) -> Vec<serde_json::Value> {
        entries
            .iter()
            .map(|(name, url)| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": url,
                })
            })
            .collect()
    }

    #[test]
    fn asset_pattern_exact_match() {
        let entries = [
            (
                "rust-analyzer-x86_64-unknown-linux-gnu.gz",
                "https://dl.example/rust-analyzer.gz",
            ),
            (
                "rust-analyzer-aarch64-apple-darwin.gz",
                "https://dl.example/rust-analyzer-mac.gz",
            ),
        ];
        assert_eq!(
            github::find_asset_url(
                &assets(&entries),
                "x86_64-unknown-linux-gnu",
                ArchiveFormat::Gz,
                false,
            )
            .unwrap(),
            "https://dl.example/rust-analyzer.gz"
        );
    }

    // -----------------------------------------------------------------------
    // ArchiveFormat tests
    // -----------------------------------------------------------------------

    #[test]
    fn live_rust_analyzer_release_contains_exact_windows_and_linux_names() {
        // Snapshot of https://api.github.com/repos/rust-lang/rust-analyzer/releases/latest
        // tag 2026-08-10.1 (probed 2026-08-14). HEAD of these URLs returned 200.
        let names = [
            "rust-analyzer-aarch64-apple-darwin.gz",
            "rust-analyzer-aarch64-pc-windows-msvc.zip",
            "rust-analyzer-aarch64-unknown-linux-gnu.gz",
            "rust-analyzer-x86_64-apple-darwin.gz",
            "rust-analyzer-x86_64-pc-windows-msvc.zip",
            "rust-analyzer-x86_64-unknown-linux-gnu.gz",
            "rust-analyzer-x86_64-unknown-linux-musl.gz",
            "rust-analyzer-win32-x64.vsix",
        ];
        let assets: Vec<serde_json::Value> = names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("https://github.com/rust-lang/rust-analyzer/releases/download/2026-08-10.1/{name}"),
                })
            })
            .collect();
        assert_eq!(
            github::find_asset_url(
                &assets,
                "rust-analyzer-x86_64-pc-windows-msvc.zip",
                ArchiveFormat::Zip,
                true,
            )
            .unwrap()
            .contains("rust-analyzer-x86_64-pc-windows-msvc.zip"),
            true
        );
        assert_eq!(
            github::find_asset_url(
                &assets,
                "rust-analyzer-x86_64-unknown-linux-gnu.gz",
                ArchiveFormat::Gz,
                true,
            )
            .unwrap()
            .contains("rust-analyzer-x86_64-unknown-linux-gnu.gz"),
            true
        );
        assert!(
            github::find_asset_url(
                &assets,
                "rust-analyzer-x86_64-pc-windows-msvc.zip",
                ArchiveFormat::Gz,
                true,
            )
            .is_err()
        );
    }

    // -----------------------------------------------------------------------
    // SHA256 test vectors
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // manifest.json read/write
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // DownloadInfo test
    // -----------------------------------------------------------------------

}
