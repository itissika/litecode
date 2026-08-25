use std::collections::HashMap;
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

use common::bindings::{binding_all_for, binding_none_tool};
use common::{default_test_global, seed_global_db as write_global_db, test_serve_settings_with_db};

fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("client")
}

fn seed_global_db(path: &std::path::Path) {
    let settings = default_test_global();
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
    let workspace = WorkspaceState {
        workspace_root: project.clone(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: WorkspacePaths::for_workspace(&project, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    };
    let settings = ConfigManager::load_global_from(&global_db_path).expect("load seeded global");
    let resolved = ConfigManager::resolve(settings, workspace.clone());
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

#[tokio::test]
async fn settings_log_get_put() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let base = format!("http://{addr}/api/settings/log");
    let client = test_http_client();

    let get: Value = client
        .get(&base)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert!(get["ok"].as_bool().unwrap_or(false));

    let put = client
        .put(&base)
        .json(&serde_json::json!({ "level": "debug" }))
        .send()
        .await
        .expect("put");
    assert!(put.status().is_success());
    let body: Value = put.json().await.expect("json");
    assert!(body["revision"].as_u64().unwrap_or(0) > 0);

    let get2: Value = client
        .get(&base)
        .send()
        .await
        .expect("get2")
        .json()
        .await
        .expect("json");
    assert_eq!(get2["level"].as_str(), Some("debug"));
}

#[tokio::test]
async fn settings_excludes_seed_and_put() {
    let _restore = RestoreBuiltinExcludes;
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let base = format!("http://{addr}/api/settings/excludes");
    let client = test_http_client();

    let get: Value = client
        .get(&base)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert!(get["ok"].as_bool().unwrap_or(false));
    let search = get["search_exclude"].as_array().expect("search_exclude");
    assert!(
        search
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("node_modules"))),
        "{search:?}"
    );
    assert_eq!(get["git_ignore"].as_bool(), Some(true));

    let put = client
        .put(&base)
        .json(&serde_json::json!({
            "files_exclude": ["**/.git"],
            "search_exclude": ["**/vendor"],
            "watcher_exclude": ["*.litecode-tmp*"],
            "git_ignore": false
        }))
        .send()
        .await
        .expect("put");
    assert!(put.status().is_success());
    let body: Value = put.json().await.expect("json");
    assert_eq!(body["git_ignore"].as_bool(), Some(false));
    let search2 = body["search_exclude"].as_array().expect("search after put");
    assert!(
        !search2
            .iter()
            .any(|v| v.as_str().is_some_and(|s| s.contains("node_modules")))
    );
    assert!(search2.iter().any(|v| v.as_str() == Some("**/vendor")));

    let disk = std::fs::read_to_string(ws.path().join(".litecode").join("excludes.json"))
        .expect("excludes.json");
    assert!(disk.contains("**/vendor"));
    assert!(!disk.contains("node_modules"));
}

struct RestoreBuiltinExcludes;

impl Drop for RestoreBuiltinExcludes {
    fn drop(&mut self) {
        litecode::workspace::filter::activate_workspace_excludes(
            litecode::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
        );
    }
}

#[tokio::test]
async fn settings_auth_endpoint_removed() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let url = format!("http://{addr}/api/settings/auth");
    let client = test_http_client();

    let resp = client
        .put(&url)
        .json(&serde_json::json!({ "token": "secret-token" }))
        .send()
        .await
        .expect("put");
    assert!(
        resp.status() == 404 || resp.status() == 405,
        "legacy settings auth write path must stay gone, got {}",
        resp.status()
    );

    let summary: Value = client
        .get(format!("http://{addr}/api/settings"))
        .send()
        .await
        .expect("summary")
        .json()
        .await
        .expect("json");
    assert!(
        summary.get("has_auth_token").is_none(),
        "has_auth_token must not reappear on settings summary"
    );
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

#[test]
fn config_set_rejects_during_turn() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    seed_global_db(&db);

    let guard = Arc::new(TurnGuard::new());
    let writer = SettingsWriter::with_path(&db, guard.clone());
    guard.begin_turn();
    let err = writer
        .set_key("log.level", "debug")
        .expect_err("should block");
    assert!(matches!(
        err,
        litecode::types::LitecodeError::Config(msg) if msg == "turn_in_progress"
    ));
}

#[test]
fn settings_summary_masks_api_key() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    seed_global_db(&db);
    let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
    let view = writer.provider_view().expect("view");
    assert!(view.api_key.as_ref().is_some_and(|k| k.contains("***")));
}

#[test]
fn write_log_rejects_invalid_level_via_api() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    seed_global_db(&db);
    let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
    let err = writer
        .write_log(litecode::config::schema::LogSettings {
            level: Some("verbose".into()),
        })
        .expect_err("invalid log level");
    assert!(matches!(
        err,
        litecode::types::LitecodeError::Config(msg) if msg.contains("log.level")
    ));
}

