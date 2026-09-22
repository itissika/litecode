//! Physical schema for `sessions.db`. The only rusqlite DDL in the crate
//! besides `global_db`.

use rusqlite::Connection;

use crate::session::model::SESSION_LOG_SCHEMA_VERSION;
use crate::types::{LitecodeError, Result};

use super::conn::BUSY_TIMEOUT;

pub const USER_VERSION: i32 = 5;

const SESSIONS_REQUIRED_COLS: &[&str] = &[
    "schema_version",
    "id",
    "project",
    "last_message",
    "agent_id",
    "model_id",
    "thinking_tier",
    "context_mode",
    "created_at",
    "updated_at",
    "checkpoint_seq",
    "kept_from_seq",
    "compacted_seq",
    "spine_from",
    "subagent_depth",
    "todos_json",
    "active_plan_slug",
    "parent_session_id",
    "parent_call_id",
];

const TRANSCRIPT_REQUIRED_COLS: &[&str] = &[
    "session_id",
    "seq",
    "turn_id",
    "turn_seq",
    "item_type",
    "kind",
    "body",
    "body_ref",
    "token_estimate",
    "created_at",
    "event_type",
    "surface_op",
    "source_seqs",
    "cites",
    "state",
];

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

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(cols)
}

fn table_row_count(conn: &Connection, table: &str) -> Result<i64> {
    Ok(
        conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
            row.get(0)
        })?,
    )
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
    if table_exists(conn, "messages")? {
        let n = table_row_count(conn, "messages")?;
        if n > 0 {
            return Err(incompatible_session_db(
                "legacy `messages` table still has rows",
            ));
        }
        conn.execute_batch("DROP TABLE IF EXISTS messages;")?;
    }

    if table_exists(conn, "sessions")? {
        let cols = table_columns(conn, "sessions")?;
        for req in SESSIONS_REQUIRED_COLS {
            if !cols.iter().any(|c| c == *req) {
                return Err(incompatible_session_db(&format!(
                    "`sessions` missing required column `{req}`"
                )));
            }
        }
        let stale: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE schema_version != ?1",
            rusqlite::params![SESSION_LOG_SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        if stale > 0 {
            return Err(incompatible_session_db(&format!(
                "sessions.schema_version is not {SESSION_LOG_SCHEMA_VERSION} (compacted [from,to) semantics)"
            )));
        }
    }

    if table_exists(conn, "transcript_items")? {
        let cols = table_columns(conn, "transcript_items")?;
        for req in TRANSCRIPT_REQUIRED_COLS {
            if !cols.iter().any(|c| c == *req) {
                return Err(incompatible_session_db(&format!(
                    "`transcript_items` missing required column `{req}`"
                )));
            }
        }
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
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
            spine_from        INTEGER NOT NULL DEFAULT 0,
            subagent_depth    INTEGER NOT NULL DEFAULT 0,
            todos_json        TEXT NOT NULL DEFAULT '[]',
            active_plan_slug  TEXT,
            plan_revision     TEXT,
            parent_session_id TEXT,
            parent_call_id    TEXT,
            responsibility    TEXT NOT NULL DEFAULT '',
            revision          INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_project
            ON sessions(project);
        CREATE INDEX IF NOT EXISTS idx_sessions_updated
            ON sessions(updated_at);
        CREATE INDEX IF NOT EXISTS idx_sessions_parent
            ON sessions(parent_session_id);
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
            cites           TEXT,
            state           TEXT NOT NULL DEFAULT 'final',
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_seq
            ON transcript_items(session_id, seq);
        CREATE INDEX IF NOT EXISTS idx_transcript_items_session_turn
            ON transcript_items(session_id, turn_id, turn_seq);
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
        CREATE TABLE IF NOT EXISTS session_change_log (
            change_id   INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id  TEXT NOT NULL,
            revision    INTEGER NOT NULL,
            kind        TEXT NOT NULL,
            from_seq    INTEGER,
            to_seq      INTEGER,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_change_log_id
            ON session_change_log(change_id);
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

    migrate_optional_columns(conn)?;
    if table_exists(conn, "session_context_meter")? {
        migrate_meter_table(conn)?;
    }
    drop_legacy_fts(conn)?;
    conn.execute_batch(&format!("PRAGMA user_version={USER_VERSION};"))?;
    Ok(())
}

fn migrate_optional_columns(conn: &Connection) -> Result<()> {
    if table_exists(conn, "sessions")? {
        let cols = table_columns(conn, "sessions")?;
        if !cols.iter().any(|c| c == "revision") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "last_assistant") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN last_assistant TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "responsibility") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN responsibility TEXT NOT NULL DEFAULT ''",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "plan_revision") {
            conn.execute("ALTER TABLE sessions ADD COLUMN plan_revision TEXT", [])?;
        }
    }
    Ok(())
}

