//! Physical schema for `sessions.db`. The only rusqlite DDL in the crate
//! besides `global_db`.

use rusqlite::Connection;

use crate::session::model::SESSION_LOG_SCHEMA_VERSION;
use crate::types::{LitecodeError, Result};

use super::conn::BUSY_TIMEOUT;

pub const USER_VERSION: i32 = 8;

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
            surface_rewrite_seq INTEGER NOT NULL DEFAULT 0,
            truncate_epoch    INTEGER NOT NULL DEFAULT 0,
            spine_from        INTEGER NOT NULL DEFAULT 0,
            subagent_depth    INTEGER NOT NULL DEFAULT 0,
            todos_json        TEXT NOT NULL DEFAULT '[]',
            active_plan_slug  TEXT,
            plan_revision     TEXT,
            parent_session_id TEXT,
            parent_call_id    TEXT,
            responsibility    TEXT NOT NULL DEFAULT '',
            revision          INTEGER NOT NULL DEFAULT 0,
            next_seq          INTEGER NOT NULL DEFAULT 0,
            deleted_children_json TEXT NOT NULL DEFAULT '{}'
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
        -- Legacy append-only change log. New writes use `session_dirty`; this
        -- table remains so v8 can seed dirty state from pre-upgrade databases.
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

    migrate_optional_columns(conn)?;
    // WP-186 P1 sidecar for pending user-gate asks.
    crate::session::data::sqlite::pending::ensure_pending_permissions_table(conn)?;
    if table_exists(conn, "session_context_meter")? {
        migrate_meter_table(conn)?;
    }
    drop_legacy_fts(conn)?;
    drop_legacy_content_outbox(conn)?;
    if user_version(conn)? < 7 && table_exists(conn, "transcript_items")? {
        migrate_reminders_v7(conn)?;
    }
    if user_version(conn)? < 8 {
        migrate_dirty_sessions_v8(conn)?;
    }
    conn.execute_batch(&format!("PRAGMA user_version={USER_VERSION};"))?;
    Ok(())
}

/// v8 replaces the append-only change log with one coalesced dirty row per
/// session. Seed the clock at the old high-water mark so an index checkpoint
/// from the previous format can advance exactly once.
fn migrate_dirty_sessions_v8(conn: &Connection) -> Result<()> {
    let initialized: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_change_clock WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if initialized > 0 {
        return Ok(());
    }
    let legacy_max: i64 = conn.query_row(
        "SELECT COALESCE(MAX(change_id), 0) FROM session_change_log",
        [],
        |row| row.get(0),
    )?;
    let seed = legacy_max.max(1);
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO session_change_clock (singleton, last_change_id) VALUES (1, ?1)",
        rusqlite::params![seed],
    )?;
    // Any pre-v8 index checkpoint below `seed` must see every session that
    // existed or changed in the old log. The next reconcile compares actual
    // searchable keys, so over-marking is safe and under-marking is not.
    tx.execute(
        "INSERT OR IGNORE INTO session_dirty (session_id, change_id)
         SELECT id, ?1 FROM sessions",
        rusqlite::params![seed],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO session_dirty (session_id, change_id)
         SELECT DISTINCT session_id, ?1 FROM session_change_log",
        rusqlite::params![seed],
    )?;
    tx.commit()?;
    Ok(())
}

/// Rewrite legacy reminder rows in place. `seq` and `next_seq` stay put.
fn migrate_reminders_v7(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "SELECT session_id, seq, kind, body FROM transcript_items
             WHERE kind IN ('reminder/job_exit', 'reminder/plan')",
        )?;
        let rows: Vec<(String, i64, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);
        for (session_id, seq, kind, body) in rows {
            let Some((new_kind, new_body)) = crate::reminder::migrate::rewrite_reminder_row(
                &kind,
                body.as_deref().unwrap_or(""),
            ) else {
                continue;
            };
            tx.execute(
                "UPDATE transcript_items
                 SET kind = ?1, event_type = ?1, body = ?2
                 WHERE session_id = ?3 AND seq = ?4",
                rusqlite::params![new_kind, new_body, session_id, seq],
            )?;
        }

        let mut compacted = tx.prepare(
            "SELECT session_id, seq, body FROM transcript_items WHERE kind = 'compacted'",
        )?;
        let compacted_rows: Vec<(String, i64, String)> = compacted
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(compacted);
        for (session_id, seq, body) in compacted_rows {
            let Ok(mut parsed) =
                serde_json::from_str::<crate::session::model::CompactedBody>(&body)
            else {
                continue;
            };
            let stripped = crate::reminder::migrate::strip_embedded_reminder(&parsed.summary);
            if stripped == parsed.summary {
                continue;
            }
            parsed.summary = stripped;
            let encoded = serde_json::to_string(&parsed)?;
            tx.execute(
                "UPDATE transcript_items SET body = ?1 WHERE session_id = ?2 AND seq = ?3",
                rusqlite::params![encoded, session_id, seq],
            )?;
        }
    }
    tx.commit()?;
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
        // WP-091 R7: tombstones of directly deleted child sessions,
        // `{parent_call_id: child_session_id}`. Additive.
        if !cols.iter().any(|c| c == "deleted_children_json") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN deleted_children_json TEXT NOT NULL DEFAULT '{}'",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "plan_revision") {
            conn.execute("ALTER TABLE sessions ADD COLUMN plan_revision TEXT", [])?;
        }
        // Latest replay-safe surface replacement seq. A provider bill whose
        // request header is at or before this watermark no longer describes the
        // current prefix and must fall back to the local request envelope.
        if !cols.iter().any(|c| c == "surface_rewrite_seq") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN surface_rewrite_seq INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        // Bumped by every tail truncate (revert). A fold holding rows above the
        // surviving tail cannot detect the deletion from seq continuity alone:
        // seqs are never reused, so the next append looks continuous.
        if !cols.iter().any(|c| c == "truncate_epoch") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN truncate_epoch INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        if !cols.iter().any(|c| c == "next_seq") {
            conn.execute(
                "ALTER TABLE sessions ADD COLUMN next_seq INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
            // Additive migration for a database written before `next_seq`
            // existed. Backfill every session to at least one past its current
            // `MAX(seq)`, so the persisted allocation high-water can never hand
            // out a seq some row already used (or that a truncate left behind).
            conn.execute(
                "UPDATE sessions
                 SET next_seq = max(
                     next_seq,
                     (SELECT COALESCE(MAX(seq), -1) + 1
                      FROM transcript_items t WHERE t.session_id = sessions.id)
                 )",
                [],
            )?;
        }
    }
    if table_exists(conn, "transcript_items")? {
        let cols = table_columns(conn, "transcript_items")?;
        // Side column for a finished request's provider bill. Readers of
        // body/kind/state do not name it. No USER_VERSION bump.
        if !cols.iter().any(|c| c == "provider_usage") {
            conn.execute(
                "ALTER TABLE transcript_items ADD COLUMN provider_usage TEXT",
                [],
            )?;
        }
        // UI-only structured tool result data (`run_code` aggregate). Additive
        // and nullable: old rows read as absent and the frontend falls back to
        // text. Not a required column and no schema-version bump.
        if !cols.iter().any(|c| c == "result_data") {
            conn.execute(
                "ALTER TABLE transcript_items ADD COLUMN result_data TEXT",
                [],
            )?;
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
            if let Err(e) = conn.execute("ALTER TABLE transcript_items DROP COLUMN search_text", [])
            {
                tracing::warn!(
                    error = %e,
                    "session transcript keep retired search_text column"
                );
            }
        }
    }
    Ok(())
}