#[test]
fn write_log_level_reloadable_from_db() {
    use litecode::config::log_filter;
    use litecode::config::schema::LogSettings;

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    seed_global_db(&db);
    let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
    writer
        .write_log(LogSettings {
            level: Some("debug".into()),
        })
        .expect("write log");
    assert_eq!(log_filter::resolve_level_from_path(&db), "debug");
}

#[test]
fn disabled_binding_changes_tools_count_after_reload() {
    use litecode::llm::provider_from_definition;
    use litecode::runtime::RuntimeHandle;
    use litecode::tool::registry::build_tool_list;

    use common::test_resolved;

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut baseline_global = test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(
        &mut baseline_global,
        "http://127.0.0.1:9",
        "test-key",
        128_000,
    );
    global_db::import_into(&db, &baseline_global).expect("seed db");

    let guard = Arc::new(TurnGuard::new());
    let (writer, engine_manager) = {
        let mut w = SettingsWriter::with_path(&db, guard.clone());
        let em = Arc::new(litecode::optional::EngineManager::new());
        w.set_engine_manager(Arc::clone(&em));
        (Arc::new(w), em)
    };
    let revision = writer.revision_handle();

    let settings = writer.load_settings().expect("load");
    let workspace = WorkspaceState::new("/tmp/p5-tools-count");
    let resolved = ConfigManager::resolve(settings.clone(), workspace.clone());
    let provider = provider_from_definition(&common::stub_test_provider_def(
        "http://127.0.0.1:9",
        "test-key",
    ))
    .expect("provider");
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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let count_before = rt
        .block_on(build_tool_list(
            &runtime.resolved,
            "default",
            provider.box_clone(),
            "test-key",
            0,
            tokio_util::sync::CancellationToken::new(),
            (*runtime.engine_manager).clone(),
            (*runtime.workspace_engines).clone(),
            Arc::clone(&runtime.ide),
            "test-parent-session",
            common::test_sessions_manager(""),
            Arc::clone(&runtime.mcp_pool),
        ))
        .len();
    assert!(
        count_before > 0,
        "expected non-empty tool list before disable"
    );

    let mut profile = settings
        .agents
        .get("default")
        .cloned()
        .expect("default agent");
    profile.tools.get_mut("todo").unwrap().enabled = false;
    writer
        .write_agent("default", profile, &runtime.workspace)
        .expect("write agent");

    runtime.reload_if_needed().expect("reload");

    let count_after = rt
        .block_on(build_tool_list(
            &runtime.resolved,
            "default",
            provider.box_clone(),
            "test-key",
            0,
            tokio_util::sync::CancellationToken::new(),
            (*runtime.engine_manager).clone(),
            (*runtime.workspace_engines).clone(),
            Arc::clone(&runtime.ide),
            "test-parent-session",
            common::test_sessions_manager(""),
            Arc::clone(&runtime.mcp_pool),
        ))
        .len();

    assert!(
        count_after < count_before,
        "expected fewer tools after disabling todo: {count_before} -> {count_after}"
    );
}

#[tokio::test]
async fn settings_custom_tool_write_blocked_during_turn() {
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
        .put(format!("http://{addr}/api/settings/custom-tools/blocked"))
        .json(&serde_json::json!({
            "name": "blocked",
            "command": "echo",
            "args": [],
            "schema": { "type": "object", "properties": {} }
        }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"].as_str(), Some("turn_in_progress"));

    turn_guard.end_turn();
}

#[tokio::test]
async fn settings_agent_write_blocked_during_turn() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let turn_guard = state.turn_guard.clone();
    let addr = spawn_server(state, web_dist).await;
    turn_guard.begin_turn();

    let settings = ConfigManager::load_global_from(&db_path).unwrap();
    let profile = settings.agents.get("default").cloned().unwrap();
    let client = test_http_client();
    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 409);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["error"].as_str(), Some("turn_in_progress"));

    turn_guard.end_turn();
}

#[tokio::test]
async fn settings_put_invalid_log_level_returns_400() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .put(format!("http://{addr}/api/settings/log"))
        .json(&serde_json::json!({ "level": "verbose" }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.expect("json");
    assert!(body["error"].as_str().unwrap_or("").contains("log.level"));
}

#[tokio::test]
async fn settings_put_empty_models_returns_400() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .put(format!("http://{addr}/api/settings/models"))
        .json(&serde_json::json!({ "models": {} }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.expect("json");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("refusing to wipe models")
    );
}

#[tokio::test]
async fn settings_put_models_replaces_registry_and_drops_removed() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let mut settings = ConfigManager::load_global_from(&db_path).unwrap();
    settings.models.insert(
        "extra".into(),
        common::ready_test_model("extra", common::TEST_PROVIDER_ID, "gpt-4", 128_000),
    );
    let models_json: serde_json::Map<String, serde_json::Value> = settings
        .models
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap()))
        .collect();
    let resp = client
        .put(format!("http://{addr}/api/settings/models"))
        .json(&serde_json::json!({ "models": models_json }))
        .send()
        .await
        .expect("put extra");
    assert_eq!(resp.status(), 200);

    settings.models.remove("extra");
    let models_json: serde_json::Map<String, serde_json::Value> = settings
        .models
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap()))
        .collect();
    let resp = client
        .put(format!("http://{addr}/api/settings/models"))
        .json(&serde_json::json!({ "models": models_json }))
        .send()
        .await
        .expect("put without extra");
    assert_eq!(resp.status(), 200);

    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(!loaded.models.contains_key("extra"));
    assert!(loaded.models.contains_key("default"));
}

