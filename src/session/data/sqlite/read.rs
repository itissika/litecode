//! Typed read queries. All sessions.db SELECTs for SessionData live here
//! or in `session.rs` in this directory.

use rusqlite::Connection;

use crate::session::event::SessionEvent;
use crate::session::model::SessionMeta;
use crate::session::surface::fold_surface;
use crate::session::transcript_file::SearchableRow;
use crate::session::working::WorkingRow;
use crate::types::{LitecodeError, Result};

use super::super::command::{DirtySession, ReadValue, SessionListRow, SessionRead};
use super::session;

pub fn execute(
    conn: &Connection,
    query: SessionRead,
    data_root: &std::path::Path,
) -> Result<ReadValue> {
    match query {
        SessionRead::Meta { session_id } => Ok(ReadValue::Meta(load_meta(conn, &session_id)?)),
        SessionRead::Memory { session_id } => Ok(ReadValue::Memory(
            session::Session::load_memory_on(conn, &session_id)?,
        )),
        SessionRead::PendingPermissions { session_id } => Ok(ReadValue::PendingPermissions(
            super::pending::list_pending(conn, &session_id)?,
        )),
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
        SessionRead::SeqCursor { session_id } => {
            let (last_seq, next_seq) = session::load_wire_seq_cursor_on(conn, &session_id)?;
            Ok(ReadValue::SeqCursor { last_seq, next_seq })
        }
        SessionRead::TruncateEpoch { session_id } => Ok(ReadValue::TruncateEpoch(
            session::Session::load_truncate_epoch_on(conn, &session_id)?,
        )),
        SessionRead::ContextMeter { session_id } => Ok(ReadValue::Meter(
            session::load_context_meter_on(conn, &session_id)?,
        )),
        SessionRead::ListSessions => Ok(ReadValue::List(list_sessions(conn)?)),
        SessionRead::ListFolders => Ok(ReadValue::Folders(super::folders::list_folders(conn)?)),
        SessionRead::ListSessionIds => Ok(ReadValue::Ids(list_session_ids(conn)?)),
        SessionRead::ListSessionsForGc => Ok(ReadValue::GcList(list_sessions_for_gc(conn)?)),
        SessionRead::ListSessionActivity { since_ms } => Ok(ReadValue::SessionActivity(
            list_session_activity(conn, since_ms)?,
        )),
        SessionRead::ListChildIds { parent_session_id } => {
            Ok(ReadValue::Ids(list_child_ids(conn, &parent_session_id)?))
        }
        SessionRead::ListOrphanChildSessions => Ok(ReadValue::Ids(list_orphan_child_ids(conn)?)),
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
        SessionRead::DeletedChildBindings { parent_session_id } => Ok(ReadValue::ChildBindings(
            deleted_child_bindings(conn, &parent_session_id)?,
        )),
        SessionRead::SubagentDepth { session_id } => {
            Ok(ReadValue::Depth(subagent_depth(conn, &session_id)?))
        }
        SessionRead::ResolveRef { refer } => Ok(ReadValue::OptionalId(resolve_ref(conn, &refer)?)),
        SessionRead::SurfaceSeqs { session_id } => {
            Ok(ReadValue::Seqs(surface_seqs(conn, &session_id, data_root)?))
        }
        SessionRead::UserDetailCount { session_id } => {
            Ok(ReadValue::Count(user_detail_count(conn, &session_id)?))
        }
        SessionRead::UserAnchorStem { session_id, seq } => {
            Ok(ReadValue::Count(user_anchor_stem(conn, &session_id, seq)?))
        }
        SessionRead::UserAnchors {
            session_id,
            anchor_seq,
            before,
            after,
        } => Ok(ReadValue::UserAnchors(session::user_anchor_window_on(
            conn,
            &session_id,
            anchor_seq,
            before,
            after,
        )?)),
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
        SessionRead::SearchableKeys { session_id } => Ok(ReadValue::SearchableKeys(
            searchable_keys(conn, session_id.as_deref())?,
        )),
        SessionRead::SearchableRowsFor { keys } => {
            Ok(ReadValue::Searchable(searchable_rows_for(conn, &keys)?))
        }
        SessionRead::DirtySessionsSince { last_change_id } => Ok(ReadValue::DirtySessions(
            dirty_sessions_since(conn, last_change_id)?,
        )),
        SessionRead::SpineReminderView { session_id } => {
            let events = load_events(conn, &session_id, data_root)?;
            Ok(ReadValue::SpineReminders(
                crate::session::surface::spine_reminder_view(&events)?,
            ))
        }
        SessionRead::RequestOrigins { session_id } => Ok(ReadValue::RequestOrigins(
            request_origins(conn, &session_id)?,
        )),
        SessionRead::ProviderUsageRollup { session_id } => Ok(ReadValue::ProviderUsageRollup(
            provider_usage_rollup(conn, &session_id)?,
        )),
        SessionRead::ProviderUsageSeries { session_id, limit } => Ok(
            ReadValue::ProviderUsageSeries(provider_usage_series(conn, &session_id, limit)?),
        ),
        SessionRead::WorkspaceUsage => Ok(ReadValue::WorkspaceUsage(workspace_usage(conn)?)),
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

/// `request/header` rows for one session, oldest first.
///
/// `body` is the request's origin record. A row whose body is absent or not JSON
/// is skipped rather than failing the read: this is a compatibility input, and an
/// unreadable header must degrade to "origin unknown", never to a broken turn.
fn provider_usage_rollup(
    conn: &Connection,
    session_id: &str,
) -> Result<session::ProviderUsageRollup> {
    let mut stmt = conn.prepare(
        "SELECT seq, provider_usage FROM transcript_items
         WHERE session_id = ?1
           AND event_type = 'request/header'
           AND provider_usage IS NOT NULL
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut parsed = Vec::new();
    for row in rows {
        let (seq, raw) = row?;
        let Ok(usage) = serde_json::from_str::<session::ProviderUsage>(&raw) else {
            continue;
        };
        parsed.push((seq, usage));
    }
    Ok(session::ProviderUsageRollup::from_rows(parsed))
}

/// One session's provider bills in transcript order, for the usage chart.
///
/// `limit` bounds only the returned sample window; `totals` always covers every
/// bill — turn requests and summarizer requests alike — so a truncated window
/// can never be read as the whole session, and no paid call is invisible.
pub(super) fn provider_usage_series(
    conn: &Connection,
    session_id: &str,
    limit: i64,
) -> Result<session::ProviderUsageSeries> {
    let limit = limit.clamp(1, 400) as usize;
    // Bills live on two row kinds: the `request/header` of a turn request, and
    // the `compacted` checkpoint a summarizer request produced. Both are real
    // provider spend, so both belong in the series and in the totals.
    let mut stmt = conn.prepare(
        "SELECT seq, turn_id, created_at, body, event_type, provider_usage
         FROM transcript_items
         WHERE session_id = ?1
           AND event_type IN ('request/header', 'compacted')
           AND provider_usage IS NOT NULL
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    let mut bills = Vec::new();
    let mut totals = session::ProviderUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        cache_hit_tokens: 0,
        cache_miss_tokens: 0,
    };
    for row in rows {
        let (seq, turn_id, at_ms, body, event_type, raw) = row?;
        let Ok(usage) = serde_json::from_str::<session::ProviderUsage>(&raw) else {
            continue;
        };
        totals.prompt_tokens = totals.prompt_tokens.saturating_add(usage.prompt_tokens);
        totals.completion_tokens = totals
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        totals.cache_hit_tokens = totals
            .cache_hit_tokens
            .saturating_add(usage.cache_hit_tokens);
        totals.cache_miss_tokens = totals
            .cache_miss_tokens
            .saturating_add(usage.cache_miss_tokens);
        let kind = if event_type == "compacted" {
            session::ProviderUsageKind::Compaction
        } else {
            session::ProviderUsageKind::Request
        };
        bills.push((seq, turn_id.unwrap_or_default(), at_ms, body, kind, usage));
    }

    let requests = bills.len() as u64;
    let truncated = bills.len() > limit;
    if truncated {
        bills.drain(0..bills.len() - limit);
    }
    let samples = bills
        .into_iter()
        .map(|(seq, turn_id, at_ms, body, kind, usage)| session::ProviderUsageSample {
            seq,
            turn_id,
            at_ms,
            prompt: usage.prompt_tokens,
            completion: usage.completion_tokens,
            hit: usage.cache_hit_tokens,
            miss: usage.cache_miss_tokens,
            kind,
            // A checkpoint body is a summary, not a request header: no model.
            model: match kind {
                session::ProviderUsageKind::Request => body.as_deref().and_then(request_model),
                session::ProviderUsageKind::Compaction => None,
            },
        })
        .collect();

    let mut stmt = conn.prepare(
        "SELECT seq, created_at FROM transcript_items
         WHERE session_id = ?1 AND event_type = 'compacted'
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok(session::ProviderUsageMark {
            seq: row.get(0)?,
            at_ms: row.get(1)?,
        })
    })?;
    let mut compactions = Vec::new();
    for row in rows {
        compactions.push(row?);
    }

    Ok(session::ProviderUsageSeries {
        samples,
        compactions,
        truncated,
        totals,
        requests,
    })
}


/// Whole-workspace spend, from authoritative bills only.
///
/// Days are server-local calendar days so a bucket matches what a person calls
/// "today". The `json_valid` filter is part of the truth claim: a bill that
/// cannot be parsed is not a number we can add, and treating it as zero would
/// assert the request cost nothing.
///
/// Totals are folded from the same filtered rows as the buckets, so the header
/// numbers can never disagree with the chart under them.
pub(super) fn workspace_usage(conn: &Connection) -> Result<session::WorkspaceUsage> {
    const BILL: &str = "provider_usage IS NOT NULL AND json_valid(provider_usage)";

    let mut stmt = conn.prepare(&format!(
        "SELECT strftime('%Y-%m-%d', created_at / 1000, 'unixepoch', 'localtime') AS day,
                count(*),
                COALESCE(SUM(json_extract(provider_usage, '$.prompt_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.completion_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.cache_hit_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.cache_miss_tokens')), 0)
         FROM transcript_items
         WHERE {BILL}
         GROUP BY day
         ORDER BY day ASC"
    ))?;
    let days = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut totals = session::ProviderUsage::default();
    let mut requests = 0u64;
    let mut buckets = Vec::with_capacity(days.len());
    for (day, count, prompt, completion, hit, miss) in days {
        let count = count.max(0) as u64;
        let prompt = prompt.max(0) as u64;
        let completion = completion.max(0) as u64;
        let hit = hit.max(0) as u64;
        let miss = miss.max(0) as u64;
        requests = requests.saturating_add(count);
        totals.prompt_tokens = totals.prompt_tokens.saturating_add(prompt);
        totals.completion_tokens = totals.completion_tokens.saturating_add(completion);
        totals.cache_hit_tokens = totals.cache_hit_tokens.saturating_add(hit);
        totals.cache_miss_tokens = totals.cache_miss_tokens.saturating_add(miss);
        buckets.push(session::WorkspaceUsageDay {
            day: day.unwrap_or_default(),
            requests: count,
            prompt,
            completion,
            hit,
            miss,
        });
    }

    // Today by hour: the `日` view is a within-day question, and a single day
    // column cannot answer it. Same filter, same arithmetic as the days above.
    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let today_start_ms = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|naive| {
            use chrono::TimeZone;
            chrono::Local
                .from_local_datetime(&naive)
                .earliest()
        })
        .map(|at| at.timestamp_millis())
        .unwrap_or(0);
    let mut hour_stmt = conn.prepare(&format!(
        "SELECT CAST(strftime('%H', created_at / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
                count(*),
                COALESCE(SUM(json_extract(provider_usage, '$.prompt_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.completion_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.cache_hit_tokens')), 0),
                COALESCE(SUM(json_extract(provider_usage, '$.cache_miss_tokens')), 0)
         FROM transcript_items
         WHERE {BILL} AND created_at >= ?1
         GROUP BY hour
         ORDER BY hour ASC",
        BILL = BILL
    ))?;
    let hours = hour_stmt
        .query_map(rusqlite::params![today_start_ms], |row| {
            Ok(session::WorkspaceUsageHour {
                hour: row.get::<_, i64>(0)?.clamp(0, 23) as u32,
                requests: row.get::<_, i64>(1)?.max(0) as u64,
                prompt: row.get::<_, i64>(2)?.max(0) as u64,
                completion: row.get::<_, i64>(3)?.max(0) as u64,
                hit: row.get::<_, i64>(4)?.max(0) as u64,
                miss: row.get::<_, i64>(5)?.max(0) as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // One integer for the header. The overview does not rank sessions, and a
    // per-session breakdown is a different question from workspace spend.
    let session_count = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT session_id) FROM transcript_items WHERE {BILL}"
        ),
        [],
        |row| row.get::<_, i64>(0),
    )?.max(0) as u64;

    Ok(session::WorkspaceUsage {
        days: buckets,
        hours,
        today,
        totals,
        requests,
        session_count,
    })
}

/// Model id recorded on a request header body, when present.
fn request_model(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("model")
        .and_then(|value| value.as_str())
        .map(|model| model.to_string())
}

fn request_origins(conn: &Connection, session_id: &str) -> Result<Vec<(i64, serde_json::Value)>> {
    let mut stmt = conn.prepare(
        "SELECT seq, body FROM transcript_items
         WHERE session_id = ?1 AND event_type = 'request/header'
         ORDER BY seq ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut origins = Vec::new();
    for row in rows {
        let (seq, body) = row?;
        let value = body
            .as_deref()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .unwrap_or(serde_json::Value::Null);
        origins.push((seq, value));
    }
    Ok(origins)
}

fn load_transcript(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<crate::types::Transcript> {
    Ok(crate::session::working::project_items(&load_working_set(
        conn, session_id, data_root,
    )?))
}

/// Cold load of the model-visible working set.
///
/// Starts at the newest checkpoint when the session has one, so the read costs
/// the kept window instead of the whole history. The anchored fold is only used
/// when it *proves* it rebuilt the same surface; any doubt falls back to the full
/// log below. The fallback is a correctness net, never a correctness shortcut:
/// what it prevents is exactly a silently different working set.
fn load_working_set(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<Vec<WorkingRow>> {
    if let Some(rows) = load_working_set_anchored(conn, session_id, data_root)? {
        return Ok(rows);
    }
    let events = load_events(conn, session_id, data_root)?;
    let mut view = crate::session::view::WorkingView::from_events(&events)?;
    view.set_epoch(session::Session::load_truncate_epoch_on(conn, session_id)?);
    view.rows()
}

/// [`load_working_set`] starting at the checkpoint instead of seq 0.
///
/// `None` means "no checkpoint, or the anchored fold refused" — the caller then
/// takes the full log. A refusal is logged: it is a store shape this code does not
/// understand, not a normal path.
fn load_working_set_anchored(
    conn: &Connection,
    session_id: &str,
    data_root: &std::path::Path,
) -> Result<Option<Vec<WorkingRow>>> {
    let Some(anchor) = session::load_compact_anchor_on(conn, session_id)? else {
        return Ok(None);
    };
    let anchored = (|| -> Result<Vec<WorkingRow>> {
        let start = i64::try_from(anchor.start).map_err(|_| {
            LitecodeError::InvalidSessionEvent(format!(
                "fold anchor {} exceeds the seq range",
                anchor.start
            ))
        })?;
        let events = load_events_range(conn, session_id, start, i64::MAX, data_root)?;
        let mut view =
            crate::session::view::WorkingView::from_events_anchored(&events, Some(anchor))?;
        view.set_epoch(session::Session::load_truncate_epoch_on(conn, session_id)?);
        let rows = view.rows()?;
        // The fold must begin at the checkpoint the pointer names. Anything else
        // means this read saw a different log than the one that wrote the pointer.
        if rows.first().and_then(|row| row.log_seq) != Some(anchor.checkpoint) {
            return Err(LitecodeError::InvalidSessionEvent(format!(
                "anchored fold at {} did not begin with checkpoint {}",
                anchor.start, anchor.checkpoint
            )));
        }
        Ok(rows)
    })();
    match anchored {
        Ok(rows) => Ok(Some(rows)),
        Err(error) => {
            tracing::warn!(
                session_id,
                checkpoint = anchor.checkpoint,
                anchor = anchor.start,
                %error,
                "anchored working-set fold refused; reading the full log instead"
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod anchored_load_tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::{load_working_set, load_working_set_anchored};
    use crate::session::data::sqlite::conn::open_readonly;
    use crate::session::{SessionData, WorkspaceWriteLease};
    use crate::types::user_text;

    struct Fixture {
        dir: TempDir,
        _lease: WorkspaceWriteLease,
        data: Arc<SessionData>,
        sid: String,
    }

    fn fixture(texts: &[&str]) -> Fixture {
        let dir = TempDir::new().expect("tempdir");
        let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");
        let data = SessionData::open(&lease, &dir.path().join("sessions.db")).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        let items: Vec<_> = texts.iter().map(|t| user_text(*t)).collect();
        data.insert_items(&sid, &items).expect("items");
        Fixture {
            dir,
            _lease: lease,
            data,
            sid,
        }
    }

    impl Fixture {
        fn compact(&self, summary: &str, kept_from: Option<i64>) {
            self.data
                .compact_from(&self.sid, summary, kept_from, 10)
                .expect("compact");
        }

        fn append(&self, text: &str) {
            self.data
                .insert_items(&self.sid, &[user_text(text)])
                .expect("append");
        }
    }

    fn read(fx: &Fixture) -> Vec<crate::session::WorkingRow> {
        let conn = open_readonly(&fx.dir.path().join("sessions.db")).expect("ro");
        load_working_set(&conn, &fx.sid, fx.dir.path()).expect("working set")
    }

    /// The whole point of the change: a compacted session loads from the
    /// checkpoint, and that shortcut must produce the full fold's rows exactly.
    #[test]
    fn anchored_load_is_used_and_matches_the_full_fold() {
        let fx = fixture(&["a", "b", "c"]);
        fx.compact("first", Some(1));
        fx.append("d");
        fx.compact("second", Some(2));
        let conn = open_readonly(&fx.dir.path().join("sessions.db")).expect("ro");
        let anchored = load_working_set_anchored(&conn, &fx.sid, fx.dir.path())
            .expect("anchored")
            .expect("a compacted session must anchor, not fall back");
        let full = load_working_set(&conn, &fx.sid, fx.dir.path()).expect("full");
        assert_eq!(anchored, full);
        assert_eq!(full.len(), 3, "checkpoint + the two kept rows");
    }

    /// A row below the anchor is never read. Poisoning one with an unparseable
    /// body proves the read really started at the checkpoint: a full read of the
    /// same log fails outright, so a passing read cannot have touched seq 0.
    #[test]
    fn rows_below_the_anchor_are_never_read() {
        let fx = fixture(&["a", "b", "c"]);
        fx.compact("sum", Some(1));
        rusqlite::Connection::open(fx.dir.path().join("sessions.db"))
            .expect("rw")
            .execute(
                "UPDATE transcript_items SET body = '' WHERE session_id = ?1 AND seq = 0",
                rusqlite::params![fx.sid],
            )
            .expect("poison");
        let conn = open_readonly(&fx.dir.path().join("sessions.db")).expect("ro");
        assert!(
            super::super::session::load_events_on(&conn, &fx.sid, fx.dir.path()).is_err(),
            "the poison must be fatal for a full read, otherwise this proves nothing"
        );
        let rows = read(&fx);
        assert_eq!(
            rows.iter().filter_map(|row| row.log_seq).collect::<Vec<_>>(),
            vec![3, 1, 2],
            "checkpoint first, then the kept rows"
        );
    }

    /// An empty keep leaves only the summary; the anchor is the checkpoint's own
    /// seq, so the read is a single row.
    #[test]
    fn empty_keep_anchors_on_the_checkpoint_itself() {
        let fx = fixture(&["a", "b"]);
        fx.compact("sum", None);
        fx.append("c");
        let rows = read(&fx);
        assert_eq!(
            rows.iter().filter_map(|row| row.log_seq).collect::<Vec<_>>(),
            vec![2, 3],
            "the summary, then the row appended after it"
        );
    }

    /// A pointer that does not name a checkpoint is a broken store, not a reason
    /// to quietly fall back to a summary-free surface.
    #[test]
    fn a_pointed_row_that_is_not_a_checkpoint_is_an_error() {
        let fx = fixture(&["a", "b", "c"]);
        fx.compact("sum", Some(1));
        rusqlite::Connection::open(fx.dir.path().join("sessions.db"))
            .expect("rw")
            .execute(
                "UPDATE sessions SET checkpoint_seq = 1 WHERE id = ?1",
                rusqlite::params![fx.sid],
            )
            .expect("break pointer");
        let conn = open_readonly(&fx.dir.path().join("sessions.db")).expect("ro");
        let error = load_working_set(&conn, &fx.sid, fx.dir.path())
            .expect_err("a pointer at a non-checkpoint row must not read as a summary");
        assert!(error.to_string().contains("checkpoint"), "unexpected: {error}");
    }
}


fn list_sessions(conn: &Connection) -> Result<Vec<SessionListRow>> {
    // Roots are capped per sidebar bucket rather than globally: a session the
    // user explicitly filed into a folder must not disappear because 50 newer
    // ungrouped sessions exist. Children are not capped (the roster needs them).
    let mut roots = query_session_list(
        conn,
        "SELECT id, project, updated_at, last_message, last_assistant, agent_id, model_id,
                parent_session_id, parent_call_id, responsibility, title, folder_id
         FROM (
             SELECT id, project, updated_at, last_message, last_assistant, agent_id, model_id,
                    parent_session_id, parent_call_id, responsibility, title, folder_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY COALESCE(folder_id, '')
                        ORDER BY updated_at DESC, id DESC
                    ) AS rn
             FROM sessions
             WHERE parent_session_id IS NULL
         )
         WHERE rn <= 50
         ORDER BY updated_at DESC, id DESC",
        [],
    )?;
    let mut children = query_session_list(
        conn,
        "SELECT id, project, updated_at, last_message, last_assistant, agent_id, model_id,
                parent_session_id, parent_call_id, responsibility, title, folder_id
         FROM sessions
         WHERE parent_session_id IS NOT NULL
         ORDER BY updated_at DESC",
        [],
    )?;
    roots.append(&mut children);
    Ok(roots)
}

fn query_session_list(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<SessionListRow>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(SessionListRow {
            id: row.get(0)?,
            project: row.get(1)?,
            updated_at: row.get(2)?,
            preview: row.get(3)?,
            assistant_preview: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            agent_id: row.get(5)?,
            model_id: row.get(6)?,
            parent_session_id: nonempty_id(row.get(7)?),
            parent_call_id: nonempty_id(row.get(8)?),
            responsibility: row.get(9)?,
            title: nonempty_id(row.get(10)?),
            folder_id: nonempty_id(row.get(11)?),
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn nonempty_id(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
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

fn list_session_activity(
    conn: &Connection,
    since_ms: i64,
) -> Result<Vec<(String, Option<String>, i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_session_id, updated_at, agent_id, last_message FROM sessions
         WHERE updated_at >= ?1
         ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map(rusqlite::params![since_ms], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn list_orphan_child_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM sessions
         WHERE parent_session_id IS NOT NULL
           AND parent_session_id NOT IN (SELECT id FROM sessions)",
    )?;
    let rows = stmt.query_map([], |row| row.get(0))?;
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

fn deleted_child_bindings(conn: &Connection, parent: &str) -> Result<Vec<(String, String)>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT deleted_children_json FROM sessions WHERE id = ?1",
            rusqlite::params![parent],
            |row| row.get(0),
        )
        .optional()?;
    let map: std::collections::BTreeMap<String, String> = raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
    Ok(map.into_iter().collect())
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

fn user_detail_count(conn: &Connection, session_id: &str) -> Result<i64> {
    conn.query_row(
        session::SQL_USER_DETAIL_COUNT,
        rusqlite::params![session_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn user_anchor_stem(conn: &Connection, session_id: &str, seq: i64) -> Result<i64> {
    session::require_user_anchor(conn, session_id, seq)?;
    seq.checked_add(1)
        .ok_or_else(|| LitecodeError::InvalidRevertAnchor(format!("seq={seq} overflow")))
}

/// Kinds the derived index can ever hold.
///
/// `compacted` is deliberately absent: a compaction summary is model-distilled
/// text, not something the session authored, and it describes a range the
/// original rows already cover. The reader-facing projection still shows it —
/// see [`PROJECTED_KINDS`]; the two views answer different questions.
const INDEXED_KINDS: &str = "('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result')";

/// Kinds the reader-facing transcript projection renders: the indexed kinds plus
/// the compaction summary, which a reader should see even though search skips it.
const PROJECTED_KINDS: &str =
    "('item/user', 'item/assistant', 'item/tool_call', 'item/tool_result', 'compacted')";

/// A row is searchable when its own content can no longer change.
///
/// The condition is deliberately row-local: the row itself is not
/// `in_progress`. Nothing earlier in the session can hold a final row back —
/// call-before-result ordering is a session write invariant, so a final row is
/// never stuck behind an unfinished one. A row that arrives in flight becomes
/// visible the moment it is sealed.
fn searchable_sql(select: &str, kinds: &str, scoped: bool) -> String {
    let order = if scoped {
        " ORDER BY t.seq"
    } else {
        " ORDER BY t.session_id, t.seq"
    };
    let scope = if scoped { " AND t.session_id = ?1" } else { "" };
    format!(
        "{select}
         FROM transcript_items t
         WHERE t.kind IN {kinds}
           AND t.state != 'in_progress'{scope}{order}"
    )
}

/// `(session_id, seq)` of every searchable row. Integers only — this is the
/// cheap reconciliation key, never a row body.
pub fn searchable_keys(conn: &Connection, session_id: Option<&str>) -> Result<Vec<(String, i64)>> {
    let sql = searchable_sql(
        "SELECT t.session_id, t.seq",
        INDEXED_KINDS,
        session_id.is_some(),
    );
    let mut stmt = conn.prepare(&sql)?;
    let map_row = |row: &rusqlite::Row<'_>| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?));
    let keys = if let Some(sid) = session_id {
        stmt.query_map(rusqlite::params![sid], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], map_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    Ok(keys)
}

/// Bodies for a specific set of searchable rows. Used by the incremental refresh
/// so it reads and decodes only what actually changed.
///
/// `state != 'in_progress'` is required here too: a key may be named while its
/// row is still settling, so the body read must never hand back a row that is
/// not yet final.
pub fn searchable_rows_for(
    conn: &Connection,
    keys: &[(String, i64)],
) -> Result<Vec<SearchableRow>> {
    const CHUNK: usize = 200;
    let mut out = Vec::with_capacity(keys.len());
    for chunk in keys.chunks(CHUNK) {
        let placeholders = std::iter::repeat_n("(?, ?)", chunk.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT session_id, seq, kind, item_type, body, body_ref FROM transcript_items
             WHERE kind IN {INDEXED_KINDS}
               AND state != 'in_progress'
               AND (session_id, seq) IN (VALUES {placeholders})"
        );
        let params: Vec<rusqlite::types::Value> = chunk
            .iter()
            .flat_map(|(sid, seq)| {
                [
                    rusqlite::types::Value::Text(sid.clone()),
                    rusqlite::types::Value::Integer(*seq),
                ]
            })
            .collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok(SearchableRow {
                session_id: row.get(0)?,
                seq: row.get(1)?,
                kind: row.get(2)?,
                item_type: row.get(3)?,
                body: row.get(4)?,
                body_ref: row.get(5)?,
            })
        })?;
        out.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    }
    out.sort_by(|a, b| (&a.session_id, a.seq).cmp(&(&b.session_id, b.seq)));
    Ok(out)
}

fn searchable_rows(conn: &Connection, session_id: Option<&str>) -> Result<Vec<SearchableRow>> {
    let sql = searchable_sql(
        "SELECT t.session_id, t.seq, t.kind, t.item_type, t.body, t.body_ref",
        PROJECTED_KINDS,
        session_id.is_some(),
    );
    let mut stmt = conn.prepare(&sql)?;
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

fn dirty_sessions_since(conn: &Connection, last_change_id: i64) -> Result<Vec<DirtySession>> {
    let mut stmt = conn.prepare(
        "SELECT change_id, session_id
         FROM session_dirty
         WHERE change_id > ?1
         ORDER BY change_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![last_change_id], |row| {
        Ok(DirtySession {
            change_id: row.get(0)?,
            session_id: row.get(1)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn latest_change_id(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT last_change_id FROM session_change_clock WHERE singleton = 1",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
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
