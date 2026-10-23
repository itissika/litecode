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
        // An existing file can still be user_version 0. CREATE IF NOT EXISTS
        // does not add columns the old tables lack.
        ensure_current_columns(conn)?;
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
    ensure_custom_tools_columns(conn)?;
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

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    Ok(conn
        .prepare(&format!(
            "SELECT 1 FROM pragma_table_info('{table}') WHERE name = '{column}'"
        ))?
        .exists([])?)
}

fn ensure_custom_tools_columns(conn: &Connection) -> Result<()> {
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'custom_tools'",
        [],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !table_exists {
        return Ok(());
    }
    if !column_exists(conn, "custom_tools", "rules_json")? {
        conn.execute("ALTER TABLE custom_tools ADD COLUMN rules_json TEXT", [])?;
    }
    if !column_exists(conn, "custom_tools", "surface")? {
        conn.execute(
            "ALTER TABLE custom_tools ADD COLUMN surface TEXT NOT NULL DEFAULT 'direct'",
            [],
        )?;
    }
    if !column_exists(conn, "custom_tools", "data_schema_json")? {
        conn.execute(
            "ALTER TABLE custom_tools ADD COLUMN data_schema_json TEXT",
            [],
        )?;
    }
    if column_exists(conn, "custom_tools", "permission_json")? {
        conn.execute("ALTER TABLE custom_tools DROP COLUMN permission_json", [])?;
    }
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