#[tokio::test]
async fn settings_put_empty_catalog_gone() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .put(format!("http://{addr}/api/settings/tool-catalog"))
        .json(&serde_json::json!({ "tool_catalog": {} }))
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn settings_put_orphan_model_ref_returns_400() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let settings = ConfigManager::load_global_from(&db_path).unwrap();
    let mut profile = settings.agents.get("default").cloned().unwrap();
    profile.model_ref = "does-not-exist".into();

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.expect("json");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("does-not-exist")
    );
}

#[test]
fn reload_if_needed_rebuilds_provider_endpoint_and_refreshes_api_key() {
    use litecode::config::schema::{ProviderConnectionConfig, ProviderDefinition};
    use litecode::runtime::RuntimeHandle;

    use common::test_resolved;

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut baseline = test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "http://old.example/v1", "sk-old", 128_000);
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
    let workspace = WorkspaceState::new("/tmp/reload-provider");
    let resolved = ConfigManager::resolve(settings.clone(), workspace.clone());
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

    assert!(
        runtime
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
            .unwrap()
            .contains("old.example")
    );
    assert_eq!(
        runtime
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.api_key.as_str()),
        Some("sk-old")
    );

    let mut updated = baseline
        .providers
        .get(common::TEST_PROVIDER_ID)
        .cloned()
        .unwrap();
    updated.config.endpoint = "http://new.example/v1".into();
    updated.config.api_key = "sk-new".into();
    writer
        .write_provider(ProviderDefinition {
            id: common::TEST_PROVIDER_ID.to_string(),
            adapter_id: updated.adapter_id,
            label: updated.label,
            config: ProviderConnectionConfig {
                endpoint: updated.config.endpoint,
                api_key: updated.config.api_key,
                auth: updated.config.auth,
            },
        })
        .expect("write provider");

    runtime.reload_if_needed().expect("reload");

    assert!(
        runtime
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
            .unwrap()
            .contains("new.example"),
        "endpoint after reload: {:?}",
        runtime
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
    );
    assert_eq!(
        runtime
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.api_key.as_str()),
        Some("sk-new"),
        "reload must refresh api_key from global DB"
    );
}

#[test]
fn runtime_clone_reloads_stale_provider_after_settings_write() {
    use litecode::config::schema::{ProviderConnectionConfig, ProviderDefinition};
    use litecode::runtime::RuntimeHandle;

    use common::test_resolved;

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut baseline = test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "http://old.example/v1", "sk-old", 128_000);
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
    let workspace = WorkspaceState::new("/tmp/reload-provider-clone");
    let resolved = ConfigManager::resolve(settings.clone(), workspace.clone());
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
        revision.clone(),
        &db,
    );

    let mut updated = baseline
        .providers
        .get(common::TEST_PROVIDER_ID)
        .cloned()
        .unwrap();
    updated.config.endpoint = "http://new.example/v1".into();
    updated.config.api_key = "sk-new".into();
    writer
        .write_provider(ProviderDefinition {
            id: common::TEST_PROVIDER_ID.to_string(),
            adapter_id: updated.adapter_id,
            label: updated.label,
            config: ProviderConnectionConfig {
                endpoint: updated.config.endpoint,
                api_key: updated.config.api_key,
                auth: updated.config.auth,
            },
        })
        .expect("write provider");

    let mut active = runtime.clone();
    active.reload_if_needed().expect("active reload");
    assert!(
        active
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
            .unwrap()
            .contains("new.example")
    );

    let mut fresh_ws = runtime.clone();
    fresh_ws.reload_if_needed().expect("ws clone reload");
    assert!(
        fresh_ws
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
            .unwrap()
            .contains("new.example"),
        "ws clone must reload from DB: {:?}",
        fresh_ws
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.endpoint.as_str())
    );
    assert_eq!(
        fresh_ws
            .resolved
            .providers()
            .get(common::TEST_PROVIDER_ID)
            .map(|p| p.config.api_key.as_str()),
        Some("sk-new")
    );
}

