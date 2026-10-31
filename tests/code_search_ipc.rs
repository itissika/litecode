//! IPC integration: spawn worker, warmup with hash embedder, search round-trip.

use litecode::engines::code_search::{
    index_dir, init_workspace_index, read_code_queue, read_meta, write_meta,
};
use litecode::engines::code_search_ipc::CodeSearchWorkerClient;
use litecode::engines::code_search_ipc::protocol::{RefreshMode, RefreshScope};
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
    first_build(&mut client);
    client
}

/// WP-143: warmup only loads (or starts an empty index); the first build runs
/// on the incremental path, as the serve tick's code refresh does.
fn first_build(client: &mut CodeSearchWorkerClient) {
    client
        .refresh_scope(RefreshScope::Code)
        .expect("first-build refresh");
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
    first_build(&mut client);

    let ping = client.ping().expect("ping after warmup");
    assert!(ping.ready);
    assert_eq!(ping.embed_device, "hash");

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
    assert!(
        read_code_queue(root).queued > 0,
        "watcher must queue dirty without consuming"
    );
    let queued = client
        .search("later_unique_symbol", None, 5)
        .expect("search after notify without consume");
    assert!(
        queued.iter().all(|h| !h.path.contains("later.rs")),
        "notify must not embed; search must miss later.rs, got {queued:?}"
    );
    let incremental = client.refresh().expect("consume after notify");
    assert_eq!(incremental.mode, RefreshMode::Incremental);
    let hits = client
        .search("later_unique_symbol", None, 5)
        .expect("search after notify + refresh");
    assert!(
        hits.iter().any(|h| h.path.contains("later.rs")),
        "notify only queues; refresh must index the file, got {hits:?}"
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
fn refresh_stays_incremental_when_all_files_dirty() {
    // 10-12 locked rule (WP-057/060; ISSUE-052): DirtyTooLarge is gone — however
    // many files are dirty, refresh is incremental; rebuild only on first run or
    // an incompatible index.
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("a.rs"), "pub fn a_symbol() {}\n").unwrap();
    std::fs::write(root.join("b.rs"), "pub fn b_symbol() {}\n").unwrap();

    let mut client = spawn_warmed(root);
    std::fs::write(root.join("a.rs"), "pub fn a_symbol_v2() {}\n").unwrap();
    std::fs::write(root.join("b.rs"), "pub fn b_symbol_v2() {}\n").unwrap();
    let refreshed = client.refresh().expect("all-dirty refresh");
    assert_eq!(refreshed.mode, RefreshMode::Incremental);
    for (sym, file) in [("a_symbol_v2", "a.rs"), ("b_symbol_v2", "b.rs")] {
        assert!(
            client
                .search(sym, None, 5)
                .unwrap()
                .iter()
                .any(|h| h.path.contains(file)),
            "incremental refresh must index {file}"
        );
    }
    shutdown(client);
}
