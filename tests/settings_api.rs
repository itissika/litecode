use std::sync::Arc;

use litecode::config::{
    ConfigManager, SettingsWriter, TurnGuard, WorkspacePaths, WorkspaceState, global_db,
    init_workspace,
};
use litecode::engines::WorkspaceEngines;
use litecode::serve::ServeState;
use litecode::serve::router;
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::TcpListener;

mod common;

use common::{default_test_global, seed_global_db as write_global_db, test_serve_settings_with_db};

fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("client")
}

fn seed_global_db(path: &std::path::Path) {
    let settings = default_test_global();
    // The catalog lives next to the global DB; seed it before the first load so
    // the fixture provider (and its models) exist for this DB path.
    common::seed_test_catalog(path, common::TEST_CATALOG_ENDPOINT, 128_000);
    write_global_db(path, &settings);
}

fn ensure_test_web_dist(project: &std::path::Path) -> std::path::PathBuf {
    let web_dist = project.join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist dir");
    std::fs::write(web_dist.join("index.html"), "<!DOCTYPE html><html></html>")
        .expect("index.html");
    web_dist
}

fn test_state(
    project: std::path::PathBuf,
    global_db_path: std::path::PathBuf,
) -> (ServeState, std::path::PathBuf) {
    init_workspace(&project).expect("init workspace");
    let web_dist = ensure_test_web_dist(&project);
    let workspace_id = litecode::config::peek_workspace_id(&project).expect("workspace identity");
    let workspace = {
        let workspace = WorkspaceState {
            workspace_root: project.clone(),
            workspace_id: workspace_id.clone(),
            contract: String::new(),
            agents_md: String::new(),
            paths: WorkspacePaths::for_workspace(&project, &workspace_id),
            workspace_tool_readiness: Default::default(),
            workspace_mcp_servers: Default::default(),
            workspace_custom_tools: Default::default(),
        };
        litecode::config::workspace::workspace_with_disk_readiness(&workspace)
    };
    let settings = ConfigManager::load_global_from(&global_db_path).expect("load seeded global");
    let resolved = ConfigManager::resolve(
        settings,
        workspace.clone(),
        common::catalog_for_db(&global_db_path),
    );
    let turn_guard = Arc::new(TurnGuard::new());
    let (settings_writer, engine_manager) =
        test_serve_settings_with_db(turn_guard.clone(), &global_db_path);
    let state = ServeState::with_project(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        Arc::new(WorkspaceEngines::new()),
        None,
        None,
        project,
        turn_guard,
        settings_writer,
    )
    .expect("serve state");
    (state, web_dist)
}

async fn spawn_server(state: ServeState, web_dist: std::path::PathBuf) -> std::net::SocketAddr {
    let watcher = litecode::workspace::spawn_watcher(state.workspace.clone()).expect("watcher");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router::router(state, web_dist);
    tokio::spawn(async move {
        let _watcher = watcher;
        axum::serve(listener, app).await.expect("serve");
    });
    let client = test_http_client();
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
    panic!("server did not become ready");
}

