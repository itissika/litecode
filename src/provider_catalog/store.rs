//! Catalog file lifecycle: stable location, first-run seeding, strict loading.
//!
//! The catalog lives next to the global database and is a *user* file: it is
//! seeded once. A file that already loads is never rewritten. A file this build
//! cannot load is upgraded by [super::migrate] and the previous text is kept
//! beside it. The editor schema next to the catalog is product-managed and may
//! be refreshed on every start.

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
/// - not initialized + file present: load (upgrading if this build cannot), then
///   register initialized;
/// - initialized + file missing: hard error (never silently re-seed);
/// - unreadable, or text that is not TOML: hard error naming the file;
/// - TOML this build cannot load: upgrade, keep the previous text in
///   `provider-catalog.toml.bak`, then load the upgraded file.
pub fn load_for_db(db_path: &Path) -> Result<Arc<ProviderCatalog>> {
    let path = catalog_path_for_db(db_path);
    let conn = global_db::open(db_path)?;
    let initialized = global_db::meta_get(&conn, INITIALIZED_MARKER)?.is_some();

    if !initialized {
        if path.is_file() {
            let catalog = load_catalog_file(&path)?;
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
    let catalog = load_catalog_file(&path)?;
    write_schema_if_changed(&schema_path_for_db(db_path))?;
    Ok(Arc::new(catalog))
}

/// Strict parse, or the startup upgrade when the text is TOML this build rejects.
fn load_catalog_file(path: &Path) -> Result<ProviderCatalog> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        LitecodeError::Config(format!(
            "provider catalog {} could not be read: {error}",
            path.display()
        ))
    })?;
    match ProviderCatalog::parse(&text, path) {
        Ok(catalog) => Ok(catalog),
        Err(error) => {
            let Some(upgraded) = super::migrate::upgrade(&text, path) else {
                return Err(error);
            };
            let backup = backup_path(path);
            std::fs::write(&backup, &text).map_err(|error| {
                LitecodeError::Config(format!(
                    "provider catalog {} could not be backed up to {}: {error}",
                    path.display(),
                    backup.display()
                ))
            })?;
            std::fs::write(path, &upgraded.text).map_err(|error| {
                LitecodeError::Config(format!(
                    "provider catalog {} could not be upgraded: {error}",
                    path.display()
                ))
            })?;
            tracing::warn!(
                path = %path.display(),
                backup = %backup.display(),
                "provider catalog did not match this build and was upgraded"
            );
            Ok(upgraded.catalog)
        }
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(CATALOG_FILE_NAME);
    path.with_file_name(format!("{name}.bak"))
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

/// A shipped provider the loaded catalog does not contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedProviderGap {
    pub id: String,
    pub endpoint: String,
    pub endpoint_type: String,
    pub auth: String,
}

/// Seed entries absent from the loaded catalog.
///
/// Id membership only: a user-added provider, or an edited endpoint on a
/// provider that is still present, does not open a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedGap {
    pub missing_providers: Vec<SeedProviderGap>,
    pub missing_models: Vec<String>,
}

impl SeedGap {
    pub fn is_empty(&self) -> bool {
        self.missing_providers.is_empty() && self.missing_models.is_empty()
    }
}

fn embedded_seed() -> &'static ProviderCatalog {
    static SEED: OnceLock<ProviderCatalog> = OnceLock::new();
    SEED.get_or_init(|| {
        ProviderCatalog::parse(DEFAULT_CATALOG, Path::new("<embedded-provider-catalog>"))
            .expect("embedded provider catalog is valid")
    })
}

/// One embedded table (`[[providers]]` or `[[models]]`) plus the comment lines
/// that sit above its header.
struct CatalogTable {
    provider_id: Option<String>,
    id: String,
    text: String,
}

/// TOML this build ships for the entries in `gap`, in catalog order.
///
/// Comments directly above each table travel with it. Entries the file already
/// has are left out. An empty gap prints nothing.
pub fn seed_blocks(gap: &SeedGap) -> String {
    if gap.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for table in catalog_tables(DEFAULT_CATALOG) {
        let keep = match &table.provider_id {
            None => gap
                .missing_providers
                .iter()
                .any(|provider| provider.id == table.id),
            Some(provider_id) => {
                let reference = format!("{provider_id}/{}", table.id);
                gap.missing_models.iter().any(|model| model == &reference)
            }
        };
        if !keep {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&table.text);
    }
    out
}

fn catalog_tables(source: &str) -> Vec<CatalogTable> {
    let lines: Vec<&str> = source.lines().collect();
    let headers: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim();
            trimmed == "[[providers]]" || trimmed == "[[models]]"
        })
        .map(|(index, _)| index)
        .collect();
    let mut tables = Vec::with_capacity(headers.len());
    for (index, &header) in headers.iter().enumerate() {
        let next = headers.get(index + 1).copied().unwrap_or(lines.len());
        let mut end = next;
        while end > header && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let mut start = header;
        while start > 0 {
            let previous = lines[start - 1].trim();
            if previous.is_empty() || previous.starts_with('#') {
                start -= 1;
            } else {
                break;
            }
        }
        while start < header && lines[start].trim().is_empty() {
            start += 1;
        }
        let body = &lines[header..end];
        let Some(id) = assignment(body, "id") else {
            continue;
        };
        let kind = lines[header].trim();
        let provider_id = if kind == "[[models]]" {
            assignment(body, "provider_id")
        } else {
            None
        };
        if kind == "[[models]]" && provider_id.is_none() {
            continue;
        }
        let mut text = lines[start..end].join("\n");
        text.push('\n');
        tables.push(CatalogTable {
            provider_id,
            id,
            text,
        });
    }
    tables
}

fn assignment(lines: &[&str], key: &str) -> Option<String> {
    let prefix = format!("{key} = ");
    for line in lines {
        let Some(rest) = line.trim().strip_prefix(&prefix) else {
            continue;
        };
        let rest = rest.trim().trim_end_matches(',');
        let quoted = rest
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                rest.strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            });
        return Some(quoted.unwrap_or(rest).to_string());
    }
    None
}

/// Providers and models this build ships that `loaded` does not have.
pub fn seed_gap(loaded: &ProviderCatalog) -> SeedGap {
    let seed = embedded_seed();
    let missing_providers = seed
        .providers()
        .iter()
        .filter(|provider| loaded.provider(&provider.id).is_none())
        .map(|provider| SeedProviderGap {
            id: provider.id.clone(),
            endpoint: provider.endpoint.clone(),
            endpoint_type: provider.endpoint_type.as_str().to_string(),
            auth: provider.auth.as_str().to_string(),
        })
        .collect();
    let missing_models = seed
        .models()
        .iter()
        .filter(|model| loaded.model(&model.reference).is_none())
        .map(|model| model.reference.clone())
        .collect();
    SeedGap {
        missing_providers,
        missing_models,
    }
}
