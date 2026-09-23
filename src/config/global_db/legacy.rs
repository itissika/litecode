//! One-time, catalog-aware migration of the legacy v6 LLM rows.
//!
//! This module is the **only** place in the product allowed to read the legacy
//! `providers` / `models` tables. It runs once per database, records a marker,
//! and never reads them again. Nothing here guesses: a credential or model
//! reference that cannot be matched uniquely is left alone and reported.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use crate::provider_catalog::{AuthKind, EndpointKind, ProviderCatalog};
use crate::types::Result;

use super::{meta_get, meta_set, store};

/// Set once the legacy rows have been considered.
pub const LEGACY_LLM_MIGRATION_MARKER: &str = "provider_catalog.legacy_llm_migrated";

#[derive(Debug, Clone)]
struct LegacyProvider {
    id: String,
    adapter_id: String,
    endpoint: String,
    auth: String,
    api_key: String,
}

#[derive(Debug, Clone)]
struct LegacyModel {
    id: String,
    provider_ref: String,
    api_model_id: String,
}

/// Migrate legacy credentials and agent model references exactly once.
pub fn migrate_once(conn: &Connection, catalog: &ProviderCatalog) -> Result<()> {
    if meta_get(conn, LEGACY_LLM_MIGRATION_MARKER)?.is_some() {
        return Ok(());
    }

    let providers = read_legacy_providers(conn)?;
    let models = read_legacy_models(conn)?;
    let mapped = map_providers(&providers, catalog);
    migrate_credentials(conn, &providers, catalog, &mapped)?;
    migrate_agent_refs(conn, &models, catalog, &mapped)?;

    meta_set(conn, LEGACY_LLM_MIGRATION_MARKER, "1")?;
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn read_legacy_providers(conn: &Connection) -> Result<Vec<LegacyProvider>> {
    if !table_exists(conn, "providers")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT id, adapter_id, config_json FROM providers")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, adapter_id, config_json) = row?;
        let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        out.push(LegacyProvider {
            id,
            adapter_id,
            endpoint: config
                .get("endpoint")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .trim_end_matches('/')
                .to_string(),
            auth: config
                .get("auth")
                .and_then(|value| value.as_str())
                .unwrap_or("bearer")
                .to_string(),
            api_key: config
                .get("api_key")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_string(),
        });
    }
    Ok(out)
}

fn read_legacy_models(conn: &Connection) -> Result<Vec<LegacyModel>> {
    if !table_exists(conn, "models")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT id, provider_ref, config_json FROM models")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, provider_ref, config_json) = row?;
        let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        out.push(LegacyModel {
            id,
            provider_ref,
            api_model_id: config
                .get("api_model_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_string(),
        });
    }
    Ok(out)
}

/// Legacy adapter id -> protocol, for endpoint matching only.
fn legacy_endpoint_kind(adapter_id: &str) -> Option<EndpointKind> {
    match adapter_id {
        "openai_responses" | "deepseek_responses" | "mimo_responses" | "ark_coding" => {
            Some(EndpointKind::Responses)
        }
        "opencode" | "commandcode" => Some(EndpointKind::ChatCompletions),
        _ => None,
    }
}

fn legacy_auth(auth: &str) -> AuthKind {
    match auth {
        "api_key" => AuthKind::ApiKey,
        _ => AuthKind::Bearer,
    }
}

/// Map legacy provider ids onto catalog provider ids.
///
/// An exact id match wins. Otherwise the endpoint (plus protocol and auth) must
/// match exactly one catalog provider. Zero or several candidates stay unmapped.
fn map_providers(
    providers: &[LegacyProvider],
    catalog: &ProviderCatalog,
) -> HashMap<String, String> {
    let mut mapped = HashMap::new();
    for legacy in providers {
        if catalog.provider(&legacy.id).is_some() {
            mapped.insert(legacy.id.clone(), legacy.id.clone());
            continue;
        }
        let Some(kind) = legacy_endpoint_kind(&legacy.adapter_id) else {
            tracing::warn!(
                provider = %legacy.id,
                adapter_id = %legacy.adapter_id,
                "legacy provider uses an unknown adapter; its API key was not migrated"
            );
            continue;
        };
        let candidates: Vec<&str> = catalog
            .providers()
            .iter()
            .filter(|provider| {
                provider.endpoint == legacy.endpoint
                    && provider.endpoint_type == kind
                    && provider.auth == legacy_auth(&legacy.auth)
            })
            .map(|provider| provider.id.as_str())
            .collect();
        match candidates.as_slice() {
            [single] => {
                mapped.insert(legacy.id.clone(), (*single).to_string());
            }
            other => {
                tracing::warn!(
                    provider = %legacy.id,
                    candidates = ?other,
                    "legacy provider could not be matched uniquely to a catalog provider; \
                     the API key was not migrated - enter it again in Settings → Providers"
                );
            }
        }
    }
    mapped
}