#[test]
fn config_set_auth_token_rejected() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    seed_global_db(&db);
    let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
    let err = writer
        .set_key("auth.token", "should-fail")
        .expect_err("auth.token must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("LITECODE_TOKEN") || msg.contains("removed"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn settings_write_blocked_during_turn() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let turn_guard = state.turn_guard.clone();
    let addr = spawn_server(state, web_dist).await;
    turn_guard.begin_turn();

    let client = test_http_client();
    let resp = client
        .put(format!("http://{addr}/api/settings/log"))
        .json(&serde_json::json!({ "level": "warn" }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"].as_str(), Some("turn_in_progress"));

    turn_guard.end_turn();
}

#[tokio::test]
async fn settings_provider_key_write_bumps_revision_and_masks_secret() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let llm_url = format!("http://{addr}/api/settings/llm");

    let before: Value = client
        .get(&llm_url)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    let revision_before = before["revision"].as_u64().expect("llm view revision");

    const SECRET: &str = "sk-super-secret-value";
    let ack: Value = client
        .put(format!(
            "http://{addr}/api/settings/providers/{}/key",
            common::TEST_PROVIDER_ID
        ))
        .json(&serde_json::json!({ "api_key": SECRET }))
        .send()
        .await
        .expect("put")
        .json()
        .await
        .expect("json");
    assert_eq!(ack["ok"], serde_json::json!(true));
    assert_eq!(
        ack["revision"].as_u64(),
        Some(revision_before + 1),
        "a provider key write must bump the settings revision"
    );
    assert!(
        ack["docs"]
            .as_array()
            .expect("docs")
            .iter()
            .any(|d| d.as_str() == Some("llm")),
        "a provider key write commits the llm document: {ack}"
    );

    let after: Value = client
        .get(&llm_url)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert_eq!(
        after["revision"].as_u64(),
        Some(revision_before + 1),
        "the projection revision follows the commit"
    );
    let provider = after["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|p| p["id"] == common::TEST_PROVIDER_ID)
        .expect("fixture provider in the llm view")
        .clone();
    assert_eq!(provider["configured"], serde_json::json!(true));
    let masked = provider["masked_api_key"]
        .as_str()
        .expect("configured provider carries a masked key");
    assert!(masked.contains("***"), "mask keeps the *** shape: {masked}");
    assert!(
        !after.to_string().contains(SECRET),
        "GET /api/settings/llm must never echo the raw key: {after}"
    );
}

#[tokio::test]
async fn set_session_model_persists_selectable_catalog_ref() {
    use litecode::client_protocol::controller::SessionController;
    use litecode::runtime::RuntimeHandle;

    let ws_dir = TempDir::new().expect("ws");
    let project = ws_dir.path().to_string_lossy().to_string();
    init_workspace(ws_dir.path()).expect("init");
    let workspace_id = litecode::config::peek_workspace_id(ws_dir.path()).expect("wid");
    let paths = WorkspacePaths::for_workspace(ws_dir.path(), &workspace_id);
    let session_db = paths.sessions_db.to_string_lossy().to_string();

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    common::seed_test_catalog(&db, common::TEST_CATALOG_ENDPOINT, 128_000);
    // Seed the full default agent set, then point it at the fixture catalog.
    let mut baseline = ConfigManager::load_global_from(&db).expect("seed default global");
    common::insert_test_llm_registry(&mut baseline, "sk-test");
    global_db::import_into(&db, &baseline).expect("seed");

    let guard = Arc::new(TurnGuard::new());
    let (writer, engine_manager) = {
        let mut w = SettingsWriter::with_path(&db, guard.clone());
        let em = Arc::new(litecode::optional::EngineManager::new());
        w.set_engine_manager(Arc::clone(&em));
        (Arc::new(w), em)
    };
    let revision = writer.revision_handle();

    let settings = writer.load_settings().expect("load");
    let workspace = WorkspaceState {
        workspace_root: ws_dir.path().to_path_buf(),
        workspace_id,
        contract: String::new(),
        agents_md: String::new(),
        paths: paths.clone(),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    };
    let resolved = ConfigManager::resolve(settings, workspace.clone(), common::catalog_for_db(&db));
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .expect("ide");
    let mut runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        workspace_engines,
        ide,
        revision,
        &db,
    );
    runtime
        .apply(litecode::config::DocId::ALL)
        .expect("ws connect");

    // Provider/model facts live in a process-wide catalog loaded once per DB
    // path: no settings write can make the runtime's copy stale, and the lazy
    // reload inside set_session_model must keep serving that same catalog.
    assert!(
        Arc::ptr_eq(
            runtime.resolved.catalog(),
            &writer.catalog().expect("writer catalog")
        ),
        "the runtime and the writer share one catalog instance"
    );
    assert!(
        runtime
            .resolved
            .model_for_agent_ref(common::TEST_PRIMARY_MODEL_REF)
            .is_some(),
        "the seeded credential makes the fixture model selectable"
    );

    let sessions = common::test_sessions_manager(&session_db);
    let sid = sessions
        .open_session_sync(&project, "default", Some(common::TEST_PRIMARY_MODEL_REF))
        .expect("open");

    let mut ctrl =
        SessionController::with_turn_guard(runtime, None, sessions.clone()).expect("ctrl");
    ctrl.subscribe(&sid).await;
    let _ = ctrl.take_outgoing_for(&sid);

    let revision_before_switch = sessions.data().revision_blocking(&sid).expect("revision");
    ctrl.set_session_model(&sid, common::TEST_COMPACTION_MODEL_REF)
        .expect("set model");
    assert_eq!(
        sessions.session_model_id(&sid).as_deref(),
        Some(common::TEST_COMPACTION_MODEL_REF),
        "set_model persists the composite catalog reference after the lazy reload"
    );
    let _ = ctrl.take_outgoing_for(&sid);

    let revision_after_first_switch = sessions.data().revision_blocking(&sid).expect("revision");
    assert_eq!(
        revision_after_first_switch,
        revision_before_switch + 1,
        "one model switch must commit state and history in one revision"
    );
    let model_changes_after_first_switch = sessions
        .data()
        .events_blocking(&sid)
        .expect("events")
        .into_iter()
        .filter(|event| event.event_type == litecode::session::EventType::ModelChanged)
        .count();
    assert_eq!(model_changes_after_first_switch, 1);

    // Re-selecting the current model is a no-op: no duplicate control-plane
    // row and no revision bump.
    ctrl.set_session_model(&sid, common::TEST_COMPACTION_MODEL_REF)
        .expect("same-model selection is accepted");
    let _ = ctrl.take_outgoing_for(&sid);
    assert_eq!(
        sessions.data().revision_blocking(&sid).expect("revision"),
        revision_after_first_switch,
        "same-model selection must not mutate the session"
    );
    let model_changes_after_repeat = sessions
        .data()
        .events_blocking(&sid)
        .expect("events")
        .into_iter()
        .filter(|event| event.event_type == litecode::session::EventType::ModelChanged)
        .count();
    assert_eq!(
        model_changes_after_repeat, model_changes_after_first_switch,
        "same-model selection must not append model/changed"
    );

    // A reference the catalog does not declare is refused on the wire and never
    // overwrites the persisted session row.
    ctrl.set_session_model(&sid, "test/ghost-model")
        .expect("a refusal is reported on the wire, not as a hard error");
    let frames = ctrl.take_outgoing_for(&sid);
    let refusal = frames
        .iter()
        .find(|f| f["method"] == "agent/operation_result" && f["params"]["op"] == "set_model")
        .unwrap_or_else(|| panic!("set_model answered on the wire: {frames:#?}"));
    assert_eq!(refusal["params"]["ok"], false);
    assert_eq!(refusal["params"]["error"]["code"], "invalid_request");
    assert_eq!(
        sessions.session_model_id(&sid).as_deref(),
        Some(common::TEST_COMPACTION_MODEL_REF),
        "a refused model must not overwrite the persisted reference"
    );
}

#[tokio::test]
async fn settings_retrieval_refresh_endpoint_accepts_when_cold() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let enable: Value = client
        .put(format!("http://{addr}/api/settings/engines"))
        .json(&serde_json::json!({
            "version": 1,
            "lsp": { "desired": false, "servers": [] },
            "retrieval": { "desired": true },
        }))
        .send()
        .await
        .expect("enable retrieval")
        .json()
        .await
        .expect("enable json");
    assert!(enable["ok"].as_bool().unwrap_or(false));
    let refresh_url = format!("http://{addr}/api/workspace/retrieval/refresh");

    let refresh: Value = client
        .post(&refresh_url)
        .send()
        .await
        .expect("retrieval refresh")
        .json()
        .await
        .expect("refresh json");
    assert!(refresh["ok"].as_bool().unwrap_or(false));
    assert_eq!(refresh["data"]["desired"], true);
    let mode = refresh["data"]["mode"].as_str().unwrap_or("");
    assert!(
        mode == "starting" || mode == "in_progress" || mode == "rebuild" || mode == "incremental",
        "unexpected mode: {mode}"
    );

    let detail: Value = client
        .get(format!("http://{addr}/api/workspace/engines/detail"))
        .send()
        .await
        .expect("detail")
        .json()
        .await
        .expect("detail json");
    // WP-131: status lives in retrieval.total; legacy index.status is gone.
    assert!(detail["data"]["retrieval"]["total"]["state"].is_string());
    assert!(detail["data"]["retrieval"]["index"]["status"].is_null());
}