#[tokio::test]
async fn set_session_model_reloads_stale_catalog_after_model_write() {
    use litecode::client_protocol::controller::SessionController;
    use litecode::runtime::RuntimeHandle;
    use litecode::session::store::Session;

    use common::{ready_test_model, test_resolved};

    let ws_dir = TempDir::new().expect("ws");
    let project = ws_dir.path().to_string_lossy().to_string();
    init_workspace(ws_dir.path()).expect("init");
    let workspace_id = litecode::config::peek_workspace_id(ws_dir.path()).expect("wid");
    let paths = WorkspacePaths::for_workspace(ws_dir.path(), &workspace_id);
    let session_db = paths.sessions_db.to_string_lossy().to_string();

    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut baseline = test_resolved("default", &[]).global().clone();
    common::insert_test_llm_registry(&mut baseline, "http://127.0.0.1:9", "sk-test", 128_000);
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
        paths: paths.clone(),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
    };
    let resolved = ConfigManager::resolve(settings, workspace.clone());
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
    runtime.reload_if_needed().expect("ws connect");

    let mut models = writer.load_settings().expect("load").models;
    models.insert(
        "fast".into(),
        ready_test_model("fast", common::TEST_PROVIDER_ID, "fast-wire", 128_000),
    );
    writer.write_models(models).expect("write models");

    assert!(
        !runtime.resolved.global().models.contains_key("fast"),
        "runtime snapshot should be stale before set_model reload"
    );

    let sessions = common::test_sessions_manager(&session_db);
    let session = Session::open(&session_db, &project, "default", None).expect("open");
    let sid = session.id.clone();
    sessions.register_for_test(session);

    let mut ctrl =
        SessionController::with_turn_guard(runtime, None, sessions.clone()).expect("ctrl");
    ctrl.subscribe(&sid).await;

    ctrl.set_session_model(&sid, "fast").expect("set model");

    assert_eq!(
        sessions.session_model_id(&sid).as_deref(),
        Some("fast"),
        "set_model should persist after lazy runtime reload"
    );
}

#[tokio::test]
async fn settings_available_tools_includes_core() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .get(format!("http://{addr}/api/settings/available-tools"))
        .send()
        .await
        .expect("get");
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("json");
    assert!(
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "read")
    );
}

#[tokio::test]
async fn settings_put_agent_empty_model_ref_ok() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let settings = ConfigManager::load_global_from(&db_path).unwrap();
    let mut profile = settings.agents.get("default").cloned().unwrap();
    profile.model_ref = String::new();

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);
    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(loaded.agents.get("default").unwrap().model_ref.is_empty());
}

#[tokio::test]
async fn settings_apply_preset_rewrites_policy_and_path_mode() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .post(format!(
            "http://{addr}/api/settings/agents/default/tools/apply-preset"
        ))
        .json(&serde_json::json!({ "preset": "SAFE" }))
        .send()
        .await
        .expect("apply");
    assert_eq!(resp.status(), 200);

    let agent: Value = client
        .get(format!("http://{addr}/api/settings/agents/default"))
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert_eq!(agent["tools"]["bash"]["last_applied_preset"], "SAFE");
    assert_eq!(agent["tools"]["bash"]["path_mode"], "workspace_only");
    assert_eq!(agent["tools"]["bash"]["policy"]["default"], "deny");
    assert_eq!(
        agent["tools"]["bash"]["policy"]["rules"][0]["id"],
        "readonly_command"
    );
    assert_eq!(agent["tools"]["write"]["policy"]["default"], "ask");
    assert_eq!(agent["tools"]["write"]["path_mode"], "workspace_only");
}

#[tokio::test]
async fn settings_put_agent_expands_last_applied_preset_per_tool() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let mut agent: Value = client
        .get(format!("http://{addr}/api/settings/agents/default"))
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    agent.as_object_mut().unwrap().remove("ok");

    // Leave other tools on ALL; flip only bash to SAFE via last_applied_preset.
    // Stale ALL policy in the body should be overwritten on write.
    agent["tools"]["bash"]["last_applied_preset"] = serde_json::json!("SAFE");
    agent["tools"]["bash"]["policy"] = serde_json::json!({ "default": "allow", "rules": [] });
    agent["tools"]["bash"]["path_mode"] = serde_json::json!("unrestricted");

    let put = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&agent)
        .send()
        .await
        .expect("put");
    assert_eq!(put.status(), 200, "put agent: {}", put.status());

    let loaded: Value = client
        .get(format!("http://{addr}/api/settings/agents/default"))
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert_eq!(loaded["tools"]["bash"]["last_applied_preset"], "SAFE");
    assert_eq!(loaded["tools"]["bash"]["path_mode"], "workspace_only");
    assert_eq!(loaded["tools"]["bash"]["policy"]["default"], "deny");
    assert_eq!(
        loaded["tools"]["bash"]["policy"]["rules"][0]["id"],
        "readonly_command"
    );
    // Sibling tool untouched by the single-tool draft change.
    assert_eq!(loaded["tools"]["write"]["last_applied_preset"], "ALL");
    assert_eq!(loaded["tools"]["write"]["path_mode"], "unrestricted");
    assert_eq!(loaded["tools"]["write"]["policy"]["default"], "allow");
}

#[tokio::test]
async fn settings_put_agent_rejects_legacy_preset_field() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let mut agent: Value = client
        .get(format!("http://{addr}/api/settings/agents/default"))
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    agent.as_object_mut().unwrap().remove("ok");
    agent["tools"]["bash"] = serde_json::json!({ "enabled": true, "preset": "SAFE" });

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&agent)
        .send()
        .await
        .expect("put");
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY,
        "legacy preset field must be rejected, not silently ignored"
    );
}