/// One-time removal of the retired in-store FTS projection.
///
/// `transcript_items.search_text` plus the `transcript_fts` shadow table were
/// superseded by the chunked sparse index under `session-index/`. The write
/// triggers still fired on every mutation while nothing read the index, so drop
/// them here to stop paying that cost. Fresh databases never create any of it.
fn drop_legacy_fts(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS transcript_items_ai;
         DROP TRIGGER IF EXISTS transcript_items_ad;
         DROP TRIGGER IF EXISTS transcript_items_au;
         DROP TABLE IF EXISTS transcript_fts;
         DROP TABLE IF EXISTS transcript_fts_state;",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("drop legacy FTS: {e}")))?;

    // Best-effort: reclaiming the stale projection must never keep the store
    // from opening. SQLite refuses DROP COLUMN while anything still references
    // the column, and that refusal is not worth failing startup over.
    if table_exists(conn, "transcript_items")? {
        let cols = table_columns(conn, "transcript_items")?;
        if cols.iter().any(|c| c == "search_text") {
            if let Err(e) = conn.execute(
                "ALTER TABLE transcript_items DROP COLUMN search_text",
                [],
            ) {
                tracing::warn!(
                    error = %e,
                    "session transcript keep retired search_text column"
                );
            }
        }
    }
    Ok(())
}

fn migrate_meter_table(conn: &Connection) -> Result<()> {
    let cols = table_columns(conn, "session_context_meter")?;
    if cols.iter().any(|c| c == "token_estimate") {
        conn.execute_batch(
            "CREATE TABLE session_context_meter__new (
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
            INSERT INTO session_context_meter__new (
                session_id, prompt_tokens, completion_tokens,
                cache_hit_tokens, cache_miss_tokens, updated_at
            )
            SELECT session_id, prompt_tokens, completion_tokens,
                   cache_hit_tokens, cache_miss_tokens, updated_at
            FROM session_context_meter;
            DROP TABLE session_context_meter;
            ALTER TABLE session_context_meter__new RENAME TO session_context_meter;",
        )?;
    }
    let cols = table_columns(conn, "session_context_meter")?;
    for (col, ddl) in [
        (
            "cum_prompt_tokens",
            "cum_prompt_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "cum_completion_tokens",
            "cum_completion_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "cum_cache_hit_tokens",
            "cum_cache_hit_tokens INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "cum_cache_miss_tokens",
            "cum_cache_miss_tokens INTEGER NOT NULL DEFAULT 0",
        ),
    ] {
        if !cols.iter().any(|c| c == col) {
            conn.execute(
                &format!("ALTER TABLE session_context_meter ADD COLUMN {ddl}"),
                [],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn fresh_db_builds_base_tables_before_fts() {
        let dir = TempDir::new().unwrap();
        let conn = Connection::open(dir.path().join("sessions.db")).unwrap();
        configure_write(&conn).unwrap();
        ensure_session_schema(&conn).unwrap();
        assert!(table_exists(&conn, "sessions").unwrap());
        assert!(table_exists(&conn, "transcript_items").unwrap());
        assert!(!table_exists(&conn, "transcript_fts").unwrap());
        assert!(
            table_columns(&conn, "sessions")
                .unwrap()
                .iter()
                .any(|column| column == "responsibility")
        );
        assert_eq!(user_version(&conn).unwrap(), USER_VERSION);
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal.to_lowercase(), "wal");
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn existing_session_db_adds_responsibility_without_losing_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute_batch("ALTER TABLE sessions DROP COLUMN responsibility;")
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (
                id, schema_version, project, agent_id, created_at, updated_at
             ) VALUES ('existing', ?1, '/project', 'default', 1, 1)",
            [SESSION_LOG_SCHEMA_VERSION],
        )
        .unwrap();

        ensure_session_schema(&conn).unwrap();

        let responsibility: String = conn
            .query_row(
                "SELECT responsibility FROM sessions WHERE id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(responsibility.is_empty());
    }

    #[test]
    fn half_old_sessions_schema_fails_closed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project TEXT NOT NULL
            );",
        )
        .unwrap();
        let err = ensure_session_schema(&conn).unwrap_err();
        assert!(err.to_string().contains("incompatible"));
    }
}
