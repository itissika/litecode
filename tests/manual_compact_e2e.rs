//! WP-018 R1: manual-compact e2e through the real
//! `SessionController::start_manual_compact` entry with provider injection
//! (`RuntimeHandle::with_test_llm_override`).
//!
//! Failure (empty summary → CompactionFailed) must release the Compact lease so
//! a second manual compact can start (Andy caliber 3, controller-level).

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::SessionDataFixture;
use common::scripted_provider::ScriptedProvider;
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::engines::WorkspaceEngines;
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::{LitecodeError, StreamEvents, assistant_text, user_text};
use tokio_util::sync::CancellationToken;

fn controller(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
    provider: Arc<dyn litecode::llm::LlmProvider>,
) -> SessionController {
    let db = workspace_root.join("global-litecode.db");
    let mut baseline = common::test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "test-key");
    common::seed_test_catalog(&db, "http://127.0.0.1:9", 128_000);
    global_db::import_into(&db, &baseline).expect("seed global db");

    let guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let revision = writer.revision_handle();
    let settings = writer.load_settings().expect("load");
    let workspace = WorkspaceState::new(workspace_root);
    let resolved = ConfigManager::resolve(settings, workspace.clone(), common::catalog_for_db(&db));
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .expect("ide");
    let runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        workspace_engines,
        ide,
        revision,
        &db,
    )
    .with_test_llm_override(provider);
    SessionController::with_turn_guard(runtime, None, sessions).expect("controller")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compact_failure_e2e_releases_lease_and_retry_starts() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    // ~100k est tokens on a 128k window: eligible (>=30%) and compactable.
    let seed: Vec<_> = (0..10)
        .map(|i| user_text(format!("manual-e2e-{i} {}", "x".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let before = sessions.data().working_set_blocking(&sid).unwrap().len();

    // Both attempts return an empty summary → CompactionFailed.
    let provider = Arc::new(ScriptedProvider::with_texts(&["", "", "", ""]));
    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path(), provider);

    // Cold UI snapshot and manual gate must agree before any provider bill:
    // the server seeds the full local envelope and publishes the decision.
    ctrl.subscribe_checked(&sid).await.expect("subscribe");
    let snap = ctrl.snapshot_for(&sid).expect("cold snapshot");
    assert!(snap.context_meter.manual_eligible());
    assert_eq!(snap.compact_eligible, snap.context_meter.manual_eligible());
    assert_eq!(
        snap.context_tokens_estimate, snap.context_meter.used_tokens,
        "legacy display estimate mirrors unified ContextMeter"
    );
    assert!(
        snap.context_meter.used_tokens > snap.context_envelope.prompt_overhead_tokens,
        "cold local fallback includes the seeded system/tools envelope"
    );

    let op1 = ctrl
        .start_manual_compact(&sid)
        .await
        .expect("first manual compact starts");
    assert!(!op1.is_empty());

    // Retry until the spawned failure drops the lease (AgentAlreadyRunning meanwhile).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let op2 = loop {
        match ctrl.start_manual_compact(&sid).await {
            Ok(id) => break id,
            Err(LitecodeError::AgentAlreadyRunning) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("second manual compact must start after failure, got {e:?}"),
        }
    };
    assert_ne!(op1, op2, "retry must be a fresh operation");

    // Let the second (also failing) attempt settle, then verify no durable change.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match sessions.try_begin_operation(
            &sid,
            litecode::session::manager::SessionOperationKind::Compact,
        ) {
            Ok(_lease) => break,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("lease leaked after second failure: {e:?}"),
        }
    }
    let after = sessions.data().working_set_blocking(&sid).unwrap().len();
    assert_eq!(
        after, before,
        "failed manual compact must not rewrite working set"
    );
    let events = sessions.data().events_blocking(&sid).unwrap();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e.event_type, EventType::Compacted)),
        "failed manual compact must not persist Compacted"
    );
}

/// WP-025 R1: first manual compact fails (empty summary), the user retries and
/// the second attempt succeeds — compact completes, working set is rewritten
/// to the compacted form, `Compacted` is persisted and the lease is released.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compact_retry_after_failure_completes() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", Some(common::TEST_PRIMARY_MODEL_REF));
    let sessions = Arc::new(fixture.manager());
    let seed: Vec<_> = (0..10)
        .map(|i| user_text(format!("manual-retry-ok-{i} {}", "x".repeat(40_000))))
        .collect();
    sessions.insert_detail_rows(&sid, &seed).unwrap();
    let before = sessions.data().working_set_blocking(&sid).unwrap().len();

    // The first manual operation exhausts its two empty attempts and fails;
    // the user's second operation gets one accepted summary.
    let provider = Arc::new(
        ScriptedProvider::with_texts_legacy_accepting_compaction_summary(&[
            "",
            "",
            "accepted summary",
        ]),
    );
    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path(), provider);

    let op1 = ctrl
        .start_manual_compact(&sid)
        .await
        .expect("first manual compact starts");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let op2 = loop {
        match ctrl.start_manual_compact(&sid).await {
            Ok(id) => break id,
            Err(LitecodeError::AgentAlreadyRunning) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) => panic!("retry must start after failure, got {e:?}"),
        }
    };
    assert_ne!(op1, op2, "retry must be a fresh operation");

    // Wait for the successful attempt to persist Compacted and release the lease.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let compacted = sessions
            .data()
            .events_blocking(&sid)
            .unwrap()
            .iter()
            .any(|e| matches!(e.event_type, EventType::Compacted));
        let lease_free = sessions
            .try_begin_operation(
                &sid,
                litecode::session::manager::SessionOperationKind::Compact,
            )
            .is_ok();
        if compacted && lease_free {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("retry did not complete: compacted={compacted} lease_free={lease_free}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let events = sessions.data().events_blocking(&sid).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.event_type, EventType::Compacted))
            .count(),
        1,
        "only the successful retry persists Compacted"
    );
    let after = sessions.data().working_set_blocking(&sid).unwrap().len();
    assert!(
        after < before,
        "successful retry must shrink working set ({before} -> {after})"
    );
    assert!(
        !sessions.is_compacting_blocking(&sid),
        "no residual Compact phase"
    );
}
