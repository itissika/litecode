//! Merges the built-in catalog with global and workspace registry files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::builtin::{BuiltinDataScope, BuiltinMeta, validate_builtin_launch, validate_install};
use super::catalog;
use super::schema::{
    AppDefinition, AppSource, definition_hash, parse_registry_file, trust_preview,
    validate_definition, validate_id,
};
use crate::config::path::strip_verbatim;

#[derive(Debug, Clone, Serialize)]
pub struct RegistryError {
    /// "error" keeps the entry out; "warning" only needs to be shown.
    pub severity: &'static str,
    pub source: String,
    pub file: String,
    pub app_id: Option<String>,
    pub message: String,
}

impl RegistryError {
    fn error(source: &str, file: &str, app_id: Option<String>, message: String) -> Self {
        Self {
            severity: "error",
            source: source.into(),
            file: file.into(),
            app_id,
            message,
        }
    }

    fn warning(source: &str, file: &str, app_id: Option<String>, message: String) -> Self {
        Self {
            severity: "warning",
            source: source.into(),
            file: file.into(),
            app_id,
            message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RegistryPaths {
    pub global: String,
    pub workspace: String,
}

#[derive(Debug, Clone)]
pub struct RegisteredApp {
    pub id: String,
    pub definition: AppDefinition,
    pub source: AppSource,
    /// Present only for bundled apps: their managed install and data scope.
    pub builtin: Option<BuiltinMeta>,
}

impl RegisteredApp {
    pub fn builtin_meta(&self) -> Option<&BuiltinMeta> {
        self.builtin.as_ref()
    }

    pub fn managed_install(&self) -> Option<&super::builtin::InstallSpec> {
        self.builtin
            .as_ref()
            .and_then(|meta| meta.managed_install())
    }
}

#[derive(Debug, Clone)]
pub struct RegistrySnapshot {
    pub apps: BTreeMap<String, RegisteredApp>,
    pub errors: Vec<RegistryError>,
    pub paths: RegistryPaths,
}

#[derive(Debug, Clone)]
pub struct AppRegistry {
    pub workspace_root: PathBuf,
    pub global_data: PathBuf,
    pub apps_root: PathBuf,
    pub workspace_hash: String,
}

impl AppRegistry {
    pub fn new(workspace_root: PathBuf, global_data: PathBuf) -> Self {
        let workspace_hash = workspace_hash(&workspace_root);
        let apps_root = global_data.join("apps");
        Self {
            workspace_root,
            global_data,
            apps_root,
            workspace_hash,
        }
    }

    pub fn paths(&self) -> RegistryPaths {
        RegistryPaths {
            global: self.global_file().to_string_lossy().to_string(),
            workspace: self.workspace_file().to_string_lossy().to_string(),
        }
    }

    pub fn global_file(&self) -> PathBuf {
        self.global_data.join("apps.json")
    }

    pub fn workspace_file(&self) -> PathBuf {
        self.workspace_root.join(".litecode").join("apps.json")
    }

    pub fn trust_file(&self) -> PathBuf {
        self.global_data.join("app-trust.json")
    }

    pub fn view_key(&self, app_id: &str) -> String {
        format!("{}:{}", self.workspace_hash, app_id)
    }

    /// Bundled apps own a directory under the global data root. A registered
    /// app keeps its files wherever the user put them.
    pub fn install_dir(&self, app: &RegisteredApp) -> Option<PathBuf> {
        app.builtin.as_ref().map(|_| self.apps_root.join(&app.id))
    }

    pub fn workspace_data_dir(&self, app_id: &str) -> PathBuf {
        self.workspace_root
            .join(".litecode")
            .join("apps")
            .join(app_id)
    }

    pub fn app_data_dir(&self, install_dir: &Path) -> PathBuf {
        install_dir.join("data")
    }

    pub fn data_dir(&self, app: &RegisteredApp, install_dir: &Path) -> Option<PathBuf> {
        let meta = app.builtin.as_ref()?;
        Some(match meta.data {
            BuiltinDataScope::Workspace => self.workspace_data_dir(&app.id),
            BuiltinDataScope::App => self.app_data_dir(install_dir),
        })
    }

    /// A stable per-workspace hash used by runtime markers and view partitions.
    pub fn workspace_hash(&self) -> &str {
        &self.workspace_hash
    }

    pub fn load(&self) -> RegistrySnapshot {
        let mut apps = BTreeMap::new();
        let mut errors = Vec::new();

        for builtin in catalog::builtins() {
            let id = builtin.id;
            if let Err(message) = validate_id(id) {
                errors.push(RegistryError::error(
                    "builtin",
                    "builtin:catalog",
                    Some(id.into()),
                    message,
                ));
                continue;
            }
            if let Err(message) = validate_definition(
                &builtin.definition,
                AppSource::Builtin,
                &self.workspace_root,
            ) {
                errors.push(RegistryError::error(
                    "builtin",
                    "builtin:catalog",
                    Some(id.into()),
                    message,
                ));
                continue;
            }
            if let Err(message) = validate_builtin_launch(&builtin.definition.launch) {
                errors.push(RegistryError::error(
                    "builtin",
                    "builtin:catalog",
                    Some(id.into()),
                    message,
                ));
                continue;
            }
            if let Some(install) = &builtin.install {
                if let Err(message) = validate_install(install) {
                    errors.push(RegistryError::error(
                        "builtin",
                        "builtin:catalog",
                        Some(id.into()),
                        message,
                    ));
                    continue;
                }
            }
            apps.insert(
                id.to_string(),
                RegisteredApp {
                    id: id.to_string(),
                    definition: builtin.definition.clone(),
                    source: AppSource::Builtin,
                    builtin: Some(builtin.meta()),
                },
            );
        }

        self.load_file(
            self.global_file(),
            AppSource::Global,
            &mut apps,
            &mut errors,
        );
        self.load_file(
            self.workspace_file(),
            AppSource::Workspace,
            &mut apps,
            &mut errors,
        );

        RegistrySnapshot {
            apps,
            errors,
            paths: self.paths(),
        }
    }

    fn load_file(
        &self,
        path: PathBuf,
        source: AppSource,
        apps: &mut BTreeMap<String, RegisteredApp>,
        errors: &mut Vec<RegistryError>,
    ) {
        let file = path.to_string_lossy().to_string();
        if !path.exists() {
            return;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                errors.push(RegistryError::error(
                    source.as_str(),
                    &file,
                    None,
                    format!("cannot read registry: {err}"),
                ));
                return;
            }
        };
        let parsed = match parse_registry_file(&text) {
            Ok(parsed) => parsed,
            Err(message) => {
                errors.push(RegistryError::error(source.as_str(), &file, None, message));
                return;
            }
        };
        for (app_id, message) in parsed.errors {
            errors.push(RegistryError::error(
                source.as_str(),
                &file,
                app_id,
                message,
            ));
        }
        for (app_id, message) in parsed.warnings {
            errors.push(RegistryError::warning(
                source.as_str(),
                &file,
                app_id,
                message,
            ));
        }
        for (id, definition) in parsed.apps {
            if let Err(message) = validate_id(&id) {
                errors.push(RegistryError::error(
                    source.as_str(),
                    &file,
                    Some(id),
                    message,
                ));
                continue;
            }
            if let Err(message) = validate_definition(&definition, source, &self.workspace_root) {
                errors.push(RegistryError::error(
                    source.as_str(),
                    &file,
                    Some(id),
                    message,
                ));
                continue;
            }
            if apps.contains_key(&id) {
                errors.push(RegistryError::error(
                    source.as_str(),
                    &file,
                    Some(id),
                    "duplicate app id; lower-priority entry ignored".into(),
                ));
                continue;
            }
            apps.insert(
                id.clone(),
                RegisteredApp {
                    id,
                    definition,
                    source,
                    builtin: None,
                },
            );
        }
    }
}

impl RegisteredApp {
    pub fn install_dir(&self, registry: &AppRegistry) -> Option<PathBuf> {
        registry.install_dir(self)
    }

    pub fn definition_hash(&self) -> Option<String> {
        self.source
            .is_custom()
            .then(|| definition_hash(&self.definition, self.source))
    }

    pub fn trust_preview(&self) -> Option<String> {
        self.source
            .is_custom()
            .then(|| trust_preview(&self.definition, self.source))
    }
}

pub fn workspace_hash(workspace_root: &Path) -> String {
    let canonical = std::fs::canonicalize(workspace_root)
        .map(|p| strip_verbatim(p))
        .unwrap_or_else(|_| strip_verbatim(workspace_root.to_path_buf()));
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    format!("{:x}", digest)[..12].to_string()
}
