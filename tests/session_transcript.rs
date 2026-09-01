//! Session business path: workspace DB →Item insert/reload →revert →schema truth.

mod common;

use std::sync::Arc;

use common::assistant_text_item;
use common::test_workspace;
use litecode::config::TurnGuard;
use litecode::session::manager::SessionManager;
use litecode::session::{SessionData, WorkspaceWriteLease};
use litecode::types::{item_text_preview, user_text};

#[test]
fn session_transcript_roundtrip_and_revert_on_workspace_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = test_workspace(dir.path());
    let db_path = workspace.paths.sessions_db.clone();
    assert!(
        db_path.ends_with("sessions.db"),
        "init_workspace must point at .litecode/sessions.db, got {}",
        db_path.display()
    );

    let lease =
        WorkspaceWriteLease::acquire(db_path.parent().expect("sessions data root")).expect("lease");
    let data = SessionData::open(&lease, &db_path).expect("open SessionData");
    let sessions = SessionManager::from_data(Arc::new(TurnGuard::new()), data.clone());
    let session_id = sessions
        .open_session_sync("/tmp/proj", "default", Some("test-model"))
        .expect("open");

    let user = user_text("user-hello");
    let assistant = assistant_text_item("assistant-hello", "msg_sess_1");
    sessions
        .insert_detail_rows(&session_id, &[user, assistant])
        .expect("insert");

    let loaded = data.transcript_blocking(&session_id).expect("load");
    assert_eq!(loaded.len(), 2);
    assert_eq!(item_text_preview(&loaded[0]), "user-hello");
    assert_eq!(item_text_preview(&loaded[1]), "assistant-hello");

    let reloaded = data.transcript_blocking(&session_id).expect("reload");
    assert_eq!(reloaded.len(), 2);

    sessions
        .insert_detail_rows(
            &session_id,
            &[
                user_text("user-two"),
                assistant_text_item("assistant-two", "msg_sess_2"),
            ],
        )
        .expect("insert turn 2");
    assert_eq!(data.transcript_blocking(&session_id).unwrap().len(), 4);

    sessions
        .entry_revert_to_user_anchor(&session_id, 1)
        .expect("revert");
    let after = data.transcript_blocking(&session_id).expect("after revert");
    assert_eq!(
        after.len(),
        2,
        "truncate user_k=1 must keep first user+assistant turn, got {after:?}"
    );
    assert_eq!(item_text_preview(&after[0]), "user-hello");
    assert_eq!(item_text_preview(&after[1]), "assistant-hello");
}

#[test]
fn transcript_items_table_exists_after_session_data_bootstrap() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = test_workspace(dir.path());
    let lease = WorkspaceWriteLease::acquire(
        workspace
            .paths
            .sessions_db
            .parent()
            .expect("sessions data root"),
    )
    .expect("lease");
    let data = SessionData::open(&lease, &workspace.paths.sessions_db).expect("open");
    let _ = data
        .create_session("/tmp/proj", "default", Some("m"))
        .unwrap();
    let listed = data.list_sessions_blocking().expect("list");
    assert_eq!(listed.len(), 1);
}
