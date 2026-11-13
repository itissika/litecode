//! The provider catalog is the embedded seed.
//!
//! The process parses that text once and never reads a catalog file.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::ProviderCatalog;

/// Embedded catalog. This is the only catalog the running app uses.
pub const DEFAULT_CATALOG: &str = include_str!("default_catalog.toml");
/// Embedded editor schema (Taplo / JSON schema). Kept for the schema contract.
pub const CATALOG_SCHEMA: &str = include_str!("provider-catalog.schema.json");

pub const CATALOG_FILE_NAME: &str = "provider-catalog.toml";

/// This build's catalog. One instance for the process.
///
/// `LITECODE_EVAL_PROVIDER_ENDPOINT` is an evaluation-only override: it points
/// one provider (default `deepseek`) at a recording proxy so a harness run can
/// be accounted and traced without changing the shipped catalog. It has no
/// effect unless set, and it is read once with the embedded catalog.
pub fn embedded() -> Arc<ProviderCatalog> {
    static CATALOG: OnceLock<Arc<ProviderCatalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| {
        let catalog = ProviderCatalog::parse(DEFAULT_CATALOG, Path::new(CATALOG_FILE_NAME))
            .expect("embedded provider catalog is valid");
        let Some(endpoint) = std::env::var("LITECODE_EVAL_PROVIDER_ENDPOINT")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Arc::new(catalog);
        };
        let provider = std::env::var("LITECODE_EVAL_PROVIDER_ID")
            .unwrap_or_else(|_| "deepseek".to_string());
        let endpoint_type = std::env::var("LITECODE_EVAL_ENDPOINT_TYPE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                super::schema::EndpointKind::parse(&value).unwrap_or_else(|| {
                    panic!(
                        "LITECODE_EVAL_ENDPOINT_TYPE must be responses, chat_completions or messages (got {value:?})"
                    )
                })
            });
        Arc::new(
            catalog
                .with_provider_endpoint_type(&provider, &endpoint, endpoint_type)
                .unwrap_or_else(|error| {
                    panic!("LITECODE_EVAL_PROVIDER_ENDPOINT is invalid: {error}")
                }),
        )
    }))
}

type PinCache = Mutex<std::collections::HashMap<PathBuf, Arc<ProviderCatalog>>>;

fn pins() -> &'static PinCache {
    static PINS: OnceLock<PinCache> = OnceLock::new();
    PINS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Catalog for `db_path`.
///
/// The running app always receives [embedded]. A test that needs a different
/// catalog calls [pin] first; nothing here reads a file.
pub fn shared_for_db(db_path: &Path) -> crate::types::Result<Arc<ProviderCatalog>> {
    let guard = pins().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(pinned) = guard.get(db_path) {
        return Ok(Arc::clone(pinned));
    }
    Ok(embedded())
}

/// Use `catalog` for later [shared_for_db] calls on `db_path`.
///
/// Tests plant a fixture this way. The running app does not call it.
pub fn pin(db_path: &Path, catalog: Arc<ProviderCatalog>) {
    pins()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(db_path.to_path_buf(), catalog);
}

/// Drop a pinned catalog so the next [shared_for_db] is the embedded seed.
pub fn forget(db_path: &Path) {
    pins()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(db_path);
}
