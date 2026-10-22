//! Unit tests for the GitHub download pipeline.

#[cfg(test)]
mod tests {
    use super::super::{ArchiveFormat, github};
    use sha2::Digest;
    
    

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

    // -----------------------------------------------------------------------
    // ArchiveFormat tests
    // -----------------------------------------------------------------------

    #[test]
    fn asset_exact_name_skips_vsix() {
        let entries = [
            ("rust-analyzer-win32-x64.vsix", "https://dl.example/vsix"),
            (
                "rust-analyzer-x86_64-pc-windows-msvc.zip",
                "https://dl.example/zip",
            ),
        ];
        assert_eq!(
            github::find_asset_url(
                &assets(&entries),
                "rust-analyzer-x86_64-pc-windows-msvc.zip",
                ArchiveFormat::Zip,
                true,
            )
            .unwrap(),
            "https://dl.example/zip"
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
