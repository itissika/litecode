//! Install engine: download, verify, extract, run, and swap atomically.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::registry::{AppRegistry, RegisteredApp};
use super::schema::{ArchiveFormat, InstallKind, InstallStep, LaunchSpec};
use super::space;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallMarker {
    pub status: String,
    pub phase: String,
    pub percent: Option<u8>,
    pub message: Option<String>,
    pub error: Option<String>,
    pub log_path: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallManifest {
    pub app_id: String,
    pub source: String,
    pub installed_at: i64,
    pub definition_hash: Option<String>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn marker_path(install_dir: &Path) -> PathBuf {
    sibling(install_dir, "install.json")
}

pub fn lock_path(install_dir: &Path) -> PathBuf {
    sibling(install_dir, "lock")
}

pub fn manifest_path(install_dir: &Path) -> PathBuf {
    install_dir.join(".litecode-app.json")
}

pub fn logs_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("logs")
}

pub fn run_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("run")
}

pub fn app_dir(install_dir: &Path) -> PathBuf {
    install_dir.join("app")
}

fn sibling(install_dir: &Path, suffix: &str) -> PathBuf {
    let mut name = install_dir
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    install_dir.with_file_name(name)
}

/// Remove abandoned staging trees left by a prior sidecar. The name is the
/// install directory plus `.staging-<uuid>`; a fresh install never shares it.
pub fn cleanup_stale_staging(apps_root: &Path) {
    cleanup_staging_in(apps_root);
    let Ok(entries) = std::fs::read_dir(apps_root.join("ws")) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            cleanup_staging_in(&entry.path());
        }
    }
}

fn cleanup_staging_in(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.contains(".staging-") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .map(|age| age > Duration::from_secs(24 * 60 * 60))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

pub fn install_in_progress(install_dir: &Path) -> bool {
    let path = lock_path(install_dir);
    if !path.exists() {
        return false;
    }
    let owner = std::fs::read_to_string(&path).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("pid="))
            .and_then(|pid| pid.trim().parse::<u32>().ok())
    });
    owner
        .map(crate::serve::shutdown::is_process_alive)
        .unwrap_or(true)
}

pub fn read_marker(install_dir: &Path) -> Option<InstallMarker> {
    let text = std::fs::read_to_string(marker_path(install_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn clear_marker(install_dir: &Path) {
    let _ = std::fs::remove_file(marker_path(install_dir));
}

fn write_marker(install_dir: &Path, marker: &InstallMarker) {
    if let Some(parent) = marker_path(install_dir).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(marker) {
        let _ = std::fs::write(marker_path(install_dir), text);
    }
}

fn marker(
    install_dir: &Path,
    phase: &str,
    percent: Option<u8>,
    message: Option<String>,
    log_path: Option<PathBuf>,
) {
    write_marker(
        install_dir,
        &InstallMarker {
            status: "installing".into(),
            phase: phase.into(),
            percent,
            message,
            error: None,
            log_path: log_path.map(|p| p.to_string_lossy().to_string()),
            updated_at: now_ms(),
        },
    );
}

struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create install parent: {e}"))?;
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "pid={}", std::process::id());
                Ok(Self { path })
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let dead_owner = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| {
                        text.lines()
                            .find_map(|line| line.strip_prefix("pid="))
                            .and_then(|pid| pid.trim().parse::<u32>().ok())
                    })
                    .map(|pid| !crate::serve::shutdown::is_process_alive(pid))
                    .unwrap_or(false);
                let stale = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| SystemTime::now().duration_since(m).ok())
                    .map(|age| age > Duration::from_secs(6 * 60 * 60))
                    .unwrap_or(false);
                if dead_owner || stale {
                    let _ = std::fs::remove_file(&path);
                    return Self::acquire(path);
                }
                Err("another install is already running for this app".into())
            }
            Err(err) => Err(format!("create install lock: {err}")),
        }
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_error(install_dir: &Path, log: &Path, message: &str) {
    write_marker(
        install_dir,
        &InstallMarker {
            status: "error".into(),
            phase: "error".into(),
            percent: None,
            message: None,
            error: Some(message.to_string()),
            log_path: Some(log.to_string_lossy().to_string()),
            updated_at: now_ms(),
        },
    );
}