#[tokio::test]
async fn settings_put_agent_unknown_tool_is_dropped() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let settings = ConfigManager::load_global_from(&db_path).unwrap();
    let mut profile = settings.agents.get("default").cloned().unwrap();
    profile
        .tools
        .insert("phantom-tool".into(), binding_all_for("phantom-tool"));

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);
    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(
        !loaded
            .agents
            .get("default")
            .unwrap()
            .tools
            .contains_key("phantom-tool")
    );
}

#[tokio::test]
async fn settings_put_agent_keeps_dormant_mcp_bind() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    let mut settings = default_test_global();
    settings
        .agents
        .get_mut("default")
        .unwrap()
        .tools
        .insert("mcp_dormant".into(), binding_all_for("mcp_dormant"));
    write_global_db(&db_path, &settings);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let mut profile = ConfigManager::load_global_from(&db_path)
        .unwrap()
        .agents
        .get("default")
        .cloned()
        .unwrap();
    profile.tools.remove("mcp_dormant");
    profile
        .tools
        .insert("webfetch".into(), binding_all_for("webfetch"));

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/default"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);
    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    let tools = &loaded.agents.get("default").unwrap().tools;
    assert!(tools.get("mcp_dormant").is_some_and(|b| b.enabled));
    assert!(tools.get("webfetch").is_some_and(|b| b.enabled));
}

