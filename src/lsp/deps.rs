//! LSP dependency probe, runnable checks, and init-time installation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::lsp::install::npm::npm_program_and_args;
use crate::lsp::install::{LanguageServerBinary, LspAdapter, adapters, ls_program_and_args};
use crate::lsp::{command_parts, detect_needed_server_commands, program_from_command, server_map};
use crate::types::LitecodeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LspDepStatus {
    Available,
    Missing,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspServerProbe {
    pub id: String,
    pub command: String,
    pub sources: Vec<String>,
    pub status: LspDepStatus,
    pub install_hint: Option<String>,
    pub size_hint: Option<String>,
    pub installed_version: Option<String>,
    pub error: Option<String>,
    pub managed_path: Option<String>,
    pub official_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ServerMeta {
    id: &'static str,
    program: &'static str,
    default_command: &'static str,
    size_hint: &'static str,
    install_hint: &'static str,
    auto_installable: bool,
}

const SERVER_METAS: &[ServerMeta] = &[
    ServerMeta {
        id: "rust-analyzer",
        program: "rust-analyzer",
        default_command: "rust-analyzer",
        size_hint: "~50MB",
        install_hint: "rustup component add rust-analyzer",
        auto_installable: true,
    },
    ServerMeta {
        id: "typescript-language-server",
        program: "typescript-language-server",
        default_command: "typescript-language-server --stdio",
        size_hint: "~30MB",
        install_hint: "npm install -g typescript-language-server",
        auto_installable: true,
    },
    ServerMeta {
        id: "pyright-langserver",
        program: "pyright-langserver",
        default_command: "pyright-langserver --stdio",
        size_hint: "~80MB",
        install_hint: "npm install -g pyright",
        auto_installable: true,
    },
    ServerMeta {
        id: "gopls",
        program: "gopls",
        default_command: "gopls",
        size_hint: "~40MB",
        install_hint: "go install golang.org/x/tools/gopls@latest",
        auto_installable: true,
    },
    ServerMeta {
        id: "clangd",
        program: "clangd",
        default_command: "clangd",
        size_hint: "~200MB+",
        install_hint: "Install clangd via your system package manager (apt/brew)",
        auto_installable: false,
    },
    ServerMeta {
        id: "csharp-ls",
        program: "csharp-ls",
        default_command: "csharp-ls",
        size_hint: "~100MB+",
        install_hint: "Install the .NET 10 SDK, then run: dotnet tool install -g csharp-ls",
        auto_installable: true,
    },
];

fn meta_for_program(program: &str) -> Option<&'static ServerMeta> {
    SERVER_METAS.iter().find(|m| m.program == program)
}

fn meta_for_id(id: &str) -> Option<&'static ServerMeta> {
    SERVER_METAS.iter().find(|m| m.id == id)
}

/// Map a full LS command string to a stable server id.
pub fn server_id_from_command(command: &str) -> String {
    let program = program_from_command(command);
    meta_for_program(&program)
        .map(|m| m.id.to_string())
        .unwrap_or(program)
}

/// Resolve configured server ids to full command strings (preserving env overrides).
pub fn commands_for_server_ids(root: &Path, ids: &[String]) -> Vec<String> {
    let needed = detect_needed_server_commands(root);
    let mut by_id: HashMap<String, String> = HashMap::new();
    for cmd in needed {
        by_id.insert(server_id_from_command(&cmd), cmd);
    }
    for meta in SERVER_METAS {
        by_id
            .entry(meta.id.to_string())
            .or_insert_with(|| meta.default_command.to_string());
    }
    ids.iter().filter_map(|id| by_id.get(id).cloned()).collect()
}

fn npm_config_prefix() -> Option<PathBuf> {
    let (program, args) = npm_program_and_args(&["config".into(), "get".into(), "prefix".into()])?;
    let output = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!prefix.is_empty()).then(|| PathBuf::from(prefix))
}

/// Writable prefix when the configured npm global dir needs root (e.g. `/usr/local`).
fn npm_fallback_prefix() -> Option<PathBuf> {
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| {
            if cfg!(windows) {
                home.join("AppData").join("Roaming").join("npm")
            } else {
                home.join(".local")
            }
        })
}