pub fn log_tail(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(20);
    Some(lines[start..].join("\n"))
}

pub fn install_blocking(registry: &AppRegistry, app: &RegisteredApp) -> Result<(), String> {
    if app.definition.install.kind != InstallKind::Steps {
        return Err("this app is external; no managed install is available".into());
    }
    let install_dir = registry.install_dir(app);
    if let Some(required) = app.definition.install.size_bytes {
        let free = space::available_bytes(&registry.apps_root).unwrap_or(u64::MAX);
        if free < required {
            return Err(format!(
                "not enough disk space: need {required} bytes, {free} bytes free"
            ));
        }
    }
    let lock = InstallLock::acquire(lock_path(&install_dir))?;
    clear_marker(&install_dir);
    std::fs::create_dir_all(&install_dir).map_err(|e| format!("create install dir: {e}"))?;
    let logs = logs_dir(&install_dir);
    std::fs::create_dir_all(&logs).map_err(|e| format!("create logs dir: {e}"))?;
    let log = logs.join("install.log");
    let _ = std::fs::write(&log, b"");
    marker(&install_dir, "preparing", Some(1), None, Some(log.clone()));

    let result = install_inner(registry, app, &install_dir, &log);
    match result {
        Ok(()) => {
            let manifest = InstallManifest {
                app_id: app.id.clone(),
                source: app.source.as_str().into(),
                installed_at: now_ms(),
                definition_hash: app.definition_hash(),
            };
            let _ = std::fs::write(
                manifest_path(&install_dir),
                serde_json::to_string_pretty(&manifest).unwrap_or_default(),
            );
            clear_marker(&install_dir);
            drop(lock);
            Ok(())
        }
        Err(err) => {
            write_error(&install_dir, &log, &err);
            drop(lock);
            Err(err)
        }
    }
}

fn install_inner(
    registry: &AppRegistry,
    app: &RegisteredApp,
    install_dir: &Path,
    log: &Path,
) -> Result<(), String> {
    let staging = sibling(install_dir, &format!("staging-{}", uuid::Uuid::new_v4()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| format!("clean staging: {e}"))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("create staging: {e}"))?;
    let stage_app = staging.join("app");
    std::fs::create_dir_all(&stage_app).map_err(|e| format!("create staging app: {e}"))?;
    let data = registry.data_dir(app, install_dir);
    std::fs::create_dir_all(&data).map_err(|e| format!("create data dir: {e}"))?;

    let result = (|| {
        for (index, step) in app.definition.install.steps.iter().enumerate() {
            let percent =
                5 + ((index as u32 * 80) / app.definition.install.steps.len().max(1) as u32) as u8;
            match step {
                InstallStep::Download { url, sha256, file } => {
                    marker(
                        install_dir,
                        "downloading",
                        Some(percent),
                        Some(url.clone()),
                        Some(log.to_path_buf()),
                    );
                    download(url, sha256.as_deref(), &staging.join(file), log)?;
                }
                InstallStep::Extract {
                    file,
                    format,
                    strip,
                    into,
                } => {
                    marker(
                        install_dir,
                        "extracting",
                        Some(percent),
                        Some(format!("extract {file}")),
                        Some(log.to_path_buf()),
                    );
                    let source = staging.join(file);
                    let target = safe_join(&stage_app, into)?;
                    std::fs::create_dir_all(&target)
                        .map_err(|e| format!("create extract target: {e}"))?;
                    extract_archive(&source, *format, *strip, &target)?;
                }
                InstallStep::Run {
                    command,
                    args,
                    cwd,
                    env,
                    timeout_ms,
                } => {
                    marker(
                        install_dir,
                        "running",
                        Some(percent),
                        Some(format!("run {command}")),
                        Some(log.to_path_buf()),
                    );
                    let workdir = match cwd.as_deref().unwrap_or("app") {
                        "workspace" => registry.workspace_root.clone(),
                        _ => stage_app.clone(),
                    };
                    let mut resolved_env = BTreeMap::new();
                    for (key, value) in env {
                        resolved_env.insert(
                            key.clone(),
                            expand_tokens(value, registry, &stage_app, &data),
                        );
                    }
                    let expanded_args: Vec<String> = args
                        .iter()
                        .map(|arg| expand_tokens(arg, registry, &stage_app, &data))
                        .collect();
                    run_command(
                        command,
                        &expanded_args,
                        &workdir,
                        &resolved_env,
                        &stage_app,
                        *timeout_ms,
                        log,
                    )?;
                }
            }
        }
        marker(
            install_dir,
            "verifying",
            Some(90),
            Some("verify install".into()),
            Some(log.to_path_buf()),
        );
        if let Some(check) = &app.definition.install.check {
            let path = safe_join(&stage_app, &check.path)?;
            if !path.exists() {
                return Err(format!("install check failed: {} is missing", check.path));
            }
        }
        if app.definition.install.size_bytes.is_some() {
            let free = space::available_bytes(&registry.apps_root).unwrap_or(u64::MAX);
            let needed = space::directory_size(&staging);
            if free < needed {
                return Err(format!(
                    "not enough disk space: need {needed} bytes, {free} bytes free"
                ));
            }
        }
        Ok(())
    })();

    if let Err(err) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(err);
    }

    swap_staging(install_dir, &staging)?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