fn migrate_credentials(
    conn: &Connection,
    providers: &[LegacyProvider],
    catalog: &ProviderCatalog,
    mapped: &HashMap<String, String>,
) -> Result<()> {
    let existing = store::provider_credentials(conn)?;
    for legacy in providers {
        let Some(catalog_id) = mapped.get(&legacy.id) else {
            continue;
        };
        if legacy.api_key.is_empty() {
            continue;
        }
        if catalog.provider(catalog_id).is_none() {
            continue;
        }
        match existing.get(catalog_id) {
            None => {
                store::set_provider_credential(conn, catalog_id, &legacy.api_key)?;
                tracing::info!(
                    provider = %catalog_id,
                    legacy = %legacy.id,
                    "migrated provider API key"
                );
            }
            Some(current) if current == &legacy.api_key => {}
            Some(_) => {
                tracing::warn!(
                    provider = %catalog_id,
                    legacy = %legacy.id,
                    "two legacy providers map to this catalog provider with different keys; \
                     keeping the configured key - verify it in Settings → Providers"
                );
            }
        }
    }
    Ok(())
}

fn migrate_agent_refs(
    conn: &Connection,
    models: &[LegacyModel],
    catalog: &ProviderCatalog,
    mapped: &HashMap<String, String>,
) -> Result<()> {
    let agents: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT id, model_ref FROM agents")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let by_id: HashMap<&str, &LegacyModel> =
        models.iter().map(|model| (model.id.as_str(), model)).collect();

    for (agent_id, model_ref) in agents {
        if model_ref.trim().is_empty() || catalog.model(&model_ref).is_some() {
            continue;
        }
        let Some(legacy) = by_id.get(model_ref.as_str()) else {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %model_ref,
                "agent model reference has no legacy definition; pick a model in Settings → Agents"
            );
            continue;
        };
        let Some(catalog_provider) = mapped.get(&legacy.provider_ref) else {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %model_ref,
                "agent model's legacy provider could not be mapped; pick a model in Settings → Agents"
            );
            continue;
        };
        let reference = ProviderCatalog::reference_of(catalog_provider, &legacy.api_model_id);
        if catalog.model(&reference).is_none() {
            tracing::warn!(
                agent = %agent_id,
                model_ref = %model_ref,
                candidate = %reference,
                "legacy model is not in the provider catalog; pick a model in Settings → Agents"
            );
            continue;
        }
        conn.execute(
            "UPDATE agents SET model_ref = ?1 WHERE id = ?2",
            params![reference, agent_id],
        )?;
        tracing::info!(agent = %agent_id, from = %model_ref, to = %reference, "migrated agent model reference");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_catalog::ProviderCatalog;
    use std::path::Path;

    const CATALOG: &str = r#"
version = 1

[[providers]]
id = "opencode"
name = "OpenCode Zen"
endpoint = "https://opencode.ai/zen/v1"
endpoint_type = "chat_completions"

[[providers]]
id = "deepseek"
name = "DeepSeek"
endpoint = "https://api.deepseek.com"
endpoint_type = "responses"

[[models]]
id = "deepseek-v4-flash"
provider_id = "opencode"

