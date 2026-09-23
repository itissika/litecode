//! Catalog file lifecycle: stable location, first-run seeding, strict loading.
//!
//! The catalog lives next to the global database and is a *user* file: it is
//! seeded once and never rewritten afterwards. The editor schema next to it is
//! product-managed and may be refreshed on every start.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::config::global_db;
use crate::types::{LitecodeError, Result};

use super::ProviderCatalog;

/// Embedded first-run catalog.
pub const DEFAULT_CATALOG: &str = include_str!("default_catalog.toml");
/// Embedded editor schema (Taplo / JSON schema).
pub const CATALOG_SCHEMA: &str = include_str!("provider-catalog.schema.json");

pub const CATALOG_FILE_NAME: &str = "provider-catalog.toml";
pub const SCHEMA_FILE_NAME: &str = "provider-catalog.schema.json";

/// Meta marker proving the catalog was seeded or already present.
pub const INITIALIZED_MARKER: &str = "provider_catalog.initialized";

/// Catalog path for a global DB path (same directory).
pub fn catalog_path_for_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(CATALOG_FILE_NAME)
}

/// Editor schema path for a global DB path (same directory).
pub fn schema_path_for_db(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SCHEMA_FILE_NAME)
}

/// Load the catalog that belongs to `db_path`.
///
/// - not initialized + file missing: seed from the embedded catalog;
/// - not initialized + file present: validate, then register initialized;
/// - initialized + file missing: hard error (never silently re-seed);
/// - unreadable / bad TOML / bad semantics: hard error naming the file.
pub fn load_for_db(db_path: &Path) -> Result<Arc<ProviderCatalog>> {
    let path = catalog_path_for_db(db_path);
    let conn = global_db::open(db_path)?;
    let initialized = global_db::meta_get(&conn, INITIALIZED_MARKER)?.is_some();

    if !initialized {
        if path.is_file() {
            let catalog = read_and_parse(&path)?;
            global_db::meta_set(&conn, INITIALIZED_MARKER, "1")?;
            write_schema_if_changed(&schema_path_for_db(db_path))?;
            return Ok(Arc::new(catalog));
        }
        seed_file(&path)?;
        let catalog = read_and_parse(&path)?;
        write_schema_if_changed(&schema_path_for_db(db_path))?;
        global_db::meta_set(&conn, INITIALIZED_MARKER, "1")?;
        return Ok(Arc::new(catalog));
    }

    if !path.is_file() {
        return Err(LitecodeError::Config(format!(
            "provider catalog {} is missing: it was initialized on a previous run, so LiteCode \
             will not silently recreate it. Restore the file (or remove the \
             '{}' marker in the global DB) and restart.",
            path.display(),
            INITIALIZED_MARKER
        )));
    }
    let catalog = read_and_parse(&path)?;
    write_schema_if_changed(&schema_path_for_db(db_path))?;
    Ok(Arc::new(catalog))
}

/// Parse a catalog file strictly.
pub fn read_and_parse(path: &Path) -> Result<ProviderCatalog> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        LitecodeError::Config(format!(
            "provider catalog {} could not be read: {error}",
            path.display()
        ))
    })?;
    ProviderCatalog::parse(&text, path)
}

type CatalogCache = Mutex<std::collections::HashMap<PathBuf, Arc<ProviderCatalog>>>;

fn cache() -> &'static CatalogCache {
    static CACHE: OnceLock<CatalogCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Process-wide shared catalog per global DB path.
///
/// The catalog is immutable for the process lifetime: edits require a restart.
pub fn shared_for_db(db_path: &Path) -> Result<Arc<ProviderCatalog>> {
    let mut guard = cache().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = guard.get(db_path) {
        return Ok(Arc::clone(existing));
    }
    let catalog = load_for_db(db_path)?;
    guard.insert(db_path.to_path_buf(), Arc::clone(&catalog));
    Ok(catalog)
}

/// Forget one cached catalog (tests that reuse a temp path).
///
/// Per path on purpose: clearing every entry would let one test observe another
/// test's reload.
pub fn forget(db_path: &Path) {
    cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(db_path);
}

/// Atomically create the catalog file with the embedded default.
fn seed_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, DEFAULT_CATALOG)?;
    tracing::info!(path = %path.display(), "seeded provider catalog");
    Ok(())
}

/// Refresh the product-managed editor schema when its content changed.
fn write_schema_if_changed(path: &Path) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == CATALOG_SCHEMA) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(path, CATALOG_SCHEMA)
}

fn atomic_write(path: &Path, contents: &str) -> Result<()> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}