fn npm_bin_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(prefix) = npm_config_prefix() {
        dirs.push(prefix.clone());
        if !cfg!(windows) {
            dirs.push(prefix.join("bin"));
        }
    }
    if let Ok(prefix) = std::env::var("NPM_CONFIG_PREFIX") {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            dirs.push(path.clone());
            if !cfg!(windows) {
                dirs.push(path.join("bin"));
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        dirs.push(
            PathBuf::from(home)
                .join("AppData")
                .join("Roaming")
                .join("npm"),
        );
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// dotnet global tools install here but on Linux/macOS it is not auto-added to
/// PATH. Include it so a freshly installed `csharp-ls` is discoverable.
fn dotnet_tools_dirs() -> Vec<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|home| vec![PathBuf::from(home).join(".dotnet").join("tools")])
        .unwrap_or_default()
}

fn candidate_in_dir(dir: &Path, program: &str) -> Option<PathBuf> {
    let candidate = dir.join(program);
    if candidate.is_file() {
        return Some(crate::config::path::canon_abs_lossy(&candidate));
    }
    #[cfg(windows)]
    if candidate.with_extension("exe").is_file() {
        return crate::config::path::os_probe_abs(candidate.with_extension("exe")).ok();
    }
    #[cfg(windows)]
    if candidate.with_extension("cmd").is_file() {
        return crate::config::path::os_probe_abs(candidate.with_extension("cmd")).ok();
    }
    None
}

fn resolve_path_candidate(program: &str) -> Option<PathBuf> {
    if Path::new(program).is_absolute() || program.contains(std::path::MAIN_SEPARATOR) {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    for dir in npm_bin_search_dirs().into_iter().chain(dotnet_tools_dirs()) {
        if let Some(path) = candidate_in_dir(&dir, program) {
            return Some(path);
        }
    }
    let path_var = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path_var) {
        if let Some(path) = candidate_in_dir(&dir, program) {
            return Some(path);
        }
    }
    None
}

fn adapter_for_program(program: &str) -> Option<Box<dyn LspAdapter>> {
    adapters()
        .into_iter()
        .find(|adapter| adapter.server_id() == program)
}

fn managed_binary_path(program: &str) -> Option<LanguageServerBinary> {
    let root = crate::lsp::paths::lsp_dir().ok()?;
    let server_dir = root.join(program);
    managed_binary_path_at(program, &server_dir)
}

pub(crate) fn verify_managed_server_at(program: &str, server_dir: &Path) -> Result<(), String> {
    let _spawn = managed_binary_path_at(program, server_dir)
        .ok_or_else(|| format!("no executable found under managed directory for '{program}'"))?;
    let adapter = adapter_for_program(program)
        .ok_or_else(|| format!("unknown language server '{program}'"))?;
    run_version_probe(&adapter.verify_binary_info(server_dir))
}

pub(crate) fn managed_binary_path_at(
    program: &str,
    server_dir: &Path,
) -> Option<LanguageServerBinary> {
    if !server_dir.is_dir() {
        return None;
    }
    let adapter = adapter_for_program(program)?;
    let binary = adapter.binary_info(server_dir);
    if !binary.path.is_file() {
        return None;
    }
    for arg in &binary.arguments {
        if arg.starts_with('-') {
            continue;
        }
        let script = Path::new(arg);
        if script.is_absolute() && !script.is_file() {
            return None;
        }
    }
    for extra in adapter.extra_managed_files(server_dir) {
        if !extra.is_file() {
            return None;
        }
    }
    Some(binary)
}

/// Verify that a managed installation has a concrete executable and responds
/// to its lightweight version probe. Used before install metadata is committed.
pub(crate) fn verify_managed_server(program: &str) -> Result<(), String> {
    let root = crate::lsp::paths::lsp_dir().map_err(|e| e.to_string())?;
    verify_managed_server_at(program, &root.join(program))
}

/// Resolve one server command to the exact executable and launch arguments.
///
/// Managed installs use adapter metadata; PATH/system installs preserve the
/// configured command arguments. Every caller (probe, warmup, spawn) should
/// use this function so installation and execution cannot diverge.
pub fn resolve_server_binary(command: &str) -> Result<LanguageServerBinary, String> {
    let parts = command_parts(command)?;
    let program = &parts[0];
    if let Some(mut binary) = managed_binary_path(program) {
        let command_args = parts[1..].to_vec();
        merge_managed_arguments(&mut binary, &command_args);
        return Ok(binary);
    }
    let path = resolve_path_candidate(program)
        .ok_or_else(|| format!("'{program}' not found in managed LSP directory or PATH"))?;
    Ok(LanguageServerBinary {
        path,
        arguments: parts[1..].to_vec(),
        env: None,
    })
}

