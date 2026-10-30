use std::sync::{Arc, Mutex};

use litecode::config::{TurnGuard, WorkspacePaths, WorkspaceState, init_workspace};
use litecode::engines::WorkspaceEngines;
use litecode::serve::ServeState;
use litecode::serve::router;
use litecode::workspace::restart_watcher;
use serde_json::Value;
use tokio::net::TcpListener;

mod common;
use common::{test_agent, test_resolved, test_serve_settings};

static WORKSPACE_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Ephemeral axum test servers can close idle keep-alive sockets; disable pooling
/// so each request uses a fresh connection and avoids hyper IncompleteMessage flakes.
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

fn test_state(
    project: std::path::PathBuf,
) -> (ServeState, common::TestServeFixture, std::path::PathBuf) {
    init_workspace(&project).expect("init workspace");
    let web_dist = ensure_test_web_dist(&project);
    let workspace_id = litecode::config::peek_workspace_id(&project).expect("workspace identity");
    let workspace = WorkspaceState {
        workspace_root: project.clone(),
        workspace_id: workspace_id.clone(),
        contract: String::new(),
        agents_md: String::new(),
        paths: WorkspacePaths::for_workspace(&project, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
        workspace_custom_suites: Default::default(),
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

async fn spawn_test_server(
    state: ServeState,
    web_dist: std::path::PathBuf,
) -> std::net::SocketAddr {
    restart_watcher(&state.watcher, state.workspace.clone())
        .await
        .expect("watcher");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router::router(state, web_dist);
    tokio::spawn(async move {
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
    panic!("test server not ready");
}

#[tokio::test]
async fn workspace_rejects_path_escape() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();

    let resp = client
        .get(format!(
            "http://{addr}/api/workspace/file?path=../secret.txt"
        ))
        .send()
        .await
        .expect("escape");
    assert_eq!(resp.status(), 403);
}

fn git_exe() -> Option<std::path::PathBuf> {
    litecode::config::git_install::find_git_exe()
}

fn init_git_repo(dir: &std::path::Path) {
    let git = git_exe().expect("git");
    let status = std::process::Command::new(&git)
        .args(["-c", "init.defaultBranch=main", "init"])
        .current_dir(dir)
        .status()
        .expect("git init");
    assert!(status.success());
    let _ = std::process::Command::new(&git)
        .args(["symbolic-ref", "HEAD", "refs/heads/main"])
        .current_dir(dir)
        .status();
    for (k, v) in [
        ("user.email", "test@litecode.local"),
        ("user.name", "Litecode Test"),
        ("commit.gpgsign", "false"),
    ] {
        assert!(
            std::process::Command::new(&git)
                .args(["config", k, v])
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }
}

#[tokio::test]
async fn workspace_git_stage_commit_and_rejects_escape() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if git_exe().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/workspace/git");

    let stage: Value = client
        .post(format!("{base}/stage"))
        .json(&serde_json::json!({ "paths": ["a.txt"] }))
        .send()
        .await
        .expect("stage")
        .json()
        .await
        .expect("json");
    assert_eq!(stage["ok"], true);

    let status: Value = client
        .get(format!("{base}/status"))
        .send()
        .await
        .expect("status")
        .json()
        .await
        .expect("json");
    assert_eq!(status["data"]["is_repo"], true);
    assert_eq!(status["data"]["branch"], "main");
    assert_eq!(status["data"]["staged"][0]["path"], "a.txt");

    let commit: Value = client
        .post(format!("{base}/commit"))
        .json(&serde_json::json!({ "message": "add a" }))
        .send()
        .await
        .expect("commit")
        .json()
        .await
        .expect("json");
    assert_eq!(commit["ok"], true);

    let log: Value = client
        .get(format!("{base}/log"))
        .send()
        .await
        .expect("log")
        .json()
        .await
        .expect("json");
    assert_eq!(log["data"]["commits"][0]["subject"], "add a");

    let escape = client
        .post(format!("{base}/stage"))
        .json(&serde_json::json!({ "paths": ["../secret.txt"] }))
        .send()
        .await
        .expect("escape");
    assert_eq!(escape.status(), 403);
}

#[tokio::test]
async fn workspace_git_show_head_index_and_rejects() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if git_exe().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let git = git_exe().expect("git");
    let run = |args: &[&str]| {
        assert!(
            std::process::Command::new(&git)
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    };
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    run(&["add", "a.txt"]);
    run(&["commit", "-m", "add a"]);
    std::fs::write(dir.path().join("bin.dat"), b"a\0b").unwrap();
    run(&["add", "bin.dat"]);
    run(&["commit", "-m", "add bin"]);
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    run(&["add", "a.txt"]);

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/workspace/git/show");

    let head: Value = client
        .get(format!("{base}?path=a.txt&rev=HEAD"))
        .send()
        .await
        .expect("head")
        .json()
        .await
        .expect("json");
    assert_eq!(head["ok"], true);
    assert_eq!(head["data"]["content"], "one\n");

    let index: Value = client
        .get(format!("{base}?path=a.txt&rev=index"))
        .send()
        .await
        .expect("index")
        .json()
        .await
        .expect("json");
    assert_eq!(index["data"]["content"], "two\n");

    let missing: Value = client
        .get(format!("{base}?path=missing.txt&rev=HEAD"))
        .send()
        .await
        .expect("missing")
        .json()
        .await
        .expect("json");
    assert_eq!(missing["ok"], true);
    assert!(missing["data"]["content"].is_null());

    let binary: Value = client
        .get(format!("{base}?path=bin.dat&rev=HEAD"))
        .send()
        .await
        .expect("binary")
        .json()
        .await
        .expect("json");
    assert_eq!(binary["ok"], true);
    assert!(binary["data"]["content"].is_null());

    let rev = client
        .get(format!("{base}?path=a.txt&rev=main"))
        .send()
        .await
        .expect("rev");
    assert_eq!(rev.status(), 400);

    let escape = client
        .get(format!("{base}?path=../secret.txt&rev=HEAD"))
        .send()
        .await
        .expect("escape");
    assert_eq!(escape.status(), 403);
}

#[tokio::test]
async fn workspace_preview_reads() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nul.txt"), b"a\0b").unwrap();
    std::fs::write(dir.path().join("utf16.txt"), [0xFF, 0xFE, b'A', 0x00]).unwrap();
    std::fs::write(dir.path().join("bad.txt"), [0xFF, 0x81]).unwrap();
    std::fs::write(dir.path().join("pic.png"), b"\x89PNG\r\n").unwrap();
    std::fs::write(dir.path().join("notes.db"), b"not a database").unwrap();
    let big = std::fs::File::create(dir.path().join("big.png")).unwrap();
    big.set_len(litecode::workspace::MAX_PREVIEW_BYTES + 1)
        .unwrap();
    drop(big);
    std::fs::create_dir(dir.path().join("sub")).unwrap();

    let db_path = dir.path().join("app.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER, name TEXT);
             INSERT INTO items VALUES (1, 'a');",
        )
        .unwrap();
    }

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/workspace");

    let nul = client
        .get(format!("{base}/file?path=nul.txt"))
        .send()
        .await
        .expect("nul");
    assert_eq!(nul.status(), 415);
    let nul_body: Value = nul.json().await.expect("nul json");
    assert_eq!(nul_body["error"], "Binary file. Can't display it here.");

    let utf16 = client
        .get(format!("{base}/file?path=utf16.txt"))
        .send()
        .await
        .expect("utf16");
    assert_eq!(utf16.status(), 415);
    let utf16_body: Value = utf16.json().await.expect("utf16 json");
    assert_eq!(
        utf16_body["error"],
        "This file is UTF-16. Opening it as UTF-8 here would corrupt it."
    );

    let bad = client
        .get(format!("{base}/file?path=bad.txt"))
        .send()
        .await
        .expect("bad utf8");
    assert_eq!(bad.status(), 415);
    let bad_body: Value = bad.json().await.expect("bad json");
    assert!(
        bad_body["error"].as_str().unwrap_or("").contains("GBK"),
        "{bad_body}"
    );

    let png = client
        .get(format!("{base}/bytes?path=pic.png"))
        .send()
        .await
        .expect("png");
    assert_eq!(png.status(), 200);
    assert_eq!(
        png.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    assert_eq!(&png.bytes().await.unwrap()[..], &b"\x89PNG\r\n"[..]);

    let too_big = client
        .get(format!("{base}/bytes?path=big.png"))
        .send()
        .await
        .expect("big");
    assert_eq!(too_big.status(), 413);
    let too_big_body: Value = too_big.json().await.expect("big json");
    assert_eq!(
        too_big_body["error"],
        "File is too large to preview in the panel."
    );

    let sqlite: Value = client
        .get(format!("{base}/sqlite?path=app.db"))
        .send()
        .await
        .expect("sqlite")
        .json()
        .await
        .expect("sqlite json");
    assert_eq!(sqlite["ok"], true);
    assert_eq!(sqlite["data"]["table"], "items");
    assert_eq!(sqlite["data"]["rows"][0][1], "a");

    let injected = client
        .get(format!(
            "{base}/sqlite?path=app.db&table={}",
            urlencoding_table("items; DROP TABLE items")
        ))
        .send()
        .await
        .expect("injected");
    assert_eq!(injected.status(), 400);

    let not_db = client
        .get(format!("{base}/sqlite?path=notes.db"))
        .send()
        .await
        .expect("not db");
    assert_eq!(not_db.status(), 400);
    // POST /api/workspace/open is intentionally gone; the route-removal
    // contract is covered by workspace_process_contract.
}

fn urlencoding_table(table: &str) -> String {
    url::form_urlencoded::byte_serialize(table.as_bytes()).collect()
}

#[tokio::test]
async fn workspace_citations_resolve_without_file_bodies() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lines.txt"),
        "alpha\nbeta LEAK_LINES_BODY\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("sym.rs"),
        "fn other() {}\nfn unique_citation_symbol() {}\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();
    let outside_name = format!(
        "secret-citation-{}.txt",
        dir.path().file_name().unwrap().to_string_lossy()
    );
    let outside = dir.path().parent().unwrap().join(&outside_name);
    std::fs::write(&outside, "SECRET_CITATION_BODY").unwrap();

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let url = format!("http://{addr}/api/workspace/citations");

    let resp = client
        .post(&url)
        .json(&serde_json::json!({
            "refs": [
                {"path": "lines.txt"},
                {"path": "lines.txt", "line": 2},
                {"path": "lines.txt", "line": 50},
                {"path": "missing.ts"},
                {"path": format!("../{outside_name}")},
                {"path": "subdir"},
                {"path": "sym.rs", "symbol": "unique_citation_symbol"},
                {"path": "sym.rs", "symbol": "not_a_symbol"},
                {"path": "lines.txt", "line": 2, "symbol": "not_a_symbol"}
            ]
        }))
        .send()
        .await
        .expect("citations");
    assert_eq!(resp.status(), 200);
    let raw = resp.text().await.expect("body");
    assert!(!raw.contains("LEAK_LINES_BODY"));
    assert!(!raw.contains("SECRET_CITATION_BODY"));
    assert!(!raw.contains("alpha"));
    let body: Value = serde_json::from_str(&raw).expect("json");
    assert_eq!(body["ok"], true);
    let refs = body["data"]["refs"].as_array().expect("refs");
    assert_eq!(refs.len(), 9);

    assert_eq!(refs[0]["exists"], true);
    assert_eq!(refs[0]["path"], "lines.txt");
    assert!(refs[0]["line"].is_null());

    assert_eq!(refs[1]["exists"], true);
    assert_eq!(refs[1]["line"], 2);

    assert_eq!(refs[2]["exists"], true);
    assert!(refs[2]["line"].is_null());

    assert_eq!(refs[3]["exists"], false);
    assert!(refs[3]["path"].is_null());

    assert_eq!(refs[4]["exists"], false);
    assert!(refs[4]["path"].is_null());

    assert_eq!(refs[5]["exists"], false);

    assert_eq!(refs[6]["exists"], true);
    assert_eq!(refs[6]["line"], 2);

    assert_eq!(refs[7]["exists"], true);
    assert!(refs[7]["line"].is_null());

    assert_eq!(refs[8]["exists"], true);
    assert_eq!(refs[8]["line"], 2);

    let too_many: Vec<_> = (0..65)
        .map(|i| serde_json::json!({"path": format!("f{i}.txt")}))
        .collect();
    let limited = client
        .post(&url)
        .json(&serde_json::json!({ "refs": too_many }))
        .send()
        .await
        .expect("limit");
    assert_eq!(limited.status(), 400);

    let _ = std::fs::remove_file(&outside);
}
