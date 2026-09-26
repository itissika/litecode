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
        paths: WorkspacePaths::for_workspace(&project, &workspace_id),
        workspace_tool_readiness: Default::default(),
        workspace_mcp_servers: Default::default(),
        workspace_custom_tools: Default::default(),
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
async fn workspace_tree_lists_files() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();

    let resp: Value = client
        .get(format!("http://{addr}/api/workspace/tree"))
        .send()
        .await
        .expect("tree")
        .json()
        .await
        .expect("json");

    assert_eq!(resp["ok"], true);
    let entries = resp["data"]["entries"].as_array().expect("entries");
    let names: Vec<_> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"a.txt"));
    assert!(names.contains(&"sub"));
}

#[tokio::test]
async fn workspace_tree_reveal_returns_by_dir_ancestors() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/a")).unwrap();
    std::fs::write(dir.path().join("src/a/b.ts"), "").unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();

    let resp: Value = client
        .get(format!(
            "http://{addr}/api/workspace/tree?path=src/a/b.ts&reveal=1"
        ))
        .send()
        .await
        .expect("reveal")
        .json()
        .await
        .expect("json");

    assert_eq!(resp["ok"], true);
    assert!(resp["data"]["entries"].is_null());
    let by_dir = resp["data"]["by_dir"].as_object().expect("by_dir");
    assert!(by_dir.contains_key(""));
    assert!(by_dir.contains_key("src"));
    assert!(by_dir.contains_key("src/a"));
    assert!(!by_dir.contains_key("src/a/b.ts"));
    let src_a: Vec<_> = by_dir["src/a"]
        .as_array()
        .expect("src/a entries")
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(src_a, vec!["src/a/b.ts"]);
}

#[tokio::test]
async fn workspace_glob_finds_nested_filename() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/FileTree.tsx"), "").unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();

    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();

    let resp: Value = client
        .get(format!("http://{addr}/api/workspace/glob?pattern=FileTree"))
        .send()
        .await
        .expect("glob")
        .json()
        .await
        .expect("json");

    assert_eq!(resp["ok"], true);
    assert_eq!(resp["data"]["truncated"], false);
    let entries = resp["data"]["entries"].as_array().expect("entries");
    let paths: Vec<_> = entries
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/FileTree.tsx"]);
}

#[tokio::test]
async fn workspace_file_crud() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/workspace/file");

    let create = client
        .post(&base)
        .json(&serde_json::json!({ "path": "new.txt", "content": "v1" }))
        .send()
        .await
        .expect("create")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(create["ok"], true);
    assert_eq!(create["data"]["path"], "new.txt");

    let dup = client
        .post(&base)
        .json(&serde_json::json!({ "path": "new.txt", "content": "v2" }))
        .send()
        .await
        .expect("dup");
    assert_eq!(dup.status(), 409);

    let read: Value = client
        .get(format!("{base}?path=new.txt"))
        .send()
        .await
        .expect("read")
        .json()
        .await
        .expect("json");
    assert_eq!(read["data"]["content"], "v1");

    let update: Value = client
        .put(format!("{base}?path=new.txt"))
        .json(&serde_json::json!({ "content": "v2" }))
        .send()
        .await
        .expect("put")
        .json()
        .await
        .expect("json");
    assert_eq!(update["ok"], true);

    let read2: Value = client
        .get(format!("{base}?path=new.txt"))
        .send()
        .await
        .expect("read2")
        .json()
        .await
        .expect("json");
    assert_eq!(read2["data"]["content"], "v2");

    let del: Value = client
        .delete(format!("{base}?path=new.txt"))
        .send()
        .await
        .expect("delete")
        .json()
        .await
        .expect("json");
    assert_eq!(del["ok"], true);

    let missing = client
        .get(format!("{base}?path=new.txt"))
        .send()
        .await
        .expect("missing");
    assert_eq!(missing.status(), 404);
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

