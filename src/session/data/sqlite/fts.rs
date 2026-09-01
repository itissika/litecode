//! FTS5 as an external-content projection of `transcript_items.search_text`.
//! Triggers keep it in the same SQLite transaction as the log write.

use rusqlite::{Connection, OptionalExtension};

use crate::types::{LitecodeError, Result};

pub const FTS_TABLE: &str = "transcript_fts";

const SEARCHABLE_KINDS: &str = "'item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted', 'reminder/job_exit'";

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    let has_fts5: bool = conn
        .query_row(
            "SELECT 1 FROM pragma_module_list WHERE name = 'fts5'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has_fts5 {
        tracing::debug!("fts5 module list probe empty; attempting CREATE VIRTUAL TABLE");
    }

    if fts_needs_rebuild(conn)? {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS transcript_items_ai;
             DROP TRIGGER IF EXISTS transcript_items_ad;
             DROP TRIGGER IF EXISTS transcript_items_au;
             DROP TABLE IF EXISTS transcript_fts;",
        )
        .map_err(|e| LitecodeError::SessionStorage(format!("drop legacy FTS: {e}")))?;
    }

    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS transcript_fts USING fts5(
            search_text,
            content='transcript_items',
            content_rowid='rowid',
            tokenize = 'unicode61 remove_diacritics 2'
        );
        CREATE TABLE IF NOT EXISTS transcript_fts_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("create transcript_fts: {e}")))?;

    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS transcript_items_ai AFTER INSERT ON transcript_items BEGIN
            INSERT INTO transcript_fts(rowid, search_text)
            SELECT new.rowid, new.search_text
            WHERE new.search_text IS NOT NULL AND length(trim(new.search_text)) > 0;
         END;
         CREATE TRIGGER IF NOT EXISTS transcript_items_ad AFTER DELETE ON transcript_items BEGIN
            INSERT INTO transcript_fts(transcript_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
         END;
         CREATE TRIGGER IF NOT EXISTS transcript_items_au AFTER UPDATE ON transcript_items BEGIN
            INSERT INTO transcript_fts(transcript_fts, rowid, search_text)
            VALUES('delete', old.rowid, old.search_text);
            INSERT INTO transcript_fts(rowid, search_text)
            SELECT new.rowid, new.search_text
            WHERE new.search_text IS NOT NULL AND length(trim(new.search_text)) > 0;
         END;",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("create FTS triggers: {e}")))?;

    backfill_search_text(conn)?;
    Ok(())
}

fn fts_needs_rebuild(conn: &Connection) -> Result<bool> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transcript_fts'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(false);
    }
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='transcript_fts'",
        [],
        |row| row.get(0),
    )?;
    Ok(
        !sql.contains("content='transcript_items'")
            && !sql.contains("content=\"transcript_items\""),
    )
}

fn backfill_search_text(conn: &Connection) -> Result<()> {
    let pending: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM transcript_items
                 WHERE kind IN ({SEARCHABLE_KINDS})
                   AND search_text IS NULL
                   AND (body IS NOT NULL OR body_ref IS NOT NULL)"
            ),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if pending == 0 {
        mark_ready(conn)?;
        return Ok(());
    }
    conn.execute(
        &format!(
            "UPDATE transcript_items
             SET search_text = COALESCE(body, '')
             WHERE kind IN ({SEARCHABLE_KINDS})
               AND search_text IS NULL
               AND body IS NOT NULL"
        ),
        [],
    )?;
    conn.execute(
        "INSERT INTO transcript_fts(transcript_fts) VALUES('rebuild')",
        [],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("FTS rebuild: {e}")))?;
    mark_ready(conn)?;
    Ok(())
}

fn mark_ready(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO transcript_fts_state (key, value) VALUES ('ready', '1')",
        [],
    )?;
    Ok(())
}

pub fn is_ready(conn: &Connection) -> Result<bool> {
    let ready: Option<String> = conn
        .query_row(
            "SELECT value FROM transcript_fts_state WHERE key = 'ready'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(ready.as_deref() == Some("1"))
}

pub fn escape_match_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            let cleaned: String = t
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect();
            if cleaned.is_empty() {
                return String::new();
            }
            format!("\"{}\"", cleaned.replace('"', ""))
        })
        .filter(|t| !t.is_empty())
        .collect();
    tokens.join(" ")
}

pub fn search(
    conn: &Connection,
    match_query: &str,
    session_id: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, i64, String)>> {
    let sql = if session_id.is_some() {
        "SELECT ti.session_id, ti.seq, COALESCE(ti.search_text, '')
         FROM transcript_fts f
         JOIN transcript_items ti ON ti.rowid = f.rowid
         WHERE transcript_fts MATCH ?1 AND ti.session_id = ?2
         LIMIT ?3"
    } else {
        "SELECT ti.session_id, ti.seq, COALESCE(ti.search_text, '')
         FROM transcript_fts f
         JOIN transcript_items ti ON ti.rowid = f.rowid
         WHERE transcript_fts MATCH ?1
         LIMIT ?2"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| LitecodeError::SessionStorage(format!("FTS search prepare: {e}")))?;
    let limit = limit as i64;
    let mut out = Vec::new();
    if let Some(sid) = session_id {
        let mapped = stmt
            .query_map(rusqlite::params![match_query, sid, limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| LitecodeError::SessionStorage(format!("FTS search: {e}")))?;
        for row in mapped {
            out.push(row.map_err(|e| LitecodeError::SessionStorage(format!("FTS row: {e}")))?);
        }
    } else {
        let mapped = stmt
            .query_map(rusqlite::params![match_query, limit], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| LitecodeError::SessionStorage(format!("FTS search: {e}")))?;
        for row in mapped {
            out.push(row.map_err(|e| LitecodeError::SessionStorage(format!("FTS row: {e}")))?);
        }
    }
    Ok(out)
}

pub fn rebuild(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO transcript_fts(transcript_fts) VALUES('rebuild')",
        [],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("FTS rebuild: {e}")))?;
    mark_ready(conn)?;
    Ok(())
}
