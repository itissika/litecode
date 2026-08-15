//! GitHub Release download pipeline — multi-source fallback, retry, checksum, and atomic install.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use tar::Archive;
use tokio::time::sleep;
use zip::ZipArchive;

use crate::lsp::paths::lsp_dir;
use crate::types::{LitecodeError, Result};

use super::{ArchiveFormat, DownloadInfo, DownloadProgress};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Download a GitHub release asset, verify its checksum, unpack it, and
/// record the installation in `manifest.json`.
pub async fn download_from_github(
    info: &DownloadInfo,
    version: &str,
    dest_dir: &Path,
    progress: Option<DownloadProgress>,
) -> Result<()> {
    let server_id = dest_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("language-server");
    tracing::info!(server_id, version, dest = %dest_dir.display(), "download_from_github start");

    // Resolve the real version string from GitHub (or use the caller-supplied one).
    let (resolved_version, asset_url, api_digest) = resolve_asset_url(info, version).await?;

    // Prefer GitHub's asset digest; fall back to a sibling `.sha256` file.
    let expected_sha256 = match api_digest {
        Some(digest) => Some(digest),
        None => fetch_sha256(&asset_url).await,
    };

    // Download + verify + unpack.
    let tmp_dir = create_temp_dir()?;
    let downloaded_path = match download_with_retry(&asset_url, &tmp_dir, info, progress).await {
        Ok(path) => path,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(e);
        }
    };

    // SHA256 check.
    if let Some(ref expected) = expected_sha256 {
        let actual = match sha256_file(&downloaded_path) {
            Ok(value) => value,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(e);
            }
        };
        if !constant_time_eq(expected.as_bytes(), actual.as_bytes()) {
            let _ = std::fs::remove_file(&downloaded_path);
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return Err(LitecodeError::Config(format!(
                "SHA256 校验失败。期望: {expected}，实际: {actual}。文件可能被篡改，已删除"
            )));
        }
        tracing::info!(server_id, sha256 = %expected, "sha256 verified");
    } else {
        tracing::warn!(
            server_id,
            "no .sha256 file found, skipping checksum verification"
        );
    }

    // Unpack into a unique sibling directory. The live install remains
    // untouched until extraction and executable verification both succeed.
    let parent = dest_dir.parent().unwrap_or(Path::new("."));
    let staging = parent.join(format!(".{}.staging-{}", server_id, uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging).map_err(|e| {
        LitecodeError::Config(format!("create staging dir {}: {e}", staging.display()))
    })?;
    if let Err(e) = unpack(
        &downloaded_path,
        &staging,
        info.format,
        info.unpack_as.as_deref(),
    ) {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }

    if let Err(e) = crate::lsp::deps::verify_managed_server_at(server_id, &staging) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(LitecodeError::Config(format!(
            "downloaded {server_id}, but its executable failed verification: {e}"
        )));
    }

    // Metadata is part of the staged install: a directory becomes live only
    // when both its binary and its version record are ready.
    if let Err(e) = write_meta(&staging, &resolved_version, expected_sha256.as_deref()) {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }
    if let Err(e) = super::replace_install_dir(&staging, dest_dir) {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&tmp_dir);

    // Update manifest.
    write_manifest_entry(server_id, &resolved_version)?;

    tracing::info!(server_id, version = %resolved_version, "install complete");
    Ok(())
}

