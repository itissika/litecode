//! IPC integration: spawn worker, warmup with hash embedder, search round-trip.

use litecode::engines::code_search::{index_dir, init_workspace_index};
use litecode::engines::code_search_ipc::CodeSearchWorkerClient;
use std::time::Duration;
use tempfile::TempDir;

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

    client.ping().expect("ping");

    client.initialize(root).expect("initialize");
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
