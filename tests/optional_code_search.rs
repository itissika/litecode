//! Stage 4 integration tests for code_search (hash embedder; no model download).

use std::sync::Arc;
use std::sync::Once;
use std::time::Duration;

use litecode::config::TurnGuard;
use litecode::config::schema::{AgentProfile, AgentToolBinding, ToolPreset, ToolReadiness};
use litecode::config::workspace::enable_code_search_engine;
use litecode::config::{ConfigManager, WorkspaceState, init_workspace};
use litecode::engines::code_search::{
    CodeSearchRuntime, EMBEDDER_ID_HASH, HashEmbedder, IndexMeta, IndexStatus, PIPELINE_VERSION,
    index_dir, index_workspace_blocking, init_workspace_index, needs_rebuild,
    open_production_embedder, production_embedder_id, read_code_queue, read_meta,
    resolve_index_view, search, warmup_index, write_meta,
};
use litecode::engines::{EngineState, WorkspaceEngines};
use litecode::optional::EngineManager;
use litecode::session::manager::SessionManager;
use litecode::tool::catalog::should_include_in_llm_list;
use litecode::tool::registry::build_tool_list;
use tempfile::TempDir;

mod common;

use common::bindings::binding_all_for;

static HASH_EMBEDDER_ENV: Once = Once::new();

fn ensure_hash_embedder_for_worker() {
    // Worker is a separate process —force hash so CI/dev does not need candle weights.
    HASH_EMBEDDER_ENV.call_once(|| unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    });
}

fn workspace_with_code_search(
    root: &std::path::Path,
) -> litecode::config::resolved::ResolvedConfig {
    ensure_hash_embedder_for_worker();
    enable_code_search_engine(root).unwrap();
    let mut global = litecode::config::schema::GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: std::collections::HashMap::from([(
                "code_search".into(),
                binding_all_for("code_search"),
            )]),
            ..Default::default()
        },
    );
    ConfigManager::resolve(
        global,
        litecode::config::workspace::workspace_with_disk_readiness(&WorkspaceState::new(root)),
        common::default_test_catalog(),
    )
}

#[test]
fn workspace_init_creates_meta_without_vectors() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    init_workspace_index(root).unwrap();

    let meta = read_meta(root).unwrap().expect("meta.json");
    assert_eq!(meta.indexed_chunks, 0);
    assert!(!root.join(".litecode/index/vectors.usearch").exists());
}

#[test]
fn small_repo_index_round_trip_and_query() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(root.join("find.rs"), "pub fn target_fn() {}\n").unwrap();

    let emb = HashEmbedder;
    let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
    index.save(root).unwrap();

    let runtime = CodeSearchRuntime::new(
        root.to_path_buf(),
        index,
        Some(Box::new(HashEmbedder)),
        None,
    );
    let hits = search(&runtime, "target_fn", None, 8).unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|h| h.path.contains("find.rs")));
    assert!(hits[0].start_line >= 1);
    assert!(!hits[0].summary.is_empty());
}

#[test]
fn warmup_creates_vectors_usearch() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();

    let mut emb = HashEmbedder;
    let _index = warmup_index(root, &mut emb).unwrap();
    assert!(root.join(".litecode/index/vectors.usearch").is_file());
}

#[test]
fn warmup_loads_compatible_index_without_full_rebuild() {
    ensure_hash_embedder_for_worker();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();

    let mut emb = HashEmbedder;
    // WP-143: the first build runs on the incremental path, not in warmup.
    let first = index_workspace_blocking(root, Box::new(HashEmbedder)).unwrap();
    let n = first.chunks().len();
    assert!(n >= 1);

    std::fs::write(root.join("later.rs"), "fn later_only() {}\n").unwrap();
    let second = warmup_index(root, &mut emb).unwrap();
    assert_eq!(second.chunks().len(), n);
    assert!(
        !second.indexed_paths().contains("later.rs"),
        "compatible warmup must load, not rebuild around new files"
    );
}

