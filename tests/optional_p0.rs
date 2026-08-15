use std::sync::Arc;

use common::bindings::binding_all_for;
use litecode::config::global_db::{self, tools};
use litecode::config::schema::{
    AgentProfile, AgentToolBinding, InitScope, ToolCatalogEntry, ToolPreset, ToolReadiness,
    ToolTier,
};
use litecode::config::{
    ConfigManager, SettingsWriter, TurnGuard, WorkspaceState, init_workspace,
    workspace::{read_workspace_engines, workspace_engines_path, workspace_readiness_from_engines},
};
use litecode::engines::WorkspaceEngines;
use litecode::llm::{LlmProvider, provider_from_definition};
use litecode::optional::{EngineManager, EngineWarmupState};
use litecode::runtime::RuntimeHandle;
use litecode::session::manager::SessionManager;
use litecode::tool::catalog::should_include_in_llm_list;
use litecode::tool::registry::build_tool_list;
use tempfile::TempDir;

mod common;

fn test_global_with_provider(dir: &TempDir) -> litecode::config::schema::GlobalSettings {
    let db = dir.path().join("litecode.db");
    let mut global = ConfigManager::load_global_from(&db).expect("seed");
    common::insert_test_llm_registry(
        &mut global,
        "https://api.example.com/v1",
        "sk-test",
        128_000,
    );
    global
}

fn optional_ready_entry(id: &str, scope: InitScope) -> ToolCatalogEntry {
    ToolCatalogEntry {
        id: id.into(),
        tier: ToolTier::Optional,
        init_scope: scope,
        catalog_enabled: true,
    }
}

#[test]
fn seed_contains_four_optional_builtin_entries() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let settings = ConfigManager::load_global_from(&db).expect("seed");
    for id in tools::optional_builtin_ids() {
        let entry = settings
            .tool_catalog
            .get(*id)
            .unwrap_or_else(|| panic!("missing optional catalog entry {id}"));
        assert_eq!(entry.tier, ToolTier::Optional);
        assert!(!entry.catalog_enabled);
    }
}

#[test]
fn gate_requires_engine_warmup_for_optional_builtin() {
    let dir = TempDir::new().expect("dir");
    let mut global = test_global_with_provider(&dir);
    global.tool_catalog.insert(
        "webfetch".into(),
        optional_ready_entry("webfetch", InitScope::Global),
    );
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools: std::collections::HashMap::from([(
                "webfetch".into(),
                binding_all_for("webfetch"),
            )]),
            ..Default::default()
        },
    );
    let mut resolved = ConfigManager::resolve(global, WorkspaceState::new("/tmp/gate"));
    // Init global scope to mark webfetch ready in runtime_catalog_state
    litecode::tool::catalog::init(&mut resolved, InitScope::Global);
    let engines = EngineManager::new();
    let workspace_engines = WorkspaceEngines::new();
    assert!(!should_include_in_llm_list(
        &resolved,
        "default",
        "webfetch",
        &engines,
        &workspace_engines
    ));
    engines.reconcile(&resolved);
    assert!(should_include_in_llm_list(
        &resolved,
        "default",
        "webfetch",
        &engines,
        &workspace_engines
    ));
}

#[test]
fn catalog_disable_stops_optional_engine() {
    let dir = TempDir::new().expect("dir");
    let mut global = test_global_with_provider(&dir);
    global.tool_catalog.insert(
        "webfetch".into(),
        optional_ready_entry("webfetch", InitScope::Global),
    );
    let workspace = WorkspaceState::new("/tmp/catalog-off");
    let mut resolved = ConfigManager::resolve(global, workspace);
    // Init global scope to mark webfetch ready in runtime_catalog_state
    litecode::tool::catalog::init(&mut resolved, InitScope::Global);
    let engines = EngineManager::new();
    engines.reconcile(&resolved);
    assert!(engines.is_warmed("webfetch", &resolved));

    let mut global = resolved.global().clone();
    global
        .tool_catalog
        .get_mut("webfetch")
        .unwrap()
        .catalog_enabled = false;
    let disabled = litecode::config::resolve(global, resolved.workspace().clone());
    engines.reconcile(&disabled);
    assert!(!engines.is_warmed("webfetch", &disabled));
    assert_eq!(engines.state("webfetch"), Some(EngineWarmupState::Stopped));
}