fn create_temp_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("litecode_lsp_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| LitecodeError::Config(format!("create temp dir {}: {e}", dir.display())))?;
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Version resolution & asset URL
// ---------------------------------------------------------------------------

async fn resolve_asset_url(
    info: &DownloadInfo,
    version: &str,
) -> Result<(String, String, Option<String>)> {
    let sources = build_source_list();
    let source_names: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
    let mut errors: Vec<String> = Vec::new();

    for source in &sources {
        match fetch_release_assets(source, info, version).await {
            Ok((tag, url, digest)) => return Ok((tag, url, digest)),
            Err(e) => {
                let msg = format!("{source}: {e}");
                tracing::warn!("{msg}");
                errors.push(msg);
            }
        }
    }

    Err(LitecodeError::Config(format!(
        "{} 下载失败：已尝试 [{}] 均不可达。错误详情: {}。请手动安装：访问 https://github.com/{}/releases",
        &info.repo[info.repo.rfind('/').map(|i| i + 1).unwrap_or(0)..],
        source_names.join(", "),
        errors.join("; "),
        info.repo,
    )))
}

async fn fetch_release_assets(
    source: &Source,
    info: &DownloadInfo,
    version: &str,
) -> Result<(String, String, Option<String>)> {
    let api_url = if version == "latest" {
        format!("https://api.github.com/repos/{}/releases/latest", info.repo)
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/tags/{version}",
            info.repo
        )
    };

    match source {
        Source::GitHubApi { token } => {
            return fetch_github_api(&api_url, token.as_deref(), info).await;
        }
        Source::Mirror { base } => {
            let mirror_api = format!("{base}{api_url}");
            return fetch_github_api(&mirror_api, None, info).await;
        }
    }
}

async fn fetch_github_api(
    url: &str,
    token: Option<&str>,
    info: &DownloadInfo,
) -> Result<(String, String, Option<String>)> {
    let client = build_http_client();
    let mut req = client
        .get(url)
        .header("User-Agent", "litecode-lsp/0.1")
        .header("Accept", "application/vnd.github+json");

    if let Some(tok) = token {
        req = req.header("Authorization", format!("Bearer {tok}"));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| classify_reqwest_error(e, url))?;
    check_github_rate_limit(&resp)?;

    if !resp.status().is_success() {
        return Err(LitecodeError::Config(format!(
            "GitHub API 返回 {}: {}",
            resp.status().as_u16(),
            url
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| LitecodeError::Config(format!("解析 GitHub API 响应失败: {e}")))?;

    let tag = body["tag_name"].as_str().unwrap_or("latest").to_string();
    let assets = body["assets"]
        .as_array()
        .ok_or_else(|| LitecodeError::Config(format!("GitHub release {} 没有 assets 数组", tag)))?;

    let (asset_url, digest) =
        find_asset(assets, &info.asset_pattern, info.format, info.exact_asset)?;
    Ok((tag, asset_url, digest))
}

#[cfg(test)]
pub(crate) fn find_asset_url(
    assets: &[serde_json::Value],
    pattern: &str,
    format: ArchiveFormat,
    exact: bool,
) -> Result<String> {
    find_asset(assets, pattern, format, exact).map(|(url, _)| url)
}

pub(crate) fn find_asset(
    assets: &[serde_json::Value],
    pattern: &str,
    format: ArchiveFormat,
    exact: bool,
) -> Result<(String, Option<String>)> {
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        let name_ok = if exact {
            name == pattern
        } else {
            name.contains(pattern)
        };
        if name_ok && asset_matches_format(name, format) {
            let url = asset["browser_download_url"]
                .as_str()
                .ok_or_else(|| LitecodeError::Config("asset 缺少 browser_download_url".into()))?;
            let digest = asset["digest"].as_str().and_then(normalize_github_digest);
            return Ok((url.to_string(), digest));
        }
    }
    Err(LitecodeError::Config(format!(
        "未找到匹配 '{pattern}' 的 asset"
    )))
}

pub(crate) fn normalize_github_digest(raw: &str) -> Option<String> {
    let hash = raw
        .strip_prefix("sha256:")
        .unwrap_or(raw)
        .trim()
        .to_lowercase();
    (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

fn asset_matches_format(name: &str, format: ArchiveFormat) -> bool {
    let lower = name.to_ascii_lowercase();
    if [".sha256", ".sha512", ".sig", ".asc", ".txt"]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return false;
    }
    match format {
        ArchiveFormat::Gz => lower.ends_with(".gz") && !lower.ends_with(".tar.gz"),
        ArchiveFormat::TarGz => lower.ends_with(".tar.gz") || lower.ends_with(".tgz"),
        ArchiveFormat::Zip => lower.ends_with(".zip"),
        ArchiveFormat::Raw => true,
    }
}

// ---------------------------------------------------------------------------
// SHA256 fetch
// ---------------------------------------------------------------------------

async fn fetch_sha256(asset_url: &str) -> Option<String> {
    let sha256_url = format!("{asset_url}.sha256");
    let client = build_http_client();

    let resp = match client
        .get(&sha256_url)
        .header("User-Agent", "litecode-lsp/0.1")
        .timeout(Duration::from_secs(15))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!(url = %sha256_url, "failed to fetch .sha256");
            return None;
        }
    };

    if !resp.status().is_success() {
        return None;
    }

    let text = match resp.text().await {
        Ok(t) => t,
        Err(_) => return None,
    };

    // The sha256 file typically contains "<hash>  <filename>" or just "<hash>".
    let hash = text
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();

    if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hash)
    } else {
        tracing::warn!(sha256_url, content = %text, "unexpected .sha256 format");
        None
    }
}