fn swap_staging(install_dir: &Path, staging: &Path) -> Result<(), String> {
    let app = app_dir(install_dir);
    let new_app = staging.join("app");
    let old_app = sibling(install_dir, &format!("old-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(install_dir).map_err(|e| format!("create install dir: {e}"))?;
    let had_old = app.exists();
    if had_old {
        std::fs::rename(&app, &old_app).map_err(|e| format!("park old app: {e}"))?;
    }
    if let Err(err) = std::fs::rename(&new_app, &app) {
        if had_old {
            let _ = std::fs::rename(&old_app, &app);
        }
        return Err(format!("activate new app: {err}"));
    }
    if had_old {
        let _ = std::fs::remove_dir_all(&old_app);
    }
    Ok(())
}

fn download(url: &str, expected: Option<&str>, dest: &Path, log: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .map_err(|e| format!("download client: {e}"))?;
    let mut response = client
        .get(url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("LiteCode/", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .map_err(|e| format!("download {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("download {url}: HTTP {}", response.status()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create download dir: {e}"))?;
    }
    let mut file = File::create(dest).map_err(|e| format!("create download: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("download body: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|e| format!("write download: {e}"))?;
    }
    file.flush().map_err(|e| format!("flush download: {e}"))?;
    if let Some(expected) = expected {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(dest);
            return Err(format!(
                "sha256 mismatch: expected {expected}, got {actual}"
            ));
        }
    }
    let _ = append_log(log, &format!("downloaded {url}"));
    Ok(())
}

fn safe_join(base: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut out = base.to_path_buf();
    let path = Path::new(relative);
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => out.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("unsafe path: {relative}"));
            }
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn apply_unix_mode(path: &Path, mode: Option<u32>) {
    use std::os::unix::fs::PermissionsExt;
    let Some(mode) = mode else { return };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777));
}

#[cfg(not(unix))]
fn apply_unix_mode(_path: &Path, _mode: Option<u32>) {}

fn strip_components(path: &Path, strip: usize) -> Option<PathBuf> {
    let parts: Vec<Component<'_>> = path.components().collect();
    if parts.len() < strip {
        return None;
    }
    let mut out = PathBuf::new();
    for component in parts.into_iter().skip(strip) {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => out.push(part),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

fn extract_archive(
    source: &Path,
    format: ArchiveFormat,
    strip: usize,
    target: &Path,
) -> Result<(), String> {
    match format {
        ArchiveFormat::Zip => extract_zip(source, strip, target),
        ArchiveFormat::TarGz => extract_tar_gz(source, strip, target),
        ArchiveFormat::SevenZ => {
            Err("7z extraction is not available; install this app manually".into())
        }
    }
}

fn extract_zip(source: &Path, strip: usize, target: &Path) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("zip entry: {e}"))?;
        let Some(name) = entry.enclosed_name() else {
            continue;
        };
        let Some(rel) = strip_components(&name, strip) else {
            continue;
        };
        let dest = target.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| format!("zip dir: {e}"))?;
            apply_unix_mode(&dest, entry.unix_mode());
        } else {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("zip parent: {e}"))?;
            }
            let mut out = File::create(&dest).map_err(|e| format!("zip file: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("zip copy: {e}"))?;
            drop(out);
            apply_unix_mode(&dest, entry.unix_mode());
        }
    }
    Ok(())
}