#[tokio::test]
async fn settings_custom_tool_crud_enable_bind_execute_and_delete() {
    use litecode::tool::registry::build_tool_list;

    fn which_all(cmd: &str) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Some(path) = std::env::var_os("PATH") else {
            return out;
        };
        for dir in std::env::split_paths(&path) {
            #[cfg(windows)]
            let candidates = [
                dir.join(cmd),
                dir.join(format!("{cmd}.exe")),
                dir.join(format!("{cmd}.cmd")),
                dir.join(format!("{cmd}.bat")),
            ];
            #[cfg(not(windows))]
            let candidates = [dir.join(cmd)];
            for candidate in candidates {
                if candidate.is_file() {
                    out.push(candidate);
                }
            }
        }
        out
    }

    fn probe_ok(bin: &std::path::Path, args: &[&str]) -> bool {
        std::process::Command::new(bin)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn probe_python() -> Option<std::path::PathBuf> {
        for cmd in ["python3", "python"] {
            for bin in which_all(cmd) {
                if probe_ok(&bin, &["-c", "print(42)"]) {
                    return Some(bin);
                }
            }
        }
        None
    }

    fn probe_node() -> Option<std::path::PathBuf> {
        for bin in which_all("node") {
            if probe_ok(&bin, &["-e", "process.exit(0)"]) {
                return Some(bin);
            }
        }
        None
    }

    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let py_script = repo.join("examples/tools/echo_py.py");
    let node_script = repo.join("examples/tools/echo_node.mjs");
    assert!(py_script.is_file(), "missing {}", py_script.display());
    assert!(node_script.is_file(), "missing {}", node_script.display());

    let python = probe_python();
    let node = probe_node();
    assert!(
        python.is_some() || node.is_some(),
        "custom tool execution gate requires python or node on PATH (no silent echo fallback)"
    );

    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let custom_url = format!("http://{addr}/api/settings/custom-tools");
    let available_url = format!("http://{addr}/api/settings/available-tools");
    let agent_url = format!("http://{addr}/api/settings/agents/default");

    let echo_schema = serde_json::json!({
        "type": "object",
        "properties": { "message": { "type": "string" } },
        "required": ["message"]
    });

    async fn register_enable_bind_execute(
        client: &reqwest::Client,
        custom_url: &str,
        catalog_url: &str,
        agent_url: &str,
        db_path: &std::path::Path,
        workspace: &std::path::Path,
        id: &str,
        command: &str,
        args: &[String],
        description: &str,
        schema: &Value,
        call_message: &str,
    ) {
        let put = client
            .put(format!("{custom_url}/{id}"))
            .json(&serde_json::json!({
                "name": id,
                "description": description,
                "command": command,
                "args": args,
                "timeout": 30,
                "schema": schema,
            }))
            .send()
            .await
            .expect("put custom");
        assert!(put.status().is_success(), "put {id}: {}", put.status());

        let available: Value = client
            .get(catalog_url)
            .send()
            .await
            .expect("available")
            .json()
            .await
            .expect("available json");
        assert!(
            available["tools"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == id && t["kind"] == "custom"),
            "custom tool must appear in available-tools: {available}"
        );

        let mut agent: Value = client
            .get(agent_url)
            .send()
            .await
            .expect("agent")
            .json()
            .await
            .expect("agent json");
        let mut tools_obj = agent
            .get("tools")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        tools_obj.insert(
            id.into(),
            serde_json::json!({
                "enabled": true,
                "policy": { "default": "allow", "default_id": "__default", "rules": [] },
                "path_mode": "unrestricted",
                "last_applied_preset": "ALL"
            }),
        );
        agent["tools"] = Value::Object(tools_obj);
        agent.as_object_mut().unwrap().remove("ok");
        let put_agent = client
            .put(agent_url)
            .json(&agent)
            .send()
            .await
            .expect("put agent");
        assert!(
            put_agent.status().is_success(),
            "put agent {id}: {}",
            put_agent.status()
        );

        let settings = ConfigManager::load_global_from(db_path).unwrap();
        let resolved = ConfigManager::resolve(
            settings,
            litecode::config::workspace::workspace_with_disk_readiness(&WorkspaceState::new(
                workspace,
            )),
        );
        let provider = litecode::llm::provider_from_definition(&common::stub_test_provider_def(
            "http://127.0.0.1:9",
            "test-key",
        ))
        .expect("provider");
        let workspace_engines = litecode::engines::WorkspaceEngines::new();
        let ide = litecode::ide_base::IdeBaseHandle::open(
            workspace,
            std::sync::Arc::new(workspace_engines.clone()),
        )
        .expect("ide");
        let tools = build_tool_list(
            &resolved,
            "default",
            provider,
            "test-key",
            0,
            tokio_util::sync::CancellationToken::new(),
            litecode::optional::EngineManager::new(),
            workspace_engines,
            ide,
            "test-parent-session",
            common::test_sessions_manager(""),
            std::sync::Arc::new(litecode::mcp::McpConnectionPool::new()),
        )
        .await;
        let tool = tools.iter().find(|t| t.name() == id).expect("in tool list");
        let ctx = litecode::context_pipeline::Context {
            cwd: workspace.to_path_buf(),
            workspace_paths: litecode::config::WorkspacePaths::for_legacy_root(workspace),
            agents_md: None,
            claude_md: None,
        };
        assert!(
            tool.description(&ctx).contains("Echo"),
            "description={}",
            tool.description(&ctx)
        );
        let result = tool.call(serde_json::json!({ "message": call_message }));
        assert!(
            !result.content.starts_with("Error:"),
            "{id} failed: {:?}",
            result.content
        );
        assert!(
            result.content.contains(call_message),
            "stdout={:?}",
            result.content
        );
    }

    if let Some(py) = python.as_ref() {
        register_enable_bind_execute(
            &client,
            &custom_url,
            &available_url,
            &agent_url,
            &db_path,
            ws.path(),
            "echo_py",
            &py.to_string_lossy(),
            &[py_script.to_string_lossy().into_owned()],
            "Echo a message via Python custom tool",
            &echo_schema,
            "hello-from-py",
        )
        .await;
    }

    if let Some(node_bin) = node.as_ref() {
        register_enable_bind_execute(
            &client,
            &custom_url,
            &available_url,
            &agent_url,
            &db_path,
            ws.path(),
            "echo_node",
            &node_bin.to_string_lossy(),
            &[node_script.to_string_lossy().into_owned()],
            "Echo a message via Node custom tool",
            &echo_schema,
            "hello-from-node",
        )
        .await;
    }

    let delete_id = if python.is_some() {
        "echo_py"
    } else {
        "echo_node"
    };
    assert!(
        ConfigManager::load_global_from(&db_path)
            .unwrap()
            .agents
            .get("default")
            .unwrap()
            .tools
            .contains_key(delete_id)
    );
    let del = client
        .delete(format!("{custom_url}/{delete_id}"))
        .send()
        .await
        .expect("delete");
    assert!(del.status().is_success());

    let after = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(!after.custom_tools.iter().any(|t| t.name == delete_id));
    assert!(
        !after
            .agents
            .get("default")
            .unwrap()
            .tools
            .contains_key(delete_id)
    );
}

