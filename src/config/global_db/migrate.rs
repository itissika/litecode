use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

const SCHEMA: &str = include_str!("schema.sql");

/// Current schema epoch. v6→v7 adds the provider credential table in place.
pub const CURRENT_USER_VERSION: i32 = 7;

/// Epochs that `migrate()` can lift to current without archive-rebuild.
pub const MIGRATABLE_FROM: &[i32] = &[5, 6];

pub fn can_migrate_in_place(version: i32) -> bool {
    version == 0 || version == CURRENT_USER_VERSION || MIGRATABLE_FROM.contains(&version)
}

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version == 0 {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
        return Ok(());
    }

    if MIGRATABLE_FROM.contains(&version) {
        migrate_to_current(conn, version)?;
        return Ok(());
    }

    if version == CURRENT_USER_VERSION {
        ensure_current_columns(conn)?;
        return Ok(());
    }

    Err(LitecodeError::Config(format!(
        "incompatible global DB user_version {version} (expected {CURRENT_USER_VERSION}, 5, 6, or empty). \
         Schema is delete-and-rebuild only; `global_db::open` archives the old file and recreates."
    )))
}

/// Additive, in-place upgrade. Legacy `providers` / `models` tables are left
/// untouched on purpose: they are historical user data, read at most once by the
/// catalog-aware credential migration.
fn migrate_to_current(conn: &Connection, version: i32) -> Result<()> {
    if version == 5 {
        conn.execute_batch("DROP TABLE IF EXISTS tool_catalog;")?;
    }
    ensure_current_columns(conn)?;
    ensure_provider_credentials_table(conn)?;
    ensure_disabled_models_table(conn)?;
    conn.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
    Ok(())
}

fn ensure_current_columns(conn: &Connection) -> Result<()> {
    ensure_agent_tools_allowed_tools_column(conn)?;
    ensure_mcp_timeout_column(conn)?;
    ensure_provider_credentials_table(conn)?;
    ensure_disabled_models_table(conn)?;
    Ok(())
}

fn ensure_disabled_models_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS disabled_models (
             model_ref TEXT PRIMARY KEY
         );",
    )?;
    Ok(())
}

fn ensure_provider_credentials_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_credentials (
             provider_id TEXT PRIMARY KEY,
             api_key     TEXT NOT NULL
         );",
    )?;
    Ok(())
}

fn ensure_mcp_timeout_column(conn: &Connection) -> Result<()> {
    let exists = conn
        .prepare("SELECT 1 FROM pragma_table_info('mcp_servers') WHERE name = 'timeout'")?
        .exists([])?;
    if !exists {
        conn.execute(
            "ALTER TABLE mcp_servers ADD COLUMN timeout INTEGER NOT NULL DEFAULT 60",
            [],
        )?;
    }
    Ok(())
}

/// Extend the current schema in-place for additive agent binding metadata.
fn ensure_agent_tools_allowed_tools_column(conn: &Connection) -> Result<()> {
    let exists = conn
        .prepare(
            "SELECT 1 FROM pragma_table_info('agent_tools') WHERE name = 'allowed_tools_json'",
        )?
        .exists([])?;
    if !exists {
        conn.execute(
            "ALTER TABLE agent_tools ADD COLUMN allowed_tools_json TEXT",
            [],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
            == 1
    }

    fn user_version(conn: &Connection) -> i32 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn config_global_db_migration_v0_to_current() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn), CURRENT_USER_VERSION);
        assert!(table_exists(&conn, "provider_credentials"));
        assert!(
            !table_exists(&conn, "providers") && !table_exists(&conn, "models"),
            "a fresh install must not create the legacy LLM tables"
        );

        let col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('agent_tools') WHERE name='allowed_tools_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);
    }

    #[test]
    fn v5_drops_tool_catalog_in_place_and_keeps_legacy_llm_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE tool_catalog (id TEXT PRIMARY KEY, tier TEXT NOT NULL);
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
            CREATE TABLE agent_tools (
                agent_id TEXT NOT NULL,
                tool_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                policy_json TEXT NOT NULL DEFAULT '{}',
                path_mode TEXT NOT NULL DEFAULT 'unrestricted',
                last_applied_preset TEXT,
                allowed_tools_json TEXT,
                PRIMARY KEY (agent_id, tool_id)
            );
            CREATE TABLE mcp_servers (
                id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                args_json TEXT NOT NULL DEFAULT '[]',
                env_json TEXT NOT NULL DEFAULT '{}',
                transport_json TEXT NOT NULL DEFAULT '{\"type\":\"stdio\"}'
            );
            INSERT INTO providers (id, adapter_id) VALUES ('deepseek', 'deepseek_responses');
            INSERT INTO agents (id, role, model_ref) VALUES ('default', 'primary', 'flash');
            PRAGMA user_version = 5;
            ",
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn), 7);
        assert!(!table_exists(&conn, "tool_catalog"));
        assert!(table_exists(&conn, "provider_credentials"));
        let providers: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(providers, 1, "legacy rows must survive the upgrade");
        let agents: i64 = conn
            .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(agents, 1);
    }

    #[test]
    fn v6_upgrade_is_additive_and_never_drops_legacy_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
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
            CREATE TABLE agent_tools (
                agent_id TEXT NOT NULL,
                tool_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                policy_json TEXT NOT NULL DEFAULT '{}',
                path_mode TEXT NOT NULL DEFAULT 'unrestricted',
                last_applied_preset TEXT,
                allowed_tools_json TEXT,
                PRIMARY KEY (agent_id, tool_id)
            );
            CREATE TABLE mcp_servers (
                id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                args_json TEXT NOT NULL DEFAULT '[]',
                env_json TEXT NOT NULL DEFAULT '{}',
                transport_json TEXT NOT NULL DEFAULT '{\"type\":\"stdio\"}',
                timeout INTEGER NOT NULL DEFAULT 60
            );
            INSERT INTO providers (id, adapter_id, config_json)
                VALUES ('opencode', 'opencode', '{\"endpoint\":\"\",\"api_key\":\"sk-zen\",\"auth\":\"bearer\"}');
            INSERT INTO models (id, adapter_id, provider_ref, config_json)
                VALUES ('zen-flash', 'opencode', 'opencode', '{\"api_model_id\":\"deepseek-v4-flash\",\"context_window\":1,\"max_tokens\":1,\"capabilities\":[\"text\"]}');
            INSERT INTO agents (id, role, model_ref) VALUES ('default', 'primary', 'zen-flash');
            PRAGMA user_version = 6;
            ",
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(user_version(&conn), 7);
        assert!(table_exists(&conn, "provider_credentials"));
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let models: i64 = conn
            .query_row("SELECT COUNT(*) FROM models", [], |row| row.get(0))
            .unwrap();
        assert_eq!(models, 1);
        let agents: String = conn
            .query_row(
                "SELECT model_ref FROM agents WHERE id='default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(agents, "zen-flash");
    }

    #[test]
    fn re_running_the_migration_is_a_noop() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), CURRENT_USER_VERSION);
    }

    #[test]
    fn config_global_db_wrong_user_version_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();

        let message = migrate(&conn)
            .expect_err("wrong version must fail")
            .to_string();
        assert!(message.contains("incompatible"), "{message}");
        assert!(message.contains("user_version"), "{message}");
    }
}
