//! App service: one merged view of the registry and the runtime.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::Serialize;

use super::install::{self, InstallMarker};
use super::recommend;
use super::registry::{AppRegistry, RegisteredApp, RegistryPaths, RegistrySnapshot};
use super::runtime::{self, AppRuntime, LaunchContext};
use super::schema::{AppDefinition, AppSource, AppStatus, LaunchSpec};
use super::trust;

#[derive(Debug)]
pub enum AppError {
    NotFound,
    BadRequest(String),
    Forbidden(String),
    Conflict(String),
    Internal(String),
}

impl AppError {
    pub fn message(&self) -> &str {
        match self {
            Self::NotFound => "app not found",
            Self::BadRequest(message)
            | Self::Forbidden(message)
            | Self::Conflict(message)
            | Self::Internal(message) => message,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgressView {
    pub phase: String,
    pub percent: Option<u8>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallErrorView {
    pub message: String,
    pub log_path: Option<String>,
    pub tail: Option<String>,
}

/// Managed install state. Only bundled apps that ship one have this at all;
/// a registered app is never "installed", it simply starts.
#[derive(Debug, Clone, Serialize)]
pub struct InstallStateView {
    pub installed: bool,
    pub can_install: bool,
    pub can_uninstall: bool,
    pub progress: Option<InstallProgressView>,
    pub error: Option<InstallErrorView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocsView {
    pub en: String,
    pub zh: String,
}

/// A recommendation the user has not registered yet. It is metadata only:
/// there is no launch to run and nothing for LiteCode to install.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendationView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub description_i18n: BTreeMap<String, String>,
    pub category: super::schema::AppCategory,
    pub icon: String,
    pub repo: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub id: String,
    pub name: String,
    pub description: String,
    /// One-line descriptions keyed by locale; `description` is the fallback.
    pub description_i18n: BTreeMap<String, String>,
    pub category: super::schema::AppCategory,
    pub icon: String,
    /// Official repository and optional docs link; both may be absent.
    pub repo: Option<String>,
    pub docs_url: Option<String>,
    /// Offline documentation for a bundled app, in both UI languages.
    pub docs: Option<DocsView>,
    pub source: AppSource,
    pub view_key: String,
    pub status: AppStatus,
    pub running: bool,
    pub url: Option<String>,
    /// A panel LiteCode paints itself (currently only the Gallery).
    pub builtin_view: Option<&'static str>,
    pub port: Option<u16>,
    pub can_start: bool,
    pub can_open: bool,
    pub trusted: bool,
    pub definition_hash: Option<String>,
    pub trust_preview: Option<String>,
    pub permissions: Vec<String>,
    pub single_instance: bool,
    pub install: Option<InstallStateView>,
    pub runtime_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppList {
    pub apps: Vec<AppView>,
    /// Unregistered external recommendations, newest catalog first.
    pub recommendations: Vec<RecommendationView>,
    pub registry_paths: RegistryPaths,
    pub registry_errors: Vec<super::registry::RegistryError>,
}

struct StartClaim {
    service: Arc<AppService>,
    id: String,
}

impl Drop for StartClaim {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = self.service.starting.lock() {
            in_flight.remove(&self.id);
        }
    }
}

pub struct AppService {
    registry: Arc<AppRegistry>,
    runtime: Arc<AppRuntime>,
    starting: Mutex<HashSet<String>>,
}

impl AppService {
    pub fn new(workspace_root: std::path::PathBuf, global_data: std::path::PathBuf) -> Arc<Self> {
        let registry = Arc::new(AppRegistry::new(workspace_root, global_data));
        install::cleanup_stale_staging(&registry.apps_root);
        let runtime = Arc::new(AppRuntime::new(Arc::clone(&registry)));
        Arc::new(Self {
            registry,
            runtime,
            starting: Mutex::new(HashSet::new()),
        })
    }

    pub fn registry(&self) -> &Arc<AppRegistry> {
        &self.registry
    }

    pub fn stop_all(&self) {
        self.runtime.stop_all();
    }

    pub fn list(&self) -> AppList {
        let snapshot = self.registry.load();
        let apps = snapshot.apps.values().map(|app| self.view(app)).collect();
        let recommendations = recommend::recommendations()
            .into_iter()
            .filter(|rec| !snapshot.apps.contains_key(rec.id))
            .map(|rec| RecommendationView {
                id: rec.id.to_string(),
                name: rec.name.to_string(),
                description: rec.description.to_string(),
                description_i18n: rec.description_i18n,
                category: rec.category,
                icon: rec.icon.to_string(),
                repo: rec.repo.to_string(),
            })
            .collect();
        AppList {
            apps,
            recommendations,
            registry_paths: snapshot.paths,
            registry_errors: snapshot.errors,
        }
    }

    fn view(&self, app: &RegisteredApp) -> AppView {
        let install_dir = self.registry.install_dir(app);
        let install_spec = app.builtin_meta().and_then(|meta| meta.install.as_ref());
        let marker = install_dir
            .as_deref()
            .filter(|_| install_spec.is_some())
            .and_then(read_live_marker);
        let installed = self.is_installed(app, install_dir.as_deref());
        let runtime = self.runtime.view(&app.id);
        let trusted = self.is_trusted(app);
        let status = self.status(install_spec.is_some(), installed, &marker, &runtime);
        let running = status == AppStatus::Running;
        let builtin_view = match app.definition.launch {
            LaunchSpec::Builtin { view } => Some(view.as_str()),
            _ => None,
        };
        let active = matches!(
            status,
            AppStatus::Running | AppStatus::Starting | AppStatus::Stopping
        );
        let can_start = installed && trusted && builtin_view.is_none() && !active;
        let can_open = builtin_view.is_some() || (running && runtime.url.is_some());
        let install = install_spec.map(|spec| {
            let progress = marker
                .as_ref()
                .filter(|m| m.status == "installing")
                .map(|m| InstallProgressView {
                    phase: m.phase.clone(),
                    percent: m.percent,
                    message: m.message.clone(),
                });
            let error = marker.as_ref().filter(|m| m.status == "error").map(|m| {
                let log_path = m.log_path.clone();
                let tail = log_path
                    .as_deref()
                    .and_then(|p| install::log_tail(Path::new(p)));
                InstallErrorView {
                    message: m.error.clone().unwrap_or_else(|| "install failed".into()),
                    log_path,
                    tail,
                }
            });
            InstallStateView {
                installed,
                can_install: spec.manages_files() && !installed && trusted && !active,
                can_uninstall: spec.manages_files() && installed && !active,
                progress,
                error,
            }
        });
        AppView {
            id: app.id.clone(),
            name: app.definition.name.clone(),
            description: app.definition.description.clone(),
            description_i18n: app.definition.description_i18n.clone(),
            category: app.definition.category,
            icon: self.icon(&app.definition),
            repo: app.definition.repo.clone(),
            docs_url: app.definition.docs.clone(),
            docs: app.builtin_meta().map(|meta| DocsView {
                en: meta.docs.en.to_string(),
                zh: meta.docs.zh.to_string(),
            }),
            source: app.source,
            view_key: self.registry.view_key(&app.id),
            status,
            running,
            url: runtime.url.clone(),
            builtin_view,
            port: runtime.port,
            can_start,
            can_open,
            trusted,
            definition_hash: app.definition_hash(),
            trust_preview: app.trust_preview(),
            permissions: app
                .definition
                .permissions
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            single_instance: app.definition.single_instance,
            install,
            runtime_error: runtime.error.clone(),
        }
    }

    /// True when the app can start: bundled apps need their managed install to
    /// be present; registered apps have nothing to install.
    fn is_installed(&self, app: &RegisteredApp, install_dir: Option<&Path>) -> bool {
        let Some(meta) = app.builtin_meta() else {
            return true;
        };
        let Some(spec) = &meta.install else {
            return true;
        };
        let Some(install_dir) = install_dir else {
            return false;
        };
        if let Some(check) = &spec.check {
            return install::app_dir(install_dir).join(&check.path).exists();
        }
        install::manifest_path(install_dir).is_file()
    }

    fn status(
        &self,
        bundled_install: bool,
        installed: bool,
        marker: &Option<InstallMarker>,
        runtime: &runtime::RuntimeView,
    ) -> AppStatus {
        if let Some(marker) = marker {
            if marker.status == "installing" {
                return AppStatus::Installing;
            }
            if marker.status == "error" {
                return AppStatus::Error;
            }
        }
        match runtime.status {
            AppStatus::Running
            | AppStatus::Starting
            | AppStatus::Stopping
            | AppStatus::Crashed
            | AppStatus::Error => runtime.status,
            _ => {
                if !installed {
                    AppStatus::NotInstalled
                } else if bundled_install {
                    AppStatus::Installed
                } else {
                    AppStatus::Stopped
                }
            }
        }
    }

    fn is_trusted(&self, app: &RegisteredApp) -> bool {
        if app.source == AppSource::Builtin {
            return true;
        }
        let Some(hash) = app.definition_hash() else {
            return false;
        };
        trust::is_trusted(&self.registry, &app.id, &hash)
    }

    fn find(&self, id: &str) -> Result<(RegistrySnapshot, RegisteredApp), AppError> {
        let snapshot = self.registry.load();
        let app = snapshot.apps.get(id).cloned().ok_or(AppError::NotFound)?;
        Ok((snapshot, app))
    }

    pub async fn install(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if app.managed_install().is_none() {
            return Err(AppError::BadRequest(
                "this app has no managed install; start it directly".into(),
            ));
        }
        if !self.is_trusted(&app) {
            return Err(AppError::Forbidden(
                "this custom app must be trusted before install".into(),
            ));
        }
        let install_dir = self
            .registry
            .install_dir(&app)
            .ok_or_else(|| AppError::BadRequest("this app has no managed install".into()))?;
        if self.is_installed(&app, Some(&install_dir)) {
            return Err(AppError::Conflict("app is already installed".into()));
        }
        let registry = Arc::clone(&self.registry);
        let app_clone = app.clone();
        let outcome =
            tokio::task::spawn_blocking(move || install::install_blocking(&registry, &app_clone))
                .await
                .map_err(|e| AppError::Internal(format!("install task: {e}")))?;
        outcome.map_err(AppError::Internal)?;
        Ok(())
    }

    pub async fn start(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if !self.is_trusted(&app) {
            return Err(AppError::Forbidden(
                "this custom app must be trusted before start".into(),
            ));
        }
        if matches!(app.definition.launch, LaunchSpec::Builtin { .. }) {
            return Err(AppError::BadRequest(
                "this app opens a built-in panel; it does not start a process".into(),
            ));
        }
        let install_dir = self.registry.install_dir(&app);
        if !self.is_installed(&app, install_dir.as_deref()) {
            return Err(AppError::Conflict(
                "this bundled app needs its managed install first".into(),
            ));
        }
        let status = self.runtime.status(&app.id);
        if matches!(
            status,
            AppStatus::Running | AppStatus::Starting | AppStatus::Stopping
        ) {
            return Err(AppError::Conflict("app is already running".into()));
        }
        let claim = {
            let mut in_flight = self.starting.lock().expect("app start lock");
            if !in_flight.insert(app.id.clone()) {
                return Err(AppError::Conflict("app is already starting".into()));
            }
            StartClaim {
                service: Arc::clone(self),
                id: app.id.clone(),
            }
        };
        let ctx = self.launch_context(&app, install_dir.as_deref());
        let result: Result<(), AppError> = async {
            match &app.definition.launch {
                LaunchSpec::Process { .. } => {
                    let runtime = Arc::clone(&self.runtime);
                    let app_clone = app.clone();
                    let started = tokio::task::spawn_blocking(move || {
                        runtime.start_process(&app_clone, &ctx)
                    })
                    .await
                    .map_err(|e| AppError::Internal(format!("start task: {e}")))?;
                    started.map_err(AppError::Internal)?;
                }
                LaunchSpec::Static { .. } => {
                    self.runtime
                        .start_static(&app, &ctx)
                        .await
                        .map(|_| ())
                        .map_err(AppError::Internal)?;
                }
                LaunchSpec::Builtin { .. } => {
                    return Err(AppError::BadRequest(
                        "this app opens a built-in panel; it does not start a process".into(),
                    ));
                }
            }
            if let Some(install_dir) = install_dir.as_deref() {
                // A successful start supersedes a stale install-error marker.
                install::clear_marker(install_dir);
            }
            Ok(())
        }
        .await;
        drop(claim);
        result
    }

    fn launch_context(&self, app: &RegisteredApp, install_dir: Option<&Path>) -> LaunchContext {
        let workspace = self.registry.workspace_root.clone();
        match (install_dir, app.builtin_meta()) {
            (Some(dir), Some(_)) => LaunchContext {
                workspace,
                app_dir: Some(install::app_dir(dir)),
                data_dir: self.registry.data_dir(app, dir),
            },
            _ => LaunchContext::registered(workspace),
        }
    }

    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if self
            .starting
            .lock()
            .map(|in_flight| in_flight.contains(&app.id))
            .unwrap_or(false)
        {
            return Err(AppError::Conflict("app is starting".into()));
        }
        let runtime = Arc::clone(&self.runtime);
        tokio::task::spawn_blocking(move || runtime.stop(&app.id))
            .await
            .map_err(|e| AppError::Internal(format!("stop task: {e}")))?;
        Ok(())
    }

    pub async fn uninstall(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if app.managed_install().is_none() {
            return Err(AppError::BadRequest(
                "this app has no managed install".into(),
            ));
        }
        let install_dir = self
            .registry
            .install_dir(&app)
            .ok_or_else(|| AppError::BadRequest("this app has no managed install".into()))?;
        if install::install_in_progress(&install_dir) {
            return Err(AppError::Conflict("app is installing".into()));
        }
        if self
            .starting
            .lock()
            .map(|in_flight| in_flight.contains(&app.id))
            .unwrap_or(false)
        {
            return Err(AppError::Conflict("app is starting".into()));
        }
        let runtime = Arc::clone(&self.runtime);
        let app_id = app.id.clone();
        tokio::task::spawn_blocking(move || runtime.stop(&app_id))
            .await
            .map_err(|e| AppError::Internal(format!("stop task: {e}")))?;
        if runtime::has_live_run_marker(&self.registry, &app.id) {
            return Err(AppError::Conflict(
                "app is running in another workspace".into(),
            ));
        }
        let app_dir = install::app_dir(&install_dir);
        if app_dir.exists() {
            std::fs::remove_dir_all(&app_dir)
                .map_err(|e| AppError::Internal(format!("remove app: {e}")))?;
        }
        let _ = std::fs::remove_dir_all(install::logs_dir(&install_dir));
        let _ = std::fs::remove_file(install::manifest_path(&install_dir));
        install::clear_marker(&install_dir);
        // The data directory is preserved on purpose: uninstall removes the
        // runtime, never the user's data.
        Ok(())
    }

    pub fn trust(&self, id: &str, hash: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if app.source == AppSource::Builtin {
            return Err(AppError::BadRequest(
                "built-in apps are already trusted".into(),
            ));
        }
        let Some(current) = app.definition_hash() else {
            return Err(AppError::Internal("cannot hash app definition".into()));
        };
        if !current.eq_ignore_ascii_case(hash.trim()) {
            return Err(AppError::Conflict(
                "app definition changed; review the new trust preview".into(),
            ));
        }
        trust::trust_app(&self.registry, &app.id, &current).map_err(AppError::Internal)
    }

    fn icon(&self, definition: &AppDefinition) -> String {
        let Some(rel) = definition.icon.strip_prefix("file:") else {
            return definition.icon.clone();
        };
        let requested = self.registry.workspace_root.join(rel);
        let root = std::fs::canonicalize(&self.registry.workspace_root).ok();
        let path = std::fs::canonicalize(&requested).ok();
        let (Some(root), Some(path)) = (root, path) else {
            return "📦".into();
        };
        if !path.starts_with(&root) {
            return "📦".into();
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return "📦".into();
        };
        if bytes.len() > 64 * 1024 {
            return "📦".into();
        }
        let mime = match path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "jpg" | "jpeg" => "image/jpeg",
            _ => "application/octet-stream",
        };
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }
}

fn read_live_marker(install_dir: &Path) -> Option<InstallMarker> {
    let mut marker = install::read_marker(install_dir);
    if marker
        .as_ref()
        .is_some_and(|entry| entry.status == "installing")
        && !install::install_in_progress(install_dir)
    {
        install::clear_marker(install_dir);
        marker = None;
    }
    marker
}

/// Kept here so the API layer does not need to know about registry internals.
pub fn registry_paths(registry: &AppRegistry) -> RegistryPaths {
    registry.paths()
}

/// A small helper for tests and diagnostics.
pub fn source_str(source: AppSource) -> &'static str {
    source.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gallery_is_a_bundled_panel_not_a_process() {
        let dir = tempfile::tempdir().expect("temp dir");
        let service = AppService::new(dir.path().join("workspace"), dir.path().join("data"));
        let apps = service.list();
        assert!(
            apps.registry_errors.is_empty(),
            "catalog errors: {:?}",
            apps.registry_errors
        );
        let gallery = apps
            .apps
            .iter()
            .find(|app| app.id == "gallery")
            .expect("gallery app");
        assert!(gallery.install.is_none());
        assert_eq!(gallery.builtin_view, Some("gallery"));
        assert_eq!(
            gallery.description_i18n.get("zh-CN").map(String::as_str),
            Some("浏览工作区里的图片、视频和音频。")
        );
        assert_eq!(gallery.status, AppStatus::Stopped);
        assert!(gallery.can_open);
        assert!(!gallery.can_start);
        assert!(
            gallery
                .docs
                .as_ref()
                .is_some_and(|docs| docs.zh.contains("媒体"))
        );
    }

    #[test]
    fn builtins_ship_docs_and_never_overlap_recommendations() {
        let dir = tempfile::tempdir().expect("temp dir");
        let service = AppService::new(dir.path().join("workspace"), dir.path().join("data"));
        let list = service.list();
        for app in &list.apps {
            if app.source == AppSource::Builtin {
                assert!(app.docs.is_some(), "builtin {} has no offline docs", app.id);
            }
        }
        for rec in &list.recommendations {
            assert!(
                !list.apps.iter().any(|app| app.id == rec.id),
                "recommendation {} is also an app",
                rec.id
            );
        }
        assert!(list.recommendations.iter().any(|rec| rec.id == "comfyui"));
        assert!(list.apps.iter().all(|app| app.id != "comfyui"));
    }
}
