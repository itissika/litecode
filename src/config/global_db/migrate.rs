use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

const SCHEMA: &str = include_str!("schema.sql");

/// Current schema epoch. v7→v8 moves agent tool intent to `preset`.
pub const CURRENT_USER_VERSION: i32 = 8;

/// Epochs that `migrate()` can lift to current without archive-rebuild.
pub const MIGRATABLE_FROM: &[i32] = &[5, 6, 7];

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
        "incompatible global DB user_version {version} (expected {CURRENT_USER_VERSION}, 5, 6, 7, or empty). \
         Schema is delete-and-rebuild only; `global_db::open` archives the old file and recreates."
    )))
}

/// Additive, in-place upgrade. Legacy `providers` / `models` tables are left
/// untouched on purpose: they are historical user data, read at most once by the
/// catalog-aware credential migration.
fn migrate_to_current(conn: &Connection, version: i32) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    if version == 5 {
        tx.execute_batch("DROP TABLE IF EXISTS tool_catalog;")?;
    }
    ensure_current_columns(&tx)?;
    ensure_provider_credentials_table(&tx)?;
    ensure_disabled_models_table(&tx)?;
    tx.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
    tx.commit()?;
    Ok(())
}

fn ensure_current_columns(conn: &Connection) -> Result<()> {
    ensure_agent_tools_preset(conn)?;
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
    if !column_exists(conn, "custom_tools", "suite_id")? {
        conn.execute("ALTER TABLE custom_tools ADD COLUMN suite_id TEXT", [])?;
    }
    if !column_exists(conn, "custom_tools", "output_schema_json")? {
        conn.execute(
            "ALTER TABLE custom_tools ADD COLUMN output_schema_json TEXT",
            [],
        )?;
    }
    if !column_exists(conn, "custom_tools", "definition_version")? {
        conn.execute(
            "ALTER TABLE custom_tools ADD COLUMN definition_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS custom_suites (
             id TEXT PRIMARY KEY,
             surface TEXT NOT NULL,
             label TEXT NOT NULL DEFAULT '',
             rules_json TEXT
         );",
    )?;
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

/// Phase 4: agent tool rows keep permission intent only. Backfill `preset` from
/// the old `last_applied_preset`, then drop the derived policy/path columns.
/// The drop is intentional and one-way; rollback relies on the pre-migration DB
/// backup created by the caller, not on reading old columns.
fn ensure_agent_tools_preset(conn: &Connection) -> Result<()> {
    let table_exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agent_tools'",
        [],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !table_exists {
        return Ok(());
    }
    if !column_exists(conn, "agent_tools", "preset")? {
        conn.execute("ALTER TABLE agent_tools ADD COLUMN preset TEXT", [])?;
    }
    if column_exists(conn, "agent_tools", "last_applied_preset")? {
        conn.execute(
            "UPDATE agent_tools
                SET preset = last_applied_preset
              WHERE preset IS NULL AND last_applied_preset IS NOT NULL",
            [],
        )?;
    }
    let had_policy = column_exists(conn, "agent_tools", "policy_json")?;
    let had_path_mode = column_exists(conn, "agent_tools", "path_mode")?;
    let had_manual = had_policy || had_path_mode;
    if had_manual {
        let policy = if had_policy { "policy_json" } else { "NULL" };
        let path_mode = if had_path_mode { "path_mode" } else { "NULL" };
        let sql = format!(
            "SELECT agent_id, tool_id, {policy}, {path_mode} FROM agent_tools LIMIT 20"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let agent_id: String = row.get(0)?;
            let tool_id: String = row.get(1)?;
            let policy: Option<String> = row.get(2)?;
            let path_mode: Option<String> = row.get(3)?;
            tracing::warn!(
                %agent_id,
                %tool_id,
                policy = policy.as_deref().unwrap_or("<none>"),
                path_mode = path_mode.as_deref().unwrap_or("<none>"),
                "discarding legacy agent tool policy/path_mode; preset is the only stored intent"
            );
        }
    }
    for column in ["policy_json", "path_mode", "last_applied_preset"] {
        if column_exists(conn, "agent_tools", column)? {
            conn.execute(&format!("ALTER TABLE agent_tools DROP COLUMN {column}"), [])?;
        }
    }
    if had_manual {
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM agent_tools", [], |row| row.get(0))?;
        tracing::info!(
            rows,
            "dropped stored policy/path_mode from agent_tools; policy derives from preset at execution"
        );
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
    fn v7_agent_tools_backfills_preset_and_drops_manual_policy() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "ALTER TABLE agent_tools DROP COLUMN preset;
             ALTER TABLE agent_tools ADD COLUMN policy_json TEXT;
             ALTER TABLE agent_tools ADD COLUMN path_mode TEXT;
             ALTER TABLE agent_tools ADD COLUMN last_applied_preset TEXT;
             INSERT INTO agent_tools
                 (agent_id, tool_id, enabled, policy_json, path_mode, last_applied_preset)
             VALUES
                 ('default', 'run', 1, '{\"default\":\"deny\"}', 'workspace_only', 'SAFE'),
                 ('default', 'read', 1, NULL, NULL, NULL);
             PRAGMA user_version = 7;",
        )
        .unwrap();

        migrate(&conn).unwrap();

        let preset: Option<String> = conn
            .query_row(
                "SELECT preset FROM agent_tools WHERE agent_id = 'default' AND tool_id = 'run'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preset.as_deref(), Some("SAFE"));
        for column in ["policy_json", "path_mode", "last_applied_preset"] {
            let exists: bool = conn
                .prepare("SELECT 1 FROM pragma_table_info('agent_tools') WHERE name = ?1")
                .unwrap()
                .exists([column])
                .unwrap();
            assert!(!exists, "{column} must be dropped");
        }
        assert_eq!(user_version(&conn), CURRENT_USER_VERSION);
        migrate(&conn).unwrap();
        assert_eq!(user_version(&conn), CURRENT_USER_VERSION);
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
