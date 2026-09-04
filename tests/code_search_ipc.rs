//! IPC integration: spawn worker, warmup with hash embedder, search round-trip.

use litecode::engines::code_search::{index_dir, init_workspace_index, read_meta, write_meta};
use litecode::engines::code_search_ipc::CodeSearchWorkerClient;
use litecode::engines::code_search_ipc::protocol::RefreshMode;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

fn force_hash_embedder() {
    unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    }
}

fn spawn_warmed(root: &Path) -> CodeSearchWorkerClient {
    let mut client = CodeSearchWorkerClient::spawn().expect("spawn worker");
    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");
    client
}

fn shutdown(mut client: CodeSearchWorkerClient) {
    client.shutdown().expect("shutdown");
    client.kill();
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn worker_ping_initialize_warmup_search_shutdown() {
    // Worker is a separate process — force hash so CI/dev does not need candle weights.
    unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    }

    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("ipc.rs"), "pub fn ipc_target() {}\n").unwrap();

    let mut client = CodeSearchWorkerClient::spawn().expect("spawn worker");
    assert!(client.pid().is_some(), "worker pid after spawn");

    client.ping().expect("ping");

    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");

    let hits = client.search("ipc_target", None, 5).expect("search");
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|h| h.path.contains("ipc.rs")));

    assert!(index_dir(root).join("vectors.usearch").is_file());

    client.shutdown().expect("shutdown");
    client.kill();

    std::thread::sleep(Duration::from_millis(50));
    assert!(client.try_wait().unwrap().is_some());
}

#[test]
fn warmup_without_session_db_then_inject_session_lane() {
    unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    }

    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("ipc.rs"), "pub fn ipc_target() {}\n").unwrap();

    let mut client = CodeSearchWorkerClient::spawn().expect("spawn worker");
    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");

    let hits = client.search("ipc_target", None, 5).expect("code search");
    assert!(hits.iter().any(|h| h.path.contains("ipc.rs")));

    let err = client
        .session_search("anything", 5, None)
        .expect_err("session lane needs SessionData nod");
    assert!(
        err.to_string().contains("SessionData reader not ready"),
        "got: {err}"
    );

    let litecode = root.join(".litecode");
    let db = litecode.join("sessions.db");
    let lease = litecode::session::WorkspaceWriteLease::acquire(&litecode).unwrap();
    let _data = litecode::session::SessionData::open(&lease, &db).unwrap();
    client.set_session_db(&db).expect("inject");

    let _ = client
        .session_search("anything", 5, None)
        .expect("session lane after inject");
    client
        .search("ipc_target", None, 5)
        .expect("code search still works after session inject");

    client.shutdown().expect("shutdown");
    client.kill();
}

#[test]
fn notify_fs_changes_then_search_hits_new_file() {
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("seed.rs"), "pub fn seed_symbol() {}\n").unwrap();

    let mut client = spawn_warmed(root);
    assert!(
        client
            .search("seed_symbol", None, 5)
            .unwrap()
            .iter()
            .any(|h| h.path.contains("seed.rs"))
    );

    std::fs::write(root.join("later.rs"), "pub fn later_unique_symbol() {}\n").unwrap();
    client
        .notify_fs_changes(&["later.rs".into()], false)
        .expect("notify");
    let hits = client
        .search("later_unique_symbol", None, 5)
        .expect("search after notify");
    assert!(
        hits.iter().any(|h| h.path.contains("later.rs")),
        "silent incremental must index the notified file, got {hits:?}"
    );

    shutdown(client);
}

#[test]
fn refresh_incremental_then_rebuild_when_meta_incompatible() {
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("seed.rs"), "pub fn seed_symbol() {}\n").unwrap();

    let mut client = spawn_warmed(root);

    std::fs::write(root.join("added.rs"), "pub fn added_after_warmup() {}\n").unwrap();
    let incremental = client.refresh().expect("incremental refresh");
    assert_eq!(incremental.mode, RefreshMode::Incremental);
    let hits = client
        .search("added_after_warmup", None, 5)
        .expect("search after incremental");
    assert!(
        hits.iter().any(|h| h.path.contains("added.rs")),
        "incremental refresh must pick up new files, got {hits:?}"
    );

    let mut stale = read_meta(root).unwrap().expect("meta after warmup");
    stale.pipeline_version = 0;
    write_meta(root, &stale).unwrap();
    let rebuilt = client.refresh().expect("rebuild refresh");
    assert_eq!(rebuilt.mode, RefreshMode::Rebuild);
    assert!(
        client
            .search("seed_symbol", None, 5)
            .unwrap()
            .iter()
            .any(|h| h.path.contains("seed.rs"))
    );

    shutdown(client);
}

