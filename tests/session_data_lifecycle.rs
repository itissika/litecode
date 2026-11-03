//! Writer queue backpressure, deadline/cancel, shutdown drain, and metrics.

use std::sync::Arc;
use std::time::Duration;

use litecode::session::data::command::{MutationId, SessionMutation};
use litecode::session::data::{WriteOptions, WriterState};
use litecode::session::{SessionData, WRITER_QUEUE_CAPACITY, WorkspaceWriteLease};
use litecode::types::{SessionWriteErrorKind, item_text_preview, user_text};
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
    let metrics = data.writer_metrics();
    assert_eq!(metrics.queue_depth, WRITER_QUEUE_CAPACITY);
    assert_eq!(metrics.backpressure, 1);

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
    let metrics = data.writer_metrics();
    assert_eq!(metrics.queue_depth, 0);
    assert_eq!(metrics.backpressure, 1);
}

#[test]
fn a_normal_write_exposes_commit_and_queue_metrics() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = data.create_session("/p", "default", None).unwrap();
    data.insert_items(&sid, &[user_text("one")]).unwrap();

    let metrics = data.writer_metrics();
    assert_eq!(metrics.state, WriterState::Ready);
    assert!(metrics.writer_alive);
    assert!(metrics.enqueued >= 2);
    assert!(metrics.committed >= 2);
    assert_eq!(metrics.queue_depth, 0);
    assert!(metrics.last_commit_at_ms.is_some());
    assert!(metrics.queue_wait_p50_ms.is_some());
    assert!(metrics.queue_wait_p95_ms.is_some());
    assert!(metrics.commit_p50_ms.is_some());
    assert!(metrics.commit_p95_ms.is_some());
    assert!(metrics.queue_wait_p50_ms <= metrics.queue_wait_p95_ms);
    assert!(metrics.commit_p50_ms <= metrics.commit_p95_ms);
}

#[test]
fn a_blocking_deadline_that_does_not_expire_commits() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = data.create_session("/p", "default", None).unwrap();
    let receipt = data
        .mutate_blocking_with(
            SessionMutation::InsertDetails {
                session_id: sid,
                expected_revision: 1,
                operation_id: MutationId::new(),
                items: vec![user_text("blocking-live")],
                turn_id: String::new(),
            },
            WriteOptions::with_timeout(Duration::from_secs(5)),
        )
        .unwrap();
    assert_eq!(receipt.revision, 2);
    assert_eq!(data.writer_metrics().timeouts, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_deadline_commits_normally() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = tokio::task::spawn_blocking({
        let data = Arc::clone(&data);
        move || data.create_session("/p", "default", None)
    })
    .await
    .unwrap()
    .unwrap();

    let receipt = data
        .mutate_with(
            SessionMutation::InsertDetails {
                session_id: sid,
                expected_revision: 1,
                operation_id: MutationId::new(),
                items: vec![user_text("live")],
                turn_id: String::new(),
            },
            WriteOptions::with_timeout(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    assert_eq!(receipt.revision, 2);
    assert_eq!(data.writer_metrics().timeouts, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deadline_that_expires_in_the_queue_skips_execution() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = tokio::task::spawn_blocking({
        let data = Arc::clone(&data);
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

    let timed = data
        .try_mutate_with(
            SessionMutation::InsertDetails {
                session_id: sid.clone(),
                expected_revision: 2,
                operation_id: MutationId::new(),
                items: vec![user_text("expired")],
                turn_id: String::new(),
            },
            WriteOptions::with_timeout(Duration::from_millis(30)),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    data.hooks().resume();

    wake.await.unwrap().unwrap();
    let result = timed.await.unwrap();
    match result {
        Err(error) => {
            assert_eq!(
                error.session_write_error_kind(),
                Some(SessionWriteErrorKind::Timeout)
            );
            assert!(error.is_retryable_session_write());
        }
        Ok(_) => panic!("expired queued command must not execute"),
    }
    assert!(data.writer_metrics().timeouts >= 1);

    data.mutate(SessionMutation::InsertDetails {
        session_id: sid.clone(),
        expected_revision: 2,
        operation_id: MutationId::new(),
        items: vec![user_text("barrier")],
        turn_id: String::new(),
    })
    .await
    .unwrap();

    let texts: Vec<String> = data
        .transcript_blocking(&sid)
        .unwrap()
        .iter()
        .map(item_text_preview)
        .collect();
    assert_eq!(texts, vec!["wake", "barrier"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_queued_receiver_cancels_before_execution() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = tokio::task::spawn_blocking({
        let data = Arc::clone(&data);
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

    let dropped = data
        .try_mutate(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 2,
            operation_id: MutationId::new(),
            items: vec![user_text("dropped")],
            turn_id: String::new(),
        })
        .unwrap();
    drop(dropped);
    data.hooks().resume();

    wake.await.unwrap().unwrap();
    data.mutate(SessionMutation::InsertDetails {
        session_id: sid.clone(),
        expected_revision: 2,
        operation_id: MutationId::new(),
        items: vec![user_text("barrier")],
        turn_id: String::new(),
    })
    .await
    .unwrap();

    let texts: Vec<String> = data
        .transcript_blocking(&sid)
        .unwrap()
        .iter()
        .map(item_text_preview)
        .collect();
    assert_eq!(texts, vec!["wake", "barrier"]);
    assert!(data.writer_metrics().canceled >= 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_drains_accepted_commands_and_rejects_new_ones() {
    let dir = TempDir::new().unwrap();
    let lease = WorkspaceWriteLease::acquire(dir.path()).unwrap();
    let data = SessionData::open(&lease, &dir.path().join("sessions.db")).unwrap();
    let sid = tokio::task::spawn_blocking({
        let data = Arc::clone(&data);
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

    let queued = data
        .try_mutate(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 2,
            operation_id: MutationId::new(),
            items: vec![user_text("queued")],
            turn_id: String::new(),
        })
        .unwrap();

    let shutdown_data = Arc::clone(&data);
    tokio::task::spawn_blocking(move || shutdown_data.shutdown())
        .await
        .unwrap();

    assert_eq!(wake.await.unwrap().unwrap().revision, 2);
    assert_eq!(queued.await.unwrap().unwrap().revision, 3);
    assert_eq!(data.writer_state(), WriterState::Closed);
    let metrics = data.writer_metrics();
    assert_eq!(metrics.state, WriterState::Closed);
    assert!(!metrics.writer_alive);
    assert_eq!(metrics.shutdowns, 1);

    let rejected = data
        .mutate(SessionMutation::InsertDetails {
            session_id: sid.clone(),
            expected_revision: 3,
            operation_id: MutationId::new(),
            items: vec![user_text("rejected")],
            turn_id: String::new(),
        })
        .await
        .unwrap_err();
    assert!(matches!(
        rejected,
        litecode::types::LitecodeError::SessionDataClosed
    ));
    assert_eq!(
        rejected.session_write_error_kind(),
        Some(SessionWriteErrorKind::Closed)
    );
    assert!(!rejected.is_retryable_session_write());
    assert!(data.writer_metrics().closed >= 1);

    let texts: Vec<String> = data
        .transcript_blocking(&sid)
        .unwrap()
        .iter()
        .map(item_text_preview)
        .collect();
    assert_eq!(texts, vec!["wake", "queued"]);
}
