//! P1 sidecar: `session_pending_permissions` table (WP-186).

use rusqlite::{Connection, OptionalExtension, params};

use crate::permission::{AskKind, AskOption, AskQuestion};
use crate::session::pending::PendingPermissionRecord;
use crate::types::{LitecodeError, Result};

pub fn ensure_pending_permissions_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_pending_permissions (
            session_id   TEXT NOT NULL,
            request_id   TEXT NOT NULL,
            turn_id      TEXT NOT NULL,
            tool         TEXT NOT NULL,
            rule_id      TEXT NOT NULL,
            summary      TEXT NOT NULL,
            kind         TEXT NOT NULL,
            free_text    INTEGER NOT NULL DEFAULT 0,
            multi_select INTEGER NOT NULL DEFAULT 0,
            options_json TEXT NOT NULL DEFAULT '[]',
            questions_json TEXT NOT NULL DEFAULT '[]',
            call_id      TEXT,
            created_at   INTEGER NOT NULL,
            PRIMARY KEY (session_id, request_id)
        );
        CREATE INDEX IF NOT EXISTS idx_session_pending_permissions_session
            ON session_pending_permissions(session_id);",
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("pending_permissions DDL: {e}")))?;
    Ok(())
}

fn kind_to_str(kind: AskKind) -> &'static str {
    match kind {
        AskKind::Permission => "permission",
        AskKind::Approval => "approval",
        AskKind::AskUser => "ask_user",
    }
}

fn kind_from_str(s: &str) -> AskKind {
    match s {
        "approval" => AskKind::Approval,
        "ask_user" => AskKind::AskUser,
        _ => AskKind::Permission,
    }
}

pub fn upsert_pending(conn: &Connection, rec: &PendingPermissionRecord) -> Result<()> {
    let options_json = serde_json::to_string(&rec.options)
        .map_err(|e| LitecodeError::SessionStorage(format!("pending options json: {e}")))?;
    let questions_json = serde_json::to_string(&rec.questions)
        .map_err(|e| LitecodeError::SessionStorage(format!("pending questions json: {e}")))?;
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO session_pending_permissions (
            session_id, request_id, turn_id, tool, rule_id, summary, kind,
            free_text, multi_select, options_json, questions_json, call_id, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(session_id, request_id) DO UPDATE SET
            turn_id=excluded.turn_id,
            tool=excluded.tool,
            rule_id=excluded.rule_id,
            summary=excluded.summary,
            kind=excluded.kind,
            free_text=excluded.free_text,
            multi_select=excluded.multi_select,
            options_json=excluded.options_json,
            questions_json=excluded.questions_json,
            call_id=excluded.call_id",
        params![
            rec.session_id,
            rec.request_id,
            rec.turn_id,
            rec.tool,
            rec.rule_id,
            rec.summary,
            kind_to_str(rec.kind),
            rec.free_text as i64,
            rec.multi_select as i64,
            options_json,
            questions_json,
            rec.call_id,
            now,
        ],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("upsert pending_permission: {e}")))?;
    Ok(())
}

pub fn list_pending(conn: &Connection, session_id: &str) -> Result<Vec<PendingPermissionRecord>> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, request_id, turn_id, tool, rule_id, summary, kind,
                    free_text, multi_select, options_json, questions_json, call_id
             FROM session_pending_permissions
             WHERE session_id = ?1
             ORDER BY created_at ASC, request_id ASC",
        )
        .map_err(|e| LitecodeError::SessionStorage(format!("list pending prepare: {e}")))?;
    let rows = stmt
        .query_map(params![session_id], |row| {
            let kind_s: String = row.get(6)?;
            let options_json: String = row.get(9)?;
            let questions_json: String = row.get(10)?;
            let options: Vec<AskOption> = serde_json::from_str(&options_json).unwrap_or_default();
            let questions: Vec<AskQuestion> =
                serde_json::from_str(&questions_json).unwrap_or_default();
            Ok(PendingPermissionRecord {
                session_id: row.get(0)?,
                request_id: row.get(1)?,
                turn_id: row.get(2)?,
                tool: row.get(3)?,
                rule_id: row.get(4)?,
                summary: row.get(5)?,
                kind: kind_from_str(&kind_s),
                free_text: row.get::<_, i64>(7)? != 0,
                multi_select: row.get::<_, i64>(8)? != 0,
                options,
                questions,
                call_id: row.get(11)?,
            })
        })
        .map_err(|e| LitecodeError::SessionStorage(format!("list pending query: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| LitecodeError::SessionStorage(format!("list pending row: {e}")))?);
    }
    Ok(out)
}

pub fn remove_pending(conn: &Connection, session_id: &str, request_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM session_pending_permissions WHERE session_id = ?1 AND request_id = ?2",
        params![session_id, request_id],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("remove pending: {e}")))?;
    Ok(())
}