#[tokio::test]
async fn settings_provider_key_write_is_effective_next_turn() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .put(format!(
            "http://{addr}/api/settings/providers/{}/key",
            common::TEST_PROVIDER_ID
        ))
        .json(&serde_json::json!({ "api_key": "sk-next-turn" }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);

    let summary: Value = client
        .get(format!("http://{addr}/api/settings"))
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert_eq!(
        summary["effective_next_turn"],
        serde_json::json!(true),
        "a credential write applies on the next turn: {summary}"
    );
    assert_eq!(summary["restart_required"], serde_json::json!(false));
    assert_eq!(
        summary["configured_provider_count"].as_u64(),
        Some(1),
        "the fixture DB configures exactly one provider: {summary}"
    );
    assert_eq!(
        summary["active_model_count"].as_u64(),
        Some(2),
        "both fixture models become selectable: {summary}"
    );
}

#[tokio::test]
async fn available_tools_stamps_a_code_surface_custom_tool() {
    use litecode::config::schema::{CallSurface, CustomToolDefinition, ToolSchema};

    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let writer = SettingsWriter::with_path(&db_path, Arc::new(TurnGuard::new()));
    writer
        .write_custom_tool(
            "code_probe",
            CustomToolDefinition {
                name: "code_probe".into(),
                data_schema: None,
                description: "code probe".into(),
                schema: ToolSchema {
                    schema_type: "object".into(),
                    properties: serde_json::json!({}),
                    required: Vec::new(),
                },
                command: "echo".into(),
                args: Vec::new(),
                timeout: 120,
                rules: Vec::new(),
                surface: CallSurface::Code,
                suite: None,
                suite_label: None,
            },
        )
        .expect("write custom code tool");

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let payload: Value = client
        .get(format!("http://{addr}/api/settings/available-tools"))
        .send()
        .await
        .expect("available tools")
        .json()
        .await
        .expect("available tools json");

    let card = payload["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .find(|tool| tool["id"] == "code_probe")
        .expect("custom tool card");
    assert_eq!(card["surface"], "code", "{payload}");
    assert!(
        payload["code_rejections"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "a valid code custom tool must not be rejected: {payload}"
    );
}
