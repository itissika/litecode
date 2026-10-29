use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSource {
    Builtin,
    Global,
    Workspace,
}

impl AppSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }

    pub fn is_custom(self) -> bool {
        !matches!(self, Self::Builtin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallScope {
    Global,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    Data,
    Analysis,
    Document,
    Design,
    Image,
    Video,
    Audio,
    Task,
    General,
}

impl AppCategory {
    pub fn order(self) -> u8 {
        match self {
            Self::Data => 0,
            Self::Analysis => 1,
            Self::Document => 2,
            Self::Design => 3,
            Self::Image => 4,
            Self::Video => 5,
            Self::Audio => 6,
            Self::Task => 7,
            Self::General => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataScope {
    Workspace,
    App,
}

impl Default for DataScope {
    fn default() -> Self {
        Self::Workspace
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Microphone,
    Camera,
    ClipboardRead,
    ClipboardWrite,
    Fullscreen,
}

impl Permission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::Camera => "camera",
            Self::ClipboardRead => "clipboard-read",
            Self::ClipboardWrite => "clipboard-write",
            Self::Fullscreen => "fullscreen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    NotInstalled,
    Installing,
    Installed,
    Starting,
    Running,
    Stopping,
    Crashed,
    Error,
    Uninstalling,
}

impl AppStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Crashed => "crashed",
            Self::Error => "error",
            Self::Uninstalling => "uninstalling",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryFile {
    pub version: u32,
    #[serde(default)]
    pub apps: BTreeMap<String, AppDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppDefinition {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// One-line descriptions keyed by locale (for example `zh-CN`).
    #[serde(default)]
    pub description_i18n: BTreeMap<String, String>,
    pub category: AppCategory,
    pub icon: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub readme_url: Option<String>,
    pub install: InstallSpec,
    pub launch: LaunchSpec,
    #[serde(default)]
    pub data: DataScope,
    #[serde(default)]
    pub single_instance: bool,
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallSpec {
    #[serde(default)]
    pub kind: InstallKind,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub steps: Vec<InstallStep>,
    #[serde(default)]
    pub check: Option<CheckSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Steps,
    None,
}

impl Default for InstallKind {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSpec {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum InstallStep {
    Download {
        url: String,
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default = "default_download_file")]
        file: String,
    },
    Extract {
        file: String,
        format: ArchiveFormat,
        #[serde(default)]
        strip: usize,
        #[serde(default = "default_into")]
        into: String,
    },
    Run {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default = "default_step_timeout")]
        timeout_ms: u64,
    },
}

fn default_download_file() -> String {
    "download.bin".into()
}

fn default_into() -> String {
    ".".into()
}

fn default_step_timeout() -> u64 {
    600_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    Zip,
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "7z")]
    SevenZ,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LaunchSpec {
    /// A panel LiteCode paints itself. Bundled apps only; nothing installs or starts.
    Builtin { view: BuiltinView },
    Process {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        port: PortSpec,
        url: String,
        #[serde(default)]
        ready_path: Option<String>,
        #[serde(default = "default_launch_timeout")]
        timeout_ms: u64,
    },
    Static {
        #[serde(default)]
        spa: bool,
    },
}

fn default_launch_timeout() -> u64 {
    30_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinView {
    Gallery,
}

impl BuiltinView {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gallery => "gallery",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PortSpec {
    Fixed(u16),
    Auto(String),
}

impl Default for PortSpec {
    fn default() -> Self {
        Self::Auto("auto".into())
    }
}

impl PortSpec {
    pub fn fixed(self) -> Option<u16> {
        match self {
            Self::Fixed(port) => Some(port),
            Self::Auto(_) => None,
        }
    }
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id == "ws" {
        return Err("app id \"ws\" is reserved".into());
    }
    let mut chars = id.chars();
    let first = chars.next().ok_or_else(|| "app id is empty".to_string())?;
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err("app id must start with [a-z0-9]".into());
    }
    if id.len() > 64 {
        return Err("app id is longer than 64 characters".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("app id may only contain [a-z0-9-]".into());
    }
    Ok(())
}

pub fn validate_definition(
    def: &AppDefinition,
    source: AppSource,
    workspace_root: &Path,
) -> Result<(), String> {
    if def.name.trim().is_empty() {
        return Err("name is empty".into());
    }
    if def.description.trim().is_empty() {
        return Err("description is empty".into());
    }
    if def.description.chars().count() > 240 {
        return Err("description is longer than 240 characters".into());
    }
    if def.summary.chars().count() > 800 {
        return Err("summary is longer than 800 characters".into());
    }
    validate_description_i18n(&def.description_i18n)?;
    validate_icon(&def.icon, source, workspace_root)?;
    if let Some(homepage) = def.homepage.as_deref() {
        let url = Url::parse(homepage).map_err(|e| format!("homepage is not a URL: {e}"))?;
        if url.scheme() != "https" || url.host_str().is_none() {
            return Err("homepage must be an absolute https URL".into());
        }
    }
    if let Some(readme) = def.readme_url.as_deref() {
        if !readme.starts_with("https://raw.githubusercontent.com/") {
            return Err("readme_url must be https://raw.githubusercontent.com/...".into());
        }
    }
    validate_install(&def.install, source)?;
    if matches!(def.launch, LaunchSpec::Builtin { .. })
        && (def.install.kind != InstallKind::None || !def.install.steps.is_empty())
    {
        return Err("a built-in launch cannot install files".into());
    }
    validate_launch(&def.launch, source)?;
    Ok(())
}

fn validate_description_i18n(texts: &BTreeMap<String, String>) -> Result<(), String> {
    for (locale, text) in texts {
        if locale.trim().is_empty() {
            return Err("description_i18n locale is empty".into());
        }
        if text.trim().is_empty() {
            return Err(format!("description_i18n.{locale} is empty"));
        }
        if text.chars().count() > 240 {
            return Err(format!(
                "description_i18n.{locale} is longer than 240 characters"
            ));
        }
    }
    Ok(())
}

fn validate_icon(icon: &str, source: AppSource, workspace_root: &Path) -> Result<(), String> {
    if let Some(rel) = icon.strip_prefix("file:") {
        if source != AppSource::Workspace {
            return Err("file: icons are only allowed in the workspace registry".into());
        }
        let path = Path::new(rel);
        if path.is_absolute() || !safe_relative(path) {
            return Err("icon file path must stay inside the workspace".into());
        }
        let full = workspace_root.join(path);
        let ext = full
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "png" | "webp" | "jpg" | "jpeg" | "gif") {
            return Err("icon file must be PNG, WebP, JPEG, or GIF".into());
        }
        if let Ok(meta) = std::fs::metadata(&full) {
            if meta.len() > 64 * 1024 {
                return Err("icon file is larger than 64 KiB".into());
            }
        }
        return Ok(());
    }
    if icon.trim().is_empty() {
        return Err("icon is empty".into());
    }
    if icon.chars().count() > 180 {
        return Err("icon is too long".into());
    }
    Ok(())
}

fn validate_install(install: &InstallSpec, source: AppSource) -> Result<(), String> {
    if install.kind == InstallKind::Steps && install.steps.is_empty() {
        return Err("install.steps is empty".into());
    }
    if let Some(check) = &install.check {
        if check.path.is_empty() || !safe_relative(Path::new(&check.path)) {
            return Err("install.check.path must stay inside app/".into());
        }
    }
    for step in &install.steps {
        match step {
            InstallStep::Download { url, sha256, file } => {
                let parsed = Url::parse(url).map_err(|e| format!("download url: {e}"))?;
                if parsed.scheme() != "https" || parsed.host_str().is_none() {
                    return Err("download url must be an absolute https URL".into());
                }
                if !safe_relative(Path::new(file)) {
                    return Err("download file must stay inside the app directory".into());
                }
                if let Some(hash) = sha256 {
                    if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err("download sha256 must be 64 hex characters".into());
                    }
                } else if source.is_custom() {
                    return Err("custom app downloads require sha256".into());
                }
            }
            InstallStep::Extract {
                file,
                strip: _,
                into,
                ..
            } => {
                if !safe_relative(Path::new(file)) {
                    return Err("extract file must stay inside the app directory".into());
                }
                if !safe_relative(Path::new(into)) {
                    return Err("extract into must stay inside the app directory".into());
                }
            }
            InstallStep::Run {
                command,
                args,
                cwd,
                env,
                timeout_ms,
            } => {
                if command.trim().is_empty() {
                    return Err("run command is empty".into());
                }
                if args.iter().any(|a| a.contains('\0')) {
                    return Err("run args contain NUL".into());
                }
                if let Some(cwd) = cwd {
                    if cwd != "app" && cwd != "workspace" {
                        return Err("run cwd must be app or workspace".into());
                    }
                }
                if *timeout_ms == 0 || *timeout_ms > 3_600_000 {
                    return Err("run timeout_ms must be between 1 and 3600000".into());
                }
                if env.keys().any(|k| k.is_empty() || k.contains('=')) {
                    return Err("run env keys are invalid".into());
                }
            }
        }
    }
    Ok(())
}

fn validate_launch(launch: &LaunchSpec, source: AppSource) -> Result<(), String> {
    match launch {
        LaunchSpec::Process {
            command,
            args,
            cwd,
            env,
            port,
            url,
            ready_path,
            timeout_ms,
        } => {
            if command.trim().is_empty() {
                return Err("launch.command is empty".into());
            }
            if args.iter().any(|a| a.contains('\0')) {
                return Err("launch args contain NUL".into());
            }
            if let Some(cwd) = cwd {
                if cwd != "app" && cwd != "workspace" {
                    return Err("launch.cwd must be app or workspace".into());
                }
            }
            if env.keys().any(|k| k.is_empty() || k.contains('=')) {
                return Err("launch env keys are invalid".into());
            }
            match port {
                PortSpec::Fixed(p) if *p == 0 => return Err("launch.port cannot be 0".into()),
                PortSpec::Auto(value) if value != "auto" => {
                    return Err("launch.port must be \"auto\" or a number".into());
                }
                _ => {}
            }
            let probe = url.replace("{port}", "1");
            let parsed = Url::parse(&probe).map_err(|e| format!("launch.url: {e}"))?;
            if parsed.scheme() != "http" {
                return Err("launch.url must use http".into());
            }
            if parsed.username() != "" || parsed.password().is_some() {
                return Err("launch.url must not contain credentials".into());
            }
            let host = parsed.host_str().unwrap_or_default();
            if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
                return Err("launch.url must point at loopback".into());
            }
            if let Some(path) = ready_path {
                if !path.starts_with('/') {
                    return Err("launch.ready_path must start with /".into());
                }
            }
            if *timeout_ms == 0 || *timeout_ms > 3_600_000 {
                return Err("launch.timeout_ms must be between 1 and 3600000".into());
            }
            Ok(())
        }
        LaunchSpec::Static { .. } => Ok(()),
        LaunchSpec::Builtin { .. } => {
            if source != AppSource::Builtin {
                return Err("launch.kind \"builtin\" is reserved for bundled apps".into());
            }
            Ok(())
        }
    }
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

/// Hash only the fields that can execute code or change a trust boundary.
pub fn definition_hash(def: &AppDefinition, source: AppSource) -> String {
    #[derive(Serialize)]
    struct Shape<'a> {
        source: &'static str,
        install: &'a InstallSpec,
        launch: &'a LaunchSpec,
        data: DataScope,
        single_instance: bool,
        permissions: &'a [Permission],
    }
    let shape = Shape {
        source: source.as_str(),
        install: &def.install,
        launch: &def.launch,
        data: def.data,
        single_instance: def.single_instance,
        permissions: &def.permissions,
    };
    let bytes = serde_json::to_vec(&shape).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn trust_preview(def: &AppDefinition, source: AppSource) -> String {
    let mut out = format!("source: {}\n", source.as_str());
    match def.install.kind {
        InstallKind::None => out.push_str("install: external (no commands)\n"),
        InstallKind::Steps => {
            out.push_str("install:\n");
            for step in &def.install.steps {
                match step {
                    InstallStep::Download { url, sha256, .. } => {
                        out.push_str(&format!("  download {url}\n"));
                        if let Some(hash) = sha256 {
                            out.push_str(&format!("  sha256 {hash}\n"));
                        }
                    }
                    InstallStep::Extract { file, format, .. } => {
                        out.push_str(&format!("  extract {file} ({format:?})\n"));
                    }
                    InstallStep::Run {
                        command,
                        args,
                        cwd,
                        env,
                        ..
                    } => {
                        out.push_str(&format!(
                            "  run {} {} (cwd: {})\n",
                            command,
                            args.join(" "),
                            cwd.as_deref().unwrap_or("app")
                        ));
                        for (key, value) in env {
                            out.push_str(&format!("    env {key}={value}\n"));
                        }
                    }
                }
            }
        }
    }
    match &def.launch {
        LaunchSpec::Process {
            command,
            args,
            cwd,
            env,
            url,
            ..
        } => {
            out.push_str(&format!(
                "launch: {} {} (cwd: {})\nurl: {}\n",
                command,
                args.join(" "),
                cwd.as_deref().unwrap_or("app"),
                url
            ));
            for (key, value) in env {
                out.push_str(&format!("  env {key}={value}\n"));
            }
        }
        LaunchSpec::Static { spa } => out.push_str(&format!("launch: static (spa: {spa})\n")),
        LaunchSpec::Builtin { view } => {
            out.push_str(&format!("launch: builtin ({})\n", view.as_str()))
        }
    }
    out.push_str(&format!(
        "data: {:?}; single_instance: {}; permissions: {}\n",
        def.data,
        def.single_instance,
        def.permissions
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_def(extra: &str) -> Result<AppDefinition, String> {
        let json = format!(
            r#"{{
                "name": "Notes",
                "description": "A tiny local notes UI.",
                {extra}
                "category": "general",
                "icon": "📝",
                "install": {{ "kind": "none", "installed": true }},
                "launch": {{ "kind": "static", "spa": false }}
            }}"#
        );
        let def: AppDefinition = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        validate_definition(&def, AppSource::Workspace, Path::new("."))?;
        Ok(def)
    }

    #[test]
    fn localized_descriptions_are_optional_and_bounded() {
        let def = workspace_def(r#""description_i18n": { "zh-CN": "一个极简的本地笔记界面。" },"#)
            .expect("localized description");
        assert_eq!(def.description_i18n["zh-CN"], "一个极简的本地笔记界面。");

        assert!(workspace_def(r#""description_i18n": { "zh-CN": "  " },"#).is_err());
        let long = format!(
            r#""description_i18n": {{ "zh-CN": "{}" }},"#,
            "长".repeat(241)
        );
        assert!(workspace_def(&long).is_err());
    }
}