[[models]]
id = "deepseek-flash"
provider_id = "deepseek"
"#;

    /// A v6 database: legacy provider/model tables plus the current tables.
    fn legacy_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE provider_credentials (provider_id TEXT PRIMARY KEY, api_key TEXT NOT NULL);
            CREATE TABLE providers (id TEXT PRIMARY KEY, adapter_id TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', config_json TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE models (id TEXT PRIMARY KEY, adapter_id TEXT NOT NULL, provider_ref TEXT NOT NULL, label TEXT NOT NULL DEFAULT '', config_json TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                model_ref TEXT NOT NULL,
                system_prompt TEXT NOT NULL DEFAULT '',
                temperature REAL NOT NULL DEFAULT 0.7,
                max_steps INTEGER NOT NULL DEFAULT 50,
                description TEXT NOT NULL DEFAULT '',
                allowed_subagents_json TEXT NOT NULL DEFAULT '[]'
            );
            INSERT INTO agents (id, role, model_ref) VALUES ('default', 'primary', 'zen-flash');
            INSERT INTO agents (id, role, model_ref) VALUES ('compaction', 'hidden', 'ghost-model');
            ",
        )
        .unwrap();
        conn
    }

    fn insert_legacy_provider(conn: &Connection, id: &str, adapter_id: &str, endpoint: &str, key: &str) {
        conn.execute(
            "INSERT INTO providers (id, adapter_id, config_json) VALUES (?1, ?2, ?3)",
            params![
                id,
                adapter_id,
                format!(
                    "{{\"endpoint\":\"{endpoint}\",\"api_key\":\"{key}\",\"auth\":\"bearer\"}}"
                )
            ],
        )
        .unwrap();
    }

    fn insert_legacy_model(conn: &Connection, id: &str, adapter_id: &str, provider_ref: &str, api: &str) {
        conn.execute(
            "INSERT INTO models (id, adapter_id, provider_ref, config_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                adapter_id,
                provider_ref,
                format!("{{\"api_model_id\":\"{api}\",\"context_window\":1000,\"max_tokens\":10,\"capabilities\":[\"text\"]}}")
            ],
        )
        .unwrap();
    }

    fn catalog() -> ProviderCatalog {
        ProviderCatalog::parse(CATALOG, Path::new("test-catalog.toml")).unwrap()
    }

    fn credential(conn: &Connection, provider_id: &str) -> Option<String> {
        conn.query_row(
            "SELECT api_key FROM provider_credentials WHERE provider_id = ?1",
            [provider_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
    }

    fn agent_ref(conn: &Connection, id: &str) -> String {
        conn.query_row("SELECT model_ref FROM agents WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn exact_provider_id_wins_and_the_agent_ref_is_rewritten() {
        let conn = legacy_db();
        insert_legacy_provider(&conn, "deepseek", "deepseek_responses", "", "sk-legacy");
        insert_legacy_provider(
            &conn,
            "opencode",
            "opencode",
            "https://opencode.ai/zen/v1",
            "sk-zen",
        );
        insert_legacy_model(&conn, "zen-flash", "opencode", "opencode", "deepseek-v4-flash");

        migrate_once(&conn, &catalog()).unwrap();

        assert_eq!(
            credential(&conn, "deepseek").as_deref(),
            Some("sk-legacy")
        );
        assert_eq!(credential(&conn, "opencode").as_deref(), Some("sk-zen"));
        assert_eq!(agent_ref(&conn, "default"), "opencode/deepseek-v4-flash");
        // The unmappable reference is preserved instead of guessed away.
        assert_eq!(agent_ref(&conn, "compaction"), "ghost-model");
        assert_eq!(
            meta_get(&conn, LEGACY_LLM_MIGRATION_MARKER).unwrap().as_deref(),
            Some("1")
        );
    }

    #[test]
    fn endpoint_match_migrates_a_renamed_provider() {
        let conn = legacy_db();
        insert_legacy_provider(
            &conn,
            "zen",
            "opencode",
            "https://opencode.ai/zen/v1/",
            "sk-zen",
        );

        migrate_once(&conn, &catalog()).unwrap();

        assert_eq!(credential(&conn, "opencode").as_deref(), Some("sk-zen"));
        assert_eq!(credential(&conn, "zen"), None);
    }

    #[test]
    fn an_ambiguous_endpoint_match_is_never_guessed() {
        let conn = legacy_db();
        // Catalog endpoint that two providers share.
        let catalog = ProviderCatalog::parse(
            "version = 1\n[[providers]]\nid = \"a\"\nname = \"A\"\nendpoint = \"https://shared.example/v1\"\nendpoint_type = \"responses\"\n\n[[providers]]\nid = \"b\"\nname = \"B\"\nendpoint = \"https://shared.example/v1\"\nendpoint_type = \"responses\"\n",
            Path::new("t.toml"),
        )
        .unwrap();
        insert_legacy_provider(&conn, "old", "openai_responses", "https://shared.example/v1", "sk-old");

        migrate_once(&conn, &catalog).unwrap();

        assert_eq!(credential(&conn, "a"), None);
        assert_eq!(credential(&conn, "b"), None);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1, "legacy rows must survive an unmapped migration");
    }

    #[test]
    fn a_key_conflict_keeps_the_configured_credential() {
        let conn = legacy_db();
        store::set_provider_credential(&conn, "deepseek", "sk-current").unwrap();
        insert_legacy_provider(&conn, "deepseek", "deepseek_responses", "", "sk-old");

        migrate_once(&conn, &catalog()).unwrap();

        assert_eq!(credential(&conn, "deepseek").as_deref(), Some("sk-current"));
    }

    #[test]
    fn migration_runs_once() {
        let conn = legacy_db();
        insert_legacy_provider(&conn, "deepseek", "deepseek_responses", "", "sk-legacy");
        migrate_once(&conn, &catalog()).unwrap();

        // A later legacy write must not be picked up: the marker is final.
        conn.execute("DELETE FROM provider_credentials", []).unwrap();
        insert_legacy_provider(&conn, "other", "opencode", "https://opencode.ai/zen/v1", "sk-new");
        migrate_once(&conn, &catalog()).unwrap();

        assert_eq!(credential(&conn, "deepseek"), None);
        assert_eq!(credential(&conn, "opencode"), None);
    }

    #[test]
    fn an_empty_key_is_not_a_credential() {
        let conn = legacy_db();
        insert_legacy_provider(&conn, "deepseek", "deepseek_responses", "", "   ");
        migrate_once(&conn, &catalog()).unwrap();
        assert_eq!(credential(&conn, "deepseek"), None);
    }
}