#[test]
fn pipeline_version_change_triggers_rebuild() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();

    let mut stale = IndexMeta::shell();
    stale.pipeline_version = 0;
    write_meta(root, &stale).unwrap();
    assert!(needs_rebuild(&stale));

    std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
    let mut emb = HashEmbedder;
    let index = warmup_index(root, &mut emb).unwrap();
    let meta = read_meta(root).unwrap().unwrap();
    assert_eq!(meta.pipeline_version, PIPELINE_VERSION);
    // WP-143: incompatible ⇒ a fresh, empty, compatible index; the files are
    // then indexed by the incremental path.
    assert!(index.chunks().is_empty());
    assert!(!needs_rebuild(&meta));
    let built = index_workspace_blocking(root, Box::new(HashEmbedder)).unwrap();
    assert!(built.chunks().len() >= 1);
}

#[tokio::test(flavor = "current_thread")]
async fn catalog_lists_tool_while_warming_and_after_warm() {
    // WP-148 (2026-10-14 rule): bind is the sole visibility gate; Warming stays listed.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let global_engines = EngineManager::new();
    let engines = WorkspaceEngines::new();

    // start() sets Warming synchronously; warmup runs on a background thread.
    engines.reconcile(&resolved);
    if engines.state("code_search") == Some(EngineState::Warming) {
        assert!(
            should_include_in_llm_list(
                &resolved,
                "default",
                "code_search",
                &global_engines,
                &engines
            ),
            "code_search must stay listed while Warming"
        );
    }
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    assert!(should_include_in_llm_list(
        &resolved,
        "default",
        "code_search",
        &global_engines,
        &engines
    ));
    assert!(index_dir(root).join("vectors.usearch").is_file());
}

#[tokio::test]
async fn engine_desired_off_stops_engine() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("x.rs"), "fn x() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    assert_eq!(engines.state("code_search"), Some(EngineState::Warm));

    litecode::config::workspace::set_workspace_engine_desired(root, "code_search", false).unwrap();
    let mut resolved = resolved;
    *resolved.workspace_mut() =
        litecode::config::workspace::workspace_with_disk_readiness(resolved.workspace());
    engines.reconcile(&resolved);
    assert_eq!(engines.state("code_search"), Some(EngineState::Stopped));
    assert!(!engines.is_warmed("code_search"));
}

#[tokio::test]
async fn unbind_does_not_stop_engine() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("x.rs"), "fn x() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    assert_eq!(engines.state("code_search"), Some(EngineState::Warm));

    engines.reconcile(&resolved);
    assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
    assert!(engines.is_warmed("code_search"));
}