// ---------------------------------------------------------------------------
// Download with retry + multi-source fallback
// ---------------------------------------------------------------------------

async fn download_with_retry(
    asset_url: &str,
    tmp_dir: &Path,
    info: &DownloadInfo,
    progress: Option<DownloadProgress>,
) -> Result<PathBuf> {
    let mirror_base = std::env::var("LITECODE_GITHUB_PROXY").unwrap_or_default();

    // Try original URL with retry.
    let first_err = match download_single_with_retry(asset_url, tmp_dir, progress.clone()).await {
        Ok(path) => return Ok(path),
        Err(e) => {
            let msg = format!("{asset_url}: {e}");
            tracing::warn!("{msg}");
            msg
        }
    };

    // Try mirror only if proxy is configured.
    if !mirror_base.is_empty() {
        let mirror_full = format!("{mirror_base}{asset_url}");

        let last_err =
            match download_single_with_retry(&mirror_full, tmp_dir, progress.clone()).await {
                Ok(path) => return Ok(path),
                Err(e) => {
                    let msg = format!("mirror {mirror_full}: {e}");
                    tracing::warn!("{msg}");
                    msg
                }
            };

        return Err(LitecodeError::Config(format!(
            "{} 下载失败：原始源错误: {first_err}，镜像错误: {last_err}。请手动安装：访问 https://github.com/{}/releases",
            info.repo, info.repo,
        )));
    }

    Err(LitecodeError::Config(format!(
        "{} 下载失败: {first_err}。请手动安装：访问 https://github.com/{}/releases",
        info.repo, info.repo,
    )))
}

async fn download_single_with_retry(
    url: &str,
    tmp_dir: &Path,
    progress: Option<DownloadProgress>,
) -> Result<PathBuf> {
    let max_retries: u32 = 3;
    let mut last_err: Option<String> = None;

    for attempt in 0..max_retries {
        match download_to_file(url, tmp_dir, progress.clone()).await {
            Ok(path) => return Ok(path),
            Err(e) => {
                let msg = e.to_string();
                // Do not retry on 4xx client errors (403, 404, etc.) — they are permanent.
                if msg.contains("HTTP 4") || msg.contains("限流") || msg.contains("未找到") {
                    return Err(e);
                }
                tracing::warn!(url, attempt, "download attempt failed");
                last_err = Some(msg);
                if attempt < max_retries - 1 {
                    let delay = Duration::from_secs(2u64.saturating_pow(attempt));
                    sleep(delay).await;
                }
            }
        }
    }

    Err(LitecodeError::Config(format!(
        "GitHub API 连接超时 (15s)，已重试 3 次。可设置 LITECODE_GITHUB_PROXY 使用镜像: {}",
        last_err.unwrap_or_default()
    )))
}

