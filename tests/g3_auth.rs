//! G3/G1 serve auth contract:
//! - /api/* accepts Authorization: Bearer only (query token rejected — would
//!   leak into access logs).
//! - /ws keeps the query token as its sole handshake channel.
//! - /ws handshake rejects cross-origin / fabricated Origin.
//! - TraceLayer spans carry method + path only (no query) — structural guarantee
//!   verified by /api query rejection + span construction (see router.rs).

use std::sync::Arc;

use litecode::config::{ConfigManager, TurnGuard, WorkspacePaths, WorkspaceState, init_workspace};
use litecode::engines::WorkspaceEngines;
use litecode::serve::ServeState;
use litecode::serve::router;
use tempfile::TempDir;
use tokio::net::TcpListener;

mod common;

use common::test_serve_settings_with_db;

const TOKEN: &str = "test-token-abc";

fn test_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(0)
        .build()
        .expect("client")
}

fn ensure_test_web_dist(project: &std::path::Path) -> std::path::PathBuf {
    let web_dist = project.join("web/dist");
    std::fs::create_dir_all(&web_dist).expect("web dist dir");
    std::fs::write(web_dist.join("index.html"), "<!DOCTYPE html><html></html>")
        .expect("index.html");
    web_dist
}

fn build_state(project: std::path::PathBuf, global_db_path: std::path::PathBuf) -> ServeState {
    init_workspace(&project).expect("init workspace");
    let workspace_id = litecode::config::peek_workspace_id(&project).expect("workspace identity");
    let workspace = WorkspaceState {
        workspace_root: project.clone(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        paths: WorkspacePaths::for_workspace(&project, &workspace_id),
        workspace_tool_readiness: Default::default(),
    };
    let settings = ConfigManager::load_global_from(&global_db_path).expect("load seeded global");
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
        Some(TOKEN.to_string()),
        project,
        turn_guard,
        settings_writer,
    )
    .expect("serve state")
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

async fn setup() -> (std::net::SocketAddr, TempDir) {
    let ws = TempDir::new().expect("ws");
    let db_dir = TempDir::new().expect("db");
    let db_path = db_dir.path().join("litecode.db");
    let state = build_state(ws.path().to_path_buf(), db_path);
    let web_dist = ensure_test_web_dist(ws.path());
    let addr = spawn_server(state, web_dist).await;
    (addr, ws)
}

#[tokio::test]
async fn api_rejects_query_token_and_accepts_bearer_only() {
    let (addr, _ws) = setup().await;

    // G3: /api/* must NOT accept a query token (it would land in access logs).
    let query_resp = test_http_client()
        .get(format!("http://{addr}/api/settings?token={TOKEN}"))
        .send()
        .await
        .expect("send");
    assert_eq!(query_resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Bearer is the only accepted channel for /api/*.
    let bearer_resp = test_http_client()
        .get(format!("http://{addr}/api/settings"))
        .header("Authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .expect("send");
    assert!(
        bearer_resp.status().is_success(),
        "Bearer must be accepted, got {}",
        bearer_resp.status()
    );

    // No auth at all is rejected.
    let none_resp = test_http_client()
        .get(format!("http://{addr}/api/settings"))
        .send()
        .await
        .expect("send");
    assert_eq!(none_resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ws_handshake_keeps_query_token_channel() {
    let (addr, _ws) = setup().await;

    // /ws?token=... is the sole WS handshake channel (native WebSocket cannot
    // set headers) — must succeed.
    let ws_url = format!("ws://{addr}/ws?token={TOKEN}");
    let (_, resp) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("ws connect");
    assert_eq!(resp.status(), reqwest::StatusCode::SWITCHING_PROTOCOLS);

    // Missing token is rejected.
    let bad = tokio_tungstenite::connect_async(&format!("ws://{addr}/ws"))
        .await
        .map(|(_, r)| r.status());
    assert!(bad.is_err() || bad.unwrap() != reqwest::StatusCode::SWITCHING_PROTOCOLS);
}

#[tokio::test]
async fn ws_handshake_rejects_cross_origin() {
    let (addr, _ws) = setup().await;

    // G1: fabricated/cross-site Origin is rejected even with a valid token.
    let url = format!("ws://{addr}/ws?token={TOKEN}");
    let request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        url.as_str(),
    )
    .expect("request");
    let mut request = request;
    request.headers_mut().insert(
        "Origin",
        "https://evil.example".parse().expect("origin header"),
    );
    let result = tokio_tungstenite::connect_async(request).await;
    assert!(
        result.is_err(),
        "cross-origin WS handshake must be rejected"
    );
}

#[tokio::test]
async fn ws_handshake_allows_localhost_origins_including_ipv6_loopback() {
    let (addr, _ws) = setup().await;

    // G1 positive: localhost origins (IPv4, hostname, bracketed IPv6 loopback)
    // are allowed — IPv6 must NOT be rejected (regression guard).
    for origin in [
        "http://127.0.0.1:5173",
        "http://localhost:5173",
        "http://[::1]:5173",
    ] {
        let url = format!("ws://{addr}/ws?token={TOKEN}");
        let request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                url.as_str(),
            )
            .expect("request");
        let mut request = request;
        request
            .headers_mut()
            .insert("Origin", origin.parse().expect("origin header"));
        let (_, resp) = tokio_tungstenite::connect_async(request)
            .await
            .unwrap_or_else(|e| panic!("origin {origin} must be allowed: {e}"));
        assert_eq!(resp.status(), reqwest::StatusCode::SWITCHING_PROTOCOLS);
    }
}
