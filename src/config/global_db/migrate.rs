use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

const SCHEMA: &str = include_str!("schema.sql");

/// Current schema epoch. v5→v6 drops `tool_catalog` in place.
pub const CURRENT_USER_VERSION: i32 = 6;

/// Epochs that `migrate()` can lift to current without archive-rebuild.
pub const MIGRATABLE_FROM: &[i32] = &[5];

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

    if version == 5 {
        migrate_v5_to_v6(conn)?;
        return Ok(());
    }

    if version == CURRENT_USER_VERSION {
        ensure_agent_tools_allowed_tools_column(conn)?;
        ensure_mcp_timeout_column(conn)?;
        return Ok(());
    }

    Err(LitecodeError::Config(format!(
        "incompatible global DB user_version {version} (expected {CURRENT_USER_VERSION}, 5, or empty). \
         Schema is delete-and-rebuild only; `global_db::open` archives the old file and recreates."
    )))
}

fn migrate_v5_to_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch("DROP TABLE IF EXISTS tool_catalog;")?;
    ensure_agent_tools_allowed_tools_column(conn)?;
    ensure_mcp_timeout_column(conn)?;
    conn.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
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
    use rusqlite::Connection;

    #[test]
    fn config_global_db_migration_v0_to_current() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            CURRENT_USER_VERSION
        );

        let catalog: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tool_catalog'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(catalog, 0);

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
    fn v5_drops_tool_catalog_in_place() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE tool_catalog (
                id TEXT PRIMARY KEY,
                tier TEXT NOT NULL,
                init_scope TEXT NOT NULL,
                catalog_enabled INTEGER NOT NULL
            );
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
            INSERT INTO tool_catalog (id, tier, init_scope, catalog_enabled)
                VALUES ('read', 'core', 'none', 1);
            INSERT INTO agents (id, role, model_ref) VALUES ('default', 'primary', '');
            PRAGMA user_version = 5;
            ",
        )
        .unwrap();

        migrate(&conn).unwrap();

        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            6
        );
        let catalog: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tool_catalog'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(catalog, 0);
        let agents: i64 = conn
            .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))
            .unwrap();
        assert_eq!(agents, 1);
    }

    #[test]
    fn current_epoch_adds_allowed_tools_column_without_rebuild() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE agent_tools (
                agent_id TEXT NOT NULL,
                tool_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                policy_json TEXT NOT NULL,
                path_mode TEXT NOT NULL,
                last_applied_preset TEXT,
                PRIMARY KEY (agent_id, tool_id)
            );
            PRAGMA user_version = {CURRENT_USER_VERSION};",
        ))
        .unwrap();

        migrate(&conn).unwrap();

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
    fn config_global_db_wrong_user_version_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();

        let err = migrate(&conn).expect_err("wrong version must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("delete") || msg.contains("Delete") || msg.contains("delete-and-rebuild"),
            "error must mention delete-and-rebuild: {msg}"
        );
        assert!(
            msg.contains("incompatible") || msg.contains("user_version"),
            "error must mention version: {msg}"
        );
    }

    #[test]
    fn config_global_db_current_version_is_noop() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            CURRENT_USER_VERSION
        );
    }
}
