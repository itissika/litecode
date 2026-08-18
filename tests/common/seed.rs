use std::path::{Path, PathBuf};
use std::sync::Arc;

use litecode::config::SettingsWriter;
use litecode::config::TurnGuard;
use litecode::optional::EngineManager;
use tempfile::TempDir;

use litecode::config::ConfigManager;
use litecode::config::global_db::{self, tools};
use litecode::config::schema::{
    ADAPTER_OPENAI_RESPONSES, CustomToolDefinition, GlobalSettings,
    InitScope, McpServerDefinition, ModelAdapterConfig, ModelCapability, ModelDefinition,
    ProviderAuth, ProviderConnectionConfig, ProviderDefinition, ToolCatalogEntry,
    ToolSchema, ToolTier,
};

/// Default provider id for integration test fixtures.
pub const TEST_PROVIDER_ID: &str = "default";

/// Keeps a tempfile-backed global DB alive for the duration of a test.
pub struct TestGlobalDb {
    _dir: TempDir,
    pub path: PathBuf,
}

/// Structurally-ready provider row for tests (`openai_responses` adapter).
pub fn ready_test_provider(id: &str, endpoint: &str, api_key: &str) -> ProviderDefinition {
    ProviderDefinition {
        id: id.into(),
        adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
        label: id.into(),
        config: ProviderConnectionConfig {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            auth: ProviderAuth::Bearer,
        },
    }
}

/// Structurally-ready model row linked to a test provider.
pub fn ready_test_model(
    id: &str,
    provider_ref: &str,
    api_model_id: &str,
    context_window: usize,
) -> ModelDefinition {
    ModelDefinition {
        id: id.into(),
        adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
        provider_ref: provider_ref.into(),
        label: id.into(),
        config: ModelAdapterConfig {
            api_model_id: api_model_id.into(),
            context_window,
            max_tokens: 8192,
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            capabilities: vec![ModelCapability::Text],
        },
    }
}

/// Insert ready provider + default/compaction models and wire agent `model_ref`s.
pub fn insert_test_llm_registry(
    settings: &mut GlobalSettings,
    endpoint: &str,
    api_key: &str,
    context_window: usize,
) {
    settings.providers.insert(
        TEST_PROVIDER_ID.into(),
        ready_test_provider(TEST_PROVIDER_ID, endpoint, api_key),
    );
    settings.models.insert(
        "default".into(),
        ready_test_model(
            "default",
            TEST_PROVIDER_ID,
            "test-primary-model",
            context_window,
        ),
    );
    settings.models.insert(
        "compaction".into(),
        ready_test_model(
            "compaction",
            TEST_PROVIDER_ID,
            "test-compaction-model",
            200_000,
        ),
    );
    if let Some(agent) = settings.agents.get_mut("default") {
        agent.model_ref = "default".into();
    }
    if let Some(agent) = settings.agents.get_mut("compaction") {
        agent.model_ref = "compaction".into();
    }
}

/// Minimal provider definition for `provider_from_definition` in tool-list tests.
pub fn stub_test_provider_def(endpoint: &str, api_key: &str) -> ProviderDefinition {
    ready_test_provider(TEST_PROVIDER_ID, endpoint, api_key)
}

/// Fresh seeded global DB (never touches `~/.local/share/litecode/litecode.db`).
pub fn fresh_test_global_db() -> TestGlobalDb {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("litecode.db");
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

/// Programmatic global settings matching fresh DB seed + test provider credentials.
pub fn default_test_global() -> GlobalSettings {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("litecode.db");
    let mut settings = ConfigManager::load_global_from(&db).expect("seed");
    insert_test_llm_registry(
        &mut settings,
        "https://api.example.com/v1",
        "sk-test",
        128_000,
    );
    settings
}

/// Build global settings with a custom tool catalog entry and default-agent binding.
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
    settings.tool_catalog.insert(
        name.into(),
        ToolCatalogEntry {
            id: name.into(),
            tier: ToolTier::Custom,
            init_scope: InitScope::Global,
            catalog_enabled: false,
        },
    );
    if let Some(agent) = settings.agents.get_mut("default") {
        agent
            .tools
            .insert(name.into(), super::bindings::binding_all_for(name));
    }
    settings
}

/// Build global settings with an MCP server and matching catalog entry.
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
        },
    );
    let catalog_id = tools::mcp_catalog_id(id);
    settings.tool_catalog.insert(
        catalog_id.clone(),
        ToolCatalogEntry {
            id: catalog_id,
            tier: ToolTier::Mcp,
            init_scope: InitScope::Global,
            catalog_enabled: false,
        },
    );
    settings
}

/// Write settings to a fresh global DB (migrate + replace_all).
pub fn seed_global_db(path: &Path, settings: &GlobalSettings) -> PathBuf {
    global_db::import_into(path, settings).expect("seed db");
    path.to_path_buf()
}
