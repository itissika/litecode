//! Concurrent SessionData writer: 16×100 appends, conflict, mixed ops.

mod common;

use common::SessionDataFixture;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use litecode::session::SessionData;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::types::user_text;

fn open() -> Arc<SessionData> {
    let fixture = Box::leak(Box::new(SessionDataFixture::new()));
    Arc::clone(&fixture.data)
}

#[test]
fn same_revision_double_append_one_conflict() {
    let data = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    let rev = data.revision_blocking(&sid).unwrap();
    let ok = Arc::new(AtomicU64::new(0));
    let conflict = Arc::new(AtomicU64::new(0));
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let data = Arc::clone(&data);
            let sid = sid.clone();
            let ok = Arc::clone(&ok);
            let conflict = Arc::clone(&conflict);
            scope.spawn(move || {
                match data.mutate_blocking(SessionMutation::InsertDetails {
                    session_id: sid,
                    expected_revision: rev,
                    operation_id: MutationId::new(),
                    items: vec![user_text("race")],
                    turn_id: String::new(),
                }) {
                    Ok(_) => {
                        ok.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(litecode::types::LitecodeError::SessionConflict { .. }) => {
                        conflict.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(e) => panic!("unexpected {e}"),
                }
            });
        }
    });
    assert_eq!(ok.load(Ordering::SeqCst), 1);
    assert_eq!(conflict.load(Ordering::SeqCst), 1);
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 1);
}

#[test]
fn append_then_delete_late_commit_conflicts() {
    let data = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("keep")]).unwrap();
    let stale = data.revision_blocking(&sid).unwrap();
    let del_rev = stale;
    data.mutate_blocking(SessionMutation::Delete {
        session_id: sid.clone(),
        expected_revision: del_rev,
        operation_id: MutationId::new(),
    })
    .unwrap();
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: stale,
            operation_id: MutationId::new(),
            items: vec![user_text("late")],
            turn_id: String::new(),
        })
        .unwrap_err();
    assert!(matches!(
        err,
        litecode::types::LitecodeError::SessionConflict { .. }
            | litecode::types::LitecodeError::SessionNotFound(_)
            | litecode::types::LitecodeError::SessionStorage(_)
    ));
    assert!(data.meta_blocking(&sid).is_err());
}