fn merge_managed_arguments(binary: &mut LanguageServerBinary, command_args: &[String]) {
    if command_args.is_empty() {
        return;
    }
    let scripts: Vec<String> = binary
        .arguments
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .cloned()
        .collect();
    if scripts.is_empty() {
        binary.arguments = command_args.to_vec();
        return;
    }
    if command_args.iter().all(|arg| arg.starts_with('-')) {
        binary.arguments = scripts
            .into_iter()
            .chain(command_args.iter().cloned())
            .collect();
    } else {
        binary.arguments = command_args.to_vec();
    }
}

fn npm_install_permission_denied(stderr: &str) -> bool {
    stderr.contains("EACCES")
        || stderr.contains("permission denied")
        || stderr.contains("Permission denied")
}

fn run_npm(args: &[&str]) -> std::io::Result<std::process::Output> {
    let owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let (program, argv) = npm_program_and_args(&owned).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "npm command not found")
    })?;
    Command::new(program)
        .args(argv)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
}

fn npm_install_package(package: &str) -> std::result::Result<(), LitecodeError> {
    let global = run_npm(&["install", "-g", package])
        .map_err(|e| LitecodeError::Config(format!("failed to run npm: {e}")))?;
    if global.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&global.stderr);
    if !npm_install_permission_denied(&stderr) {
        return Err(LitecodeError::Config(format!(
            "npm install -g {package} failed: {stderr}"
        )));
    }

    let Some(prefix) = npm_fallback_prefix() else {
        return Err(LitecodeError::Config(format!(
            "npm install -g {package} failed (permission denied) and no writable prefix found (set HOME or NPM_CONFIG_PREFIX)"
        )));
    };
    std::fs::create_dir_all(&prefix).map_err(|e| {
        LitecodeError::Config(format!(
            "failed to create npm prefix '{}': {e}",
            prefix.display()
        ))
    })?;
    let prefix_str = prefix.to_string_lossy();
    tracing::info!(
        package,
        prefix = %prefix_str,
        "retrying npm install with user-writable prefix"
    );

    let prefixed = run_npm(&["install", "-g", "--prefix", &prefix_str, package])
        .map_err(|e| LitecodeError::Config(format!("failed to run npm: {e}")))?;
    if prefixed.status.success() {
        return Ok(());
    }
    let prefixed_stderr = String::from_utf8_lossy(&prefixed.stderr);
    Err(LitecodeError::Config(format!(
        "npm install -g --prefix {} {package} failed: {prefixed_stderr}",
        prefix.display()
    )))
}

fn is_rustup_shim(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name == "rustup" || name == "rustup.exe")
}

fn rust_analyzer_runnable() -> Result<(), String> {
    let output = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("rustup not available: {e}"))?;
    if !output.status.success() {
        return Err(
            "rust-analyzer component not installed (run: rustup component add rust-analyzer)"
                .into(),
        );
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return Err("rustup which rust-analyzer returned empty path".into());
    }
    run_version_probe(&LanguageServerBinary {
        path: PathBuf::from(path),
        arguments: vec![],
        env: None,
    })
}

