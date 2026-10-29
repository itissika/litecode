//! App service: one merged view of registry, install state, and runtime.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;

use super::install::{self, InstallMarker};
use super::readme::ReadmeCache;
use super::registry::{AppRegistry, RegisteredApp, RegistryError, RegistryPaths, RegistrySnapshot};
use super::runtime::{AppRuntime, RuntimeView};
use super::schema::{
    AppDefinition, AppSource, AppStatus, DataScope, InstallKind, InstallScope, LaunchSpec,
};
use super::space;
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

#[derive(Debug, Clone, Serialize)]
pub struct AppView {
    pub id: String,
    pub name: String,
    pub description: String,
    /// One-line descriptions keyed by locale; `description` is the fallback.
    pub description_i18n: BTreeMap<String, String>,
    pub summary: String,
    pub category: super::schema::AppCategory,
    pub icon: String,
    pub homepage: Option<String>,
    pub source: AppSource,
    pub install_scope: InstallScope,
    pub install_dir: String,
    pub data_dir: String,
    pub view_key: String,
    pub status: AppStatus,
    pub installed: bool,
    pub running: bool,
    pub url: Option<String>,
    /// A panel LiteCode paints itself (currently only the Gallery).
    pub builtin_view: Option<&'static str>,
    pub port: Option<u16>,
    pub required_bytes: Option<u64>,
    pub size_bytes: Option<u64>,
    pub can_install: bool,
    pub can_uninstall: bool,
    pub can_start: bool,
    pub can_open: bool,
    pub trusted: bool,
    pub definition_hash: Option<String>,
    pub trust_preview: Option<String>,
    pub permissions: Vec<String>,
    pub data_scope: String,
    pub single_instance: bool,
    pub install_progress: Option<InstallProgressView>,
    pub install_error: Option<InstallErrorView>,
    pub runtime_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppList {
    pub apps: Vec<AppView>,
    pub available_bytes: Option<u64>,
    pub registry_paths: RegistryPaths,
    pub registry_errors: Vec<RegistryError>,
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
    readme: ReadmeCache,
    size_cache: Mutex<HashMap<String, (Instant, u64)>>,
    starting: Mutex<HashSet<String>>,
}

impl AppService {
    pub fn new(workspace_root: PathBuf, global_data: PathBuf) -> Arc<Self> {
        let registry = Arc::new(AppRegistry::new(workspace_root, global_data));
        install::cleanup_stale_staging(&registry.apps_root);
        let runtime = Arc::new(AppRuntime::new(Arc::clone(&registry)));
        Arc::new(Self {
            registry,
            runtime,
            readme: ReadmeCache::new(),
            size_cache: Mutex::new(HashMap::new()),
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
        self.list_with(true)
    }

    /// Same snapshot without walking install trees. Agent tools use this so a
    /// first call against a large app (models, node_modules) stays cheap.
    pub fn list_brief(&self) -> AppList {
        self.list_with(false)
    }

    fn list_with(&self, with_size: bool) -> AppList {
        let snapshot = self.registry.load();
        let apps = snapshot
            .apps
            .values()
            .map(|app| self.view(app, with_size))
            .collect();
        AppList {
            apps,
            available_bytes: self.available_bytes(),
            registry_paths: snapshot.paths,
            registry_errors: snapshot.errors,
        }
    }

    /// Walking an install tree can be expensive (models, node_modules), so
    /// the polled list reuses a recent measurement.
    fn size_bytes(&self, app_id: &str, install_dir: &Path) -> u64 {
        const TTL: Duration = Duration::from_secs(120);
        if let Ok(cache) = self.size_cache.lock() {
            if let Some((at, bytes)) = cache.get(app_id) {
                if at.elapsed() < TTL {
                    return *bytes;
                }
            }
        }
        let bytes = space::directory_size(&install_dir.join("app"))
            .saturating_add(space::directory_size(&install_dir.join("data")));
        if let Ok(mut cache) = self.size_cache.lock() {
            cache.insert(app_id.to_string(), (Instant::now(), bytes));
        }
        bytes
    }

    fn available_bytes(&self) -> Option<u64> {
        let path = self
            .registry
            .apps_root
            .parent()
            .unwrap_or(self.registry.apps_root.as_path());
        space::available_bytes(path)
    }

    fn view(&self, app: &RegisteredApp, with_size: bool) -> AppView {
        let install_dir = self.registry.install_dir(app);
        let data_dir = self.registry.data_dir(app, &install_dir);
        let mut marker = install::read_marker(&install_dir);
        if marker
            .as_ref()
            .is_some_and(|entry| entry.status == "installing")
            && !install::install_in_progress(&install_dir)
        {
            install::clear_marker(&install_dir);
            marker = None;
        }
        let installed = self.is_installed(app, &install_dir);
        let runtime = self.runtime.view(&app.id);
        let trusted = self.is_trusted(app);
        let definition_hash = app.definition_hash();
        let status = self.status(installed, &marker, &runtime);
        let running = status == AppStatus::Running;
        let install_progress = marker
            .as_ref()
            .filter(|m| m.status == "installing")
            .map(|m| InstallProgressView {
                phase: m.phase.clone(),
                percent: m.percent,
                message: m.message.clone(),
            });
        let install_error = marker.as_ref().filter(|m| m.status == "error").map(|m| {
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
        let can_install = app.definition.install.kind == InstallKind::Steps
            && !installed
            && trusted
            && !matches!(
                status,
                AppStatus::Installing | AppStatus::Starting | AppStatus::Stopping
            );
        let can_uninstall = app.definition.install.kind == InstallKind::Steps
            && installed
            && !matches!(
                status,
                AppStatus::Installing | AppStatus::Starting | AppStatus::Stopping
            );
        let builtin_view = match app.definition.launch {
            LaunchSpec::Builtin { view } => Some(view.as_str()),
            _ => None,
        };
        let can_start = installed
            && trusted
            && builtin_view.is_none()
            && !matches!(
                status,
                AppStatus::Running | AppStatus::Starting | AppStatus::Stopping
            );
        let can_open = builtin_view.is_some() || (running && runtime.url.is_some());
        AppView {
            id: app.id.clone(),
            name: app.definition.name.clone(),
            description: app.definition.description.clone(),
            description_i18n: app.definition.description_i18n.clone(),
            summary: app.definition.summary.clone(),
            category: app.definition.category,
            icon: self.icon(&app.definition),
            homepage: app.definition.homepage.clone(),
            source: app.source,
            install_scope: app.install_scope,
            install_dir: install_dir.to_string_lossy().to_string(),
            data_dir: data_dir.to_string_lossy().to_string(),
            view_key: self.registry.view_key(&app.id),
            status,
            installed,
            running,
            url: runtime.url.clone(),
            builtin_view,
            port: runtime.port,
            required_bytes: app.definition.install.size_bytes,
            size_bytes: (with_size && installed && builtin_view.is_none())
                .then(|| self.size_bytes(&app.id, &install_dir)),
            can_install,
            can_uninstall,
            can_start,
            can_open,
            trusted,
            definition_hash,
            trust_preview: app.trust_preview(),
            permissions: app
                .definition
                .permissions
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            data_scope: match app.definition.data {
                DataScope::Workspace => "workspace".into(),
                DataScope::App => "app".into(),
            },
            single_instance: app.definition.single_instance,
            install_progress,
            install_error,
            runtime_error: runtime.error.clone(),
        }
    }

    fn status(
        &self,
        installed: bool,
        marker: &Option<InstallMarker>,
        runtime: &RuntimeView,
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
            _ if installed => AppStatus::Installed,
            _ => AppStatus::NotInstalled,
        }
    }

    fn is_installed(&self, app: &RegisteredApp, install_dir: &Path) -> bool {
        if matches!(app.definition.launch, LaunchSpec::Builtin { .. }) {
            return true;
        }
        if app.external_installed() {
            let app_dir = install::app_dir(install_dir);
            if let Some(check) = &app.definition.install.check {
                return app_dir.join(&check.path).exists();
            }
            return match &app.definition.launch {
                LaunchSpec::Builtin { .. } => true,
                LaunchSpec::Static { .. } => app_dir.join("index.html").is_file(),
                LaunchSpec::Process { command, .. } => command_available(command, &app_dir),
            };
        }
        if install::manifest_path(install_dir).is_file() {
            return true;
        }
        app.definition
            .install
            .check
            .as_ref()
            .map(|check| install::app_dir(install_dir).join(&check.path).exists())
            .unwrap_or(false)
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
        if !self.is_trusted(&app) {
            return Err(AppError::Forbidden(
                "this custom app must be trusted before install".into(),
            ));
        }
        if app.definition.install.kind != InstallKind::Steps {
            return Err(AppError::BadRequest(
                "this app is external; no managed install is available".into(),
            ));
        }
        let install_dir = self.registry.install_dir(&app);
        if self.is_installed(&app, &install_dir) {
            return Err(AppError::Conflict("app is already installed".into()));
        }
        let registry = Arc::clone(&self.registry);
        let app_clone = app.clone();
        tokio::task::spawn_blocking(move || install::install_blocking(&registry, &app_clone))
            .await
            .map_err(|e| AppError::Internal(format!("install task: {e}")))?
            .map_err(AppError::Internal)
    }

    pub async fn start(self: &Arc<Self>, id: &str) -> Result<(), AppError> {
        let (_, app) = self.find(id)?;
        if !self.is_trusted(&app) {
            return Err(AppError::Forbidden(
                "this custom app must be trusted before start".into(),
            ));
        }
        let install_dir = self.registry.install_dir(&app);
        if !self.is_installed(&app, &install_dir) {
            return Err(AppError::Conflict("app is not installed".into()));
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
        let result: Result<(), AppError> = async {
            match &app.definition.launch {
                LaunchSpec::Process { .. } => {
                    let runtime = Arc::clone(&self.runtime);
                    let app_clone = app.clone();
                    let data_dir = self.registry.data_dir(&app, &install_dir);
                    let process_install_dir = install_dir.clone();
                    tokio::task::spawn_blocking(move || {
                        runtime.start_process(&app_clone, &process_install_dir, &data_dir)
                    })
                    .await
                    .map_err(|e| AppError::Internal(format!("start task: {e}")))?
                    .map(|_| ())
                    .map_err(AppError::Internal)?;
                }
                LaunchSpec::Static { .. } => {
                    self.runtime
                        .start_static(&app, &install_dir)
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
            // A successful start supersedes a stale install-error marker.
            install::clear_marker(&install_dir);
            Ok(())
        }
        .await;
        drop(claim);
        result
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
        if app.definition.install.kind != InstallKind::Steps {
            return Err(AppError::BadRequest(
                "external apps are not managed by LiteCode".into(),
            ));
        }
        let install_dir = self.registry.install_dir(&app);
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
        if crate::apps::runtime::has_live_run_marker(&install_dir) {
            return Err(AppError::Conflict(
                "app is running in another workspace".into(),
            ));
        }
        let app_dir = install::app_dir(&install_dir);
        if app_dir.exists() {
            std::fs::remove_dir_all(&app_dir)
                .map_err(|e| AppError::Internal(format!("remove app: {e}")))?;
        }
        for dir in [
            install::logs_dir(&install_dir),
            install::run_dir(&install_dir),
        ] {
            let _ = std::fs::remove_dir_all(dir);
        }
        let _ = std::fs::remove_file(install::manifest_path(&install_dir));
        install::clear_marker(&install_dir);
        // The data directory is preserved on purpose; workspace data lives elsewhere.
        Ok(())
    }

    pub fn readme(&self, id: &str) -> Result<String, AppError> {
        let (_, app) = self.find(id)?;
        if let Some(url) = app.definition.readme_url.as_deref() {
            if let Some(text) = self.readme.fetch(url) {
                return Ok(text);
            }
        }
        if !app.definition.summary.trim().is_empty() {
            return Ok(app.definition.summary.clone());
        }
        Ok(app.definition.description.clone())
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

fn command_available(command: &str, app_dir: &Path) -> bool {
    if command.eq_ignore_ascii_case("npm") || command.eq_ignore_ascii_case("npx") {
        return crate::lsp::install::npm::resolve_npm_shim().is_some();
    }
    let path = Path::new(command);
    if path.is_absolute() {
        return path.is_file();
    }
    if command.contains('/') || command.contains('\\') || command.ends_with(".exe") {
        return app_dir.join(path).is_file();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            for ext in [".exe", ".cmd", ".bat", ".com"] {
                let candidate = dir.join(format!("{command}{ext}"));
                if candidate.is_file() {
                    return true;
                }
            }
        }
        false
    })
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
        let gallery = apps
            .apps
            .iter()
            .find(|app| app.id == "gallery")
            .expect("gallery app");
        assert!(gallery.installed);
        assert_eq!(gallery.builtin_view, Some("gallery"));
        assert_eq!(
            gallery.description_i18n.get("zh-CN").map(String::as_str),
            Some("浏览工作区里的图片、视频和音频。")
        );
        assert_eq!(gallery.status, AppStatus::Installed);
        assert!(gallery.can_open);
        assert!(!gallery.can_start);
        assert!(!gallery.can_uninstall);
        assert!(gallery.size_bytes.is_none());
    }
}
