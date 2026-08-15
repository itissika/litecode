//! Session business path: workspace DB → Item insert/reload → revert → schema truth.

mod common;

use common::assistant_text_item;
use common::test_workspace;
use litecode::session::store::Session;
use litecode::types::{item_text_preview, user_text};
use rusqlite::Connection;

#[test]
fn session_transcript_roundtrip_and_revert_on_workspace_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = test_workspace(dir.path());
    let db_path = workspace.paths.sessions_db.to_string_lossy().to_string();
    assert!(
        db_path.ends_with("sessions.db"),
        "init_workspace must create .litecode/sessions.db, got {db_path}"
    );

    let session =
        Session::open(&db_path, "/tmp/proj", "default", Some("test-model")).expect("open");
    let session_id = session.id.clone();

    // WAL mode (previously asserted in archived session tests).
    let journal: String = Connection::open(&db_path)
        .expect("open for pragma")
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal_mode");
    assert_eq!(
        journal.to_lowercase(),
        "wal",
        "Session::open must enable WAL"
    );

    let user = user_text("user-hello");
    let assistant = assistant_text_item("assistant-hello", "msg_sess_1");
    session
        .insert_detail_rows(&[user, assistant])
        .expect("insert");

    let loaded = session.load_transcript().expect("load");
    assert_eq!(loaded.len(), 2);
    assert_eq!(item_text_preview(&loaded[0]), "user-hello");
    assert_eq!(item_text_preview(&loaded[1]), "assistant-hello");

    // Resume from disk (business reload path).
    drop(session);
    let resumed = Session::resume(&db_path, &session_id).expect("resume");
    let reloaded = resumed.load_transcript().expect("reload");
    assert_eq!(reloaded.len(), 2);
    assert_eq!(item_text_preview(&reloaded[0]), "user-hello");
    assert_eq!(item_text_preview(&reloaded[1]), "assistant-hello");

    // Second user turn, then revert to second user anchor — keep first turn only.
    resumed
        .insert_detail_rows(&[
            user_text("user-two"),
            assistant_text_item("assistant-two", "msg_sess_2"),
        ])
        .expect("insert turn 2");
    assert_eq!(resumed.load_transcript().unwrap().len(), 4);

    resumed.revert_to_user_anchor(1).expect("revert");
    let after = resumed.load_transcript().expect("after revert");
    assert_eq!(
        after.len(),
        2,
        "revert_to_user_anchor(1) must keep first user+assistant turn, got {after:?}"
    );
    assert_eq!(item_text_preview(&after[0]), "user-hello");
    assert_eq!(item_text_preview(&after[1]), "assistant-hello");
}

#[test]
fn transcript_items_table_exists_messages_fossil_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = test_workspace(dir.path());
    let db_path = workspace.paths.sessions_db.to_string_lossy().to_string();
    let _session = Session::open(&db_path, "/tmp/proj", "default", Some("m")).unwrap();

    let conn = Connection::open(&db_path).expect("open schema check");
    let has_ti: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transcript_items'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let fossil = "messages";
    let messages_table: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{fossil}'"),
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_ti, 1, "transcript_items must exist");
    assert_eq!(messages_table, 0, "fossil messages table must be absent");
}
