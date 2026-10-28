//! Display-only session layout: title override + folder placement.
//!
//! Every write here is revision-neutral. `sessions.revision` is the transcript
//! CAS used by `CommitTurnDelta`; a rename must never make a live turn's next
//! flush fail. `updated_at` is the sidebar order / empty-GC clock; a rename
//! must not move a session or keep it alive. No `session_dirty` either: the
//! search corpus does not index titles or folder names.
//!
//! The folder FK is `ON DELETE SET NULL`: deleting a folder returns its
//! sessions to the ungrouped bucket and never deletes a conversation.

use rusqlite::{Connection, OptionalExtension};

use crate::session::data::command::SessionFolderRow;
use crate::types::{LitecodeError, Result};

/// Folders in sidebar order. Empty folders are intentionally returned.
pub fn list_folders(conn: &Connection) -> Result<Vec<SessionFolderRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, sort_order
         FROM session_folders
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SessionFolderRow {
            id: row.get(0)?,
            name: row.get(1)?,
            sort_order: row.get(2)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn folder_exists(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM session_folders WHERE id = ?1", [id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

/// Set or clear the user title override. Whitespace-only becomes NULL.
pub fn set_title(conn: &Connection, session_id: &str, title: Option<&str>) -> Result<()> {
    let title = normalize_title(title)?;
    let updated = conn.execute(
        "UPDATE sessions SET title = ?1 WHERE id = ?2",
        rusqlite::params![title, session_id],
    )?;
    if updated != 1 {
        return Err(LitecodeError::SessionNotFound(session_id.to_string()));
    }
    Ok(())
}

/// Move a session into a folder, or to the ungrouped bucket when NULL.
pub fn set_folder(conn: &Connection, session_id: &str, folder_id: Option<&str>) -> Result<()> {
    if let Some(folder_id) = folder_id {
        if !folder_exists(conn, folder_id)? {
            return Err(LitecodeError::SessionStorage(format!(
                "folder not found: {folder_id}"
            )));
        }
    }
    let updated = conn.execute(
        "UPDATE sessions SET folder_id = ?1 WHERE id = ?2",
        rusqlite::params![folder_id, session_id],
    )?;
    if updated != 1 {
        return Err(LitecodeError::SessionNotFound(session_id.to_string()));
    }
    Ok(())
}

pub fn create_folder(conn: &Connection, id: &str, name: &str) -> Result<()> {
    let name = normalize_folder_name(name)?;
    let sort_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM session_folders",
        [],
        |row| row.get(0),
    )?;
    let now = now_ms();
    conn.execute(
        "INSERT INTO session_folders (id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![id, name, sort_order, now],
    )?;
    Ok(())
}

pub fn rename_folder(conn: &Connection, id: &str, name: &str) -> Result<()> {
    let name = normalize_folder_name(name)?;
    let updated = conn.execute(
        "UPDATE session_folders SET name = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![name, now_ms(), id],
    )?;
    if updated != 1 {
        return Err(LitecodeError::SessionStorage(format!(
            "folder not found: {id}"
        )));
    }
    Ok(())
}

/// Delete the folder row only. The FK sets every member's folder_id to NULL in
/// this same transaction.
pub fn delete_folder(conn: &Connection, id: &str) -> Result<()> {
    let deleted = conn.execute("DELETE FROM session_folders WHERE id = ?1", [id])?;
    if deleted != 1 {
        return Err(LitecodeError::SessionStorage(format!(
            "folder not found: {id}"
        )));
    }
    Ok(())
}

fn normalize_title(title: Option<&str>) -> Result<Option<&str>> {
    let title = title.map(str::trim).filter(|value| !value.is_empty());
    if let Some(title) = title {
        if title.chars().count() > 200 {
            return Err(LitecodeError::SessionStorage(
                "session title is too long".into(),
            ));
        }
    }
    Ok(title)
}

fn normalize_folder_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(LitecodeError::SessionStorage(
            "folder name must not be empty".into(),
        ));
    }
    if name.chars().count() > 120 {
        return Err(LitecodeError::SessionStorage(
            "folder name is too long".into(),
        ));
    }
    Ok(name)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