async fn download_to_file(
    url: &str,
    tmp_dir: &Path,
    progress: Option<DownloadProgress>,
) -> Result<PathBuf> {
    let client = build_http_client();
    let resp = client
        .get(url)
        .header("User-Agent", "litecode-lsp/0.1")
        .timeout(Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| classify_reqwest_error(e, url))?;

    let status = resp.status();
    if status == StatusCode::FORBIDDEN {
        return Err(LitecodeError::Config(
            "GitHub API 限流 (403)，剩余 0/60 次。可设置 GITHUB_TOKEN 提高限额".into(),
        ));
    }
    if status == StatusCode::NOT_FOUND {
        return Err(LitecodeError::Config(format!(
            "未找到对应版本，link: {url}"
        )));
    }
    if !status.is_success() {
        return Err(LitecodeError::Config(format!(
            "下载失败: HTTP {status}, link: {url}"
        )));
    }

    // Determine filename from URL.
    let filename = url
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("download");

    let dest = tmp_dir.join(filename);
    let mut file = std::fs::File::create(&dest)
        .map_err(|e| LitecodeError::Config(format!("create temp file {}: {e}", dest.display())))?;

    let total = resp.content_length();
    let mut downloaded: u64 = 0;
    if let Some(callback) = &progress {
        callback(0, total);
    }
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            let msg = classify_reqwest_error(e, url);
            if let Some(total) = total {
                let mb = |n: u64| n as f64 / 1_048_576.0;
                LitecodeError::Config(format!(
                    "下载中断 (已传输 {:.1}/{:.1} MB)。网络不稳定，请重试或手动安装",
                    mb(downloaded),
                    mb(total)
                ))
            } else {
                msg
            }
        })?;
        file.write_all(&chunk)
            .map_err(|e| LitecodeError::Config(format!("write download file: {e}")))?;
        downloaded += chunk.len() as u64;
        if let Some(callback) = &progress {
            callback(downloaded, total);
        }
    }

    file.flush()
        .map_err(|e| LitecodeError::Config(format!("flush download file: {e}")))?;

    Ok(dest)
}

