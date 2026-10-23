//! Writer-side SQL helpers that must not live outside this directory.

use rusqlite::{Connection, OptionalExtension};

use crate::types::Result;

use super::super::command::{CommitKind, CommitReceipt};

/// Which durable bookkeeping a commit needs. A receipt is only useful when a
/// logical operation can be retried. A dirty marker is only useful when the
/// commit changes searchable state. Internal stream checkpoints need neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JournalWrite {
    Full,
    ReceiptOnly,
    DirtyOnly,
    None,
}

impl JournalWrite {
    fn writes_receipt(self) -> bool {
        matches!(self, Self::Full | Self::ReceiptOnly)
    }

    fn marks_dirty(self) -> bool {
        matches!(self, Self::Full | Self::DirtyOnly)
    }
}

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

pub(crate) fn record_commit(
    conn: &Connection,
    receipt: &CommitReceipt,
    journal: JournalWrite,
) -> Result<()> {
    if journal.writes_receipt() {
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
    }
    if !receipt.session_id.is_empty() {
        if matches!(receipt.outcome, CommitKind::Deleted) {
            // A delete removes the `sessions` row in this same transaction, so
            // there is no revision column left to bump. The CAS guarantee is
            // that the row is really gone; a surviving row means the delete did
            // not take effect and the receipt must not commit.
            let still_there: Option<i64> = conn
                .query_row(
                    "SELECT 1 FROM sessions WHERE id = ?1",
                    rusqlite::params![receipt.session_id],
                    |row| row.get(0),
                )
                .optional()?;
            if still_there.is_some() {
                return Err(crate::types::LitecodeError::SessionStorage(format!(
                    "delete receipt for session {} but its sessions row still exists",
                    receipt.session_id
                )));
            }
        } else {
            let updated = conn.execute(
                "UPDATE sessions SET revision = ?1 WHERE id = ?2",
                rusqlite::params![receipt.revision as i64, receipt.session_id],
            )?;
            if updated != 1 {
                return Err(crate::types::LitecodeError::SessionStorage(format!(
                    "sessions.revision update affected {updated} rows for session {}",
                    receipt.session_id
                )));
            }
        }
        if journal.marks_dirty() {
            mark_session_dirty(conn, &receipt.session_id)?;
        }
    }
    Ok(())
}

/// Advance the global dirty clock and coalesce this session into one row.
///
/// The search index runs on a coarse tick and only needs the set of sessions
/// whose searchable keys changed since its checkpoint. The clock orders those
/// changes; the row count stays bounded by the number of sessions.
pub(crate) fn mark_session_dirty(conn: &Connection, session_id: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE session_change_clock
         SET last_change_id = last_change_id + 1
         WHERE singleton = 1",
        [],
    )?;
    if changed != 1 {
        return Err(crate::types::LitecodeError::SessionStorage(
            "session_change_clock singleton row is missing".into(),
        ));
    }
    let change_id: i64 = conn.query_row(
        "SELECT last_change_id FROM session_change_clock WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO session_dirty (session_id, change_id)
         VALUES (?1, ?2)
         ON CONFLICT(session_id) DO UPDATE SET change_id = excluded.change_id",
        rusqlite::params![session_id, change_id],
    )?;
    Ok(())
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
    // Union the ref table with body_ref columns still present in
    // transcript_items: any writer path that spills a body without registering
    // a ref row must not cause GC to delete a live blob.
    let mut stmt = conn.prepare(
        "SELECT DISTINCT blob_id FROM session_blob_refs
         UNION
         SELECT DISTINCT substr(body_ref, 7, instr(body_ref, ']') - 7)
         FROM transcript_items
         WHERE body_ref LIKE '[blob:%]'",
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::data::sqlite::schema::ensure_session_schema;

    fn receipt(session_id: &str, revision: u64) -> CommitReceipt {
        CommitReceipt {
            session_id: session_id.to_string(),
            operation_id: "op-1".into(),
            revision,
            outcome: CommitKind::MetaUpdated,
            preview: None,
            assistant_preview: None,
            working_set: None,
        }
    }

    #[test]
    fn full_journal_rejects_deleted_receipt_for_surviving_session_row() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (
                id, project, agent_id, created_at, updated_at, schema_version, revision
             ) VALUES (?1, '/p', 'default', 1, 1, 3, 1)",
            rusqlite::params!["s1"],
        )
        .unwrap();
        let mut r = receipt("s1", 2);
        r.outcome = CommitKind::Deleted;
        let err = record_commit(&conn, &r, JournalWrite::Full)
            .unwrap_err()
            .to_string();
        assert!(err.contains("still exists"), "got {err}");
    }

    #[test]
    fn legacy_receipt_json_with_removed_change_id_still_loads() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        ensure_session_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO session_operations
                (session_id, operation_id, receipt_json, created_at)
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![
                "s1",
                "op-legacy",
                r#"{"session_id":"s1","operation_id":"op-legacy","revision":7,"change_id":9,"outcome":"MetaUpdated"}"#
            ],
        )
        .unwrap();

        let receipt = load_operation(&conn, "s1", "op-legacy")
            .unwrap()
            .expect("legacy receipt");
        assert_eq!(receipt.revision, 7);
        assert_eq!(receipt.outcome, CommitKind::MetaUpdated);
    }
}
