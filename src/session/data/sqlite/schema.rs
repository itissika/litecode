//! Physical schema for `sessions.db`. The only rusqlite DDL in the crate
//! besides `global_db`.

use rusqlite::Connection;

use crate::types::{LitecodeError, Result};

use super::conn::BUSY_TIMEOUT;

/// Bump on any DDL change. A store at another version is rejected outright:
/// sessions are delete-and-rebuild, never migrated in place.
pub const USER_VERSION: i32 = 9;

pub fn configure_write(conn: &Connection) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| LitecodeError::SessionStorage(format!("busy_timeout: {e}")))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=FULL;",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("configure write pragmas: {e}")))?;
    Ok(())
}

#[cfg(test)]
fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn incompatible_session_db(detail: &str) -> LitecodeError {
    LitecodeError::Config(format!(
        "incompatible session DB ({detail}). Delete `.litecode/sessions.db` (or this DB path) \
         and restart — there is no upgrade path; schema is delete-and-rebuild only."
    ))
}

pub fn user_version(conn: &Connection) -> Result<i32> {
    let v: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(v)
}

pub fn ensure_session_schema(conn: &Connection) -> Result<()> {
    let v = user_version(conn)?;
    if v == USER_VERSION {
        return Ok(());
    }
    if v != 0 {
        return Err(incompatible_session_db(&format!(
            "user_version {v} != {USER_VERSION}"
        )));
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_folders (
            id                TEXT PRIMARY KEY,
            name              TEXT NOT NULL,
            sort_order        INTEGER NOT NULL DEFAULT 0,
            created_at        INTEGER NOT NULL,
            updated_at        INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_folders_sort
            ON session_folders(sort_order, created_at);
        CREATE TABLE IF NOT EXISTS sessions (
            id                TEXT PRIMARY KEY,
            schema_version    INTEGER NOT NULL DEFAULT 3,
            project           TEXT NOT NULL,
            last_message      TEXT NOT NULL DEFAULT '',
            last_assistant    TEXT NOT NULL DEFAULT '',
            agent_id          TEXT NOT NULL,
            model_id          TEXT,
            thinking_tier     TEXT NOT NULL DEFAULT 'medium',
            context_mode      TEXT NOT NULL DEFAULT 'standard',
            created_at        INTEGER NOT NULL,
            updated_at        INTEGER NOT NULL,
            checkpoint_seq    INTEGER NOT NULL DEFAULT 0,
            kept_from_seq     INTEGER NOT NULL DEFAULT 0,
            compacted_seq     INTEGER,
            surface_rewrite_seq INTEGER NOT NULL DEFAULT 0,
            truncate_epoch    INTEGER NOT NULL DEFAULT 0,
            spine_from        INTEGER NOT NULL DEFAULT 0,
            subagent_depth    INTEGER NOT NULL DEFAULT 0,
            todos_json        TEXT NOT NULL DEFAULT '[]',
            memory_json       TEXT NOT NULL DEFAULT '{\"next_id\":1,\"entries\":[]}',
            active_plan_slug  TEXT,
            plan_revision     TEXT,
            parent_session_id TEXT,
            parent_call_id    TEXT,
            responsibility    TEXT NOT NULL DEFAULT '',
            revision          INTEGER NOT NULL DEFAULT 0,
            next_seq          INTEGER NOT NULL DEFAULT 0,
            deleted_children_json TEXT NOT NULL DEFAULT '{}',
            title             TEXT,
            folder_id         TEXT REFERENCES session_folders(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_project
            ON sessions(project);
        CREATE INDEX IF NOT EXISTS idx_sessions_updated
            ON sessions(updated_at);
        CREATE INDEX IF NOT EXISTS idx_sessions_parent
            ON sessions(parent_session_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_folder
            ON sessions(folder_id);
        CREATE TABLE IF NOT EXISTS transcript_items (
            session_id      TEXT NOT NULL,
            seq             INTEGER NOT NULL,
            turn_id         TEXT NOT NULL DEFAULT '',
            turn_seq        INTEGER NOT NULL DEFAULT 0,
            item_type       TEXT NOT NULL,
            kind            TEXT NOT NULL,
            body            TEXT,
            body_ref        TEXT,
            token_estimate  INTEGER NOT NULL DEFAULT 0,
            created_at      INTEGER NOT NULL,
            event_type      TEXT NOT NULL,
            surface_op      TEXT NOT NULL,
            source_seqs     TEXT,
            state           TEXT NOT NULL DEFAULT 'final',
            provider_usage  TEXT,
            result_data     TEXT,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_seq
            ON transcript_items(session_id, seq);
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_turn
            ON transcript_items(session_id, turn_id, turn_seq);
        CREATE INDEX IF NOT EXISTS idx_transcript_items_user_seq
            ON transcript_items(session_id, seq)
            WHERE kind = 'item/user';
        CREATE TABLE IF NOT EXISTS session_context_meter (
            session_id         TEXT PRIMARY KEY,
            prompt_tokens      INTEGER NOT NULL DEFAULT 0,
            completion_tokens  INTEGER NOT NULL DEFAULT 0,
            cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
            cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
            cum_prompt_tokens      INTEGER NOT NULL DEFAULT 0,
            cum_completion_tokens  INTEGER NOT NULL DEFAULT 0,
            cum_cache_hit_tokens   INTEGER NOT NULL DEFAULT 0,
            cum_cache_miss_tokens  INTEGER NOT NULL DEFAULT 0,
            updated_at         INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS session_operations (
            session_id   TEXT NOT NULL,
            operation_id TEXT NOT NULL,
            receipt_json TEXT NOT NULL,
            created_at   INTEGER NOT NULL,
            PRIMARY KEY (session_id, operation_id)
        );
        -- Creation has no session id before commit, so idempotency replay looks
        -- up by operation_id alone. Without this index that lookup scans the
        -- whole receipt table before every new root or child session.
        CREATE INDEX IF NOT EXISTS idx_session_operations_operation_id
            ON session_operations(operation_id, created_at DESC);
        -- One coalesced dirty row per session. Search indexing needs only the
        -- set of sessions changed since its checkpoint, not every mutation.
        CREATE TABLE IF NOT EXISTS session_change_clock (
            singleton      INTEGER PRIMARY KEY CHECK (singleton = 1),
            last_change_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_dirty (
            session_id TEXT PRIMARY KEY,
            change_id  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_dirty_change
            ON session_dirty(change_id);
        CREATE TABLE IF NOT EXISTS session_blobs (
            blob_id    TEXT PRIMARY KEY,
            sha256     TEXT NOT NULL,
            bytes      INTEGER NOT NULL,
            rel_path   TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS session_blob_refs (
            blob_id    TEXT NOT NULL,
            session_id TEXT NOT NULL,
            seq        INTEGER NOT NULL,
            PRIMARY KEY (blob_id, session_id, seq),
            FOREIGN KEY (blob_id) REFERENCES session_blobs(blob_id)
        );",
    )?;

    // WP-186 P1 sidecar for pending user-gate asks.
    crate::session::data::sqlite::pending::ensure_pending_permissions_table(conn)?;
    conn.execute_batch(&format!(
        "INSERT OR IGNORE INTO session_change_clock (singleton, last_change_id) VALUES (1, 1);
         PRAGMA user_version = {USER_VERSION};"
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn fresh_database_stamps_user_version_and_has_no_retired_tables() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();

        assert_eq!(user_version(&conn).unwrap(), USER_VERSION);
        assert!(table_exists(&conn, "sessions").unwrap());
        assert!(table_exists(&conn, "transcript_items").unwrap());
        assert!(table_exists(&conn, "session_pending_permissions").unwrap());
        assert!(
            !table_exists(&conn, "session_change_log").unwrap(),
            "the retired per-mutation log must not exist"
        );
        assert!(
            !table_exists(&conn, "messages").unwrap(),
            "the retired legacy message table must not exist"
        );
        let seeded: i64 = conn
            .query_row(
                "SELECT last_change_id FROM session_change_clock WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(seeded, 1, "the change clock starts at 1");
    }

    #[test]
    fn current_version_is_a_no_op() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        ensure_session_schema(&conn).unwrap();
        assert_eq!(user_version(&conn).unwrap(), USER_VERSION);
    }

    #[test]
    fn incompatible_user_version_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute_batch(&format!("PRAGMA user_version = {};", USER_VERSION - 1))
            .unwrap();

        let error = ensure_session_schema(&conn).expect_err("an old DB must not open");
        let message = error.to_string();
        assert!(
            message.contains(&format!("user_version {} != {USER_VERSION}", USER_VERSION - 1)),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("delete-and-rebuild"),
            "the error must say how to recover: {message}"
        );
    }
}
