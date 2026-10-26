use std::sync::Arc;

use common::bindings::binding_all_for;
use litecode::config::global_db::{self};
use litecode::config::{
    ConfigManager, SettingsWriter, TurnGuard, WorkspaceState,
};
use litecode::engines::WorkspaceEngines;
use litecode::optional::{EngineManager, EngineWarmupState};
use litecode::runtime::RuntimeHandle;
use litecode::session::manager::SessionManager;
use litecode::tool::registry::build_tool_list;
use tempfile::TempDir;

mod common;

fn test_global_with_provider(dir: &TempDir) -> litecode::config::schema::GlobalSettings {
    let db = dir.path().join("litecode.db");
    common::seed_test_catalog(&db, "https://api.example.com/v1", 128_000);
    let mut global = ConfigManager::load_global_from(&db).expect("seed");
    common::insert_test_llm_registry(&mut global, "sk-test");
    global
}

/// WP-167 (Tools-signed): network core engines are lazy. `reconcile` does not
/// build the HTTP client; the first tool call (`ensure_warm`) does. Stopping
/// still records `Stopped`. Name kept for history; see
/// `optional::tests::reconcile_starts_warmup_for_network_core`.
#[test]
fn network_core_engines_always_warmup() {
    let dir = TempDir::new().expect("dir");
    let global = test_global_with_provider(&dir);
    let workspace = WorkspaceState::new("/tmp/catalog-off");
    let resolved = ConfigManager::resolve(global, workspace, common::default_test_catalog());
    let engines = EngineManager::new();
    engines.reconcile(&resolved);
    assert!(
        !engines.is_warmed("webfetch", &resolved),
        "reconcile must not pre-build webfetch (WP-167 lazy warm)"
    );
    assert_eq!(engines.ensure_warm("webfetch"), Ok(()));
    assert!(engines.is_warmed("webfetch", &resolved));
    engines.stop_all();
    assert!(!engines.is_warmed("webfetch", &resolved));
    assert_eq!(engines.state("webfetch"), Some(EngineWarmupState::Stopped));
}

#[test]
fn settings_reload_reconciles_engine_manager() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut global = test_global_with_provider(&dir);
    global
        .agents
        .get_mut("default")
        .unwrap()
        .tools
        .insert("webfetch".into(), binding_all_for("webfetch"));
    global_db::import_into(&db, &global).expect("import");

    let guard = Arc::new(TurnGuard::new());
    let mut writer = SettingsWriter::with_path(&db, guard);
    let engine_manager = Arc::new(EngineManager::new());
    writer.set_engine_manager(Arc::clone(&engine_manager));
    let writer = Arc::new(writer);
    let revision = writer.revision_handle();

    let workspace = WorkspaceState::new("/tmp/reload-reconcile");
    let resolved = ConfigManager::resolve(
        writer.load_settings().expect("load"),
        workspace.clone(),
        common::catalog_for_db(&db),
    );
    let workspace_engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::open(
        workspace.workspace_root.clone(),
        Arc::clone(&workspace_engines),
    )
    .expect("ide");
    let mut runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        workspace.clone(),
        engine_manager,
        workspace_engines,
        ide,
        revision,
        &db,
    );

    writer
        .write_log(litecode::config::schema::LogSettings { level: None })
        .expect("bump");
    // DocId::ALL includes Engines → RuntimeHandle::apply calls
    // engine_manager.reconcile (WP-167: configure only; no client yet).
    runtime.apply(litecode::config::DocId::ALL).expect("reload");
    assert!(
        !runtime
            .engine_manager
            .is_warmed("webfetch", &runtime.resolved),
        "settings reload reconcile must stay lazy (WP-167)"
    );
    assert_eq!(runtime.engine_manager.ensure_warm("webfetch"), Ok(()));
    assert!(
        runtime
            .engine_manager
            .is_warmed("webfetch", &runtime.resolved),
        "first ensure_warm after reload must build the client"
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tools = rt.block_on(build_tool_list(
        &runtime,
        "default",
        0,
        tokio_util::sync::CancellationToken::new(),
        "test-parent-session",
        Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
        None,
    ))
    .expect("build tool list");
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"webfetch"));
}
