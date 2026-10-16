//! Test-only sessions.db helpers. Keeps rusqlite/SQL inside the sqlite module
//! so architecture death-lists stay green for callers outside this tree.

use std::path::Path;

use rusqlite::Connection;

use super::conn::BUSY_TIMEOUT;

/// Rename the `sessions` table so subsequent list/create reads fail.
/// Used by WP-166 error-path coverage for `session/list`.
pub fn rename_sessions_table(db: &Path, new_name: &str) {
    let conn = Connection::open(db).expect("open db");
    conn.busy_timeout(BUSY_TIMEOUT).unwrap();
    // new_name is a fixed test identifier, not user input.
    conn.execute_batch(&format!("ALTER TABLE sessions RENAME TO {new_name};"))
        .expect("break sessions table");
}

/// Force `updated_at` on the given session ids (e.g. age them for empty-session GC).
pub fn set_sessions_updated_at(db: &Path, ids: &[&str], updated_at: i64) {
    let conn = Connection::open(db).expect("open db");
    conn.busy_timeout(BUSY_TIMEOUT).unwrap();
    for id in ids {
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![updated_at, id],
        )
        .expect("age session");
    }
}
