//! Cold-start session list: `list_sessions` skips rows `is_session_empty` says are empty.
//!
//! Reinstall / reopen is a writer shutdown + new `SessionData` (WAL recover, blob GC
//! on open). Each case leaves durable SQLite rows, then lists through a fresh manager.
//! Assertions are the user contract: a session with conversation rows must stay listed.
//! A red test is a reproduced hide.

mod common;

use std::sync::Arc;

use litecode::config::TurnGuard;
use litecode::session::manager::SessionManager;
use litecode::session::{SessionData, WorkspaceWriteLease};
use litecode::types::user_text;
use rusqlite::Connection;
use tempfile::TempDir;

struct Env {
    dir: TempDir,
    db: std::path::PathBuf,
    lease: WorkspaceWriteLease,
}

impl Env {
    fn new() -> Self {
        let dir = TempDir::new().expect("tempdir");
        let db = dir.path().join("sessions.db");
        let lease = WorkspaceWriteLease::acquire(dir.path()).expect("lease");
        Self { dir, db, lease }
    }

    fn open(&self) -> Arc<SessionData> {
        SessionData::open(&self.lease, &self.db).expect("open sessions.db")
    }

    fn poke(&self, f: impl FnOnce(&Connection)) {
        let conn = Connection::open(&self.db).expect("poke sqlite");
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .expect("busy_timeout");
        f(&conn);
    }
}

fn persist(data: &SessionData, items: &[litecode::types::Item]) -> String {
    let sid = data
        .create_session("/proj", "default", None)
        .expect("create");
    if !items.is_empty() {
        data.insert_items(&sid, items).expect("insert");
    }
    sid
}

fn diagnose(data: &SessionData, sid: &str) -> String {
    match data.working_set_blocking(sid) {
        Ok(rows) => format!("working_set_len={}", rows.len()),
        Err(error) => format!("working_set_err={error}"),
    }
}

async fn listed(data: Arc<SessionData>) -> Vec<String> {
    let mgr = SessionManager::from_data(Arc::new(TurnGuard::new()), data);
    let rows = mgr.data().list_sessions_blocking().expect("sql list");
    let mut ids = Vec::new();
    for row in rows {
        // Mirrors build_session_list: an empty root is hidden only when it has
        // no title and no folder. Empty children stay listed.
        let hidden = row.parent_session_id.is_none()
            && row.title.is_none()
            && row.folder_id.is_none()
            && mgr.is_session_empty(&row.id).await;
        if !hidden {
            ids.push(row.id);
        }
    }
    ids
}

async fn assert_listed_after_reopen(env: &Env, sid: &str, case: &str) {
    let data = env.open();
    let ids = listed(Arc::clone(&data)).await;
    assert!(
        ids.iter().any(|id| id == sid),
        "{case}: session {sid} hidden after reopen ({})",
        diagnose(&data, sid)
    );
}

async fn assert_hidden_after_reopen(env: &Env, sid: &str, case: &str) {
    let data = env.open();
    let ids = listed(Arc::clone(&data)).await;
    assert!(
        ids.iter().all(|id| id != sid),
        "{case}: session {sid} should stay hidden ({})",
        diagnose(&data, sid)
    );
}

// ── controls ──────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn cold_start_user_message_stays_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("hello from listed session")]);
        data.shutdown();
        sid
    };
    assert_listed_after_reopen(&env, &sid, "normal user row").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_new_session_stays_hidden() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[]);
        data.shutdown();
        sid
    };
    assert_hidden_after_reopen(&env, &sid, "empty create").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_session_with_title_stays_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[]);
        data.set_session_title_blocking(&sid, Some("Named before the first turn"))
            .expect("set title");
        data.shutdown();
        sid
    };
    assert_listed_after_reopen(&env, &sid, "titled empty session").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_session_in_folder_stays_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[]);
        let folder = data
            .create_session_folder_blocking("Planning")
            .expect("create folder");
        data.set_session_folder_blocking(&sid, Some(&folder))
            .expect("move session");
        data.shutdown();
        sid
    };
    assert_listed_after_reopen(&env, &sid, "foldered empty session").await;
}

