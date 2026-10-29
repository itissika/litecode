//! The public app contract: a registry entry names a web app and how to launch
//! it. Everything else (dependencies, data, logs) belongs to the app.

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

/// NotInstalled through Uninstalling only appear for bundled apps that ship a
/// managed install. Every other app is Stopped until the user starts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    NotInstalled,
    Installing,
    Installed,
    Starting,
    Running,
    Stopping,
    Stopped,
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
            Self::Stopped => "stopped",
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
    /// One-line descriptions keyed by locale (for example zh-CN).
    #[serde(default)]
    pub description_i18n: BTreeMap<String, String>,
    pub category: AppCategory,
    pub icon: String,
    /// Official repository. Shown in the panel; never executed.
    #[serde(default)]
    pub repo: Option<String>,
    /// Optional official docs link. Shown only when present.
    #[serde(default)]
    pub docs: Option<String>,
    pub launch: LaunchSpec,
    #[serde(default)]
    pub single_instance: bool,
    #[serde(default)]
    pub permissions: Vec<Permission>,
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
        #[serde(default = "default_cwd")]
        cwd: String,
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
        dir: String,
        #[serde(default)]
        spa: bool,
    },
}

fn default_cwd() -> String {
    "{workspace}".into()
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn fixed(&self) -> Option<u16> {
        match self {
            Self::Fixed(port) => Some(*port),
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
    validate_description_i18n(&def.description_i18n)?;
    validate_icon(&def.icon, source, workspace_root)?;
    if let Some(repo) = &def.repo {
        validate_https_link("repo", repo)?;
    }
    if let Some(docs) = &def.docs {
        validate_https_link("docs", docs)?;
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

fn validate_https_link(field: &str, value: &str) -> Result<(), String> {
    let parsed = Url::parse(value).map_err(|e| format!("{field}: {e}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(format!("{field} must be an absolute https URL"));
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

/// The bundled catalog points at its own install directory with these tokens.
/// A registered app has neither, so it must not use them.
fn reject_bundled_tokens(field: &str, value: &str) -> Result<(), String> {
    for token in ["{appdir}", "{data}"] {
        if value.contains(token) {
            return Err(format!(
                "launch.{field} uses {token}, which is reserved for bundled apps"
            ));
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
            if cwd.trim().is_empty() {
                return Err("launch.cwd is empty".into());
            }
            if source.is_custom() {
                if cwd == "app" {
                    return Err(
                        "launch.cwd \"app\" is no longer supported; use \"{workspace}\" or a path relative to the workspace"
                            .into(),
                    );
                }
                reject_bundled_tokens("command", command)?;
                reject_bundled_tokens("cwd", cwd)?;
                for arg in args {
                    reject_bundled_tokens("args", arg)?;
                }
                for value in env.values() {
                    reject_bundled_tokens("env", value)?;
                }
                reject_bundled_tokens("url", url)?;
                if let Some(path) = ready_path {
                    reject_bundled_tokens("ready_path", path)?;
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
        LaunchSpec::Static { dir, .. } => {
            if dir.trim().is_empty() {
                return Err("launch.dir is empty".into());
            }
            if source.is_custom() {
                reject_bundled_tokens("dir", dir)?;
                if dir == "app" {
                    return Err(
                        "launch.dir \"app\" is no longer supported; point it at a directory in the workspace"
                            .into(),
                    );
                }
            }
            Ok(())
        }
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
        launch: &'a LaunchSpec,
        single_instance: bool,
        permissions: &'a [Permission],
    }
    let shape = Shape {
        source: source.as_str(),
        launch: &def.launch,
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
    match &def.launch {
        LaunchSpec::Process {
            command,
            args,
            cwd,
            env,
            url,
            ready_path,
            ..
        } => {
            out.push_str(&format!(
                "run: {} {}\ncwd: {}\nurl: {}\n",
                command,
                args.join(" "),
                cwd,
                url
            ));
            if let Some(path) = ready_path {
                out.push_str(&format!("ready: {path}\n"));
            }
            for (key, value) in env {
                out.push_str(&format!("env {key}={value}\n"));
            }
        }
        LaunchSpec::Static { dir, spa } => {
            out.push_str(&format!("static: {dir} (spa: {spa})\n"));
        }
        LaunchSpec::Builtin { view } => out.push_str(&format!("builtin: {}\n", view.as_str())),
    }
    out.push_str(&format!(
        "single_instance: {}; permissions: {}\n",
        def.single_instance,
        def.permissions
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    out
}

// ---------------------------------------------------------------------------
// Registry parsing (version 2, with a read-only path for version 1)
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ParsedRegistry {
    pub apps: Vec<(String, AppDefinition)>,
    /// Definition-level problems: the rest of the file still loads.
    pub errors: Vec<(Option<String>, String)>,
    /// Loaded, but worth telling the user about.
    pub warnings: Vec<(Option<String>, String)>,
}

/// Parse a registry file. A broken app is reported per entry; only a broken
/// file (not JSON, unknown version) is a whole-file error.
pub fn parse_registry_file(text: &str) -> Result<ParsedRegistry, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let version = value
        .get("version")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let Some(apps) = value.get("apps").and_then(|apps| apps.as_object()) else {
        return Err("apps must be an object".into());
    };
    let mut parsed = ParsedRegistry::default();
    for (id, raw) in apps {
        let definition = match version {
            2 => serde_json::from_value::<AppDefinition>(raw.clone())
                .map_err(|err| format!("invalid definition: {err}")),
            1 => serde_json::from_value::<LegacyAppDefinition>(raw.clone())
                .map_err(|err| format!("invalid definition: {err}"))
                .and_then(LegacyAppDefinition::into_definition),
            other => return Err(format!("unsupported version {other} (expected 1 or 2)")),
        };
        match definition {
            Ok(definition) => parsed.apps.push((id.clone(), definition)),
            Err(message) => parsed.errors.push((Some(id.clone()), message)),
        }
    }
    if version == 1 {
        parsed.warnings.push((
            None,
            "registry version 1 is deprecated; rewrite it as version 2 (install, data, summary, homepage, and readme_url are ignored)"
                .into(),
        ));
    }
    Ok(parsed)
}

#[derive(Debug, Deserialize)]
struct LegacyAppDefinition {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    description_i18n: BTreeMap<String, String>,
    category: AppCategory,
    icon: String,
    launch: LegacyLaunch,
    #[serde(default)]
    single_instance: bool,
    #[serde(default)]
    permissions: Vec<Permission>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LegacyLaunch {
    Builtin {
        view: BuiltinView,
    },
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

impl LegacyAppDefinition {
    fn into_definition(self) -> Result<AppDefinition, String> {
        let launch = match self.launch {
            LegacyLaunch::Builtin { view } => LaunchSpec::Builtin { view },
            LegacyLaunch::Process {
                command,
                args,
                cwd,
                env,
                port,
                url,
                ready_path,
                timeout_ms,
            } => {
                let cwd = match cwd.as_deref() {
                    None | Some("") | Some("workspace") => "{workspace}".to_string(),
                    Some("app") => {
                        return Err(
                            "launch.cwd \"app\" is no longer supported; use \"{workspace}\" or a path relative to the workspace"
                                .into(),
                        )
                    }
                    Some(value) => value.to_string(),
                };
                LaunchSpec::Process {
                    command,
                    args,
                    cwd,
                    env,
                    port,
                    url,
                    ready_path,
                    timeout_ms,
                }
            }
            LegacyLaunch::Static { .. } => {
                return Err(
                    "launch.kind \"static\" now needs \"dir\": point it at the directory that holds index.html"
                        .into(),
                )
            }
        };
        Ok(AppDefinition {
            name: self.name,
            description: self.description,
            description_i18n: self.description_i18n,
            category: self.category,
            icon: self.icon,
            repo: None,
            docs: None,
            launch,
            single_instance: self.single_instance,
            permissions: self.permissions,
        })
    }
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
                "icon": "N",
                "launch": {{ "kind": "static", "dir": "site" }}
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

    #[test]
    fn version_two_parses_a_minimal_app() {
        let parsed = parse_registry_file(
            r#"{
                "version": 2,
                "apps": {
                    "notes": {
                        "name": "Notes",
                        "description": "Local notes.",
                        "category": "general",
                        "icon": "N",
                        "launch": { "kind": "process", "command": "npx", "args": ["serve"], "url": "http://127.0.0.1:{port}/" }
                    }
                }
            }"#,
        )
        .expect("parse");
        assert_eq!(parsed.apps.len(), 1);
        assert!(parsed.errors.is_empty());
        assert!(parsed.warnings.is_empty());
        match &parsed.apps[0].1.launch {
            LaunchSpec::Process { cwd, port, .. } => {
                assert_eq!(cwd, "{workspace}");
                assert_eq!(*port, PortSpec::Auto("auto".into()));
            }
            other => panic!("unexpected launch {other:?}"),
        }
    }

    #[test]
    fn version_two_rejects_the_removed_install_block() {
        let parsed = parse_registry_file(
            r#"{
                "version": 2,
                "apps": {
                    "notes": {
                        "name": "Notes",
                        "description": "Local notes.",
                        "category": "general",
                        "icon": "N",
                        "install": { "kind": "none" },
                        "launch": { "kind": "static", "dir": "site" }
                    }
                }
            }"#,
        )
        .expect("parse");
        assert!(parsed.apps.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(
            parsed.errors[0].1.contains("install"),
            "{:?}",
            parsed.errors
        );
    }

    #[test]
    fn version_one_is_converted_with_a_warning() {
        let parsed = parse_registry_file(
            r#"{
                "version": 1,
                "apps": {
                    "notes": {
                        "name": "Notes",
                        "description": "Local notes.",
                        "category": "general",
                        "icon": "N",
                        "summary": "ignored",
                        "install": { "kind": "none", "installed": true },
                        "data": "app",
                        "launch": { "kind": "process", "command": "node", "args": ["server.js"], "cwd": "workspace", "url": "http://127.0.0.1:{port}/" }
                    }
                }
            }"#,
        )
        .expect("parse");
        assert_eq!(parsed.apps.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        match &parsed.apps[0].1.launch {
            LaunchSpec::Process { cwd, .. } => assert_eq!(cwd, "{workspace}"),
            other => panic!("unexpected launch {other:?}"),
        }
    }

    #[test]
    fn version_one_reports_the_app_directory_cwd() {
        let parsed = parse_registry_file(
            r#"{
                "version": 1,
                "apps": {
                    "notes": {
                        "name": "Notes",
                        "description": "Local notes.",
                        "category": "general",
                        "icon": "N",
                        "launch": { "kind": "process", "command": "node", "cwd": "app", "url": "http://127.0.0.1:{port}/" }
                    }
                }
            }"#,
        )
        .expect("parse");
        assert!(parsed.apps.is_empty());
        assert_eq!(parsed.errors.len(), 1);
        assert!(
            parsed.errors[0].1.contains("launch.cwd"),
            "{:?}",
            parsed.errors
        );
    }

    #[test]
    fn bundled_tokens_are_rejected_for_registered_apps() {
        let def: AppDefinition = serde_json::from_str(
            r#"{
                "name": "Notes",
                "description": "A tiny local notes UI.",
                "category": "general",
                "icon": "N",
                "launch": {
                    "kind": "process",
                    "command": "node",
                    "args": ["{data}/x.js"],
                    "url": "http://127.0.0.1:{port}/"
                }
            }"#,
        )
        .expect("definition");
        let err = validate_definition(&def, AppSource::Workspace, Path::new("."))
            .expect_err("data token");
        assert!(err.contains("{data}"), "{err}");
    }
}
