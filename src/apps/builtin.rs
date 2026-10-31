//! Bundled apps only: the managed one-click installer. Nothing here is part
//! of the registry contract a user writes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use super::schema::{AppDefinition, LaunchSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallSpec {
    #[serde(default)]
    pub steps: Vec<InstallStep>,
    #[serde(default)]
    pub check: Option<CheckSpec>,
}

impl InstallSpec {
    /// True when LiteCode downloads or runs something to install this app.
    pub fn manages_files(&self) -> bool {
        !self.steps.is_empty()
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
    /// Ship a small text file the bundled definition carries inline, such as
    /// the entry page for a static site whose release ships assets only.
    Write {
        file: String,
        contents: String,
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

/// Where a bundled app keeps user data. Registered apps decide that themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinDataScope {
    Workspace,
    App,
}

impl Default for BuiltinDataScope {
    fn default() -> Self {
        Self::Workspace
    }
}

/// Offline documentation shipped with a bundled app, in both UI languages.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinDocs {
    pub en: &'static str,
    pub zh: &'static str,
}

/// What only a bundled app has: an optional managed install, a data scope, and
/// offline documentation.
#[derive(Debug, Clone)]
pub struct BuiltinMeta {
    pub install: Option<InstallSpec>,
    pub data: BuiltinDataScope,
    pub docs: BuiltinDocs,
}

impl BuiltinMeta {
    pub fn managed_install(&self) -> Option<&InstallSpec> {
        self.install.as_ref().filter(|spec| spec.manages_files())
    }
}

#[derive(Debug, Clone)]
pub struct BuiltinApp {
    pub id: &'static str,
    pub definition: AppDefinition,
    pub install: Option<InstallSpec>,
    pub data: BuiltinDataScope,
    pub docs: BuiltinDocs,
}

impl BuiltinApp {
    pub fn meta(&self) -> BuiltinMeta {
        BuiltinMeta {
            install: self.install.clone(),
            data: self.data,
            docs: self.docs,
        }
    }
}

pub fn validate_install(install: &InstallSpec) -> Result<(), String> {
    if let Some(check) = &install.check {
        if check.path.is_empty() || !safe_relative(&check.path) {
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
                if !safe_relative(file) {
                    return Err("download file must stay inside the app directory".into());
                }
                match sha256 {
                    Some(hash)
                        if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) => {}
                    Some(_) => return Err("download sha256 must be 64 hex characters".into()),
                    None => return Err("bundled downloads must pin sha256".into()),
                }
            }
            InstallStep::Extract {
                file,
                strip: _,
                into,
                ..
            } => {
                if !safe_relative(file) {
                    return Err("extract file must stay inside the app directory".into());
                }
                if !safe_relative(into) {
                    return Err("extract into must stay inside the app directory".into());
                }
            }
            InstallStep::Write { file, contents } => {
                if !safe_relative(file) {
                    return Err("write file must stay inside the app directory".into());
                }
                if contents.is_empty() {
                    return Err("write contents are empty".into());
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

/// Bundled definitions may still use the install directory tokens; this is the
/// only place they are meaningful.
pub fn validate_builtin_launch(launch: &LaunchSpec) -> Result<(), String> {
    match launch {
        LaunchSpec::Process { cwd, .. } => {
            if !matches!(cwd.as_str(), "{appdir}" | "{workspace}") {
                return Err("bundled launch.cwd must be \"{appdir}\" or \"{workspace}\"".into());
            }
            Ok(())
        }
        LaunchSpec::Static { dir, .. } => {
            if dir != "{appdir}" {
                return Err("bundled launch.dir must be \"{appdir}\"".into());
            }
            Ok(())
        }
        LaunchSpec::Builtin { .. } => Ok(()),
    }
}

fn safe_relative(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}