// ── working_set / fold: durable item rows stay listed (T3 table-driven) ────

#[tokio::test(flavor = "multi_thread")]
async fn malformed_transcript_rows_still_listed_after_reopen() {
    // Six former near-duplicate tests → one table. Extreme samples in-table (≤2):
    // seq hole, unknown kind.
    #[derive(Clone, Copy)]
    enum Poke {
        SeqHole,
        UnknownKind,
        SurfaceOpNotJson,
        SeqStartsAtOne,
        EmptySurfaceOp,
        LegacyDetailKind,
    }

    let cases: &[(&str, &[&str], Poke)] = &[
        ("seq hole", &["u0", "u1"], Poke::SeqHole),
        (
            "unknown kind",
            &["keep me", "broken kind"],
            Poke::UnknownKind,
        ),
        (
            "surface_op=append (not json)",
            &["visible preview"],
            Poke::SurfaceOpNotJson,
        ),
        ("seq starts at 1", &["shifted seq"], Poke::SeqStartsAtOne),
        ("empty surface_op", &["no surface_op"], Poke::EmptySurfaceOp),
        (
            "legacy detail kind",
            &["legacy kind conversation"],
            Poke::LegacyDetailKind,
        ),
    ];

    for (case, texts, poke) in cases {
        let env = Env::new();
        let items: Vec<_> = texts.iter().map(|t| user_text(*t)).collect();
        let sid = {
            let data = env.open();
            let sid = persist(&data, &items);
            data.shutdown();
            sid
        };
        env.poke(|conn| match poke {
            Poke::SeqHole => {
                conn.execute(
                    "UPDATE transcript_items SET seq = 3 WHERE session_id = ?1 AND seq = 1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
            Poke::UnknownKind => {
                conn.execute(
                    "UPDATE transcript_items SET kind = 'not-a-kind', event_type = 'item/user'
             WHERE session_id = ?1 AND seq = 1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
            Poke::SurfaceOpNotJson => {
                conn.execute(
                    "UPDATE transcript_items SET surface_op = 'append' WHERE session_id = ?1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
            Poke::SeqStartsAtOne => {
                conn.execute(
                    "UPDATE transcript_items SET seq = seq + 1 WHERE session_id = ?1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
            Poke::EmptySurfaceOp => {
                conn.execute(
                    "UPDATE transcript_items SET surface_op = '' WHERE session_id = ?1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
            Poke::LegacyDetailKind => {
                conn.execute(
                    "UPDATE transcript_items SET kind = 'detail', event_type = 'detail' WHERE session_id = ?1",
                    rusqlite::params![sid],
                )
                .unwrap();
            }
        });
        assert_listed_after_reopen(&env, &sid, case).await;
    }
}

// ── fold succeeds but working_set is empty ────────────────────────────────

// ── blobs / GC on writer open (reinstall) ─────────────────────────────────

// ── in-progress / crash mid-turn ──────────────────────────────────────────

// ── busy bypass vs cold ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn busy_empty_session_is_listed_until_cold_start() {
    let env = Env::new();
    let data = env.open();
    let sid = persist(&data, &[]);
    let mgr = SessionManager::from_data(Arc::new(TurnGuard::new()), Arc::clone(&data));
    mgr.ensure_entry(&sid).await.expect("hydrate");
    mgr.reserve_turn(&sid, "turn-1".into(), 8, "default", "/proj")
        .expect("busy");
    assert!(
        !mgr.is_session_empty(&sid).await,
        "running empty session must show in the live list"
    );

    data.shutdown();
    assert_hidden_after_reopen(&env, &sid, "busy empty after cold start").await;
}

// ── SQL parent filter (children are listed with parent ids) ───────────────

