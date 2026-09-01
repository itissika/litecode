//! Writer-side SQL helpers that must not live outside this directory.

use rusqlite::{Connection, OptionalExtension};

use crate::types::Result;

use super::super::command::CommitReceipt;

pub fn load_operation(
    conn: &Connection,
    session_id: &str,
    operation_id: &str,
) -> Result<Option<CommitReceipt>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT receipt_json FROM session_operations
             WHERE session_id = ?1 AND operation_id = ?2",
            rusqlite::params![session_id, operation_id],
            |row| row.get(0),
        )
        .optional()?;
    match json {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

/// Creation has no session id before it commits. Mutation ids are globally
/// generated, so a prior receipt with the same id identifies an idempotent
/// create without inventing a second session row.
pub fn load_operation_by_id(
    conn: &Connection,
    operation_id: &str,
) -> Result<Option<CommitReceipt>> {
    let json: Option<String> = conn
        .query_row(
            "SELECT receipt_json FROM session_operations
             WHERE operation_id = ?1
             ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![operation_id],
            |row| row.get(0),
        )
        .optional()?;
    match json {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

pub fn persist_receipt(conn: &Connection, receipt: &CommitReceipt) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO session_operations (session_id, operation_id, receipt_json, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            receipt.session_id,
            receipt.operation_id,
            serde_json::to_string(receipt)?,
            chrono::Utc::now().timestamp_millis()
        ],
    )?;
    if !receipt.session_id.is_empty() {
        let _ = conn.execute(
            "UPDATE sessions SET revision = ?1 WHERE id = ?2",
            rusqlite::params![receipt.revision as i64, receipt.session_id],
        );
        conn.execute(
            "INSERT INTO session_change_log (session_id, revision, kind, from_seq, to_seq, created_at)
             VALUES (?1, ?2, ?3, NULL, NULL, ?4)",
            rusqlite::params![
                receipt.session_id,
                receipt.revision as i64,
                format!("{:?}", receipt.outcome),
                chrono::Utc::now().timestamp_millis()
            ],
        )?;
    }
    Ok(())
}

pub fn latest_change_id(conn: &Connection) -> Result<i64> {
    let id: Option<i64> =
        conn.query_row("SELECT MAX(change_id) FROM session_change_log", [], |row| {
            row.get(0)
        })?;
    Ok(id.unwrap_or(0))
}

pub fn register_blob_ref(
    conn: &Connection,
    blob_id: &str,
    session_id: &str,
    seq: i64,
    bytes: i64,
    rel_path: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO session_blobs (blob_id, sha256, bytes, rel_path, created_at)
         VALUES (?1, ?1, ?2, ?3, ?4)",
        rusqlite::params![
            blob_id,
            bytes,
            rel_path,
            chrono::Utc::now().timestamp_millis()
        ],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO session_blob_refs (blob_id, session_id, seq)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![blob_id, session_id, seq],
    )?;
    Ok(())
}

pub fn drop_blob_refs_ge(conn: &Connection, session_id: &str, from_seq: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM session_blob_refs WHERE session_id = ?1 AND seq >= ?2",
        rusqlite::params![session_id, from_seq],
    )?;
    Ok(())
}

pub fn drop_blob_refs_session(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM session_blob_refs WHERE session_id = ?1",
        rusqlite::params![session_id],
    )?;
    Ok(())
}

pub fn unreferenced_blob_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT b.blob_id FROM session_blobs b
         LEFT JOIN session_blob_refs r ON r.blob_id = b.blob_id
         WHERE r.blob_id IS NULL",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

pub fn referenced_blob_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT blob_id FROM session_blob_refs")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut ids = Vec::new();
    for row in rows {
        ids.push(row?);
    }
    Ok(ids)
}

pub fn delete_blob_rows(conn: &Connection, ids: &[String]) -> Result<()> {
    for id in ids {
        conn.execute(
            "DELETE FROM session_blobs WHERE blob_id = ?1",
            rusqlite::params![id],
        )?;
    }
    Ok(())
}
