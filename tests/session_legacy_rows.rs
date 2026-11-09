//! Old logs keep loading after their producers were deleted.
//!
//! A row type is retired by removing its writer, never its reader: real
//! databases still carry `reminder/plan_changed`, `reminder/mentions`,
//! `model/changed`, `request/context` and `step/*` rows. This fixture writes
//! the rows the retired writers produced and proves the session opens, the
//! reminders stay in model-visible history, and the control rows degrade to
//! ignorable rows instead of failing the load.

mod common;

use std::sync::Arc;

use litecode::config::TurnGuard;
use litecode::reminder::ReminderKind;
use litecode::session::manager::SessionManager;
use litecode::session::surface::derive_messages;
use litecode::session::{EventType, SessionData, WorkspaceWriteLease};
use litecode::types::user_text;
use rusqlite::Connection;
use serde_json::json;
use tempfile::TempDir;

const PLAN_TEXT: &str = "[Plan updated] .litecode/plan/a.md changed since you last read it.";
const MENTIONS_TEXT: &str =
    "Context the user referenced in the message above, in mention order, snapshot at send time.";

fn insert_reminder(conn: &Connection, sid: &str, seq: i64, kind: &str, body: &serde_json::Value) {
    conn.execute(
        "INSERT INTO transcript_items
            (session_id, seq, item_type, kind, body, token_estimate, created_at, event_type, surface_op)
         VALUES (?1, ?2, ?3, ?3, ?4, 0, 0, ?3, '\"append\"')",
        rusqlite::params![sid, seq, kind, body.to_string()],
    )
    .expect("insert reminder row");
}

fn insert_control(conn: &Connection, sid: &str, seq: i64, kind: &str, body: &serde_json::Value) {
    conn.execute(
        "INSERT INTO transcript_items
            (session_id, seq, item_type, kind, body, token_estimate, created_at, event_type, surface_op)
         VALUES (?1, ?2, ?3, ?3, ?4, 0, 0, ?3, '')",
        rusqlite::params![sid, seq, kind, body.to_string()],
    )
    .expect("insert control row");
}

#[test]
fn a_legacy_log_opens_and_keeps_its_reminders() {
    let dir = TempDir::new().expect("tempdir");
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");
    let sid = {
        let data = SessionData::open(&lease, &db).expect("open");
        let sid = data.create_session("/p", "default", None).expect("create");
        data.insert_items(&sid, &[user_text("hello")])
            .expect("user row");
        data.shutdown();
        sid
    };

    let conn = Connection::open(&db).expect("rw");
    insert_reminder(
        &conn,
        &sid,
        1,
        "reminder/plan_changed",
        &json!({
            "kind": "plan_changed",
            "relative_path": ".litecode/plan/a.md",
            "revision": "rev-1",
            "text": PLAN_TEXT,
        }),
    );
    insert_reminder(
        &conn,
        &sid,
        2,
        "reminder/mentions",
        &json!({ "kind": "mentions", "refs": [], "text": MENTIONS_TEXT }),
    );
    insert_control(
        &conn,
        &sid,
        3,
        "model/changed",
        &json!({ "schema": 1, "from": null, "to": "p/b", "at_seq": 1, "report": null }),
    );
    insert_control(
        &conn,
        &sid,
        4,
        "request/context",
        &json!({ "instructions": "x", "tools": [] }),
    );
    insert_control(&conn, &sid, 5, "step/start", &json!({ "step": 1 }));
    drop(conn);

    let data = SessionData::open(&lease, &db).expect("reopen");
    let manager = SessionManager::from_data(Arc::new(TurnGuard::new()), Arc::clone(&data));
    let events = manager
        .data()
        .events_blocking(&sid)
        .expect("a log with retired rows must still load");

    let plan = events
        .iter()
        .find(|event| event.seq == 1)
        .expect("plan row");
    assert!(
        matches!(
            plan.event_type,
            EventType::Reminder(ReminderKind::PlanChanged)
        ),
        "{:?}",
        plan.event_type
    );
    let mentions = events
        .iter()
        .find(|event| event.seq == 2)
        .expect("mentions row");
    assert!(
        matches!(
            mentions.event_type,
            EventType::Reminder(ReminderKind::Mentions)
        ),
        "{:?}",
        mentions.event_type
    );

    for seq in [3, 4, 5] {
        let event = events
            .iter()
            .find(|event| event.seq == seq)
            .expect("control row");
        assert!(
            matches!(event.event_type, EventType::Unknown(_)),
            "retired row {seq} must degrade to unknown: {:?}",
            event.event_type
        );
        assert!(event.ignorable, "retired row {seq} must stay ignorable");
    }

    // Model-visible history: the frozen reminder text still renders, and no
    // control row leaks into it.
    let messages = derive_messages(&events).expect("derive messages");
    let rendered = serde_json::to_string(&messages).expect("json");
    assert!(rendered.contains("hello"), "{rendered}");
    assert!(rendered.contains(PLAN_TEXT), "{rendered}");
    assert!(rendered.contains("mention order"), "{rendered}");
    assert!(!rendered.contains("model/changed"), "{rendered}");
    assert!(!rendered.contains("request/context"), "{rendered}");
}