// ---------------------------------------------------------------------------
// SHA256 file hashing
// ---------------------------------------------------------------------------

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| LitecodeError::Config(format!("open for sha256 {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| LitecodeError::Config(format!("read for sha256: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Unpack
// ---------------------------------------------------------------------------

fn unpack(
    archive_path: &Path,
    dest_dir: &Path,
    format: ArchiveFormat,
    unpack_as: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(dest_dir).map_err(|e| {
        LitecodeError::Config(format!("create unpack dir {}: {e}", dest_dir.display()))
    })?;
    unpack_to(archive_path, dest_dir, format, unpack_as)?;
    if let Some(wanted) = unpack_as {
        promote_extracted_binary(dest_dir, wanted)?;
    }
    Ok(())
}

fn promote_extracted_binary(dest_dir: &Path, wanted: &str) -> Result<()> {
    let dest = dest_dir.join(wanted);
    if dest.is_file() {
        set_executable(&dest);
        return Ok(());
    }

    let mut matches = Vec::new();
    let mut dirs = vec![dest_dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| {
            LitecodeError::Config(format!("read unpack dir {}: {e}", dir.display()))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some(wanted) {
                matches.push(path);
            }
        }
    }
    if matches.len() != 1 {
        return Err(LitecodeError::Config(format!(
            "expected exactly one '{wanted}' after unpack, found {}",
            matches.len()
        )));
    }
    std::fs::rename(&matches[0], &dest).map_err(|e| {
        LitecodeError::Config(format!(
            "move {} -> {}: {e}",
            matches[0].display(),
            dest.display()
        ))
    })?;
    set_executable(&dest);
    Ok(())
}

fn unpack_to(
    archive_path: &Path,
    dest_dir: &Path,
    format: ArchiveFormat,
    unpack_as: Option<&str>,
) -> Result<()> {
    match format {
        ArchiveFormat::Raw => {
            let name = unpack_as
                .map(Path::new)
                .map(|p| p.as_os_str().to_os_string())
                .unwrap_or_else(|| {
                    archive_path
                        .file_name()
                        .unwrap_or(std::ffi::OsStr::new("binary"))
                        .to_os_string()
                });
            let dest = dest_dir.join(name);
            std::fs::copy(archive_path, &dest).map_err(|e| {
                LitecodeError::Config(format!(
                    "copy {} -> {}: {e}",
                    archive_path.display(),
                    dest.display()
                ))
            })?;
            set_executable(&dest);
        }
        ArchiveFormat::Gz => {
            let out_name = unpack_as.map(|s| s.to_string()).unwrap_or_else(|| {
                let name = archive_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("binary");
                name.strip_suffix(".gz").unwrap_or(name).to_string()
            });
            let dest = dest_dir.join(out_name);
            let input = std::fs::File::open(archive_path).map_err(|e| {
                LitecodeError::Config(format!("open gz {}: {e}", archive_path.display()))
            })?;
            let mut decoder = GzDecoder::new(input);
            let mut output = std::fs::File::create(&dest)
                .map_err(|e| LitecodeError::Config(format!("create {}: {e}", dest.display())))?;
            std::io::copy(&mut decoder, &mut output).map_err(|e| {
                LitecodeError::Config(format!("gunzip {}: {e}", archive_path.display()))
            })?;
            set_executable(&dest);
        }
        ArchiveFormat::TarGz => {
            let file = std::fs::File::open(archive_path).map_err(|e| {
                LitecodeError::Config(format!("open tar.gz {}: {e}", archive_path.display()))
            })?;
            let decoder = GzDecoder::new(file);
            let mut archive = Archive::new(decoder);
            archive.unpack(dest_dir).map_err(|e| {
                LitecodeError::Config(format!("untar {}: {e}", archive_path.display()))
            })?;
        }
        ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive_path).map_err(|e| {
                LitecodeError::Config(format!("open zip {}: {e}", archive_path.display()))
            })?;
            let mut archive = ZipArchive::new(file).map_err(|e| {
                LitecodeError::Config(format!("read zip {}: {e}", archive_path.display()))
            })?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| {
                    LitecodeError::Config(format!("read zip entry {}: {e}", archive_path.display()))
                })?;
                let Some(enclosed) = entry.enclosed_name() else {
                    return Err(LitecodeError::Config("zip contains an unsafe path".into()));
                };
                let output = dest_dir.join(enclosed);
                if entry.is_dir() {
                    std::fs::create_dir_all(&output)?;
                } else {
                    if let Some(parent) = output.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut out = std::fs::File::create(&output)?;
                    std::io::copy(&mut entry, &mut out)?;
                    set_executable(&output);
                }
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

// ---------------------------------------------------------------------------
// Metadata (.meta file)
// ---------------------------------------------------------------------------

fn write_meta(dest_dir: &Path, version: &str, digest: Option<&str>) -> Result<()> {
    let meta = serde_json::json!({
        "version": version,
        "digest": digest.unwrap_or(""),
        "installed_at": chrono::Utc::now().to_rfc3339(),
    });
    let meta_path = dest_dir.join(".meta");
    let json = serde_json::to_string_pretty(&meta)?;
    std::fs::write(&meta_path, json)
        .map_err(|e| LitecodeError::Config(format!("write .meta {}: {e}", meta_path.display())))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// manifest.json management (serialised via a global lock)
// ---------------------------------------------------------------------------

static MANIFEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn manifest_path() -> Result<PathBuf> {
    Ok(lsp_dir()?.join("manifest.json"))
}

pub(crate) fn read_manifest() -> Result<HashMap<String, String>> {
    let _guard = MANIFEST_LOCK.lock().unwrap();
    read_manifest_unlocked()
}

fn read_manifest_unlocked() -> Result<HashMap<String, String>> {
    let path = manifest_path()?;
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let data = std::fs::read_to_string(&path)
        .map_err(|e| LitecodeError::Config(format!("read manifest {}: {e}", path.display())))?;
    if data.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&data)
        .map_err(|e| LitecodeError::Config(format!("parse manifest {}: {e}", path.display())))
}

pub(crate) fn write_manifest_entry(server_id: &str, version: &str) -> Result<()> {
    let _guard = MANIFEST_LOCK.lock().unwrap();
    let mut manifest = read_manifest_unlocked()?;
    manifest.insert(server_id.to_string(), version.to_string());
    let path = manifest_path()?;
    let json = serde_json::to_string_pretty(&manifest)?;
    atomic_write(&path, json.as_bytes())?;
    Ok(())
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let temp = parent.join(format!(".{name}.write-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::File::create(&temp)
        .map_err(|e| LitecodeError::Config(format!("create {}: {e}", temp.display())))?;
    use std::io::Write;
    if let Err(e) = file.write_all(contents).and_then(|_| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(LitecodeError::Config(format!(
            "write {}: {e}",
            path.display()
        )));
    }
    let backup = parent.join(format!(".{name}.backup-{}", uuid::Uuid::new_v4()));
    let had_previous = path.exists();
    if had_previous {
        std::fs::rename(path, &backup).map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            LitecodeError::Config(format!("move existing {} aside: {e}", path.display()))
        })?;
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        if had_previous {
            let _ = std::fs::rename(&backup, path);
        }
        let _ = std::fs::remove_file(&temp);
        return Err(LitecodeError::Config(format!(
            "replace {}: {e}",
            path.display()
        )));
    }
    if had_previous {
        let _ = std::fs::remove_file(backup);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Source list for multi-source fallback
// ---------------------------------------------------------------------------

enum Source {
    GitHubApi { token: Option<String> },
    Mirror { base: String },
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::GitHubApi { token: Some(_) } => write!(f, "api.github.com (token)"),
            Source::GitHubApi { token: None } => write!(f, "api.github.com"),
            Source::Mirror { base } => write!(f, "{base}"),
        }
    }
}

fn build_source_list() -> Vec<Source> {
    let mut sources = Vec::new();

    // 1. GitHub API with token.
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());
    sources.push(Source::GitHubApi {
        token: token.clone(),
    });

    // 2. GitHub API without token (always add if we had a token, so we try both).
    if token.is_some() {
        sources.push(Source::GitHubApi { token: None });
    }

    // 3. Mirror (only if LITECODE_GITHUB_PROXY is set).
    let mirror = std::env::var("LITECODE_GITHUB_PROXY").unwrap_or_default();
    if !mirror.is_empty() {
        sources.push(Source::Mirror { base: mirror });
    }

    sources
}

// ---------------------------------------------------------------------------
// HTTP client helper
// ---------------------------------------------------------------------------

fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()
        .expect("reqwest client")
}

