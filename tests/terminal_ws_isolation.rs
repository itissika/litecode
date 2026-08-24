//! Real WebSocket terminal ownership and event-routing contract.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use litecode::config::{ConfigManager, TurnGuard, WorkspacePaths, WorkspaceState, init_workspace};
use litecode::engines::WorkspaceEngines;
use litecode::serve::{ServeState, router};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

mod common;

use common::test_serve_settings_with_db;

const TOKEN: &str = "terminal-isolation-token";
type TestSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn build_state(project: std::path::PathBuf, global_db_path: std::path::PathBuf) -> ServeState {
    init_workspace(&project).expect("init workspace");
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
    let settings = ConfigManager::load_global_from(&global_db_path).expect("load global");
    let resolved = ConfigManager::resolve(settings, workspace.clone());
    let turn_guard = Arc::new(TurnGuard::new());
    let (settings_writer, engine_manager) =
        test_serve_settings_with_db(turn_guard.clone(), &global_db_path);
    ServeState::with_project(
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
    .expect("serve state")
}

async fn rpc(ws: &mut TestSocket, id: u64, method: &str, params: Value) -> (Value, Vec<Value>) {
    ws.send(Message::Text(
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
            .to_string()
            .into(),
    ))
    .await
    .expect("send rpc");

    let mut notifications = Vec::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let message = ws.next().await.expect("socket open").expect("ws message");
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("JSON frame");
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return value;
            }
            notifications.push(value);
        }
    })
    .await
    .map(|response| (response, notifications))
    .expect("rpc response timeout")
}

fn is_terminal_event(value: &Value, method: &str, terminal_id: &str) -> bool {
    value.get("method").and_then(Value::as_str) == Some(method)
        && value
            .pointer("/params/id")
            .and_then(Value::as_str)
            .is_some_and(|id| id == terminal_id)
}

async fn wait_for_terminal_event(
    ws: &mut TestSocket,
    queued: &[Value],
    method: &str,
    terminal_id: &str,
) -> Value {
    if let Some(event) = queued
        .iter()
        .find(|value| is_terminal_event(value, method, terminal_id))
    {
        return event.clone();
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = ws.next().await.expect("socket open").expect("ws message");
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("JSON frame");
            if is_terminal_event(&value, method, terminal_id) {
                return value;
            }
        }
    })
    .await
    .expect("terminal event timeout")
}

async fn wait_for_terminal_data(
    ws: &mut TestSocket,
    queued: &[Value],
    terminal_id: &str,
    needle: &str,
) -> Value {
    let matches = |value: &Value| {
        is_terminal_event(value, "terminal/data", terminal_id)
            && value
                .pointer("/params/data")
                .and_then(Value::as_str)
                .is_some_and(|body| body.contains(needle))
    };
    if let Some(event) = queued.iter().find(|value| matches(value)) {
        return event.clone();
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = ws.next().await.expect("socket open").expect("ws message");
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("JSON frame");
            if matches(&value) {
                return value;
            }
        }
    })
    .await
    .expect("terminal data timeout")
}

