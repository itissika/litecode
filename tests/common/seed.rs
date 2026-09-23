use std::path::{Path, PathBuf};
use std::sync::Arc;

use litecode::config::SettingsWriter;
use litecode::config::TurnGuard;
use litecode::optional::EngineManager;
use tempfile::TempDir;

use litecode::config::ConfigManager;
use litecode::config::global_db;
use litecode::config::schema::{
    CustomToolDefinition, GlobalSettings, McpServerDefinition, ToolSchema,
};
use litecode::provider_catalog::ProviderCatalog;

/// Provider id every integration fixture catalog declares.
pub const TEST_PROVIDER_ID: &str = "test";

/// Composite model reference of the fixture catalog's primary model.
pub const TEST_PRIMARY_MODEL_REF: &str = "test/test-primary-model";

/// Composite model reference of the fixture catalog's compaction model.
pub const TEST_COMPACTION_MODEL_REF: &str = "test/test-compaction-model";

/// Endpoint in-memory fixture catalogs use when no request is expected.
pub const TEST_CATALOG_ENDPOINT: &str = "http://127.0.0.1:9";

/// Keeps a tempfile-backed global DB alive for the duration of a test.
pub struct TestGlobalDb {
    _dir: TempDir,
    pub path: PathBuf,
}

/// The catalog text a test DB seeds with.
///
/// The catalog lives next to the global DB, so every fixture that wants a
/// working LLM binding writes this file beside its temp database.
pub fn test_catalog_toml(endpoint: &str, context_window: usize, max_output: u32) -> String {
    format!(
        r#"version = 1

[[providers]]
id = "{TEST_PROVIDER_ID}"
name = "Test"
endpoint = "{endpoint}"
endpoint_type = "responses"
tiers = {{ low = "low", medium = "medium", high = "high" }}

[[models]]
id = "test-primary-model"
provider_id = "{TEST_PROVIDER_ID}"
context_window = {context_window}
context_window_max = {context_window}
max_output = {max_output}

[[models]]
id = "test-compaction-model"
provider_id = "{TEST_PROVIDER_ID}"
context_window = 200000
context_window_max = 200000
max_output = 8192
"#
    )
}

/// Catalog text with a distinct standard window and Max ceiling (the two
/// catalog facts [ContextMode](litecode::platform_knobs::ContextMode) selects between).
pub fn test_catalog_toml_windows(
    endpoint: &str,
    context_window: usize,
    context_window_max: usize,
    max_output: u32,
) -> String {
    format!(
        r#"version = 1

[[providers]]
id = "{TEST_PROVIDER_ID}"
name = "Test"
endpoint = "{endpoint}"
endpoint_type = "responses"
tiers = {{ low = "low", medium = "medium", high = "high" }}

[[models]]
id = "test-primary-model"
provider_id = "{TEST_PROVIDER_ID}"
context_window = {context_window}
context_window_max = {context_window_max}
max_output = {max_output}

[[models]]
id = "test-compaction-model"
provider_id = "{TEST_PROVIDER_ID}"
context_window = 200000
context_window_max = 200000
max_output = 8192
"#
    )
}

/// In-memory catalog with distinct standard / Max context windows.
pub fn test_catalog_windows(
    endpoint: &str,
    context_window: usize,
    context_window_max: usize,
    max_output: u32,
) -> Arc<ProviderCatalog> {
    Arc::new(
        ProviderCatalog::parse(
            &test_catalog_toml_windows(endpoint, context_window, context_window_max, max_output),
            Path::new("<test-catalog-windows>"),
        )
        .expect("fixture provider catalog must parse"),
    )
}

/// In-memory fixture catalog (no DB file needed) for fixtures that build a
/// `ResolvedConfig` directly.
pub fn test_catalog(endpoint: &str, context_window: usize, max_output: u32) -> Arc<ProviderCatalog> {
    Arc::new(
        ProviderCatalog::parse(
            &test_catalog_toml(endpoint, context_window, max_output),
            Path::new("<test-catalog>"),
        )
        .expect("fixture provider catalog must parse"),
    )
}

/// In-memory fixture catalog at the standard unreachable test endpoint.
pub fn default_test_catalog() -> Arc<ProviderCatalog> {
    test_catalog(TEST_CATALOG_ENDPOINT, 128_000, 8192)
}

/// The process-shared catalog for a DB path the test itself seeded.
pub fn catalog_for_db(db_path: &Path) -> Arc<ProviderCatalog> {
    litecode::provider_catalog::shared_for_db(db_path).expect("provider catalog for test db")
}