/// One-time removal of the retired content-impact outbox.
///
/// The derived index no longer consumes an impact stream, so the outbox tables
/// are dead weight: every mutation used to insert a batch while nothing read it.
/// Drop them here to stop paying that cost. Fresh databases never create them.
fn drop_legacy_content_outbox(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS session_content_impacts;
         DROP TABLE IF EXISTS session_content_change_batches;",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("drop legacy content outbox: {e}")))?;
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

    #[test]
    fn existing_session_db_adds_result_data_column() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute_batch("ALTER TABLE transcript_items DROP COLUMN result_data;")
            .unwrap();
        assert!(
            !table_columns(&conn, "transcript_items")
                .unwrap()
                .iter()
                .any(|c| c == "result_data")
        );

        ensure_session_schema(&conn).unwrap();

        assert!(
            table_columns(&conn, "transcript_items")
                .unwrap()
                .iter()
                .any(|c| c == "result_data")
        );
    }

    #[test]
    fn existing_session_db_backfills_next_seq_to_max_plus_one() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        // Simulate a database written before `next_seq` existed. `next_seq` is
        // deliberately **not** a required column, so this must migrate, not fail
        // the pre-migration rejection.
        conn.execute_batch("ALTER TABLE sessions DROP COLUMN next_seq;")
            .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, schema_version, project, agent_id, created_at, updated_at)
             VALUES ('s1', ?1, '/p', 'default', 1, 1)",
            [SESSION_LOG_SCHEMA_VERSION],
        )
        .unwrap();
        // Non-contiguous seqs (0 and 4): the backfill must clear the maximum.
        conn.execute_batch(
            "INSERT INTO transcript_items
                (session_id, seq, item_type, kind, created_at, event_type, surface_op)
             VALUES ('s1', 0, 'message', 'item/user', 1, 'item/user', 'append'),
                    ('s1', 4, 'message', 'item/assistant', 1, 'item/assistant', 'append');",
        )
        .unwrap();
        assert!(
            !table_columns(&conn, "sessions")
                .unwrap()
                .iter()
                .any(|c| c == "next_seq")
        );

        ensure_session_schema(&conn).unwrap();

        let next: i64 = conn
            .query_row("SELECT next_seq FROM sessions WHERE id = 's1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(next, 5, "backfill must be MAX(seq)+1 = 5");
    }

    #[test]
    fn v8_coalesces_legacy_change_log_into_one_dirty_row_per_session() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE session_dirty;
             DROP TABLE session_change_clock;
             PRAGMA user_version = 7;",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, schema_version, project, agent_id, created_at, updated_at)
             VALUES ('s1', ?1, '/p', 'default', 1, 1)",
            [SESSION_LOG_SCHEMA_VERSION],
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO session_change_log (session_id, revision, kind, created_at)
             VALUES ('s1', 2, 'Created', 1),
                    ('s1', 3, 'MetaUpdated', 2);",
        )
        .unwrap();

        ensure_session_schema(&conn).unwrap();

        let clock: i64 = conn
            .query_row(
                "SELECT last_change_id FROM session_change_clock WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(clock, 2, "clock must keep the legacy high-water mark");
        let dirty: Vec<(String, i64)> = {
            let mut stmt = conn
                .prepare("SELECT session_id, change_id FROM session_dirty")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        assert_eq!(dirty, vec![("s1".to_string(), 2)]);
        let index: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_session_operations_operation_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index, 1);
    }
}