pub fn clear_pending(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM session_pending_permissions WHERE session_id = ?1",
        params![session_id],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("clear pending: {e}")))?;
    Ok(())
}

/// True when this call_id (or tool name fallback) should skip interrupt seal.
pub fn should_skip_seal_for_call(
    conn: &Connection,
    session_id: &str,
    call_id: &str,
    tool_name: &str,
) -> Result<bool> {
    if PendingPermissionRecord::is_permission_class_tool_name(tool_name) {
        return Ok(true);
    }
    let hit: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM session_pending_permissions
             WHERE session_id = ?1 AND call_id = ?2
             LIMIT 1",
            params![session_id, call_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| LitecodeError::SessionStorage(format!("skip-seal lookup: {e}")))?;
    if hit.is_some() {
        return Ok(true);
    }
    // Also match by tool name against any pending row for this session (no call_id yet).
    let by_tool: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM session_pending_permissions
             WHERE session_id = ?1 AND tool = ?2
             LIMIT 1",
            params![session_id, tool_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| LitecodeError::SessionStorage(format!("skip-seal tool lookup: {e}")))?;
    Ok(by_tool.is_some())
}

/// R4: old DB with unanswered ask_user tail but no sidecar row.
/// Synthesize a durable pending so snapshot can re-hang the card (stable request_id).
pub fn ensure_name_fallback_pending(
    conn: &Connection,
    session_id: &str,
    turn_id: &str,
    call_id: &str,
    tool_name: &str,
) -> Result<()> {
    // Only synthesize for name-class tools; sidecar-backed skips already have a row.
    if !PendingPermissionRecord::is_permission_class_tool_name(tool_name) {
        return Ok(());
    }
    let recovered_id = format!("recovered:{call_id}");
    let hit: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM session_pending_permissions
             WHERE session_id = ?1 AND (call_id = ?2 OR request_id = ?3)
             LIMIT 1",
            params![session_id, call_id, recovered_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| LitecodeError::SessionStorage(format!("fallback lookup: {e}")))?;
    if hit.is_some() {
        return Ok(());
    }
    upsert_pending(
        conn,
        &PendingPermissionRecord {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            request_id: recovered_id,
            tool: tool_name.to_string(),
            rule_id: tool_name.to_string(),
            summary: format!("Recovered pending {tool_name}"),
            kind: AskKind::AskUser,
            free_text: false,
            options: Vec::new(),
            multi_select: false,
            questions: Vec::new(),
            call_id: Some(call_id.to_string()),
        },
    )
}

/// Fill `call_id` on the oldest pending row for this session+tool that still
/// lacks one (WP-186: link sidecar during seal_orphaned / late discovery).
pub fn backfill_call_id(
    conn: &Connection,
    session_id: &str,
    tool_name: &str,
    call_id: &str,
) -> Result<()> {
    if call_id.is_empty() {
        return Ok(());
    }
    conn.execute(
        "UPDATE session_pending_permissions
         SET call_id = ?1
         WHERE rowid = (
             SELECT rowid FROM session_pending_permissions
             WHERE session_id = ?2
               AND tool = ?3
               AND (call_id IS NULL OR call_id = '')
             ORDER BY created_at ASC, request_id ASC
             LIMIT 1
         )",
        params![call_id, session_id, tool_name],
    )
    .map_err(|e| LitecodeError::SessionStorage(format!("backfill pending call_id: {e}")))?;
    Ok(())
}
