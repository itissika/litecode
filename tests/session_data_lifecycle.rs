//! Writer queue backpressure and shutdown drain.

use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::{SessionData, WRITER_QUEUE_CAPACITY, WorkspaceWriteLease};
use litecode::types::user_text;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread")]
async fn queue_capacity_keeps_overflow_pending_until_cancelled() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = tokio::task::spawn_blocking({
        let data = std::sync::Arc::clone(&data);
        move || data.create_session("/p", "default", None)
    })
    .await
    .unwrap()
    .unwrap();
    data.hooks().pause();
    let wake = data
        .try_mutate(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 1,
            operation_id: MutationId::new(),
            items: vec![user_text("wake")],
            turn_id: String::new(),
        })
        .unwrap();
    tokio::task::spawn_blocking({
        let hooks = data.hooks();
        move || hooks.wait_until_parked()
    })
    .await
    .unwrap();

    let mut accepted = Vec::new();
    for i in 0..WRITER_QUEUE_CAPACITY {
        let rev = 1 + i as u64;
        let rx = data
            .try_mutate(SessionMutation::InsertDetails {
                session_id: sid.clone(),
                expected_revision: rev,
                operation_id: MutationId::new(),
                items: vec![user_text(format!("q-{i}"))],
                turn_id: String::new(),
            })
            .expect("accepted into queue");
        accepted.push(rx);
    }
    let overflow = data.try_mutate(SessionMutation::InsertDetails {
        session_id: sid.clone(),
        expected_revision: 1,
        operation_id: MutationId::new(),
        items: vec![user_text("overflow")],
        turn_id: String::new(),
    });
    assert!(matches!(
        overflow,
        Err(litecode::types::LitecodeError::SessionBackpressure)
    ));

    let pending = tokio::spawn({
        let data = std::sync::Arc::clone(&data);
        let sid = sid.clone();
        async move {
            data.mutate(SessionMutation::InsertDetails {
                session_id: sid,
                expected_revision: 1,
                operation_id: MutationId::new(),
                items: vec![user_text("pending")],
                turn_id: String::new(),
            })
            .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    pending.abort();
    let aborted = pending.await;
    assert!(aborted.is_err());

    data.hooks().resume();
    let _ = wake.await;
    for rx in accepted {
        let _ = rx.await;
    }
}

#[test]
fn shutdown_drains_accepted_and_rejects_new() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("sessions.db");
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &db).unwrap();
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("kept")]).unwrap();
    data.shutdown();
    let err = data
        .mutate_blocking(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 2,
            operation_id: MutationId::new(),
            items: vec![user_text("after close")],
            turn_id: String::new(),
        })
        .unwrap_err();
    assert!(matches!(
        err,
        litecode::types::LitecodeError::SessionDataClosed
    ));
    let data2 = SessionData::open(&lease, &db).unwrap();
    assert_eq!(data2.transcript_blocking(&sid).unwrap().len(), 1);
}