fn run_version_probe(binary: &LanguageServerBinary) -> Result<(), String> {
    let extra: Vec<String> = binary
        .arguments
        .iter()
        .filter(|arg| !arg.starts_with('-'))
        .cloned()
        .collect();
    let mut args = extra;
    args.push("--version".into());
    let (program, launch_args) = ls_program_and_args(&binary.path, &args);
    let display = binary.path.display().to_string();
    let mut command = Command::new(&program);
    command.args(&launch_args);
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run '{display}': {e}"))?;

    let status = wait_with_timeout(&mut child, Duration::from_secs(5))
        .map_err(|e| format!("'{display}' version probe: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("'{display}' --version exited with {status}"))
    }
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    use std::time::Instant;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "process timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Verify a language-server program can actually run (not just exist on PATH).
pub fn command_runnable(program: &str) -> Result<(), String> {
    command_runnable_command(program)
}

/// Resolve and verify a complete configured command. Probe, warmup, and
/// spawn use this same resolution contract.
pub fn command_runnable_command(command: &str) -> Result<(), String> {
    let program = program_from_command(command);
    if managed_binary_path(&program).is_some() {
        return verify_managed_server(&program);
    }
    if program == "rust-analyzer" {
        let binary = resolve_server_binary(command)?;
        if is_rustup_shim(&binary.path) {
            return rust_analyzer_runnable();
        }
        return run_version_probe(&binary);
    }
    let binary = resolve_server_binary(command)?;
    if program == "pyright-langserver" {
        // The langserver entry only accepts --stdio; it has no --version.
        return if binary.path.is_file() {
            Ok(())
        } else {
            Err(format!("'{}' not found", binary.path.display()))
        };
    }
    run_version_probe(&binary)
}

fn sources_for_server(root: &Path, program: &str) -> Vec<String> {
    let map = server_map();
    let mut sources = HashSet::new();
    if program == "rust-analyzer" && root.join("Cargo.toml").exists() {
        sources.insert("Cargo.toml".into());
    }
    if program == "gopls" && root.join("go.mod").exists() {
        sources.insert("go.mod".into());
    }
    if program == "typescript-language-server"
        && (root.join("package.json").exists() || root.join("tsconfig.json").exists())
    {
        sources.insert("package.json / tsconfig.json".into());
    }
    if (program == "pyright-langserver" || program == "pyright")
        && (root.join("pyproject.toml").exists() || root.join("setup.py").exists())
    {
        sources.insert("pyproject.toml / setup.py".into());
    }
    for (ext, cmd) in &map {
        if program_from_command(cmd) == program {
            sources.insert(format!("*.{ext}"));
        }
    }
    let mut out: Vec<_> = sources.into_iter().collect();
    out.sort();
    out
}

fn official_url(program: &str) -> Option<String> {
    let url = match program {
        "rust-analyzer" => "https://rust-analyzer.github.io/",
        "gopls" => "https://pkg.go.dev/golang.org/x/tools/gopls",
        "typescript-language-server" => {
            "https://github.com/typescript-language-server/typescript-language-server"
        }
        "pyright-langserver" => "https://github.com/microsoft/pyright",
        "csharp-ls" => "https://github.com/razzmatazz/csharp-language-server",
        "clangd" => "https://clangd.llvm.org/installation",
        _ => return None,
    };
    Some(url.to_string())
}

/// Probe workspace for required language servers and their install status.
pub fn probe_workspace_servers(root: &Path) -> Vec<LspServerProbe> {
    let commands = detect_needed_server_commands(root);
    let mut seen = HashSet::new();
    let mut probes = Vec::new();

    for cmd in commands {
        let program = program_from_command(&cmd);
        if !seen.insert(program.clone()) {
            continue;
        }
        let meta = meta_for_program(&program);
        let id = meta
            .map(|m| m.id.to_string())
            .unwrap_or_else(|| program.clone());
        let result = command_runnable_command(&cmd);
        let status = match &result {
            Ok(()) => LspDepStatus::Available,
            Err(msg) if msg.contains("not found") => LspDepStatus::Missing,
            Err(_) => LspDepStatus::Broken,
        };
        let error = result.err();
        let managed_path = crate::lsp::paths::lsp_dir()
            .ok()
            .map(|dir| dir.join(&id).display().to_string());
        let installed_version = crate::lsp::install::github::installed_version(&id)
            .ok()
            .flatten();
        probes.push(LspServerProbe {
            id: id.clone(),
            command: cmd,
            sources: sources_for_server(root, &program),
            status,
            install_hint: meta.map(|m| m.install_hint.to_string()),
            size_hint: meta.map(|m| m.size_hint.to_string()),
            installed_version,
            error,
            managed_path,
            official_url: official_url(&program),
        });
    }
    probes.sort_by(|a, b| a.id.cmp(&b.id));
    probes
}

fn install_command_for(id: &str) -> Option<(&'static str, Vec<&'static str>)> {
    let meta = meta_for_id(id)?;
    if !meta.auto_installable {
        return None;
    }
    match id {
        "rust-analyzer" => Some(("rustup", vec!["component", "add", "rust-analyzer"])),
        "gopls" => Some(("go", vec!["install", "golang.org/x/tools/gopls@latest"])),
        "csharp-ls" => Some(("dotnet", vec!["tool", "install", "-g", "csharp-ls"])),
        // npm-based servers use `npm_install_package` in `install_server`
        "typescript-language-server" | "pyright-langserver" => None,
        _ => None,
    }
}

fn npm_package_for(id: &str) -> Option<&'static str> {
    match id {
        "typescript-language-server" => Some("typescript-language-server"),
        "pyright-langserver" => Some("pyright"),
        _ => None,
    }
}

/// Parse `dotnet --list-sdks` output and report whether a 10.x SDK is present.
/// csharp-ls targets .NET 10, so a 10.x SDK is required to run it.
fn has_dotnet_10_sdk(sdks_output: &str) -> bool {
    sdks_output.lines().any(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|v| v.starts_with("10."))
    })
}

