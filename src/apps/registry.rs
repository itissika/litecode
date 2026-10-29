//! Merges the built-in catalog with global and workspace registry files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::catalog;
use super::schema::{
    definition_hash, trust_preview, validate_definition, validate_id, AppDefinition, AppSource,
    DataScope, InstallKind, InstallScope, RegistryFile,
};
use crate::config::path::strip_verbatim;

#[derive(Debug, Clone, Serialize)]
pub struct RegistryError {
    pub source: String,
    pub file: String,
    pub app_id: Option<String>,
    pub message: String,
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
    pub install_scope: InstallScope,
}

#[derive(Debug, Clone)]
pub struct RegistrySnapshot {
    pub apps: BTreeMap<String, RegisteredApp>,
    pub errors: Vec<RegistryError>,
    pub paths: RegistryPaths,
}

#[derive(Debug, Clone)]
pub struct CatalogApp {
    pub id: String,
    pub definition: AppDefinition,
    pub source: AppSource,
    pub install_scope: InstallScope,
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

    pub fn install_dir(&self, app: &RegisteredApp) -> PathBuf {
        match app.install_scope {
            InstallScope::Global => self.apps_root.join(&app.id),
            InstallScope::Workspace => self
                .apps_root
                .join("ws")
                .join(&self.workspace_hash)
                .join(&app.id),
        }
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

    pub fn data_dir(&self, app: &RegisteredApp, install_dir: &Path) -> PathBuf {
        match app.definition.data {
            DataScope::Workspace => self.workspace_data_dir(&app.id),
            DataScope::App => self.app_data_dir(install_dir),
        }
    }

    /// A stable per-workspace hash used by install paths and view partitions.
    pub fn workspace_hash(&self) -> &str {
        &self.workspace_hash
    }

    pub fn load(&self) -> RegistrySnapshot {
        let mut apps = BTreeMap::new();
        let mut errors = Vec::new();

        for (id, definition) in catalog::builtins() {
            if let Err(message) = validate_id(id) {
                errors.push(RegistryError {
                    source: "builtin".into(),
                    file: "builtin:catalog".into(),
                    app_id: Some(id.into()),
                    message,
                });
                continue;
            }
            if let Err(message) =
                validate_definition(&definition, AppSource::Builtin, &self.workspace_root)
            {
                errors.push(RegistryError {
                    source: "builtin".into(),
                    file: "builtin:catalog".into(),
                    app_id: Some(id.into()),
                    message,
                });
                continue;
            }
            apps.insert(
                id.to_string(),
                RegisteredApp {
                    id: id.to_string(),
                    definition,
                    source: AppSource::Builtin,
                    install_scope: InstallScope::Global,
                },
            );
        }

        self.load_file(
            self.global_file(),
            AppSource::Global,
            InstallScope::Global,
            &mut apps,
            &mut errors,
        );
        self.load_file(
            self.workspace_file(),
            AppSource::Workspace,
            InstallScope::Workspace,
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
        scope: InstallScope,
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
                errors.push(RegistryError {
                    source: source.as_str().into(),
                    file,
                    app_id: None,
                    message: format!("cannot read registry: {err}"),
                });
                return;
            }
        };
        let parsed: RegistryFile = match serde_json::from_str(&text) {
            Ok(parsed) => parsed,
            Err(err) => {
                errors.push(RegistryError {
                    source: source.as_str().into(),
                    file,
                    app_id: None,
                    message: format!("invalid JSON: {err}"),
                });
                return;
            }
        };
        if parsed.version != 1 {
            errors.push(RegistryError {
                source: source.as_str().into(),
                file,
                app_id: None,
                message: format!("unsupported version {} (expected 1)", parsed.version),
            });
            return;
        }
        for (id, definition) in parsed.apps {
            if let Err(message) = validate_id(&id) {
                errors.push(RegistryError {
                    source: source.as_str().into(),
                    file: file.clone(),
                    app_id: Some(id),
                    message,
                });
                continue;
            }
            if let Err(message) = validate_definition(&definition, source, &self.workspace_root) {
                errors.push(RegistryError {
                    source: source.as_str().into(),
                    file: file.clone(),
                    app_id: Some(id),
                    message,
                });
                continue;
            }
            if apps.contains_key(&id) {
                errors.push(RegistryError {
                    source: source.as_str().into(),
                    file: file.clone(),
                    app_id: Some(id),
                    message: "duplicate app id; lower-priority entry ignored".into(),
                });
                continue;
            }
            apps.insert(
                id.clone(),
                RegisteredApp {
                    id,
                    definition,
                    source,
                    install_scope: scope,
                },
            );
        }
    }
}

impl RegisteredApp {
    pub fn install_dir(&self, registry: &AppRegistry) -> PathBuf {
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

    pub fn external_installed(&self) -> bool {
        self.definition.install.kind == InstallKind::None && self.definition.install.installed
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
