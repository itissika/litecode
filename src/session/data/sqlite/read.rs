//! Typed read queries. All sessions.db SELECTs for SessionData live here
//! or in `session.rs` / `fts.rs` in this directory.

use rusqlite::Connection;

use crate::session::event::{SessionEvent, spine_agent_item};
use crate::session::model::SessionMeta;
use crate::session::surface::{fold_surface, project_working_pairs};
use crate::session::transcript_file::SearchableRow;
use crate::session::working::WorkingRow;
use crate::types::{LitecodeError, Result};

use super::super::command::{ReadValue, SessionChange, SessionRead};
use super::fts;
use super::session::{self, TranscriptRow};

pub fn execute(
    conn: &Connection,
    query: SessionRead,
    data_root: &std::path::Path,
) -> Result<ReadValue> {
    match query {
        SessionRead::Meta { session_id } => Ok(ReadValue::Meta(load_meta(conn, &session_id)?)),
        SessionRead::Transcript { session_id } => Ok(ReadValue::Transcript(load_transcript(
            conn,
            &session_id,
            data_root,
        )?)),
        SessionRead::WorkingSet { session_id } => Ok(ReadValue::WorkingSet(load_working_set(
            conn,
            &session_id,
            data_root,
        )?)),
        SessionRead::Events { session_id } => Ok(ReadValue::Events(load_events(
            conn,
            &session_id,
            data_root,
        )?)),
        SessionRead::EventsRange {
            session_id,
            from,
            to,
        } => Ok(ReadValue::Events(load_events_range(
            conn,
            &session_id,
            from,
            to,
            data_root,
        )?)),
        SessionRead::ContextMeter { session_id } => Ok(ReadValue::Meter(
            session::load_context_meter_on(conn, &session_id)?,
        )),
        SessionRead::ListSessions => Ok(ReadValue::List(list_sessions(conn)?)),
        SessionRead::ListSessionIds => Ok(ReadValue::Ids(list_session_ids(conn)?)),
        SessionRead::ListSessionsForGc => Ok(ReadValue::GcList(list_sessions_for_gc(conn)?)),
        SessionRead::ListChildIds { parent_session_id } => {
            Ok(ReadValue::Ids(list_child_ids(conn, &parent_session_id)?))
        }
        SessionRead::ChildForCall {
            parent_session_id,
            parent_call_id,
        } => Ok(ReadValue::OptionalId(child_for_call(
            conn,
            &parent_session_id,
            &parent_call_id,
        )?)),
        SessionRead::ChildBindings { parent_session_id } => Ok(ReadValue::ChildBindings(
            child_bindings(conn, &parent_session_id)?,
        )),
        SessionRead::SubagentDepth { session_id } => {
            Ok(ReadValue::Depth(subagent_depth(conn, &session_id)?))
        }
        SessionRead::ResolveRef { refer } => Ok(ReadValue::OptionalId(resolve_ref(conn, &refer)?)),
        SessionRead::SurfaceSeqs { session_id } => {
            Ok(ReadValue::Seqs(surface_seqs(conn, &session_id, data_root)?))
        }
        SessionRead::UserDetailBefore {
            session_id,
            from_seq,
        } => Ok(ReadValue::Count(user_detail_before(
            conn,
            &session_id,
            from_seq,
        )?)),
        SessionRead::SnapshotStem { session_id, k } => {
            Ok(ReadValue::Count(snapshot_stem(conn, &session_id, k)?))
        }
        SessionRead::CheckpointSeq { session_id } => {
            let v: i64 = conn.query_row(
                "SELECT checkpoint_seq FROM sessions WHERE id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )?;
            Ok(ReadValue::Count(v))
        }
        SessionRead::Revision { session_id } => {
            Ok(ReadValue::Revision(load_revision(conn, &session_id)?))
        }
        SessionRead::SearchableRows { session_id } => Ok(ReadValue::Searchable(searchable_rows(
            conn,
            session_id.as_deref(),
        )?)),
        SessionRead::FtsSearch {
            query,
            session_id,
            limit,
        } => {
            let escaped = fts::escape_match_query(&query);
            if escaped.is_empty() {
                return Ok(ReadValue::FtsHits(Vec::new()));
            }
            Ok(ReadValue::FtsHits(fts::search(
                conn,
                &escaped,
                session_id.as_deref(),
                limit,
            )?))
        }
        SessionRead::ChangeLogSince { last_change_id } => {
            Ok(ReadValue::Changes(change_log_since(conn, last_change_id)?))
        }
        SessionRead::LatestChangeId => Ok(ReadValue::Count(latest_change_id(conn)?)),
    }
}

pub fn load_revision(conn: &Connection, session_id: &str) -> Result<u64> {
    let v: i64 = conn
        .query_row(
            "SELECT revision FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .map_err(|_| LitecodeError::SessionNotFound(session_id.to_string()))?;
    Ok(v.max(0) as u64)
}

fn load_meta(conn: &Connection, session_id: &str) -> Result<SessionMeta> {
    session::load_meta_on(conn, session_id)
}

fn load_events(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<Vec<SessionEvent>> {
    session::load_events_on(conn, session_id, data_root)
}

fn load_events_range(
    conn: &Connection,
    session_id: &str,
    from: i64,
    to: i64,
    data_root: &std::path::Path,
) -> Result<Vec<SessionEvent>> {
    session::load_events_range_on(conn, session_id, from, to, data_root)
}

fn load_transcript(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<crate::types::Transcript> {
    Ok(load_working_set(conn, session_id, data_root)?
        .into_iter()
        .map(|row| row.item)
        .collect())
}

fn load_working_set(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<Vec<WorkingRow>> {
    let events = load_events(conn, session_id, data_root)?;
    let surface = fold_surface(&events)?;
    let pairs = project_working_pairs(&surface, |seq| {
        events
            .iter()
            .find(|e| e.seq == seq)
            .ok_or_else(|| LitecodeError::InvalidSessionEvent(format!("surface seq {seq} missing")))
            .and_then(spine_agent_item)
    })?;
    Ok(pairs
        .into_iter()
        .map(|(seq, item)| WorkingRow::persisted(seq, item))
        .collect())
}

fn list_sessions(
    conn: &Connection,
) -> Result<Vec<(String, String, i64, String, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, updated_at, last_message, agent_id, model_id
         FROM sessions
         WHERE parent_session_id IS NULL
         ORDER BY updated_at DESC LIMIT 50",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn list_session_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM sessions")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn list_sessions_for_gc(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare("SELECT id, updated_at FROM sessions")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn list_child_ids(conn: &Connection, parent: &str) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT id FROM sessions WHERE parent_session_id = ?1 ORDER BY created_at ASC")?;
    let rows = stmt.query_map(rusqlite::params![parent], |row| row.get(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn child_for_call(conn: &Connection, parent: &str, call_id: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions
         WHERE parent_session_id = ?1 AND parent_call_id = ?2
         ORDER BY created_at DESC LIMIT 1",
    )?;
    match stmt.query_row(rusqlite::params![parent, call_id], |row| row.get(0)) {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn child_bindings(conn: &Connection, parent: &str) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(parent_call_id, ''), id FROM sessions WHERE parent_session_id = ?1",
    )?;
    let rows = stmt.query_map(rusqlite::params![parent], |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn subagent_depth(conn: &Connection, session_id: &str) -> Result<u32> {
    conn.query_row(
        "SELECT subagent_depth FROM sessions WHERE id = ?1",
        rusqlite::params![session_id],
        |row| row.get(0),
    )
    .map_err(|_| LitecodeError::SessionNotFound(session_id.to_string()))
}

fn resolve_ref(conn: &Connection, refer: &str) -> Result<Option<String>> {
    let refer = refer.trim();
    if refer.is_empty() {
        return Ok(None);
    }
    let exact: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions WHERE id = ?1",
            rusqlite::params![refer],
            |row| row.get(0),
        )
        .optional()?;
    if exact.is_some() {
        return Ok(exact);
    }
    let mut stmt = conn.prepare("SELECT id FROM sessions WHERE id LIKE ?1 OR id LIKE ?2")?;
    let pattern_prefix = format!("{refer}%");
    let pattern_suffix = format!("%{refer}");
    let rows = stmt.query_map(rusqlite::params![pattern_prefix, pattern_suffix], |row| {
        row.get::<_, String>(0)
    })?;
    let mut matches = Vec::new();
    for row in rows {
        matches.push(row?);
    }
    match matches.len() {
        1 => Ok(Some(matches.remove(0))),
        0 => Ok(None),
        _ => Err(LitecodeError::Config(format!(
            "ambiguous session ref '{refer}'"
        ))),
    }
}

fn surface_seqs(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<Vec<i64>> {
    let events = load_events(conn, session_id, data_root)?;
    let surface = fold_surface(&events)?;
    Ok(surface.nodes.into_iter().map(|s| s as i64).collect())
}

fn user_detail_before(conn: &Connection, session_id: &str, from_seq: i64) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM transcript_items t
         WHERE t.session_id = ?1 AND t.kind = 'item/user' AND t.seq < ?2",
        rusqlite::params![session_id, from_seq],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn snapshot_stem(conn: &Connection, session_id: &str, k: i64) -> Result<i64> {
    conn.query_row(
        session::SQL_ANCHOR_SEQ,
        rusqlite::params![session_id, k],
        |row| row.get(0),
    )
    .map_err(|_| LitecodeError::InvalidRevertAnchor(format!("k={k}")))
}

fn searchable_rows(conn: &Connection, session_id: Option<&str>) -> Result<Vec<SearchableRow>> {
    let sql = if session_id.is_some() {
        "SELECT session_id, seq, kind, item_type, body, body_ref FROM transcript_items
         WHERE kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')
           AND session_id = ?1
         ORDER BY seq"
    } else {
        "SELECT session_id, seq, kind, item_type, body, body_ref FROM transcript_items
         WHERE kind IN ('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')
         ORDER BY session_id, seq"
    };
    let mut stmt = conn.prepare(sql)?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(SearchableRow {
            session_id: row.get(0)?,
            seq: row.get(1)?,
            kind: row.get(2)?,
            item_type: row.get(3)?,
            body: row.get(4)?,
            body_ref: row.get(5)?,
        })
    };
    let rows = if let Some(sid) = session_id {
        stmt.query_map(rusqlite::params![sid], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

fn change_log_since(conn: &Connection, last_change_id: i64) -> Result<Vec<SessionChange>> {
    let mut stmt = conn.prepare(
        "SELECT change_id, session_id, revision, kind, from_seq, to_seq
         FROM session_change_log
         WHERE change_id > ?1
         ORDER BY change_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![last_change_id], |row| {
        Ok(SessionChange {
            change_id: row.get(0)?,
            session_id: row.get(1)?,
            revision: row.get::<_, i64>(2)? as u64,
            kind: row.get(3)?,
            from_seq: row.get(4)?,
            to_seq: row.get(5)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn latest_change_id(conn: &Connection) -> Result<i64> {
    let id: Option<i64> =
        conn.query_row("SELECT MAX(change_id) FROM session_change_log", [], |row| {
            row.get(0)
        })?;
    Ok(id.unwrap_or(0))
}

trait OptionalExt<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalExt<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
