//! IPC integration: spawn worker, warmup with hash embedder, search round-trip.

use litecode::engines::code_search::{
    index_dir, init_workspace_index, read_code_queue, read_meta, write_meta,
};
use litecode::engines::code_search_ipc::CodeSearchWorkerClient;
use litecode::engines::code_search_ipc::protocol::{RefreshMode, RefreshScope};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn force_hash_embedder() {
    unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    }
}

fn spawn_warmed(root: &Path) -> CodeSearchWorkerClient {
    let client = CodeSearchWorkerClient::spawn().expect("spawn worker");
    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");
    first_build(&client);
    client
}

/// WP-143: warmup only loads (or starts an empty index); the first build runs
/// on the incremental path, as the serve tick's code refresh does.
fn first_build(client: &CodeSearchWorkerClient) {
    client
        .refresh_scope(RefreshScope::Code)
        .expect("first-build refresh");
}

fn shutdown(client: CodeSearchWorkerClient) {
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

    let client = CodeSearchWorkerClient::spawn().expect("spawn worker");
    assert!(client.pid().is_some(), "worker pid after spawn");

    client.ping().expect("ping");

    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");
    first_build(&client);

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

    let client = spawn_warmed(root);
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

    let client = spawn_warmed(root);

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

    let client = spawn_warmed(root);
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

#[test]
fn notify_ping_and_search_not_blocked_by_inflight_refresh() {
    force_hash_embedder();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("seed.rs"), "pub fn seed_preempt_target() {}\n").unwrap();

    // Spawn worker with per-batch embed delay so a multi-batch refresh runs for >1s.
    unsafe {
        std::env::set_var("LITECODE_TEST_HASH_EMBED_DELAY_MS", "120");
    }
    let client = Arc::new(CodeSearchWorkerClient::spawn().expect("spawn worker"));
    unsafe {
        std::env::remove_var("LITECODE_TEST_HASH_EMBED_DELAY_MS");
    }
    client.initialize(root, None).expect("initialize");
    client.warmup().expect("warmup");
    client
        .refresh_scope(RefreshScope::Code)
        .expect("initial seed refresh");

    // Write enough dirty files (> 8 batches of EMBED_BATCH=32) so refresh takes >1s.
    for i in 0..320 {
        std::fs::write(
            root.join(format!("bulk_{i:04}.rs")),
            format!("pub fn bulk_fn_{i}() {{ let x = {i}; }}\n"),
        )
        .unwrap();
    }

    let refresh_client = Arc::clone(&client);
    let refresh_started = Instant::now();
    let refresh_handle =
        std::thread::spawn(move || refresh_client.refresh_scope(RefreshScope::Code));

    // Give the maintenance thread a moment to enter batch embedding.
    std::thread::sleep(Duration::from_millis(80));

    // 1. One-way FS notification must return immediately (zero HOL wait).
    let notify_start = Instant::now();
    client
        .notify_fs_changes_async(&["extra.rs".into()], false)
        .expect("async notify");
    assert!(
        notify_start.elapsed() < Duration::from_millis(50),
        "async notify blocked: {:?}",
        notify_start.elapsed()
    );

    // 2. Control ping must reply immediately while refresh is running.
    let ping_start = Instant::now();
    let ping = client.ping().expect("ping during refresh");
    assert!(ping.ready);
    assert!(
        ping_start.elapsed() < Duration::from_millis(250),
        "ping head-of-line blocked by refresh: {:?}",
        ping_start.elapsed()
    );

    // 3. Interactive search must preempt between micro-batches and finish well
    // before the >1s refresh completes.
    let search_start = Instant::now();
    let hits = client
        .search("seed_preempt_target", None, 5)
        .expect("search during refresh");
    let search_elapsed = search_start.elapsed();
    assert!(
        hits.iter().any(|h| h.path.contains("seed.rs")),
        "expected seed.rs hit during refresh, got {hits:?}"
    );
    assert!(
        search_elapsed < Duration::from_millis(750),
        "search starved behind refresh ({search_elapsed:?})"
    );
    assert!(
        !refresh_handle.is_finished(),
        "refresh finished too early ({:?}); test did not overlap refresh",
        refresh_started.elapsed()
    );

    let refreshed = refresh_handle
        .join()
        .expect("refresh thread")
        .expect("refresh ok");
    assert_eq!(refreshed.mode, RefreshMode::Incremental);

    client.shutdown().expect("shutdown");
    client.kill();
}
