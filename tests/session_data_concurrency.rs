//! Concurrent SessionData writer: 16×100 appends, conflict, mixed ops.

mod common;

use common::SessionDataFixture;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use litecode::session::SessionData;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::types::{item_text_preview, user_text};

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
    assert_eq!(data.writer_metrics().conflicts, 1);
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 1);
}

#[test]
fn a_queued_manual_compact_and_tail_append_stay_serialized() {
    let data = open();
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("a"), user_text("b"), user_text("c")])
        .unwrap();
    let revision = data.revision_blocking(&sid).unwrap();
    let rows = data.working_set_blocking(&sid).unwrap();
    let prior = rows
        .first()
        .and_then(|row| row.checkpoint().and(row.log_seq));
    let from = rows.first().and_then(|row| row.log_seq).unwrap_or(0);

    data.hooks().pause();
    let compact = data
        .try_mutate(SessionMutation::Compact {
            session_id: sid.clone(),
            operation_id: MutationId::new(),
            summary: "sum".into(),
            token_estimate: 10,
            kept_from: Some(1),
            at: None,
            expected_prior_checkpoint: prior,
            expected_from: from,
            usage: None,
            provider_id: None,
        })
        .unwrap();
    data.hooks().wait_until_parked();

    let late = data
        .try_mutate(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: revision,
            operation_id: MutationId::new(),
            items: vec![user_text("late")],
            turn_id: String::new(),
        })
        .unwrap();
    data.hooks().resume();

    compact.blocking_recv().unwrap().unwrap();
    let conflict = late.blocking_recv().unwrap().unwrap_err();
    assert!(matches!(
        conflict,
        litecode::types::LitecodeError::SessionConflict { .. }
    ));
    assert_eq!(
        conflict.session_write_error_kind(),
        Some(litecode::types::SessionWriteErrorKind::Conflict)
    );
    assert!(
        data.transcript_blocking(&sid)
            .unwrap()
            .iter()
            .all(|item| item_text_preview(item) != "late")
    );
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
