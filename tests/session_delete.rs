//! `session/delete` after a cold start: list is SQLite, the in-memory record is not.

mod common;

use std::sync::Arc;

use common::SessionDataFixture;
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::event::EventType;
use litecode::session::manager::SessionManager;
use litecode::types::{UserInput, user_text};

fn controller(
    sessions: Arc<SessionManager>,
    workspace_root: &std::path::Path,
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
    );
    SessionController::with_turn_guard(runtime, None, sessions).expect("controller")
}

#[tokio::test]
async fn delete_listed_idle_session_after_cold_start_without_subscribe() {
    // Previous-run durable session: SQLite only. Fresh manager has no SessionRecord
    // (subscribe / start_turn never ran). This is "just launched, delete from the list".
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", None);
    fixture.insert_items(&sid, &[user_text("listed session")]);

    let sessions = Arc::new(fixture.manager());
    let mut ctrl = controller(sessions.clone(), fixture.dir.path());

    let listed = ctrl.list_sessions().await.expect("list");
    assert!(
        listed.sessions.iter().any(|s| s.id == sid),
        "session must be visible in the list before delete"
    );

    ctrl.delete_session(&sid)
        .await
        .expect("idle listed session should delete without a prior subscribe");

    let listed = ctrl.list_sessions().await.expect("list after delete");
    assert!(
        listed.sessions.iter().all(|s| s.id != sid),
        "deleted session must leave the list"
    );
    assert!(sessions.data().meta_blocking(&sid).is_err());
}

#[tokio::test]
async fn ui_revert_goes_through_controller_and_service() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", None);
    fixture.insert_items(&sid, &[user_text("first"), user_text("second")]);
    let sessions = Arc::new(fixture.manager());
    let anchor = sessions
        .data()
        .events_blocking(&sid)
        .expect("events")
        .into_iter()
        .find(|event| event.event_type == EventType::ItemUser)
        .expect("user anchor")
        .seq as u64;

    let mut ctrl = controller(Arc::clone(&sessions), fixture.dir.path());
    ctrl.subscribe(&sid).await;
    sessions
        .enqueue_user_input(&sid, UserInput::text("queued"))
        .expect("pending message");
    ctrl.revert_to_user_anchor(&sid, anchor)
        .expect("controller revert");

    assert!(sessions.pending_messages_snapshot(&sid).is_empty());
    let out = ctrl.take_outgoing_for(&sid);
    assert!(
        out.iter().any(|frame| frame["method"] == "buffer/reverted"),
        "UI revert must emit buffer/reverted after the service operation"
    );
    assert!(
        out.iter()
            .any(|frame| frame["method"] == "agent/operation_result"),
        "UI revert must emit its operation result after refresh"
    );
}
