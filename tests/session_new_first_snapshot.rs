//! PR-3/F5: the first `session/new` snapshot must carry the same full
//! server-owned request envelope the first subscribe would seed, not a 0/0 cold
//! placeholder that makes the ring/limits lie until the next frame.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use litecode::config::{ConfigManager, TurnGuard, WorkspacePaths, WorkspaceState, init_workspace};
use litecode::engines::WorkspaceEngines;
use litecode::serve::ServeState;
use litecode::serve::router;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

mod common;

const TOKEN: &str = "session-new-first-snapshot-token";

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn ensure_web_dist(project: &std::path::Path) -> std::path::PathBuf {
    let web_dist = project.join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist");
    std::fs::write(web_dist.join("index.html"), "<!doctype html>").expect("index");
    web_dist
}

fn test_state(
    project: std::path::PathBuf,
    db: &std::path::Path,
) -> (ServeState, std::path::PathBuf) {
    init_workspace(&project).expect("workspace");
    let web_dist = ensure_web_dist(&project);
    let workspace_id = litecode::config::peek_workspace_id(&project).expect("workspace id");
    let workspace = {
        let ws = WorkspaceState {
            workspace_root: project.clone(),
            workspace_id,
            contract: String::new(),
            agents_md: String::new(),
            paths: WorkspacePaths::for_workspace(&project, "test"),
            workspace_tool_readiness: Default::default(),
            workspace_mcp_servers: Default::default(),
            workspace_custom_tools: Default::default(),
            workspace_custom_suites: Default::default(),
        };
        litecode::config::workspace::workspace_with_disk_readiness(&ws)
    };
    let settings = ConfigManager::load_global_from(db).expect("seeded global");
    let resolved = ConfigManager::resolve(settings, workspace.clone(), common::catalog_for_db(db));
    let turn_guard = Arc::new(TurnGuard::new());
    let (settings_writer, engine_manager) =
        common::test_serve_settings_with_db(turn_guard.clone(), db);
    let state = ServeState::with_project(
        resolved,
        "default".into(),
        workspace,
        engine_manager,
        Arc::new(WorkspaceEngines::new()),
        None,
        Some(TOKEN.into()),
        project,
        turn_guard,
        settings_writer,
    )
    .expect("serve state");
    (state, web_dist)
}

async fn next_frame(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(15), ws.next())
            .await
            .expect("frame timeout")
            .expect("socket open")
            .expect("ws message");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("json frame");
        }
    }
}

async fn send_new_and_capture(
    state: ServeState,
    web_dist: std::path::PathBuf,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Vec<Value>,
    String,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router::router(state, web_dist);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws?token={TOKEN}"))
        .await
        .expect("ws connect");
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {}
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send new");
    let mut frames = Vec::new();
    let reply = loop {
        let frame = next_frame(&mut ws).await;
        if frame.get("id").and_then(Value::as_u64) == Some(1) {
            break frame;
        }
        frames.push(frame);
    };
    let sid = reply
        .pointer("/result/session_id")
        .and_then(Value::as_str)
        .expect("session id")
        .to_string();
    (ws, frames, sid)
}

