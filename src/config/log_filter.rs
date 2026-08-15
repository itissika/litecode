//! In-process reload of the stderr tracing filter when `log.level` changes.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::registry::Registry;
use tracing_subscriber::reload::{Handle, Layer as ReloadLayer};

use super::manager::ConfigManager;

static RELOAD: OnceLock<Mutex<Option<Handle<EnvFilter, Registry>>>> = OnceLock::new();

fn reload_slot() -> &'static Mutex<Option<Handle<EnvFilter, Registry>>> {
    RELOAD.get_or_init(|| Mutex::new(None))
}

pub fn install_handle(handle: Handle<EnvFilter, Registry>) {
    *reload_slot().lock().unwrap() = Some(handle);
}

/// Build a reload layer pair for subscriber init (`setup_logging`).
pub fn new_reload_layer(
    level: &str,
) -> (
    ReloadLayer<EnvFilter, Registry>,
    Handle<EnvFilter, Registry>,
) {
    ReloadLayer::new(level_to_filter(level))
}

/// Resolve effective log level: `LITECODE_LOG` env overrides DB.
pub fn resolve_level_from_db() -> String {
    std::env::var("LITECODE_LOG")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| ConfigManager::load_global().ok().and_then(|g| g.log.level))
        .unwrap_or_else(|| "info".into())
}

pub fn resolve_level_from_path(db_path: &Path) -> String {
    std::env::var("LITECODE_LOG")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            ConfigManager::load_global_from(db_path)
                .ok()
                .and_then(|g| g.log.level)
        })
        .unwrap_or_else(|| "info".into())
}

pub fn level_to_filter(level: &str) -> EnvFilter {
    EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"))
}

fn env_override_active() -> bool {
    std::env::var("LITECODE_LOG")
        .ok()
        .is_some_and(|s| !s.is_empty())
}

/// Reload console tracing filter from the global DB (no-op if `LITECODE_LOG` is set).
pub fn reload_from_db() {
    if env_override_active() {
        return;
    }
    reload_filter(&resolve_level_from_db());
}

/// Reload console tracing filter from a specific global DB path.
pub fn reload_from_path(db_path: &Path) {
    if env_override_active() {
        return;
    }
    reload_filter(&resolve_level_from_path(db_path));
}

pub fn reload_filter(level: &str) {
    let filter = level_to_filter(level);
    if let Some(handle) = reload_slot().lock().unwrap().as_ref() {
        let _ = handle.reload(filter);
    }
}
