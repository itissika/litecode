//! Process-level workspace contract: one serve process = one workspace for life.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use litecode::config::{
    TurnGuard, WorkspacePaths, WorkspaceState, init_workspace, load_workspace_state,
};
use litecode::engines::WorkspaceEngines;
use litecode::serve::ServeState;
use litecode::serve::router;
use litecode::session::WorkspaceLock;
use litecode::workspace::restart_watcher;
use tokio::net::TcpListener;

mod common;
use common::{test_agent, test_resolved, test_serve_settings};

static CONTRACT_LOCK: Mutex<()> = Mutex::new(());

fn ensure_test_web_dist(project: &std::path::Path) -> PathBuf {
    let web_dist = project.join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist dir");
    std::fs::write(web_dist.join("index.html"), "<!DOCTYPE html><html></html>")
        .expect("index.html");
    web_dist
}

fn test_state(project: PathBuf) -> (ServeState, common::TestServeFixture, PathBuf) {
    init_workspace(&project).expect("init workspace");
    let web_dist = ensure_test_web_dist(&project);
    let workspace_id = litecode::config::peek_workspace_id(&project).expect("workspace identity");
    let workspace = WorkspaceState {
        workspace_root: project.clone(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: WorkspacePaths::for_workspace(&project, &workspace_id),
        workspace_tool_readiness: Default::default(),
    };
    let agent = test_agent(vec![], "default", 50);
    let resolved = test_resolved("default", &agent.tools);
    let turn_guard = Arc::new(TurnGuard::new());
    let serve = test_serve_settings(turn_guard.clone());
    let state = ServeState::with_project(
        resolved,
        "default".into(),
        workspace,
        serve.engine_manager.clone(),
        Arc::new(WorkspaceEngines::new()),
        None,
        None,
        project,
        turn_guard,
        serve.settings_writer.clone(),
    )
    .expect("serve state");
    (state, serve, web_dist)
}

async fn spawn_test_server(state: ServeState, web_dist: PathBuf) -> std::net::SocketAddr {
    restart_watcher(&state.watcher, state.workspace.clone())
        .await
        .expect("watcher");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router::router(state, web_dist);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    let health = format!("http://{addr}/health");
    for _ in 0..100 {
        if client
            .get(&health)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return addr;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("test server not ready");
}

#[test]
fn load_workspace_state_sets_runtime_paths_without_chdir() {
    let _guard = CONTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::current_dir().expect("prev cwd");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let state = load_workspace_state(Some(dir.path())).expect("load");
    // No global chdir side effect: the process cwd is left untouched.
    assert_eq!(
        std::env::current_dir()
            .expect("cwd")
            .canonicalize()
            .unwrap(),
        prev.canonicalize().unwrap(),
        "load_workspace_state must not chdir the process"
    );
    // RUNTIME_PATHS is explicitly initialized: consumers resolve the workspace
    // through active_paths() even though the process cwd is elsewhere.
    let active = litecode::config::workspace::active_paths();
    assert!(
        active.plan_dir.starts_with(&state.workspace_root)
            || state.workspace_root.starts_with(&active.plan_dir),
        "active_paths must point into the loaded workspace, got {:?} vs root {:?}",
        active.plan_dir,
        state.workspace_root
    );
    assert_eq!(state.workspace_root.canonicalize().unwrap(), root);
    assert!(
        !state.workspace_id.is_empty(),
        "load_workspace_state must bind a stable workspace_id"
    );
    assert_eq!(
        litecode::config::peek_workspace_id(&state.workspace_root).as_deref(),
        Some(state.workspace_id.as_str())
    );
    litecode::config::workspace::clear_runtime_paths();
    let _ = std::env::set_current_dir(prev);
}

#[test]
fn workspace_lock_second_acquire_same_root_fails() {
    let _guard = CONTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    init_workspace(dir.path()).unwrap();
    let litecode = dir.path().join(".litecode");
    let _first = WorkspaceLock::acquire(&litecode).expect("first");
    let err = WorkspaceLock::acquire(&litecode).expect_err("second must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("already open") || msg.contains("lock busy"),
        "unexpected: {msg}"
    );
}

#[test]
fn workspace_lock_different_roots_can_coexist() {
    let _guard = CONTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    init_workspace(a.path()).unwrap();
    init_workspace(b.path()).unwrap();
    let _la = WorkspaceLock::acquire(&a.path().join(".litecode")).expect("a");
    let _lb = WorkspaceLock::acquire(&b.path().join(".litecode")).expect("b");
}

#[tokio::test]
async fn workspace_open_http_endpoint_is_gone() {
    let _guard = CONTRACT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();
    let open_path = format!("/api/workspace/{}", "open");
    let resp = client
        .post(format!("http://{addr}{open_path}"))
        .json(&serde_json::json!({ "path": dir.path().to_string_lossy() }))
        .send()
        .await
        .expect("request");
    assert!(
        matches!(resp.status().as_u16(), 404 | 405),
        "workspace open HTTP route must not exist (got {})",
        resp.status()
    );
}
