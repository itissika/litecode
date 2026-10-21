//! `session/subscribe` must hydrate the pushed snapshot with live hub state
//! (bash jobs + subagent workers) so (re)subscribe is a full sync point and
//! transient jobs cannot leak as permanently "running" after a missed edge.

mod common;

use std::sync::Arc;

use common::SessionDataFixture;
use litecode::client_protocol::controller::SessionController;
use litecode::config::global_db;
use litecode::config::{ConfigManager, SettingsWriter, TurnGuard, WorkspaceState};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::manager::SessionManager;

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

fn snapshot_frame(frames: &[serde_json::Value], sid: &str) -> serde_json::Value {
    frames
        .iter()
        .find(|f| f["method"] == "session/snapshot" && f["params"]["session_id"] == sid)
        .cloned()
        .unwrap_or_else(|| panic!("no session/snapshot frame for {sid}: {frames:?}"))
}

#[tokio::test]
async fn subscribe_hydrates_bash_jobs_and_clears_after_kill() {
    let fixture = SessionDataFixture::new();
    let sid = fixture.create("/proj", "default", None);
    let sessions = Arc::new(fixture.manager());
    let root = fixture.dir.path();
    let mut ctrl = controller(sessions, root);

    let job_id = "bg_a1b2c3d4";
    ctrl.runtime
        .ide
        .terminal
        .jobs
        .insert_live_pipe_job(job_id, &sid, "call_hydrate", "echo LITECODE_HYDRATE", root)
        .expect("pipe job");

    // 1. (Re)subscribe must carry the live bash + subagent hub state.
    ctrl.subscribe_checked(&sid).await.unwrap();
    let out = ctrl.take_outgoing_for(&sid);
    let snap = snapshot_frame(&out, &sid);
    assert_eq!(snap["params"]["session_id"], sid);
    let jobs = snap["params"]["bash"]["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("bash.jobs should be an array: {snap:?}"));
    assert!(
        jobs.iter()
            .any(|j| j["id"] == job_id && j["call_id"] == "call_hydrate"),
        "hydrated bash.jobs must contain the spawned job: {jobs:?}"
    );
    // The wire snapshot has no `subagent.jobs` field (transient bash state is
    // what (re)subscribe hydrates): the subagent half of the projection is the
    // durable session row, so a root session must hydrate at depth 0.
    assert_eq!(
        snap["params"]["meta"]["subagent_depth"], 0,
        "a root session must hydrate with zero subagent depth: {snap:?}"
    );

    // 2. After the job exits, a fresh (re)subscribe must not report it.
    ctrl.runtime
        .ide
        .terminal
        .kill(job_id)
        .expect("kill background job");
    ctrl.unsubscribe(&sid);
    ctrl.subscribe_checked(&sid).await.unwrap();
    let out = ctrl.take_outgoing_for(&sid);
    let snap = snapshot_frame(&out, &sid);
    let jobs = snap["params"]["bash"]["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("bash.jobs should be an array: {snap:?}"));
    assert!(
        !jobs.iter().any(|j| j["id"] == job_id),
        "killed job must not be hydrated again: {jobs:?}"
    );
}
