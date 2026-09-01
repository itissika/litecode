//! SessionData bootstrap: empty workspace, reopen, schema, WAL (via sqlite unit tests).

use litecode::session::{SessionData, WorkspaceWriteLease};
use tempfile::TempDir;

#[test]
fn empty_workspace_opens_and_lists_nothing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &db).unwrap();
    assert!(data.list_session_ids_blocking().unwrap().is_empty());
    let sid = data.create_session("/proj", "default", Some("m")).unwrap();
    assert_eq!(data.list_session_ids_blocking().unwrap(), vec![sid.clone()]);
    data.shutdown();
    let data2 = SessionData::open(&lease, &db).unwrap();
    assert_eq!(data2.list_session_ids_blocking().unwrap(), vec![sid]);
}

#[test]
fn reopen_with_held_lease_is_readable() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let a = SessionData::open(&lease, &db).unwrap();
    let sid = a.create_session("/p", "default", None).unwrap();
    a.insert_items(&sid, &[litecode::types::user_text("hello")])
        .unwrap();
    drop(a);
    let b = SessionData::open(&lease, &db).unwrap();
    let transcript = b.transcript_blocking(&sid).unwrap();
    assert_eq!(transcript.len(), 1);
}
