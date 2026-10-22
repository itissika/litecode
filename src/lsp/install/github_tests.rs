//! Unit tests for the GitHub download pipeline.

#[cfg(test)]
mod tests {
    use super::super::{ArchiveFormat, DownloadInfo, github};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::sync::Mutex;

    /// Serialize tests that mutate `LITECODE_LSP_DIR` to avoid parallel env var races.
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn asset_pattern_no_match() {
        let entries = [(
            "gopls_windows_amd64.zip",
            "https://dl.example/gopls-win.zip",
        )];
        assert!(
            github::find_asset_url(&assets(&entries), "linux_amd64", ArchiveFormat::Zip, false)
                .is_err()
        );
    }

    #[test]
    fn asset_pattern_partial_match() {
        let entries = [
            ("clangd-linux-18.1.3.zip", "https://dl.example/clangd.zip"),
            ("clangd-mac-18.1.3.zip", "https://dl.example/clangd-mac.zip"),
        ];
        assert_eq!(
            github::find_asset_url(&assets(&entries), "linux", ArchiveFormat::Zip, false).unwrap(),
            "https://dl.example/clangd.zip"
        );
    }

    #[test]
    fn asset_match_excludes_checksum_for_binary_pattern() {
        let entries = [
            (
                "server-linux-x86_64.tar.gz.sha256",
                "https://dl.example/checksum",
            ),
            ("server-linux-x86_64.tar.gz", "https://dl.example/server"),
        ];
        assert_eq!(
            github::find_asset_url(
                &assets(&entries),
                "linux-x86_64",
                ArchiveFormat::TarGz,
                false
            )
            .unwrap(),
            "https://dl.example/server"
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

    #[test]
    fn github_digest_strips_prefix() {
        let hash = "a".repeat(64);
        assert_eq!(
            github::normalize_github_digest(&format!("sha256:{hash}")).as_deref(),
            Some(hash.as_str())
        );
        assert!(github::normalize_github_digest("not-a-hash").is_none());
    }

    #[test]
    fn archive_format_equality() {
        assert_eq!(ArchiveFormat::Gz, ArchiveFormat::Gz);
        assert_ne!(ArchiveFormat::Gz, ArchiveFormat::TarGz);
        assert_eq!(ArchiveFormat::Raw, ArchiveFormat::Raw);
    }

    #[test]
    fn archive_format_copy() {
        let fmt = ArchiveFormat::TarGz;
        let copied = fmt;
        assert_eq!(copied, ArchiveFormat::TarGz);
    }

    // -----------------------------------------------------------------------
    // SHA256 test vectors
    // -----------------------------------------------------------------------

    #[test]
    fn sha256_empty_string() {
        let mut hasher = Sha256::new();
        hasher.update(b"");
        assert_eq!(
            format!("{:x}", hasher.finalize()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_known_vector() {
        let mut hasher = Sha256::new();
        hasher.update(b"hello world");
        assert_eq!(
            format!("{:x}", hasher.finalize()),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn sha256_file_matches() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(b"hello world").unwrap();
        tmp.flush().unwrap();

        let digest = github::sha256_file(tmp.path()).unwrap();
        assert_eq!(
            digest,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn constant_time_eq_matches() {
        assert!(github::constant_time_eq(b"abc", b"abc"));
    }

    #[test]
    fn constant_time_eq_mismatch() {
        assert!(!github::constant_time_eq(b"abc", b"abd"));
    }

    #[test]
    fn constant_time_eq_different_length() {
        assert!(!github::constant_time_eq(b"abc", b"ab"));
    }

    // -----------------------------------------------------------------------
    // manifest.json read/write
    // -----------------------------------------------------------------------

    #[test]
    fn manifest_read_write_roundtrip() {
        let _lock = ENV_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("LITECODE_LSP_DIR", dir.path().to_string_lossy().as_ref());
        }

        // Initially empty.
        let m = github::read_manifest().unwrap();
        assert!(m.is_empty());

        // Write an entry.
        github::write_manifest_entry("rust-analyzer", "2024-12-16").unwrap();

        // Read back.
        let m = github::read_manifest().unwrap();
        assert_eq!(
            m.get("rust-analyzer").map(String::as_str),
            Some("2024-12-16")
        );

        // Write another entry.
        github::write_manifest_entry("gopls", "v0.18.0").unwrap();

        let m = github::read_manifest().unwrap();
        assert_eq!(
            m.get("rust-analyzer").map(String::as_str),
            Some("2024-12-16")
        );
        assert_eq!(m.get("gopls").map(String::as_str), Some("v0.18.0"));

        // Overwrite.
        github::write_manifest_entry("rust-analyzer", "2025-01-01").unwrap();
        let m = github::read_manifest().unwrap();
        assert_eq!(
            m.get("rust-analyzer").map(String::as_str),
            Some("2025-01-01")
        );
    }

    #[test]
    fn installed_version_none_for_unknown() {
        let _lock = ENV_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("LITECODE_LSP_DIR", dir.path().to_string_lossy().as_ref());
        }
        let v = github::installed_version("nonexistent").unwrap();
        assert!(v.is_none());
    }

    // -----------------------------------------------------------------------
    // DownloadInfo test
    // -----------------------------------------------------------------------

    #[test]
    fn download_info_clone() {
        let info = DownloadInfo {
            repo: "rust-lang/rust-analyzer".to_string(),
            asset_pattern: "x86_64-unknown-linux-gnu".to_string(),
            exact_asset: true,
            format: ArchiveFormat::Gz,
            unpack_as: Some("rust-analyzer".into()),
        };
        let cloned = info.clone();
        assert_eq!(info.repo, cloned.repo);
        assert_eq!(info.asset_pattern, cloned.asset_pattern);
        assert_eq!(info.format, cloned.format);
    }
}
