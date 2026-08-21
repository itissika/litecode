//! Transcript FTS5 index for always-on session lexical search (SQLite bm25).
//!
//! Lives under `sessions.db` so it is ready without the code_search engine.

use rusqlite::{Connection, OptionalExtension};

use crate::types::{LitecodeError, Result};

pub const FTS_TABLE: &str = "transcript_fts";

/// Ensure FTS5 virtual table exists. No-op if already present.
pub fn ensure_schema(conn: &Connection) -> Result<()> {
    // Probe FTS5 availability early with a clear error.
    let has_fts5: bool = conn
        .query_row(
            "SELECT 1 FROM pragma_module_list WHERE name = 'fts5'",
            [],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| LitecodeError::Config(format!("fts5 probe: {e}")))?
        .unwrap_or(false);
    if !has_fts5 {
        // Some builds omit pragma_module_list; try CREATE and surface errors.
        tracing::debug!("fts5 module list probe empty; attempting CREATE VIRTUAL TABLE");
    }
    conn.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS transcript_fts USING fts5(
            session_id UNINDEXED,
            seq UNINDEXED,
            body,
            tokenize = 'unicode61 remove_diacritics 2'
        );",
    )
    .map_err(|e| LitecodeError::Config(format!("create transcript_fts: {e}")))?;
    Ok(())
}

/// Insert or replace one detail row's searchable text.
pub fn upsert(conn: &Connection, session_id: &str, seq: i64, body: &str) -> Result<()> {
    if body.trim().is_empty() {
        delete_one(conn, session_id, seq)?;
        return Ok(());
    }
    delete_one(conn, session_id, seq)?;
    conn.execute(
        "INSERT INTO transcript_fts(session_id, seq, body) VALUES (?1, ?2, ?3)",
        rusqlite::params![session_id, seq, body],
    )
    .map_err(|e| LitecodeError::Config(format!("transcript_fts upsert: {e}")))?;
    Ok(())
}

pub fn delete_one(conn: &Connection, session_id: &str, seq: i64) -> Result<()> {
    let rowids = rowids_matching(conn, session_id, Some(seq), None)?;
    for id in rowids {
        conn.execute(
            "DELETE FROM transcript_fts WHERE rowid = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| LitecodeError::Config(format!("transcript_fts delete: {e}")))?;
    }
    Ok(())
}

/// Delete FTS rows for `seq >= from_seq` in a session (revert truncate).
pub fn delete_seq_ge(conn: &Connection, session_id: &str, from_seq: i64) -> Result<()> {
    let rowids = rowids_matching(conn, session_id, None, Some(from_seq))?;
    for id in rowids {
        conn.execute(
            "DELETE FROM transcript_fts WHERE rowid = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| LitecodeError::Config(format!("transcript_fts delete_ge: {e}")))?;
    }
    Ok(())
}

pub fn delete_session(conn: &Connection, session_id: &str) -> Result<()> {
    let rowids = rowids_matching(conn, session_id, None, None)?;
    for id in rowids {
        conn.execute(
            "DELETE FROM transcript_fts WHERE rowid = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| LitecodeError::Config(format!("transcript_fts delete_session: {e}")))?;
    }
    Ok(())
}

fn rowids_matching(
    conn: &Connection,
    session_id: &str,
    seq_eq: Option<i64>,
    seq_ge: Option<i64>,
) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    if let Some(seq) = seq_eq {
        let mut stmt = conn
            .prepare("SELECT rowid FROM transcript_fts WHERE session_id = ?1 AND seq = ?2")
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid prepare: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id, seq], |r| r.get(0))
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid query: {e}")))?;
        for row in rows {
            out.push(row.map_err(|e| LitecodeError::Config(format!("transcript_fts rowid: {e}")))?);
        }
    } else if let Some(ge) = seq_ge {
        let mut stmt = conn
            .prepare("SELECT rowid FROM transcript_fts WHERE session_id = ?1 AND seq >= ?2")
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid prepare: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id, ge], |r| r.get(0))
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid query: {e}")))?;
        for row in rows {
            out.push(row.map_err(|e| LitecodeError::Config(format!("transcript_fts rowid: {e}")))?);
        }
    } else {
        let mut stmt = conn
            .prepare("SELECT rowid FROM transcript_fts WHERE session_id = ?1")
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid prepare: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![session_id], |r| r.get(0))
            .map_err(|e| LitecodeError::Config(format!("transcript_fts rowid query: {e}")))?;
        for row in rows {
            out.push(row.map_err(|e| LitecodeError::Config(format!("transcript_fts rowid: {e}")))?);
        }
    }
    Ok(out)
}

/// Escape a user query for FTS5 MATCH (phrase-ish token AND).
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
            // Quote each token so FTS operators in user text cannot break MATCH.
            format!("\"{}\"", cleaned.replace('"', ""))
        })
        .filter(|t| !t.is_empty())
        .collect();
    tokens.join(" ")
}

#[derive(Debug, Clone)]
pub struct FtsHit {
    pub session_id: String,
    pub seq: i64,
    /// Lower is better for SQLite bm25(); converted by callers.
    pub bm25: f64,
}

/// BM25 ranked FTS hits. `limit` caps rows before caller filters.
pub fn search(
    conn: &Connection,
    match_query: &str,
    limit: usize,
    include_session_id: Option<&str>,
) -> Result<Vec<FtsHit>> {
    if match_query.trim().is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let limit_i = limit as i64;
    let mut out = Vec::new();
    if let Some(sid) = include_session_id {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, seq, bm25(transcript_fts) AS rank
                 FROM transcript_fts
                 WHERE transcript_fts MATCH ?1 AND session_id = ?2
                 ORDER BY rank ASC LIMIT ?3",
            )
            .map_err(|e| LitecodeError::Config(format!("transcript_fts search prepare: {e}")))?;
        let mapped = stmt
            .query_map(rusqlite::params![match_query, sid, limit_i], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| LitecodeError::Config(format!("transcript_fts search: {e}")))?;
        for row in mapped {
            let (session_id, seq, bm25) =
                row.map_err(|e| LitecodeError::Config(format!("transcript_fts search row: {e}")))?;
            out.push(FtsHit {
                session_id,
                seq,
                bm25,
            });
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, seq, bm25(transcript_fts) AS rank
                 FROM transcript_fts
                 WHERE transcript_fts MATCH ?1
                 ORDER BY rank ASC LIMIT ?2",
            )
            .map_err(|e| LitecodeError::Config(format!("transcript_fts search prepare: {e}")))?;
        let mapped = stmt
            .query_map(rusqlite::params![match_query, limit_i], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| LitecodeError::Config(format!("transcript_fts search: {e}")))?;
        for row in mapped {
            let (session_id, seq, bm25) =
                row.map_err(|e| LitecodeError::Config(format!("transcript_fts search row: {e}")))?;
            out.push(FtsHit {
                session_id,
                seq,
                bm25,
            });
        }
    }
    Ok(out)
}

/// True when FTS row count is far below detail count (needs backfill).
pub fn needs_backfill(conn: &Connection) -> Result<bool> {
    let details: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transcript_items
             WHERE kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if details == 0 {
        return Ok(false);
    }
    let fts: i64 = conn
        .query_row("SELECT COUNT(*) FROM transcript_fts", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(fts + 8 < details) // small slack for empty-text details
}
