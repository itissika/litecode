//! Cold-start session list: `list_sessions` skips rows `is_session_empty` says are empty.
//!
//! Reinstall / reopen is a writer shutdown + new `SessionData` (WAL recover, blob GC
//! on open). Each case leaves durable SQLite rows, then lists through a fresh manager.
//! Assertions are the user contract: a session with conversation rows must stay listed.
//! A red test is a reproduced hide.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use common::assistant_text_item;
use litecode::config::TurnGuard;
use litecode::session::data::command::{MutationId, SessionMutation};
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

    fn root(&self) -> &Path {
        self.dir.path()
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
    for (id, ..) in rows {
        if !mgr.is_session_empty(&id).await {
            ids.push(id);
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

fn spilled_assistant() -> litecode::types::Item {
    assistant_text_item(&"x".repeat(40_000), "asst_spill")
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
async fn older_session_stays_listed_when_newer_is_poked() {
    let env = Env::new();
    let (old, new) = {
        let data = env.open();
        let old = persist(&data, &[user_text("older conversation")]);
        let new = persist(&data, &[user_text("newest conversation")]);
        data.shutdown();
        (old, new)
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET seq = seq + 10 WHERE session_id = ?1",
            rusqlite::params![new],
        )
        .unwrap();
    });
    let data = env.open();
    let ids = listed(Arc::clone(&data)).await;
    assert!(
        ids.contains(&old),
        "older session must remain listed ({})",
        diagnose(&data, &old)
    );
    assert!(
        ids.contains(&new),
        "newest session with durable rows must remain listed after seq poke ({})",
        diagnose(&data, &new)
    );
}

// ── working_set error is treated as empty ─────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn seq_hole_still_listed_when_transcript_rows_exist() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("u0"), user_text("u1")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET seq = 3 WHERE session_id = ?1 AND seq = 1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "seq hole").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_kind_on_one_row_still_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("keep me"), user_text("broken kind")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET kind = 'not-a-kind', event_type = 'item/user'
             WHERE session_id = ?1 AND seq = 1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "unknown kind").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn surface_op_not_json_still_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("visible preview")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET surface_op = 'append' WHERE session_id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "surface_op=append (not json)").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn seq_starting_at_one_still_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("shifted seq")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET seq = seq + 1 WHERE session_id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "seq starts at 1").await;
}

// ── fold succeeds but working_set is empty ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn empty_surface_op_still_listed_when_item_rows_exist() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("no surface_op")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET surface_op = '' WHERE session_id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "empty surface_op").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn last_message_with_only_control_plane_rows_still_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE sessions SET last_message = 'preview from a real turn' WHERE id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcript_items (
                 session_id, seq, turn_id, turn_seq, item_type, kind, body, body_ref,
                 token_estimate, created_at, event_type, surface_op, source_seqs, cites, state
             ) VALUES (?1, 0, '', 0, 'turn/start', 'turn/start', '{}', NULL,
                       0, 1, 'turn/start', '', NULL, NULL, 'final')",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "control-plane + last_message").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_detail_kind_still_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("legacy kind conversation")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE transcript_items SET kind = 'detail', event_type = 'detail' WHERE session_id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "legacy detail kind").await;
}

// ── blobs / GC on writer open (reinstall) ─────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn spilled_assistant_survives_clean_reopen() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("ask"), spilled_assistant()]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        let refs: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_blob_refs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(refs > 0, "spilled assistant must register blob refs");
    });
    assert_listed_after_reopen(&env, &sid, "spilled assistant with refs").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_blob_files_still_listed_when_user_row_is_inline() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("inline user"), spilled_assistant()]);
        data.shutdown();
        sid
    };
    let blobs = env.root().join("blobs");
    if blobs.exists() {
        fs::remove_dir_all(&blobs).expect("wipe blobs");
    }
    assert_listed_after_reopen(&env, &sid, "deleted blob files").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_blob_refs_then_reopen_gc_still_listed() {
    // Legacy / half-migrated DB: body_ref rows exist, session_blob_refs is empty.
    // Writer open runs cleanup_orphan_blob_files → deletes every blob.
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("inline user"), spilled_assistant()]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute("DELETE FROM session_blob_refs", []).unwrap();
    });
    assert_listed_after_reopen(&env, &sid, "blob GC after empty refs").await;
}

// ── in-progress / crash mid-turn ──────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn in_progress_assistant_with_user_stays_listed() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("please continue")]);
        let live = assistant_text_item("hel", "asst_live");
        let live = match live {
            litecode::types::Item::Message(
                litecode::authority::responses::MessageItem::Output(mut m),
            ) => {
                m.status = litecode::authority::responses::OutputStatus::InProgress;
                litecode::types::Item::Message(litecode::authority::responses::MessageItem::Output(
                    m,
                ))
            }
            other => panic!("expected assistant message, got {other:?}"),
        };
        data.mutate_blocking(SessionMutation::PersistItem {
            session_id: sid.clone(),
            expected_revision: data.revision_blocking(&sid).unwrap(),
            operation_id: MutationId::new(),
            item: live,
        })
        .expect("persist in_progress");
        data.shutdown();
        sid
    };
    assert_listed_after_reopen(&env, &sid, "in_progress assistant").await;
}

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

// ── SQL parent filter (not empty-filter; exclusion) ───────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn child_session_is_omitted_from_root_list() {
    let env = Env::new();
    let sid = {
        let data = env.open();
        let sid = persist(&data, &[user_text("child work")]);
        data.shutdown();
        sid
    };
    env.poke(|conn| {
        conn.execute(
            "UPDATE sessions SET parent_session_id = 'parent-1' WHERE id = ?1",
            rusqlite::params![sid],
        )
        .unwrap();
    });
    let data = env.open();
    let sql_ids: Vec<String> = data
        .list_sessions_blocking()
        .unwrap()
        .into_iter()
        .map(|(id, ..)| id)
        .collect();
    assert!(
        !sql_ids.contains(&sid),
        "child sessions are filtered by parent_session_id IS NULL, not is_session_empty"
    );
}