async fn send_subscribe_and_capture(ws: &mut Socket, sid: &str, id: u64) -> Vec<Value> {
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/subscribe",
            "params": {"session_id": sid}
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send subscribe");
    let mut frames = Vec::new();
    loop {
        let frame = next_frame(ws).await;
        if frame.get("id").and_then(Value::as_u64) == Some(id) {
            return frames;
        }
        frames.push(frame);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_new_session_snapshot_has_full_server_envelope_and_meter() {
    let project = TempDir::new().expect("project");
    let db_dir = TempDir::new().expect("db dir");
    let db = db_dir.path().join("litecode.db");
    common::seed_test_catalog(&db, common::TEST_CATALOG_ENDPOINT, 128_000);
    common::seed_global_db(&db, &common::default_test_global());
    let (state, web_dist) = test_state(project.path().to_path_buf(), &db);
    let (mut ws, frames, sid) = send_new_and_capture(state, web_dist).await;
    let snapshot = frames
        .iter()
        .find(|frame| {
            frame.get("method").and_then(Value::as_str) == Some("session/snapshot")
                && frame.pointer("/params/session_id").and_then(Value::as_str) == Some(sid.as_str())
        })
        .unwrap_or_else(|| panic!("first snapshot missing; frames={frames:?}"));
    let params = snapshot.get("params").expect("snapshot params");
    let envelope = params.get("context_envelope").expect("envelope");
    let meter = params.get("context_meter").expect("meter");
    let instructions = envelope["instructions_tokens"].as_u64().unwrap_or(0);
    let tools = envelope["tools_tokens"].as_u64().unwrap_or(0);
    let overhead = envelope["prompt_overhead_tokens"].as_u64().unwrap_or(0);
    assert!(
        instructions > 0,
        "first snapshot needs full instructions: {snapshot}"
    );
    assert!(
        tools > 0,
        "first snapshot needs resolved tool schemas: {snapshot}"
    );
    assert!(overhead > 0, "fixed prompt overhead must be server-owned");
    assert_eq!(params["context_window"].as_u64(), Some(128_000));
    assert_eq!(meter["context_window"].as_u64(), Some(128_000));
    assert_eq!(meter["source"].as_str(), Some("local_envelope"));
    assert_eq!(meter["prompt_limit"].as_u64(), Some(128_000 - 8_192));
    assert_eq!(
        meter["used_tokens"].as_u64(),
        Some(instructions + tools + overhead),
        "empty cold request is fixed envelope only"
    );
    assert_eq!(meter["used_tokens"], params["context_tokens_estimate"]);
    assert_eq!(params["compact_eligible"].as_bool(), Some(false));

    // The following subscribe is the second sync point and must publish the
    // same complete envelope/meter, not a cold placeholder.
    let first_envelope = params["context_envelope"].clone();
    let first_meter = params["context_meter"].clone();
    let subscribe_frames = send_subscribe_and_capture(&mut ws, &sid, 2).await;
    let subscribe_snapshot = subscribe_frames
        .iter()
        .find(|frame| {
            frame.get("method").and_then(Value::as_str) == Some("session/snapshot")
                && frame.pointer("/params/session_id").and_then(Value::as_str) == Some(sid.as_str())
        })
        .unwrap_or_else(|| panic!("subscribe snapshot missing; frames={subscribe_frames:?}"));
    assert_eq!(
        subscribe_snapshot["params"]["context_envelope"],
        first_envelope
    );
    assert_eq!(subscribe_snapshot["params"]["context_meter"], first_meter);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_session_without_cold_runtime_omits_context_instead_of_faking_zero() {
    let project = TempDir::new().expect("project");
    let db_dir = TempDir::new().expect("db dir");
    let db = db_dir.path().join("litecode.db");
    common::seed_test_catalog(&db, common::TEST_CATALOG_ENDPOINT, 128_000);
    let mut settings = common::default_test_global();
    // Keep the model reference but remove its credential so the cold runtime
    // cannot resolve a request shape. session/new must still succeed.
    settings.provider_credentials.clear();
    common::seed_global_db(&db, &settings);
    let (state, web_dist) = test_state(project.path().to_path_buf(), &db);
    let (mut ws, frames, sid) = send_new_and_capture(state, web_dist).await;

    let snapshot = frames
        .iter()
        .find(|frame| {
            frame.get("method").and_then(Value::as_str) == Some("session/snapshot")
                && frame.pointer("/params/session_id").and_then(Value::as_str) == Some(sid.as_str())
        })
        .unwrap_or_else(|| panic!("first snapshot missing; frames={frames:?}"));
    let params = snapshot.get("params").expect("snapshot params");
    assert!(params.get("context_meter").is_none(), "{snapshot}");
    assert!(params.get("context_envelope").is_none(), "{snapshot}");
    assert!(params.get("context_window").is_none(), "{snapshot}");
    assert!(
        params.get("context_tokens_estimate").is_none(),
        "{snapshot}"
    );
    assert!(
        params.get("context_token_breakdown").is_none(),
        "{snapshot}"
    );
    assert!(params.get("compact_eligible").is_none(), "{snapshot}");

    // The operation_result nested snapshot must carry the same absent fields.
    let operation = frames
        .iter()
        .find(|frame| frame.get("method").and_then(Value::as_str) == Some("agent/operation_result"))
        .expect("operation result");
    let nested = operation
        .pointer("/params/snapshot")
        .expect("nested snapshot");
    assert!(nested.get("context_meter").is_none(), "{operation}");
    assert!(nested.get("context_envelope").is_none(), "{operation}");

    // A following subscribe has no persisted envelope either. It must keep the
    // same unavailable contract instead of emitting a 0/0 LocalEnvelope.
    let subscribe_frames = send_subscribe_and_capture(&mut ws, &sid, 2).await;
    let subscribe_snapshot = subscribe_frames
        .iter()
        .find(|frame| {
            frame.get("method").and_then(Value::as_str) == Some("session/snapshot")
                && frame.pointer("/params/session_id").and_then(Value::as_str) == Some(sid.as_str())
        })
        .unwrap_or_else(|| panic!("subscribe snapshot missing; frames={subscribe_frames:?}"));
    let subscribe_params = subscribe_snapshot.get("params").expect("subscribe params");
    for field in [
        "context_window",
        "context_tokens_estimate",
        "context_token_breakdown",
        "context_meter",
        "context_envelope",
        "compact_eligible",
    ] {
        assert!(
            subscribe_params.get(field).is_none(),
            "subscribe must omit {field}: {subscribe_snapshot}"
        );
    }
}
