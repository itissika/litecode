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

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
    let pending = tokio::spawn({
        let data = std::sync::Arc::clone(&data);
        let sid = sid.clone();
        async move {
            // T4: notify when mutate is about to park on the full queue (replaces sleep(50ms)).
            let _ = entered_tx.send(());
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
    // Poll with deadline until the spawned task signals entry, then yield so
    // mutate()'s send().await is polled and parks on the full channel.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), entered_rx).await;
    tokio::task::yield_now().await;
    pending.abort();
    let aborted = pending.await;
    assert!(aborted.is_err());

    data.hooks().resume();
    let _ = wake.await;
    for rx in accepted {
        let _ = rx.await;
    }
}

