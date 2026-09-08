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

/// Crates that log INFO on a hot path (indexer merge/GC). Product debug never
/// needs those lines; a bare `info`/`debug` level still mutes them. A full
/// EnvFilter spec (`info,tantivy=trace`) is left unchanged.
const NOISY_CRATES: &str = "tantivy=warn";

/// Expand a settings/env level into an EnvFilter spec.
pub fn compose_filter(level: &str) -> String {
    let level = level.trim();
    if level.is_empty() {
        return format!("info,{NOISY_CRATES}");
    }
    if level.contains('=') || level.contains(',') {
        return level.to_string();
    }
    format!("{level},{NOISY_CRATES}")
}

pub fn level_to_filter(level: &str) -> EnvFilter {
    let spec = compose_filter(level);
    EnvFilter::try_new(&spec).unwrap_or_else(|_| EnvFilter::new(compose_filter("info")))
}

/// File sink stays at info even when the console is debug/trace.
pub fn file_filter() -> EnvFilter {
    level_to_filter("info")
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

#[cfg(test)]
mod tests {
    use super::compose_filter;

    #[test]
    fn bare_level_mutes_tantivy() {
        assert_eq!(compose_filter("info"), "info,tantivy=warn");
        assert_eq!(compose_filter("debug"), "debug,tantivy=warn");
        assert_eq!(compose_filter(""), "info,tantivy=warn");
    }

    #[test]
    fn raw_spec_is_passed_through() {
        assert_eq!(compose_filter("info,litecode=debug"), "info,litecode=debug");
        assert_eq!(compose_filter("tantivy=trace"), "tantivy=trace");
        assert_eq!(compose_filter("info,tantivy=trace"), "info,tantivy=trace");
    }
}
