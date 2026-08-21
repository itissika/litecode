use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

const SCHEMA: &str = include_str!("schema.sql");

/// Current schema epoch only. The old stepwise 001→005 chain is deleted;
/// `user_version` is not a migration ladder — incompatible DBs must be deleted.
pub const CURRENT_USER_VERSION: i32 = 5;

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version == 0 {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
        return Ok(());
    }

    if version == CURRENT_USER_VERSION {
        ensure_agent_tools_allowed_tools_column(conn)?;
        return Ok(());
    }

    Err(LitecodeError::Config(format!(
        "incompatible global DB user_version {version} (expected {CURRENT_USER_VERSION} or empty). \
         Schema is delete-and-rebuild only; `global_db::open` archives the old file and recreates."
    )))
}

/// Extend the current schema in-place for additive agent binding metadata.
///
/// The global DB intentionally has no historical migration ladder: incompatible
/// epochs are rebuilt. This column is additive and must instead be available to
/// existing current-epoch databases without losing settings.
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
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            0
        );

        migrate(&conn).unwrap();

        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            CURRENT_USER_VERSION
        );

        let col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('agents') WHERE name='allowed_subagents_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);

        let col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('agent_tools') WHERE name='allowed_tools_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);

        let col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('models') WHERE name='config_json'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);

        let col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('providers') WHERE name='adapter_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col, 1);

        let readiness: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tool_catalog') WHERE name='readiness'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(readiness, 0);

        let description: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('custom_tools') WHERE name='description'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(description, 1);
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
        conn.execute_batch("PRAGMA user_version = 6;").unwrap();

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