#[tokio::test]
async fn per_workspace_indexes_are_isolated() {
    // One process = one workspace; indexes are per-root. Two engines on two roots
    // must not share index state (no in-process root switch).
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let root_a = dir_a.path();
    let root_b = dir_b.path();

    std::fs::write(root_a.join("a.rs"), "fn workspace_a() {}\n").unwrap();
    std::fs::write(root_b.join("b.rs"), "fn workspace_b() {}\n").unwrap();

    init_workspace_index(root_a).unwrap();
    init_workspace_index(root_b).unwrap();

    index_workspace_blocking(root_a, Box::new(HashEmbedder)).unwrap();
    index_workspace_blocking(root_b, Box::new(HashEmbedder)).unwrap();

    let meta_a = read_meta(root_a).unwrap().unwrap();
    let meta_b = read_meta(root_b).unwrap().unwrap();
    assert!(meta_a.indexed_chunks >= 1);
    assert!(meta_b.indexed_chunks >= 1);

    let engines_a = WorkspaceEngines::new();
    let resolved_a = workspace_with_code_search(root_a);
    engines_a.reconcile(&resolved_a);
    assert!(
        engines_a
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    let hits_a = engines_a
        .code_search()
        .search("workspace_a", None, 5)
        .unwrap();
    assert!(hits_a.iter().any(|h| h.path.contains("a.rs")));

    let engines_b = WorkspaceEngines::new();
    let resolved_b = workspace_with_code_search(root_b);
    engines_b.reconcile(&resolved_b);
    assert!(
        engines_b
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    let hits_b = engines_b
        .code_search()
        .search("workspace_b", None, 5)
        .unwrap();
    assert!(hits_b.iter().any(|h| h.path.contains("b.rs")));

    engines_a.stop_all();
    engines_b.stop_all();
}

#[test]
fn embedder_id_recorded_in_meta_and_triggers_rebuild() {
    ensure_hash_embedder_for_worker();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("c.rs"), "fn c() {}\n").unwrap();

    let emb = HashEmbedder;
    let index = index_workspace_blocking(root, Box::new(emb)).unwrap();
    index.save(root).unwrap();

    let meta = read_meta(root).unwrap().unwrap();
    assert_eq!(meta.embedder_id, EMBEDDER_ID_HASH);
    assert_eq!(production_embedder_id(), EMBEDDER_ID_HASH);
    assert!(!needs_rebuild(&meta));

    let mut stale = meta.clone();
    stale.embedder_id = "granite97-ort-q8q4".into();
    write_meta(root, &stale).unwrap();
    assert!(needs_rebuild(&stale));
}

#[tokio::test(flavor = "current_thread")]
async fn stop_during_warmup_leaves_no_zombie_worker() {
    ensure_hash_embedder_for_worker();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    init_workspace_index(root).unwrap();
    for i in 0..60 {
        std::fs::write(
            root.join(format!("w_{i}.rs")),
            format!("pub fn w_{i}() {{\n{}\n}}\n", "let _ = 1;\n".repeat(150)),
        )
        .unwrap();
    }

    let resolved = workspace_with_code_search(root);

    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    engines.stop_all();

    tokio::time::sleep(Duration::from_millis(800)).await;

    let engine = engines.code_search();
    assert!(!engine.worker_alive());
    assert_eq!(engines.state("code_search"), Some(EngineState::Stopped));
}

#[test]
fn hash_embedder_used_in_tests_without_model_download() {
    ensure_hash_embedder_for_worker();
    let _emb = open_production_embedder().expect("hash embedder via LITECODE_CODE_SEARCH_USE_HASH");
    assert_eq!(production_embedder_id(), EMBEDDER_ID_HASH);
}

#[tokio::test(flavor = "current_thread")]
async fn build_tool_list_includes_code_search_after_warmup() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("tool.rs"), "fn tool_fn() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let global_engines = EngineManager::new();
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    engines
        .wait_until_warmed("code_search", Duration::from_secs(30))
        .await;

    let ide = litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::new(engines.clone()))
        .expect("ide");
    let runtime = litecode::runtime::RuntimeHandle::new(
        resolved,
        "default".into(),
        litecode::config::WorkspaceState::new(root),
        Arc::new(global_engines),
        Arc::new(engines.clone()),
        ide,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        root.join("global.db"),
    );
    let tools = build_tool_list(
        &runtime,
        "default",
        0,
        tokio_util::sync::CancellationToken::new(),
        "test-parent-session",
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
    )
    .await;
    assert!(tools.iter().any(|t| t.name() == "code_search"));
}

#[tokio::test(flavor = "current_thread")]
async fn engines_json_enables_code_search_for_workspace() {
    // Enabling retrieval via engines.json is a per-workspace boot concern,
    // not an in-process root switch.
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    init_workspace(root).unwrap();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

    enable_code_search_engine(root).unwrap();
    let resolved = workspace_with_code_search(root);
    assert!(read_meta(root).unwrap().is_some());
    assert_eq!(
        resolved.workspace_tool_readiness().get("code_search"),
        Some(&ToolReadiness::Ready)
    );

    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );
    assert!(index_dir(root).join("vectors.usearch").is_file());
    engines.stop_all();
}

#[tokio::test(flavor = "current_thread")]
async fn worker_crash_demotes_engine() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("crash.rs"), "fn crash_target() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );

    let engine = engines.code_search();
    let pid = engine.worker_pid().expect("worker pid");
    #[cfg(windows)]
    let status = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .status();
    #[cfg(not(windows))]
    let status = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
    assert!(status.expect("terminate worker").success());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while engine.worker_alive() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let result = engine.search("crash_target", None, 5);
    assert!(result.is_err());
    assert!(!engine.worker_alive());
    assert!(!engines.is_warmed("code_search"));
    assert_eq!(engines.state("code_search"), Some(EngineState::Idle));
}