#[tokio::test]
async fn workspace_mkdir_rename_copy_blob() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let base = format!("http://{addr}/api/workspace");

    let mkdir: Value = client
        .post(format!("{base}/mkdir"))
        .json(&serde_json::json!({ "path": "sub" }))
        .send()
        .await
        .expect("mkdir")
        .json()
        .await
        .expect("json");
    assert_eq!(mkdir["ok"], true);
    assert!(dir.path().join("sub").is_dir());

    let dup = client
        .post(format!("{base}/mkdir"))
        .json(&serde_json::json!({ "path": "sub" }))
        .send()
        .await
        .expect("dup mkdir");
    assert_eq!(dup.status(), 409);

    let renamed: Value = client
        .post(format!("{base}/rename"))
        .json(&serde_json::json!({ "from": "a.txt", "to": "b.txt" }))
        .send()
        .await
        .expect("rename")
        .json()
        .await
        .expect("json");
    assert_eq!(renamed["data"]["from"], "a.txt");
    assert_eq!(renamed["data"]["to"], "b.txt");
    assert!(dir.path().join("b.txt").exists());
    assert!(!dir.path().join("a.txt").exists());

    let copied: Value = client
        .post(format!("{base}/copy"))
        .json(&serde_json::json!({ "from": "b.txt", "to": "sub/c.txt" }))
        .send()
        .await
        .expect("copy")
        .json()
        .await
        .expect("json");
    assert_eq!(copied["ok"], true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sub/c.txt")).unwrap(),
        "hello"
    );

    let self_copy = client
        .post(format!("{base}/copy"))
        .json(&serde_json::json!({ "from": "sub", "to": "sub/nested" }))
        .send()
        .await
        .expect("self copy");
    assert_eq!(self_copy.status(), 400);

    let escape = client
        .post(format!("{base}/rename"))
        .json(&serde_json::json!({ "from": "b.txt", "to": "../out.txt" }))
        .send()
        .await
        .expect("escape");
    assert_eq!(escape.status(), 403);

    let blob = client
        .post(format!("{base}/blob?path=pic.bin"))
        .header("content-type", "application/octet-stream")
        .body(vec![0u8, 1, 255])
        .send()
        .await
        .expect("blob");
    assert_eq!(blob.status(), 201);
    assert_eq!(
        std::fs::read(dir.path().join("pic.bin")).unwrap(),
        vec![0, 1, 255]
    );

    let blob_dup = client
        .post(format!("{base}/blob?path=pic.bin"))
        .header("content-type", "application/octet-stream")
        .body(vec![2u8])
        .send()
        .await
        .expect("blob dup");
    assert_eq!(blob_dup.status(), 409);

    std::fs::create_dir(dir.path().join("keep")).unwrap();
    let overwrite_dir = client
        .post(format!("{base}/rename"))
        .json(&serde_json::json!({
            "from": "b.txt",
            "to": "keep",
            "overwrite": true
        }))
        .send()
        .await
        .expect("overwrite dir");
    assert_eq!(overwrite_dir.status(), 400);
    assert!(dir.path().join("keep").is_dir());
    assert!(dir.path().join("b.txt").exists());
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
async fn workspace_git_status_empty_when_not_a_repo() {
    let _guard = WORKSPACE_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if git_exe().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (state, _serve, web_dist) = test_state(dir.path().to_path_buf());
    let addr = spawn_test_server(state, web_dist).await;
    let client = test_http_client();
    let resp: Value = client
        .get(format!("http://{addr}/api/workspace/git/status"))
        .send()
        .await
        .expect("status")
        .json()
        .await
        .expect("json");
    assert_eq!(resp["ok"], true);
    assert_eq!(resp["data"]["is_repo"], false);
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
async fn workspace_preview_reads_and_open_guards() {
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
    assert_eq!(nul_body["error"], "二进制，无法在这里显示");

    let utf16 = client
        .get(format!("{base}/file?path=utf16.txt"))
        .send()
        .await
        .expect("utf16");
    assert_eq!(utf16.status(), 415);
    let utf16_body: Value = utf16.json().await.expect("utf16 json");
    assert_eq!(utf16_body["error"], "这是 UTF-16，这里按 UTF-8 打开会损坏");

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
    assert_eq!(too_big_body["error"], "文件太大，无法在面板里预览");

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

    let missing = client
        .post(format!("{base}/open"))
        .json(&serde_json::json!({ "path": "nope.txt" }))
        .send()
        .await
        .expect("missing open");
    assert_eq!(missing.status(), 404);

    let directory = client
        .post(format!("{base}/open"))
        .json(&serde_json::json!({ "path": "sub" }))
        .send()
        .await
        .expect("dir open");
    assert_eq!(directory.status(), 400);

    let escape = client
        .post(format!("{base}/open"))
        .json(&serde_json::json!({ "path": "../secret.txt" }))
        .send()
        .await
        .expect("escape open");
    assert_eq!(escape.status(), 403);

    #[cfg(not(windows))]
    {
        let unsupported = client
            .post(format!("{base}/open"))
            .json(&serde_json::json!({ "path": "pic.png" }))
            .send()
            .await
            .expect("open");
        assert_eq!(unsupported.status(), 501);
    }
}

fn urlencoding_table(table: &str) -> String {
    url::form_urlencoded::byte_serialize(table.as_bytes()).collect()
}