/// Write the fixture catalog next to `db_path` (same directory as the DB).
pub fn seed_test_catalog(db_path: &Path, endpoint: &str, context_window: usize) -> PathBuf {
    seed_test_catalog_with(db_path, endpoint, context_window, 8192)
}

pub fn seed_test_catalog_with(
    db_path: &Path,
    endpoint: &str,
    context_window: usize,
    max_output: u32,
) -> PathBuf {
    let path = litecode::provider_catalog::catalog_path_for_db(db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("catalog dir");
    }
    std::fs::write(&path, test_catalog_toml(endpoint, context_window, max_output))
        .expect("write test catalog");
    path
}

/// Credential for the fixture provider plus agent model references into the
/// fixture catalog.
pub fn insert_test_llm_registry(settings: &mut GlobalSettings, api_key: &str) {
    settings
        .provider_credentials
        .insert(TEST_PROVIDER_ID.into(), api_key.into());
    if let Some(agent) = settings.agents.get_mut("default") {
        agent.model_ref = format!("{TEST_PROVIDER_ID}/test-primary-model");
    }
    if let Some(agent) = settings.agents.get_mut("compaction") {
        agent.model_ref = format!("{TEST_PROVIDER_ID}/test-compaction-model");
    }
}

/// Fresh seeded global DB + fixture catalog (never touches the user's data dir).
pub fn fresh_test_global_db() -> TestGlobalDb {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("litecode.db");
    seed_test_catalog(&path, "https://api.example.com/v1", 128_000);
    seed_global_db(&path, &default_test_global());
    TestGlobalDb { _dir: dir, path }
}

/// Serve test wiring with an isolated global DB.
pub struct TestServeFixture {
    pub global_db: TestGlobalDb,
    pub settings_writer: Arc<SettingsWriter>,
    pub engine_manager: Arc<EngineManager>,
}

/// Test serve wiring: isolated global DB + settings writer reconcile hook.
pub fn test_serve_settings(turn_guard: Arc<TurnGuard>) -> TestServeFixture {
    let global_db = fresh_test_global_db();
    let (settings_writer, engine_manager) =
        test_serve_settings_with_db(turn_guard, &global_db.path);
    TestServeFixture {
        global_db,
        settings_writer,
        engine_manager,
    }
}

pub fn test_serve_settings_with_db(
    turn_guard: Arc<TurnGuard>,
    db_path: impl Into<PathBuf>,
) -> (Arc<SettingsWriter>, Arc<EngineManager>) {
    let engine_manager = Arc::new(EngineManager::new());
    let mut writer = SettingsWriter::with_path(db_path, turn_guard);
    writer.set_engine_manager(Arc::clone(&engine_manager));
    (Arc::new(writer), engine_manager)
}

/// Programmatic global settings matching a fresh DB seed plus test credentials.
pub fn default_test_global() -> GlobalSettings {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("litecode.db");
    let mut settings = ConfigManager::load_global_from(&db).expect("seed");
    insert_test_llm_registry(&mut settings, "sk-test");
    settings
}

/// Build global settings with a custom tool and default-agent binding.
#[allow(dead_code)] // kept for removed-suite / future fixtures
pub fn build_global_with_custom_tool(
    name: &str,
    command: &str,
    args: Vec<String>,
    schema: ToolSchema,
) -> GlobalSettings {
    let mut settings = default_test_global();
    let def = CustomToolDefinition {
        name: name.into(),
        description: String::new(),
        schema,
        command: command.into(),
        args,
        timeout: 120,
    };
    settings.custom_tools.push(def);
    if let Some(agent) = settings.agents.get_mut("default") {
        agent
            .tools
            .insert(name.into(), super::bindings::binding_all_for(name));
    }
    settings
}

/// Build global settings with an MCP server.
#[allow(dead_code)] // MCP productization pending; seed helper retained
pub fn build_global_with_mcp_server(id: &str, command: &str, args: Vec<String>) -> GlobalSettings {
    let mut settings = default_test_global();
    settings.mcp_servers.insert(
        id.into(),
        McpServerDefinition {
            command: command.into(),
            args,
            env: Default::default(),
            transport: Default::default(),
            ..Default::default()
        },
    );
    settings
}

/// Write settings to a fresh global DB (migrate + replace_all).
pub fn seed_global_db(path: &Path, settings: &GlobalSettings) -> PathBuf {
    global_db::import_into(path, settings).expect("seed db");
    path.to_path_buf()
}