#[tokio::test(flavor = "current_thread")]
async fn code_search_lifecycle_rebuild_stale_consume_stop() {
    ensure_hash_embedder_for_worker();
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("seed.rs"), "pub fn seed_symbol() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await,
        "first desired with no index must auto-rebuild"
    );
    assert!(index_dir(root).join("vectors.usearch").is_file());
    // WP-143: the first build is the incremental path, run by the code tick.
    let build_deadline = std::time::Instant::now() + Duration::from_secs(30);
    while resolve_index_view(root, engines.state("code_search")).status != IndexStatus::Ready {
        assert!(
            std::time::Instant::now() < build_deadline,
            "first build did not finish via the code tick"
        );
        let _ = engines.code_index_tick();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let ready = resolve_index_view(root, engines.state("code_search"));
    assert_eq!(ready.status, IndexStatus::Ready);

    std::fs::write(root.join("later.rs"), "pub fn later_unique_symbol() {}\n").unwrap();
    engines
        .code_search()
        .notify_fs_changes(&["later.rs".into()], false);
    let hint_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while read_code_queue(root).queued == 0 && std::time::Instant::now() < hint_deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        read_code_queue(root).queued > 0,
        "watcher must mark the index stale without consuming"
    );
    let stale = resolve_index_view(root, engines.state("code_search"));
    assert_eq!(stale.status, IndexStatus::Stale);
    let queued = engines
        .code_search()
        .search("later_unique_symbol", None, 5)
        .unwrap();
    assert!(
        queued.iter().all(|h| !h.path.contains("later.rs")),
        "stale worker must not digest later.rs on search, got {queued:?}"
    );

    engines.request_refresh(&resolved).expect("consume");
    let consume_deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if read_code_queue(root).queued == 0
            && engines
                .code_search()
                .search("later_unique_symbol", None, 5)
                .unwrap()
                .iter()
                .any(|h| h.path.contains("later.rs"))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < consume_deadline,
            "refresh/consume did not index later.rs in time"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    litecode::config::workspace::set_workspace_engine_desired(root, "code_search", false).unwrap();
    let mut resolved = resolved;
    *resolved.workspace_mut() =
        litecode::config::workspace::workspace_with_disk_readiness(resolved.workspace());
    engines.reconcile(&resolved);
    assert_eq!(engines.state("code_search"), Some(EngineState::Stopped));
    assert!(!engines.code_search().worker_alive());
}

/// WP-079 acceptance 1: after warm, a written file is indexed by the silent
/// code tick alone — no refresh call. Interval is shortened by driving
/// `code_index_tick()` every 100 ms instead of the 30 s serve loop.
#[tokio::test]
async fn code_index_tick_syncs_without_refresh() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(root.join("a.rs"), "pub fn alpha() {}\n").unwrap();
    let resolved = workspace_with_code_search(root);
    let engines = WorkspaceEngines::new();
    engines.reconcile(&resolved);
    assert!(
        engines
            .wait_until_warmed("code_search", Duration::from_secs(30))
            .await
    );

    std::fs::write(
        root.join("b.rs"),
        "pub fn zebra_tick_unique_token() -> u32 { 42 }\n",
    )
    .unwrap();
    // Same call the serve watcher makes (router.rs); no refresh/consume call.
    engines
        .code_search()
        .notify_fs_changes(&["b.rs".to_string()], false);
    assert!(
        read_code_queue(root).queued > 0,
        "change must be pending first"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut started = false;
    loop {
        if engines.code_index_tick() == litecode::engines::CodeIndexTick::Started {
            started = true;
        }
        let hit = engines
            .code_search()
            .search("zebra_tick_unique_token", None, 5)
            .map(|h| h.iter().any(|h| h.path.contains("b.rs")))
            .unwrap_or(false);
        if read_code_queue(root).queued == 0 && hit && !engines.is_refresh_busy() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "tick did not sync: pending={} started={started}",
            read_code_queue(root).queued
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(started, "the tick itself must have started the refresh");
    assert_eq!(engines.state("code_search"), Some(EngineState::Warm));
    engines.stop_all();
}