async fn assert_no_terminal_event(ws: &mut TestSocket, terminal_ids: &[&str]) {
    let result = tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            let message = ws.next().await.expect("socket open").expect("ws message");
            let Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("JSON frame");
            let is_terminal = matches!(
                value.get("method").and_then(Value::as_str),
                Some("terminal/data" | "terminal/exit")
            );
            let id = value.pointer("/params/id").and_then(Value::as_str);
            if is_terminal && id.is_some_and(|id| terminal_ids.contains(&id)) {
                return value;
            }
        }
    })
    .await;
    assert!(result.is_err(), "unexpected terminal event: {result:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_terminals_are_owner_isolated_and_disconnect_cleaned() {
    let project = TempDir::new().expect("project");
    let db_dir = TempDir::new().expect("db");
    let state = build_state(
        project.path().to_path_buf(),
        db_dir.path().join("litecode.db"),
    );
    let hub = state.terminal_hub.clone();
    let web_dist = project.path().join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist");
    std::fs::write(web_dist.join("index.html"), "<!doctype html>").expect("index");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, router::router(state, web_dist))
            .await
            .expect("serve");
    });

    let url = format!("ws://{addr}/ws?token={TOKEN}");
    let (mut a, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect A");
    let (mut b, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connect B");

    let (created, _) = rpc(
        &mut a,
        1,
        "terminal/create",
        json!({ "cols": 80, "rows": 24 }),
    )
    .await;
    let terminal_a = created
        .pointer("/result/id")
        .and_then(Value::as_str)
        .unwrap();

    for (request_id, method, params) in [
        (
            2,
            "terminal/write",
            json!({ "id": terminal_a, "data": "echo BAD\r" }),
        ),
        (
            3,
            "terminal/resize",
            json!({ "id": terminal_a, "cols": 100, "rows": 30 }),
        ),
        (4, "terminal/close", json!({ "id": terminal_a })),
    ] {
        let (response, notifications) = rpc(&mut b, request_id, method, params).await;
        assert_eq!(
            response.pointer("/error/code").and_then(Value::as_i64),
            Some(-32000)
        );
        assert!(
            response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .is_some_and(|message| message.ends_with("not found")),
            "foreign and absent sessions must share the not-found surface: {response}"
        );
        assert!(
            !notifications.iter().any(|value| {
                is_terminal_event(value, "terminal/data", terminal_a)
                    || is_terminal_event(value, "terminal/exit", terminal_a)
            }),
            "B received A's terminal event"
        );
    }

    let (write_response, queued_a) = rpc(
        &mut a,
        5,
        "terminal/write",
        json!({ "id": terminal_a, "data": "echo LITECODE_WS_OWNER_OK\r" }),
    )
    .await;
    assert_eq!(
        write_response.pointer("/result/ok"),
        Some(&Value::Bool(true))
    );
    let _ = wait_for_terminal_data(&mut a, &queued_a, terminal_a, "LITECODE_WS_OWNER_OK").await;
    let (resize_response, _) = rpc(
        &mut a,
        50,
        "terminal/resize",
        json!({ "id": terminal_a, "cols": 120, "rows": 40 }),
    )
    .await;
    assert_eq!(
        resize_response.pointer("/result/ok"),
        Some(&Value::Bool(true))
    );
    assert_no_terminal_event(&mut b, &[terminal_a]).await;

    let background = hub
        .spawn_command(
            "echo LITECODE_AGENT_BG",
            Some(project.path()),
            project.path(),
            "test",
            "",
        )
        .expect("agent background");
    tokio::time::timeout(Duration::from_secs(10), async {
        while hub.session_info(&background.id).is_ok() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("background natural cleanup");
    let background_output =
        std::fs::read_to_string(&background.output_path).expect("background output file");
    assert!(background_output.contains("LITECODE_AGENT_BG"));
    assert_no_terminal_event(&mut a, &[&background.id]).await;
    assert_no_terminal_event(&mut b, &[&background.id]).await;

    let (close_response, queued_close) =
        rpc(&mut a, 6, "terminal/close", json!({ "id": terminal_a })).await;
    assert_eq!(
        close_response.pointer("/result/ok"),
        Some(&Value::Bool(true))
    );
    let _ = wait_for_terminal_event(&mut a, &queued_close, "terminal/exit", terminal_a).await;
    assert_no_terminal_event(&mut a, &[terminal_a]).await;
    assert_no_terminal_event(&mut b, &[terminal_a]).await;

    let (created, _) = rpc(
        &mut a,
        7,
        "terminal/create",
        json!({ "cols": 80, "rows": 24 }),
    )
    .await;
    let natural = created
        .pointer("/result/id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    let (_, queued_natural) = rpc(
        &mut a,
        8,
        "terminal/write",
        json!({ "id": natural, "data": "exit\r" }),
    )
    .await;
    let _ = wait_for_terminal_event(&mut a, &queued_natural, "terminal/exit", &natural).await;
    assert!(
        hub.session_info(&natural).is_err(),
        "natural exit must remove the session"
    );

    let (created, _) = rpc(
        &mut a,
        9,
        "terminal/create",
        json!({ "cols": 80, "rows": 24 }),
    )
    .await;
    let orphan = created
        .pointer("/result/id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    a.close(None).await.expect("disconnect A");
    tokio::time::timeout(Duration::from_secs(10), async {
        while hub.session_info(&orphan).is_ok() {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("disconnect cleanup");

    b.close(None).await.expect("disconnect B");
    server.abort();
}