/// Verify a compatible .NET 10 SDK is present before installing .NET-based
/// language servers. csharp-ls targets .NET 10, so a 10.x SDK is required.
/// The SDK itself is NOT auto-installed — we only detect and advise.
pub(crate) fn ensure_dotnet_sdk() -> Result<(), LitecodeError> {
    let output = match Command::new("dotnet")
        .args(["--list-sdks"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            return Err(LitecodeError::Config(dotnet_sdk_install_hint(&format!(
                "cannot run dotnet ({e}); csharp-ls cannot be installed"
            ))));
        }
    };

    if !output.status.success() {
        return Err(LitecodeError::Config(dotnet_sdk_install_hint(
            "dotnet --list-sdks failed",
        )));
    }

    let sdks = String::from_utf8_lossy(&output.stdout);
    if has_dotnet_10_sdk(&sdks) {
        Ok(())
    } else {
        Err(LitecodeError::Config(dotnet_sdk_install_hint(
            ".NET 10 SDK not detected (csharp-ls requires .NET 10)",
        )))
    }
}

/// Per-platform instructions for installing the .NET 10 SDK. Does not perform
/// the install itself, only returns guidance surfaced in errors/hints.
fn dotnet_sdk_install_hint(reason: &str) -> String {
    let platform = if cfg!(windows) {
        "Windows: install the .NET 10 SDK from https://dotnet.microsoft.com/download, or run `winget install Microsoft.DotNet.SDK.10`"
    } else {
        "Ubuntu/Linux: `sudo apt-get update && sudo apt-get install -y dotnet-sdk-10.0` (official packages from 24.04; other distros: `curl -sSL https://dot.net/v1/dotnet-install.sh | bash -s -- --channel 10.0`)"
    };
    format!(
        "{reason}. Install the .NET 10 SDK first:\n{platform}\nThen retry `dotnet tool install -g csharp-ls`."
    )
}

