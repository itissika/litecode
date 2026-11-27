use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

const SCHEMA: &str = include_str!("schema.sql");

/// Current schema epoch. Any other non-zero `user_version` is incompatible:
/// `global_db::open` deletes and rebuilds the file instead of migrating it.
pub const CURRENT_USER_VERSION: i32 = 10;

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version == 0 {
        conn.execute_batch(SCHEMA)?;
        conn.execute_batch(&format!("PRAGMA user_version = {CURRENT_USER_VERSION};"))?;
        return Ok(());
    }

    if version == CURRENT_USER_VERSION {
        return Ok(());
    }

    Err(LitecodeError::Config(format!(
        "incompatible global DB user_version {version} (expected {CURRENT_USER_VERSION}); \
         `global_db::open` deletes and rebuilds the file"
    )))
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