#[test]
fn enable_code_search_engine_writes_engines_json_and_reload_restores_readiness() {
    let dir = TempDir::new().expect("dir");
    let root = dir.path().to_path_buf();
    init_workspace(&root).expect("init");
    let db = dir.path().join("litecode.db");
    let global = test_global_with_provider(&dir);
    global_db::import_into(&db, &global).expect("import");

    litecode::config::workspace::enable_code_search_engine(&root).expect("enable retrieval");

    let engines = read_workspace_engines(&root).expect("read engines");
    assert!(!engines.lsp.desired);
    assert!(engines.lsp.servers.is_empty());
    assert!(engines.retrieval.desired);
    assert!(workspace_engines_path(&root).is_file());
    assert_eq!(
        workspace_readiness_from_engines(&root).get("code_search"),
        Some(&ToolReadiness::Ready)
    );
    assert_eq!(workspace_readiness_from_engines(&root).get("lsp"), None);

    let workspace = WorkspaceState {
        workspace_root: root.clone(),
        workspace_id: litecode::config::peek_workspace_id(&root).expect("workspace identity"),
        contract: String::new(),
        paths: {
            let id = litecode::config::peek_workspace_id(&root).expect("workspace identity");
            litecode::config::WorkspacePaths::for_workspace(&root, &id)
        },
        workspace_tool_readiness: workspace_readiness_from_engines(&root),
    };
    let writer = SettingsWriter::with_path(&db, Arc::new(TurnGuard::new()));
    let resolved = ConfigManager::resolve(writer.load_settings().expect("load"), workspace);
    assert_eq!(resolved.workspace_tool_readiness().get("lsp"), None);
    assert_eq!(
        resolved.workspace_tool_readiness().get("code_search"),
        Some(&ToolReadiness::Ready)
    );
}

#[test]
fn stub_engine_warmup_reports_not_implemented() {
    use litecode::optional::StubEngine;
    use litecode::optional::ToolEngine;
    // R5: a stub engine must not silently report readiness; warmup must fail
    // so the gate cannot present an unimplemented tool as available.
    let stub = StubEngine::new("unknown_builtin");
    let result = stub.warmup();
    assert!(
        result.is_err(),
        "stub engine warmup must return Err (not implemented)"
    );
    assert!(
        result.unwrap_err().to_string().contains("not implemented"),
        "error should explain the engine is a stub"
    );
}

#[test]
fn settings_reload_reconciles_engine_manager() {
    let dir = TempDir::new().expect("dir");
    let db = dir.path().join("litecode.db");
    let mut global = test_global_with_provider(&dir);
    global.tool_catalog.insert(
        "webfetch".into(),
        ToolCatalogEntry {
            catalog_enabled: false,
            ..optional_ready_entry("webfetch", InitScope::Global)
        },
    );
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
    let resolved = ConfigManager::resolve(writer.load_settings().expect("load"), workspace.clone());
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

    let mut catalog = writer.load_settings().expect("load").tool_catalog;
    let mut webfetch = catalog.get("webfetch").cloned().expect("webfetch");
    webfetch.catalog_enabled = true;
    catalog.insert("webfetch".into(), webfetch);
    writer
        .write_tool_catalog(catalog, &workspace)
        .expect("enable catalog");
    // Global readiness is applied on settings reload (same path serve uses after PUT).
    runtime.reload_if_needed().expect("reload");
    assert!(
        runtime
            .engine_manager
            .is_warmed("webfetch", &runtime.resolved)
    );

    let provider = provider_from_definition(&common::stub_test_provider_def(
        "http://127.0.0.1:9",
        "test-key",
    ))
    .expect("provider");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let tools = rt.block_on(build_tool_list(
        &runtime.resolved,
        "default",
        provider.box_clone(),
        "test-key",
        0,
        tokio_util::sync::CancellationToken::new(),
        (*runtime.engine_manager).clone(),
        (*runtime.workspace_engines).clone(),
        Arc::clone(&runtime.ide),
        "test-parent-session",
        Arc::new(SessionManager::new(
            Arc::new(TurnGuard::new()),
            String::new(),
        )),
    ));
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"webfetch"));
}