// ---------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------

fn classify_reqwest_error(e: reqwest::Error, url: &str) -> LitecodeError {
    let msg = e.to_string();
    if e.is_timeout() {
        LitecodeError::Config(format!(
            "GitHub API 连接超时 (15s)，已重试 3 次。可设置 LITECODE_GITHUB_PROXY 使用镜像: {url}"
        ))
    } else if e.is_connect() {
        if msg.contains("dns") || msg.contains("resolve") || msg.contains("No such host") {
            LitecodeError::Config(format!(
                "无法访问 GitHub (api.github.com): DNS 解析失败。请检查网络或设置 HTTP_PROXY"
            ))
        } else {
            LitecodeError::Config(format!(
                "无法连接到 GitHub: {msg}。请检查网络或设置 HTTP_PROXY"
            ))
        }
    } else {
        LitecodeError::Config(format!("网络错误: {msg}"))
    }
}

fn check_github_rate_limit(resp: &reqwest::Response) -> Result<()> {
    if resp.status() != StatusCode::FORBIDDEN && resp.status() != StatusCode::TOO_MANY_REQUESTS {
        return Ok(());
    }

    let remaining = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0");

    let limit = resp
        .headers()
        .get("x-ratelimit-limit")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("60");

    Err(LitecodeError::Config(format!(
        "GitHub API 限流 (403)，剩余 {remaining}/{limit} 次。可设置 GITHUB_TOKEN 提高限额"
    )))
}

// ---------------------------------------------------------------------------
// Public utilities
// ---------------------------------------------------------------------------

/// Read manifest.json and return the installed version for a server, if any.
pub fn installed_version(server_id: &str) -> Result<Option<String>> {
    let manifest = read_manifest()?;
    Ok(manifest.get(server_id).cloned())
}
