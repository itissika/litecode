//! Stage 4 integration tests for code_search (hash embedder; no model download).

use std::sync::Arc;
use std::sync::Once;
use std::time::Duration;

use litecode::config::TurnGuard;
use litecode::config::schema::{
    AgentProfile, AgentToolBinding, InitScope, ToolCatalogEntry, ToolPreset, ToolReadiness,
    ToolTier,
};
use litecode::config::workspace::enable_code_search_engine;
use litecode::config::{ConfigManager, WorkspaceState, init_workspace};
use litecode::engines::code_search::{
    CodeSearchRuntime, EMBEDDER_ID_HASH, HashEmbedder, IndexMeta, PIPELINE_VERSION,
    build_full_index, index_dir, init_workspace_index, needs_rebuild, open_production_embedder,
    production_embedder_id, read_meta, search, warmup_index, write_meta,
};
use litecode::engines::{EngineState, WorkspaceEngines};
use litecode::llm::provider_from_definition;
use litecode::optional::EngineManager;
use litecode::session::manager::SessionManager;
use litecode::tool::catalog::{refresh_workspace_engine_readiness, should_include_in_llm_list};
use litecode::tool::registry::build_tool_list;
use tempfile::TempDir;

mod common;

use common::bindings::binding_all_for;

static HASH_EMBEDDER_ENV: Once = Once::new();

fn ensure_hash_embedder_for_worker() {
    // Worker is a separate process — force hash so CI/dev does not need candle weights.
    HASH_EMBEDDER_ENV.call_once(|| unsafe {
        std::env::set_var("LITECODE_CODE_SEARCH_USE_HASH", "1");
    });
}

fn optional_ready_entry(id: &str, scope: InitScope) -> ToolCatalogEntry {
    ToolCatalogEntry {
        id: id.into(),
        tier: ToolTier::Optional,
        init_scope: scope,
        catalog_enabled: true,
    }
}

fn workspace_with_code_search(
    root: &std::path::Path,
) -> litecode::config::resolved::ResolvedConfig {
    ensure_hash_embedder_for_worker();
    enable_code_search_engine(root).unwrap();
    let mut global = litecode::config::schema::GlobalSettings::default();
    global.tool_catalog.insert(
        "code_search".into(),
        optional_ready_entry("code_search", InitScope::Workspace),
    );
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
    let mut resolved = ConfigManager::resolve(global, WorkspaceState::new(root));
    refresh_workspace_engine_readiness(&mut resolved);
    resolved
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

    let mut emb = HashEmbedder;
    let index = build_full_index(root, &mut emb).unwrap();
    index.save(root).unwrap();

    let runtime = CodeSearchRuntime::new(root.to_path_buf(), index, Some(Box::new(HashEmbedder)));
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
    assert!(index.chunks().len() >= 1);
}

#[tokio::test(flavor = "current_thread")]
async fn catalog_on_warmup_enables_tool_in_list() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    init_workspace(root).unwrap();
    init_workspace_index(root).unwrap();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();

    let resolved = workspace_with_code_search(root);
    let global_engines = EngineManager::new();
    let engines = WorkspaceEngines::new();
    assert!(!should_include_in_llm_list(
        &resolved,
        "default",
        "code_search",
        &global_engines,
        &engines
    ));

    engines.reconcile(&resolved);
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
    refresh_workspace_engine_readiness(&mut resolved);
    engines.reconcile(&resolved);
    assert_eq!(engines.state("code_search"), Some(EngineState::Stopped));
    assert!(!engines.is_warmed("code_search"));
}

#[tokio::test]
async fn catalog_off_does_not_stop_engine() {
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

    let mut global = resolved.global().clone();
    global
        .tool_catalog
        .get_mut("code_search")
        .unwrap()
        .catalog_enabled = false;
    let resolved = litecode::config::resolve(global, resolved.workspace().clone());
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

    let mut emb = HashEmbedder;
    warmup_index(root_a, &mut emb).unwrap();
    let mut emb2 = HashEmbedder;
    warmup_index(root_b, &mut emb2).unwrap();

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

    let mut emb = HashEmbedder;
    let index = build_full_index(root, &mut emb).unwrap();
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

    let mut global = litecode::config::schema::GlobalSettings::default();
    global.tool_catalog.insert(
        "code_search".into(),
        optional_ready_entry("code_search", InitScope::Workspace),
    );
    let resolved = ConfigManager::resolve(global, WorkspaceState::new(root));

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

    let provider = provider_from_definition(&common::stub_test_provider_def(
        "http://localhost:11434/v1",
        "test",
    ))
    .unwrap();
    let tools = build_tool_list(
        &resolved,
        "default",
        provider,
        "test",
        0,
        tokio_util::sync::CancellationToken::new(),
        global_engines,
        engines.clone(),
        litecode::ide_base::IdeBaseHandle::open(root, std::sync::Arc::new(engines.clone()))
            .expect("ide"),
        "test-parent-session",
        Arc::new(SessionManager::new(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        Arc::new(litecode::mcp::McpConnectionPool::new()),
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
    let mut resolved = workspace_with_code_search(root);
    refresh_workspace_engine_readiness(&mut resolved);
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