/// Install a language server by id via system package manager.
/// For managed lsp_dir installs, use `POST /api/workspace/lsp/install` instead.
pub fn install_server(id: &str) -> std::result::Result<(), LitecodeError> {
    let Some(meta) = meta_for_id(id) else {
        return Err(LitecodeError::Config(format!(
            "unknown language server id: {id}"
        )));
    };

    // npm-based servers: install globally via npm (system-level).
    if let Some(package) = npm_package_for(id) {
        tracing::info!(server = %id, package, "installing language server via npm");
        return npm_install_package(package);
    }

    let Some((installer, args)) = install_command_for(id) else {
        return Err(LitecodeError::Config(format!(
            "{} is not auto-installable; {}",
            id, meta.install_hint
        )));
    };

    // dotnet global tools require a matching .NET SDK; detect it (no auto-install)
    // and advise per platform before attempting `dotnet tool install`.
    if installer == "dotnet" {
        ensure_dotnet_sdk()?;
    }

    tracing::info!(server = %id, installer, "installing language server");

    let output = Command::new(installer)
        .args(&args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            LitecodeError::Config(format!("failed to run installer '{installer}': {e}"))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(LitecodeError::Config(format!(
            "install {id} failed: {stderr}"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspInitFailure {
    pub id: String,
    pub error: String,
}

/// Ensure server ids are runnable; optionally install missing ones into lsp_dir.
pub async fn ensure_servers(ids: &[String], install: bool) -> (Vec<String>, Vec<LspInitFailure>) {
    let mut failures = Vec::new();
    let mut ready = Vec::new();

    for id in ids {
        let program = meta_for_id(id).map(|m| m.program).unwrap_or(id.as_str());

        if command_runnable(program).is_err() && install
            && let Err(e) = crate::lsp::install::install_server_to_lsp_dir(id, None).await {
                failures.push(LspInitFailure {
                    id: id.clone(),
                    error: e.to_string(),
                });
                continue;
            }

        match command_runnable(program) {
            Ok(()) => ready.push(id.clone()),
            Err(e) => {
                let hint = meta_for_id(id)
                    .map(|m| m.install_hint)
                    .unwrap_or("install manually");
                failures.push(LspInitFailure {
                    id: id.clone(),
                    error: format!("{e}. Hint: {hint}"),
                });
            }
        }
    }

    (ready, failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_id_from_rust_analyzer_command() {
        assert_eq!(server_id_from_command("rust-analyzer"), "rust-analyzer");
        assert_eq!(
            server_id_from_command("typescript-language-server --stdio"),
            "typescript-language-server"
        );
    }

    #[test]
    fn probe_empty_repo_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_workspace_servers(dir.path()).is_empty());
    }

    #[test]
    fn probe_rust_repo_lists_rust_analyzer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"t\"\n").unwrap();
        let probes = probe_workspace_servers(dir.path());
        assert!(probes.iter().any(|p| p.id == "rust-analyzer"));
    }

    #[cfg(windows)]
    #[test]
    fn candidate_finds_windows_command_shim() {
        let dir = tempfile::tempdir().unwrap();
        let shim = dir.path().join("typescript-language-server.cmd");
        std::fs::write(&shim, "@echo off\r\n").unwrap();
        assert_eq!(
            candidate_in_dir(dir.path(), "typescript-language-server"),
            Some(crate::config::path::os_probe_abs(&shim).unwrap())
        );
    }

    #[test]
    fn has_dotnet_10_sdk_detects_10() {
        assert!(has_dotnet_10_sdk(
            "8.0.100 [/usr/share/dotnet/sdk]\n10.0.100 [/usr/share/dotnet/sdk]\n"
        ));
        // preview/rc builds still count as 10.x
        assert!(has_dotnet_10_sdk("10.0.100-rc.1 [/usr/share/dotnet/sdk]\n"));
    }

    #[test]
    fn has_dotnet_10_sdk_rejects_other_versions() {
        assert!(!has_dotnet_10_sdk(""));
        assert!(!has_dotnet_10_sdk("8.0.100 [/usr/share/dotnet/sdk]\n"));
        assert!(!has_dotnet_10_sdk("9.0.300 [/usr/share/dotnet/sdk]\n"));
    }

    #[test]
    fn dotnet_tools_dir_derives_from_home() {
        let prev_home = std::env::var_os("HOME");
        let prev_up = std::env::var_os("USERPROFILE");
        unsafe {
            std::env::set_var("HOME", "/home/tester");
            std::env::remove_var("USERPROFILE");
        }

        let dirs = dotnet_tools_dirs();

        unsafe {
            match prev_home {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
            if let Some(up) = prev_up {
                std::env::set_var("USERPROFILE", up);
            }
        }

        assert_eq!(dirs, vec![PathBuf::from("/home/tester/.dotnet/tools")]);
    }

    #[test]
    fn dotnet_sdk_install_hint_is_complete() {
        let hint = dotnet_sdk_install_hint("reason-here");
        assert!(hint.contains("reason-here"));
        assert!(hint.contains("dotnet tool install -g csharp-ls"));
        if cfg!(windows) {
            assert!(hint.contains("winget"));
        } else {
            assert!(hint.contains("apt-get"));
        }
    }

    #[test]
    fn csharp_adapter_is_dotnet_tool() {
        let adapter = crate::lsp::install::adapters::csharp::adapter();
        assert_eq!(
            adapter.install_kind(),
            crate::lsp::install::InstallKind::DotnetTool
        );
    }

    #[test]
    fn managed_path_requires_declared_binary() {
        let dir = tempfile::tempdir().unwrap();
        let server_dir = dir.path().join("rust-analyzer");
        std::fs::create_dir_all(&server_dir).unwrap();
        std::fs::write(
            server_dir.join("rust-analyzer-x86_64-unknown-linux-gnu"),
            b"",
        )
        .unwrap();
        assert!(managed_binary_path_at("rust-analyzer", &server_dir).is_none());
    }

    #[test]
    fn merge_keeps_node_entry_when_command_only_has_flags() {
        let mut binary = LanguageServerBinary {
            path: PathBuf::from("node"),
            arguments: vec!["/tmp/cli.mjs".into(), "--stdio".into()],
            env: None,
        };
        merge_managed_arguments(&mut binary, &["--stdio".into()]);
        assert_eq!(
            binary.arguments,
            vec!["/tmp/cli.mjs".to_string(), "--stdio".to_string()]
        );
    }
}
