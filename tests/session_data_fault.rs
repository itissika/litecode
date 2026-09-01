//! Injected writer faults: rollback vs commit-after-reply-loss.

mod common;

use common::SessionDataFixture;
use litecode::session::FaultKind;
use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::types::user_text;

fn open() -> (
    SessionDataFixture,
    std::sync::Arc<litecode::session::SessionData>,
    String,
) {
    let fixture = SessionDataFixture::new();
    let data = fixture.data.clone();
    let sid = data.create_session("/p", "default", None).unwrap();
    (fixture, data, sid)
}

#[test]
fn fault_before_begin_leaves_db_unchanged() {
    let (_dir, data, sid) = open();
    data.hooks().inject(FaultKind::BeforeBegin);
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: MutationId::new(),
            items: vec![user_text("nope")],
            turn_id: String::new(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("before begin"));
    assert!(data.transcript_blocking(&sid).unwrap().is_empty());
    assert_eq!(data.revision_blocking(&sid).unwrap(), 1);
}

#[test]
fn fault_before_commit_rolls_back() {
    let (_dir, data, sid) = open();
    data.hooks().inject(FaultKind::BeforeCommit);
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: MutationId::new(),
            items: vec![user_text("nope")],
            turn_id: String::new(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("before commit"));
    assert!(data.transcript_blocking(&sid).unwrap().is_empty());
}

#[test]
fn fault_after_commit_keeps_rows_and_operation_is_idempotent() {
    let (_dir, data, sid) = open();
    let op = MutationId("after-commit-op".into());
    data.hooks().inject(FaultKind::AfterCommit);
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: op.clone(),
            items: vec![user_text("committed")],
            turn_id: String::new(),
        })
        .unwrap_err();
    assert!(err.to_string().contains("after commit"));
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 1);
    let retry = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: op,
            items: vec![user_text("committed")],
            turn_id: String::new(),
        })
        .unwrap();
    assert_eq!(retry.revision, 2);
    assert_eq!(data.transcript_blocking(&sid).unwrap().len(), 1);
}
