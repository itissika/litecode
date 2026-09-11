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

fn controller(sessions: Arc<SessionManager>, workspace_root: &std::path::Path) -> SessionController {
    let db = workspace_root.join("global-litecode.db");
    let mut baseline = common::test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "http://127.0.0.1:9", "test-key", 128_000);
    global_db::import_into(&db, &baseline).expect("seed global db");

    let guard = Arc::new(TurnGuard::new());
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let revision = writer.revision_handle();
    let settings = writer.load_settings().expect("load");
    let workspace = WorkspaceState::new(workspace_root);
    let resolved = ConfigManager::resolve(settings, workspace.clone());
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

/// Mirror of `find_git_bash` in src/terminal/shell.rs — used to pick
/// shell-appropriate commands for the Windows branches below.
#[cfg(windows)]
fn windows_uses_git_bash() -> bool {
    use std::path::PathBuf;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
        candidates.push(PathBuf::from(pf).join("Git").join("bin").join("bash.exe"));
    }
    candidates.push(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe"));
    candidates.push(PathBuf::from(r"C:\Program Files (x86)\Git\bin\bash.exe"));
    candidates.iter().any(|p| p.exists())
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

    #[cfg(windows)]
    let command: &str = if windows_uses_git_bash() {
        "echo LITECODE_HYDRATE; sleep 30"
    } else {
        "Write-Output 'LITECODE_HYDRATE'; Start-Sleep -Seconds 30"
    };
    #[cfg(not(windows))]
    let command = "echo LITECODE_HYDRATE; sleep 30";

    let spawned = ctrl
        .runtime
        .ide
        .terminal
        .spawn_command(command, Some(root), root, &sid, "call_hydrate")
        .expect("spawn background job");

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
            .any(|j| j["id"] == spawned.id && j["call_id"] == "call_hydrate"),
        "hydrated bash.jobs must contain the spawned job: {jobs:?}"
    );
    assert!(
        snap["params"]["subagent"]["jobs"].is_array(),
        "subagent.jobs should be an array: {snap:?}"
    );

    // 2. After the job exits, a fresh (re)subscribe must not report it.
    ctrl.runtime
        .ide
        .terminal
        .kill(&spawned.id)
        .expect("kill background job");
    ctrl.unsubscribe(&sid);
    ctrl.subscribe_checked(&sid).await.unwrap();
    let out = ctrl.take_outgoing_for(&sid);
    let snap = snapshot_frame(&out, &sid);
    let jobs = snap["params"]["bash"]["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("bash.jobs should be an array: {snap:?}"));
    assert!(
        !jobs.iter().any(|j| j["id"] == spawned.id),
        "killed job must not be hydrated again: {jobs:?}"
    );
}