fn extract_tar_gz(source: &Path, strip: usize, target: &Path) -> Result<(), String> {
    let file = File::open(source).map_err(|e| format!("open tar.gz: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries().map_err(|e| format!("read tar.gz: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar path: {e}"))?
            .into_owned();
        let Some(rel) = strip_components(&path, strip) else {
            continue;
        };
        let dest = target.join(rel);
        let mode = entry.header().mode().ok();
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| format!("tar dir: {e}"))?;
            apply_unix_mode(&dest, mode);
        } else if entry.header().entry_type().is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("tar parent: {e}"))?;
            }
            let mut out = File::create(&dest).map_err(|e| format!("tar file: {e}"))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("tar copy: {e}"))?;
            drop(out);
            apply_unix_mode(&dest, mode);
        }
    }
    Ok(())
}

fn expand_tokens(value: &str, registry: &AppRegistry, app_dir: &Path, data: &Path) -> String {
    value
        .replace("{workspace}", &registry.workspace_root.to_string_lossy())
        .replace("{appdir}", &app_dir.to_string_lossy())
        .replace("{data}", &data.to_string_lossy())
}

fn resolve_program(command: &str, app_dir: &Path) -> Result<(PathBuf, Vec<String>), String> {
    let (program, extra) = if command.eq_ignore_ascii_case("npm") {
        let shim = crate::lsp::install::npm::resolve_npm_shim()
            .ok_or_else(|| "npm was not found on PATH".to_string())?;
        (shim, Vec::new())
    } else if command.eq_ignore_ascii_case("npx") {
        let shim = crate::lsp::install::npm::resolve_npm_shim()
            .ok_or_else(|| "npm was not found on PATH".to_string())?;
        (shim, vec!["exec".to_string()])
    } else {
        let path = Path::new(command);
        if path.is_absolute() {
            (path.to_path_buf(), Vec::new())
        } else if command.contains('/') || command.contains('\\') || command.ends_with(".exe") {
            (app_dir.join(path), Vec::new())
        } else {
            (path.to_path_buf(), Vec::new())
        }
    };
    Ok((program, extra))
}

fn run_command(
    command: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    app_dir: &Path,
    timeout_ms: u64,
    log: &Path,
) -> Result<(), String> {
    let (program, mut all_args) = resolve_program(command, app_dir)?;
    all_args.extend(args.iter().cloned());
    let mut cmd = Command::new(&program);
    cmd.args(&all_args)
        .current_dir(cwd)
        .envs(env)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| format!("open install log: {e}"))?;
    let _ = writeln!(
        log_file,
        "\n$ {} {}\n(cwd: {})",
        program.display(),
        all_args.join(" "),
        cwd.display()
    );
    let stdout = log_file
        .try_clone()
        .map_err(|e| format!("clone install log: {e}"))?;
    let stderr = log_file
        .try_clone()
        .map_err(|e| format!("clone install log: {e}"))?;
    cmd.stdout(Stdio::from(stdout)).stderr(Stdio::from(stderr));
    crate::proc_lifetime::configure_parent_death(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", program.display()))?;
    crate::proc_lifetime::bind_child_to_parent(child.id());
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    return Ok(());
                }
                return Err(format!("command failed with {}: {}", status, command));
            }
            Ok(None) => {
                if started.elapsed() > Duration::from_millis(timeout_ms) {
                    let pid = child.id();
                    crate::terminal::kill_process_tree(pid);
                    let _ = child.wait();
                    return Err(format!("command timed out after {timeout_ms}ms: {command}"));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(err) => return Err(format!("wait for command: {err}")),
        }
    }
}

fn append_log(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// A compact preview used by tests and diagnostics.
pub fn describe_launch(launch: &LaunchSpec) -> String {
    match launch {
        LaunchSpec::Process { command, args, .. } => format!("{command} {}", args.join(" ")),
        LaunchSpec::Static { .. } => "static".into(),
        LaunchSpec::Builtin { view } => format!("builtin:{}", view.as_str()),
    }
}