#[test]
fn protocol_search_hit_roundtrip_serde() {
    use litecode::engines::code_search::SearchHit;

    let hit = SearchHit {
        path: "a.rs".into(),
        start_line: 1,
        end_line: 2,
        summary: "fn main".into(),
        score: 0.42,
    };
    let json = serde_json::to_string(&hit).unwrap();
    let back: SearchHit = serde_json::from_str(&json).unwrap();
    assert_eq!(hit, back);
}

#[test]
fn warmup_and_refresh_follow_disk_excludes() {
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("keep.rs"), "pub fn keep_unique_symbol() {}\n").unwrap();
    std::fs::write(root.join("secret.rs"), "pub fn secret_unique_symbol() {}\n").unwrap();

    let mut lists = litecode::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
    lists.search_exclude.push("secret.rs".into());
    litecode::workspace::filter::persist_workspace_excludes(root, &lists).unwrap();

    let mut client = spawn_warmed(root);
    let keep = client.search("keep_unique_symbol", None, 8).unwrap();
    assert!(
        keep.iter().any(|h| h.path.contains("keep.rs")),
        "keep.rs must stay searchable: {keep:?}"
    );
    let secret = client.search("secret_unique_symbol", None, 8).unwrap();
    assert!(
        secret.iter().all(|h| !h.path.contains("secret.rs")),
        "excluded secret.rs must not be in the corpus: {secret:?}"
    );

    lists.search_exclude.retain(|g| g != "secret.rs");
    litecode::workspace::filter::persist_workspace_excludes(root, &lists).unwrap();
    let incremental = client.refresh().expect("refresh after widening excludes");
    assert_eq!(incremental.mode, RefreshMode::Incremental);
    let secret = client.search("secret_unique_symbol", None, 8).unwrap();
    assert!(
        secret.iter().any(|h| h.path.contains("secret.rs")),
        "widened excludes must admit secret.rs: {secret:?}"
    );

    lists.search_exclude.push("secret.rs".into());
    litecode::workspace::filter::persist_workspace_excludes(root, &lists).unwrap();
    client.refresh().expect("refresh after tightening excludes");
    let secret = client.search("secret_unique_symbol", None, 8).unwrap();
    assert!(
        secret.iter().all(|h| !h.path.contains("secret.rs")),
        "tightened excludes must drop secret.rs: {secret:?}"
    );

    shutdown(client);
}

#[test]
fn refresh_honors_gitignore_switch_from_disk() {
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::write(root.join(".gitignore"), "hidden.rs\n").unwrap();
    std::fs::write(
        root.join("visible.rs"),
        "pub fn visible_unique_symbol() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("hidden.rs"), "pub fn hidden_unique_symbol() {}\n").unwrap();
    litecode::workspace::filter::persist_workspace_excludes(
        root,
        &litecode::workspace::filter::WorkspaceExcludesFile::builtin_defaults(),
    )
    .unwrap();

    let mut client = spawn_warmed(root);
    assert!(
        client
            .search("visible_unique_symbol", None, 8)
            .unwrap()
            .iter()
            .any(|h| h.path.contains("visible.rs"))
    );
    let hidden = client.search("hidden_unique_symbol", None, 8).unwrap();
    assert!(
        hidden.iter().all(|h| !h.path.contains("hidden.rs")),
        "gitignored file must stay out while git_ignore=true: {hidden:?}"
    );

    let mut lists = litecode::workspace::filter::WorkspaceExcludesFile::builtin_defaults();
    lists.git_ignore = false;
    litecode::workspace::filter::persist_workspace_excludes(root, &lists).unwrap();
    client.refresh().expect("refresh after git_ignore=false");
    let hidden = client.search("hidden_unique_symbol", None, 8).unwrap();
    assert!(
        hidden.iter().any(|h| h.path.contains("hidden.rs")),
        "git_ignore=false must admit the gitignored file: {hidden:?}"
    );

    shutdown(client);
}