#[tokio::test]
async fn settings_custom_tool_rejects_builtin_id() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);
    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let res = client
        .put(format!("http://{addr}/api/settings/custom-tools/read"))
        .json(&serde_json::json!({
            "name": "read",
            "command": "echo",
            "args": [],
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .send()
        .await
        .expect("put");
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_custom_tool_validation_rejects_bad_inputs() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);
    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/settings/custom-tools");

    let bad_id = client
        .put(format!("{base}/Bad-Name"))
        .json(&serde_json::json!({
            "name": "Bad-Name",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .send()
        .await
        .expect("bad id");
    assert_eq!(bad_id.status(), reqwest::StatusCode::BAD_REQUEST);

    let mismatch = client
        .put(format!("{base}/echo_x"))
        .json(&serde_json::json!({
            "name": "other_name",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .send()
        .await
        .expect("mismatch");
    assert_eq!(mismatch.status(), reqwest::StatusCode::BAD_REQUEST);

    let empty_cmd = client
        .put(format!("{base}/echo_x"))
        .json(&serde_json::json!({
            "name": "echo_x",
            "command": "  ",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .send()
        .await
        .expect("empty cmd");
    assert_eq!(empty_cmd.status(), reqwest::StatusCode::BAD_REQUEST);

    let optional_id = client
        .put(format!("{base}/webfetch"))
        .json(&serde_json::json!({
            "name": "webfetch",
            "command": "echo",
            "schema": { "type": "object", "properties": {}, "required": [] }
        }))
        .send()
        .await
        .expect("optional conflict");
    assert_eq!(optional_id.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_mcp_server_crud_catalog_and_stdio_probe() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);
    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let script = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mock_mcp_server.py");
    let def = serde_json::json!({
        "command": "python3",
        "args": [script.display().to_string()],
        "env": {},
        "transport": { "type": "stdio" }
    });
    let put = client
        .put(format!("http://{addr}/api/settings/mcp-servers/mockecho"))
        .json(&def)
        .send()
        .await
        .expect("put mcp");
    assert!(
        put.status().is_success(),
        "{}",
        put.text().await.unwrap_or_default()
    );

    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(loaded.mcp_servers.contains_key("mockecho"));

    let list: Value = client
        .get(format!("http://{addr}/api/settings/mcp-servers"))
        .send()
        .await
        .expect("list")
        .json()
        .await
        .expect("json");
    assert!(
        list["global"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "mockecho")
    );

    let probe: Value = client
        .post(format!(
            "http://{addr}/api/settings/mcp-servers/mockecho/start"
        ))
        .json(&def)
        .send()
        .await
        .expect("start")
        .json()
        .await
        .expect("start json");
    assert_eq!(probe["ready"], true, "{probe}");
    assert_eq!(probe["status"], "running", "{probe}");
    let tools = probe["tools"].as_array().expect("tools");
    assert!(tools.iter().any(|t| t == "echo"));

    let listed: Value = client
        .get(format!("http://{addr}/api/settings/mcp-servers"))
        .send()
        .await
        .expect("list after start")
        .json()
        .await
        .expect("list json");
    let row = listed["mcp_servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "mockecho")
        .expect("row");
    assert_eq!(row["status"], "running");

    let restarted: Value = client
        .post(format!(
            "http://{addr}/api/settings/mcp-servers/mockecho/restart"
        ))
        .json(&def)
        .send()
        .await
        .expect("restart")
        .json()
        .await
        .expect("restart json");
    assert_eq!(restarted["ready"], true, "{restarted}");
    assert_eq!(restarted["status"], "running");

    let stopped: Value = client
        .post(format!(
            "http://{addr}/api/settings/mcp-servers/mockecho/stop"
        ))
        .send()
        .await
        .expect("stop")
        .json()
        .await
        .expect("stop json");
    assert_eq!(stopped["status"], "stopped", "{stopped}");

    let del = client
        .delete(format!("http://{addr}/api/settings/mcp-servers/mockecho"))
        .send()
        .await
        .expect("delete");
    assert!(del.status().is_success());
    let after = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(!after.mcp_servers.contains_key("mockecho"));
}

#[tokio::test]
async fn settings_mcp_server_rejects_builtin_id() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);
    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let res = test_http_client()
        .put(format!("http://{addr}/api/settings/mcp-servers/bash"))
        .json(&serde_json::json!({
            "command": "echo",
            "args": [],
            "transport": { "type": "stdio" }
        }))
        .send()
        .await
        .expect("put");
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn settings_engine_enable_reflects_readiness_in_catalog_get() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    litecode::config::workspace::enable_code_search_engine(ws.path()).expect("enable retrieval");

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let available_url = format!("http://{addr}/api/settings/available-tools");

    let get: Value = client
        .get(&available_url)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json");
    assert!(
        get["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "code_search" && t["kind"] == "engine")
    );
}

#[tokio::test]
async fn settings_retrieval_engine_init_and_stop_use_engine_api() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let init_url = format!("http://{addr}/api/workspace/retrieval/init");
    let stop_url = format!("http://{addr}/api/workspace/retrieval/stop");
    let engines_url = format!("http://{addr}/api/workspace/engines");
    let available_url = format!("http://{addr}/api/settings/available-tools");

    let init: Value = client
        .post(&init_url)
        .send()
        .await
        .expect("retrieval init")
        .json()
        .await
        .expect("init json");
    assert!(init["ok"].as_bool().unwrap_or(false));
    assert_eq!(init["data"]["desired"], true);

    let available_after_init: Value = client
        .get(&available_url)
        .send()
        .await
        .expect("available get")
        .json()
        .await
        .expect("available json");
    assert!(
        available_after_init["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "code_search")
    );

    let stop: Value = client
        .post(&stop_url)
        .send()
        .await
        .expect("retrieval stop")
        .json()
        .await
        .expect("stop json");
    assert!(stop["ok"].as_bool().unwrap_or(false));
    assert_eq!(stop["data"]["desired"], false);

    let engines: Value = client
        .get(&engines_url)
        .send()
        .await
        .expect("engines")
        .json()
        .await
        .expect("engines json");
    assert_eq!(engines["data"]["engines"]["code_search"]["desired"], false);

    let available_after_stop: Value = client
        .get(&available_url)
        .send()
        .await
        .expect("available get after stop")
        .json()
        .await
        .expect("available json");
    assert!(
        !available_after_stop["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "code_search")
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
    assert!(detail["data"]["retrieval"]["index"]["status"].is_string());
}

#[tokio::test]
async fn settings_engines_detail_exposes_engine_native_state() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    litecode::config::workspace::enable_code_search_engine(ws.path()).expect("retrieval");
    litecode::config::workspace::write_lsp_init(ws.path(), vec!["rust-analyzer".into()])
        .expect("lsp");

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let body: Value = test_http_client()
        .get(format!("http://{addr}/api/workspace/engines/detail"))
        .send()
        .await
        .expect("detail")
        .json()
        .await
        .expect("detail json");

    assert_eq!(body["ok"], true);
    assert_eq!(body["data"]["retrieval"]["desired"], true);
    assert!(body["data"]["retrieval"]["model"].is_object());
    assert!(body["data"]["retrieval"]["index"].is_object());
    assert!(body["data"]["retrieval"]["index"]["status"].is_string());
    assert!(body["data"]["retrieval"]["policy"].is_object());
    assert_eq!(body["data"]["lsp"]["desired"], true);
    assert_eq!(
        body["data"]["lsp"]["configured_servers"][0],
        "rust-analyzer"
    );
    assert!(body["data"]["lsp"]["probes"].is_array());
}

#[tokio::test]
async fn settings_lsp_stop_preserves_servers_and_clears_desired() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    litecode::config::workspace::write_lsp_init(
        ws.path(),
        vec!["rust_analyzer".into(), "typescript".into()],
    )
    .expect("seed lsp");

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();
    let stop_url = format!("http://{addr}/api/workspace/lsp/stop");
    let available_url = format!("http://{addr}/api/settings/available-tools");

    let stop: Value = client
        .post(&stop_url)
        .send()
        .await
        .expect("lsp stop")
        .json()
        .await
        .expect("stop json");
    assert!(stop["ok"].as_bool().unwrap_or(false));
    assert_eq!(stop["data"]["desired"], false);

    assert_eq!(
        litecode::config::workspace::lsp_servers_from_engines(ws.path()),
        vec!["rust_analyzer".to_string(), "typescript".to_string()]
    );
    assert!(!litecode::config::workspace::workspace_engine_desired(
        ws.path(),
        "lsp"
    ));

    let available: Value = client
        .get(&available_url)
        .send()
        .await
        .expect("available")
        .json()
        .await
        .expect("available json");
    assert!(
        !available["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == "lsp")
    );
}

#[tokio::test]
async fn settings_provider_put_returns_restart_required_false() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let settings = ConfigManager::load_global_from(&db_path).unwrap();
    let providers = settings.providers.clone();
    let mut provider = providers
        .get(common::TEST_PROVIDER_ID)
        .cloned()
        .expect("test provider");
    provider.config.endpoint = "http://127.0.0.1:19999/v1".into();

    let resp: Value = client
        .put(format!("http://{addr}/api/settings/providers"))
        .json(&serde_json::json!({
            "providers": {
                common::TEST_PROVIDER_ID: provider
            }
        }))
        .send()
        .await
        .expect("put")
        .json()
        .await
        .expect("json");
    assert!(resp["ok"].as_bool().unwrap_or(false));
    assert_eq!(resp["restart_required"], serde_json::json!(false));
}

#[tokio::test]
async fn settings_put_new_agent_creates_profile() {
    use litecode::config::schema::{AgentProfile, AgentRole};

    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let profile = AgentProfile {
        role: AgentRole::Subagent,
        model_ref: "default".into(),
        system_prompt: "review helper".into(),
        description: "Reviewer".into(),
        ..Default::default()
    };

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/reviewer"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);

    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(loaded.agents.contains_key("reviewer"));
    assert_eq!(loaded.agents["reviewer"].role, AgentRole::Subagent);
}

#[tokio::test]
async fn settings_delete_protected_agent_returns_400() {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path);
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .delete(format!("http://{addr}/api/settings/agents/default"))
        .send()
        .await
        .expect("delete");
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn settings_put_subagent_strips_subagent_launch_binding() {
    use litecode::config::schema::{AgentProfile, AgentRole, AgentToolBinding};

    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    seed_global_db(&db_path);

    let (state, web_dist) = test_state(ws.path().to_path_buf(), db_path.clone());
    let addr = spawn_server(state, web_dist).await;
    let client = test_http_client();

    let profile = AgentProfile {
        role: AgentRole::Subagent,
        model_ref: "default".into(),
        tools: HashMap::from([("subagent_launch".into(), binding_none_tool())]),
        ..Default::default()
    };

    let resp = client
        .put(format!("http://{addr}/api/settings/agents/worker"))
        .json(&profile)
        .send()
        .await
        .expect("put");
    assert_eq!(resp.status(), 200);

    let loaded = ConfigManager::load_global_from(&db_path).unwrap();
    assert!(
        !loaded.agents["worker"]
            .tools
            .contains_key("subagent_launch")
    );
}
