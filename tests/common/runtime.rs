//! Test helpers for integration tests (Item / Responses product path).

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use litecode::client_protocol::observer::NoopObserver;
use litecode::config::global_db::tools::{core_configurable_tools, core_none_tools};
use litecode::config::resolved::{WorkspaceState, resolve};
use litecode::config::schema::{AgentProfile, AgentRole, GlobalSettings};
use litecode::config::workspace::set_runtime_paths;
use litecode::config::{AgentConfig, ResolvedConfig, TurnGuard};
use litecode::engines::WorkspaceEngines;
use litecode::llm::LlmProvider;
use litecode::optional::EngineManager;
use litecode::runtime::{AgentRuntime, RuntimeHandle, TurnLlmBinding};
use litecode::session::manager::SessionManager;

use super::bindings::{binding_all_for, binding_none_tool};
use super::permission::test_auto_approve_sink;
use super::seed::{
    TEST_CATALOG_ENDPOINT, TEST_COMPACTION_MODEL_REF, TEST_PRIMARY_MODEL_REF,
    insert_test_llm_registry, test_catalog,
};
use super::workspace_fixture::test_workspace;

thread_local! {
    static TEST_CONTEXT_WINDOW: Cell<usize> = const { Cell::new(128_000) };
}

#[derive(Clone)]
pub struct TestAgentSpec {
    pub agent: AgentConfig,
    pub tools: Vec<String>,
}

pub fn test_agent(tools: Vec<String>, _permission: &str, max_steps: u32) -> TestAgentSpec {
    TestAgentSpec {
        agent: AgentConfig {
            max_steps,
            ..Default::default()
        },
        tools,
    }
}

fn test_context_window() -> usize {
    TEST_CONTEXT_WINDOW.with(|c| c.get())
}

fn seed_models(global: &mut GlobalSettings) {
    insert_test_llm_registry(global, "test-key");
}

/// Build a test `ResolvedConfig` with core catalog + per-tool bindings.
pub fn test_resolved(agent_name: &str, tool_names: &[String]) -> ResolvedConfig {
    test_resolved_with_budget(agent_name, tool_names, test_context_window())
}

pub fn test_resolved_with_budget(
    agent_name: &str,
    tool_names: &[String],
    context_window: usize,
) -> ResolvedConfig {
    let mut global = GlobalSettings::default();
    seed_models(&mut global);

    let all_core = tool_names.is_empty() || tool_names.iter().any(|t| t == "*");
    let mut bindings = HashMap::new();
    if all_core {
        for id in core_configurable_tools() {
            bindings.insert((*id).to_string(), binding_all_for(id));
        }
        for id in core_none_tools() {
            bindings.insert((*id).to_string(), binding_none_tool());
        }
    } else {
        for name in tool_names {
            let binding = if core_configurable_tools().contains(&name.as_str()) {
                binding_all_for(name)
            } else if core_none_tools().contains(&name.as_str()) {
                binding_none_tool()
            } else {
                binding_all_for("custom")
            };
            bindings.insert(name.clone(), binding);
        }
    }

    global.agents.insert(
        agent_name.into(),
        AgentProfile {
            model_ref: TEST_PRIMARY_MODEL_REF.into(),
            tools: bindings,
            ..Default::default()
        },
    );
    global.agents.insert(
        "compaction".into(),
        AgentProfile {
            role: AgentRole::Hidden,
            model_ref: TEST_COMPACTION_MODEL_REF.into(),
            system_prompt: "builtin:compaction".into(),
            ..Default::default()
        },
    );

    resolve(
        global,
        WorkspaceState::new("/tmp/test"),
        test_catalog(TEST_CATALOG_ENDPOINT, context_window, 8192),
    )
}

/// Empty `SessionManager` for tests that only need tool catalog assembly.
pub fn test_sessions_manager(db_path: impl Into<String>) -> Arc<SessionManager> {
    Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.into(),
    ))
}

/// Build a turn binding for a catalog model reference (composite
/// \`{provider_id}/{model_id}\`) against a caller-supplied provider.
pub fn test_turn_binding(
    resolved: &ResolvedConfig,
    provider: Arc<dyn LlmProvider>,
    api_key: &str,
    model_ref: &str,
) -> TurnLlmBinding {
    let model = resolved
        .catalog()
        .model(model_ref)
        .cloned()
        .unwrap_or_else(|| {
            panic!("fixture catalog does not declare model '{model_ref}'");
        });
    TurnLlmBinding {
        provider_id: model.provider_id.clone(),
        model_ref: model.reference.clone(),
        api_model_id: model.id.clone(),
        context_window: model.context_window,
        max_tokens: model.max_output,
        thinking_tier: Default::default(),
        context_mode: Default::default(),
        provider,
        api_key: api_key.to_string(),
        model,
    }
}

/// Build `AgentRuntime` against a live `LlmProvider` (Responses product path).
///
/// `cwd` must already contain an initialized `.litecode/` (see [`test_workspace`]).
/// `max_steps` is applied via the `with_mcp_pool` override (product path).
pub fn build_runtime_with_provider(
    cwd: &Path,
    spec: TestAgentSpec,
    provider: Arc<dyn LlmProvider>,
) -> AgentRuntime {
    let workspace = test_workspace(cwd);
    set_runtime_paths(workspace.paths.clone());

    let tool_names = spec.tools.clone();
    let mut global = {
        let resolved = test_resolved_with_budget("default", &tool_names, test_context_window());
        resolved.global().clone()
    };
    if let Some(p) = global.agents.get_mut("default") {
        p.max_steps = spec.agent.max_steps;
    }
    let resolved = resolve(
        global,
        workspace.clone(),
        test_catalog(TEST_CATALOG_ENDPOINT, test_context_window(), 8192),
    );

    let project = cwd.to_string_lossy().to_string();
    let db_path = workspace.paths.sessions_db.clone();
    let model_ref = resolved
        .agents()
        .get("default")
        .map(|p| p.model_ref.as_str())
        .filter(|s| !s.is_empty());
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.to_string_lossy().to_string(),
    ));
    let session_id = sessions
        .open_session_sync(&project, "default", model_ref)
        .unwrap();

    let model_ref = resolved
        .agents()
        .get("default")
        .map(|p| p.model_ref.as_str())
        .unwrap_or(TEST_PRIMARY_MODEL_REF);
    let binding = test_turn_binding(&resolved, provider, "test-key", model_ref);
    let workspace_engines = WorkspaceEngines::new();
    let ide = litecode::ide_base::IdeBaseHandle::open(cwd, Arc::new(workspace_engines.clone()))
        .expect("ide base");
    let mut runtime = {
        let handle = RuntimeHandle::new(
            resolved,
            "default".into(),
            workspace,
            Arc::new(EngineManager::new()),
            Arc::new(workspace_engines),
            ide,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            db_path,
        );
        AgentRuntime::with_mcp_pool(
            handle,
            session_id,
            sessions,
            binding,
            "default",
            0,
            test_auto_approve_sink(),
            Arc::new(NoopObserver),
            None,
            Some(spec.agent.max_steps),
        )
        .expect("AgentRuntime::with_mcp_pool")
    };
    runtime.set_context_cwd(cwd.to_path_buf());
    runtime
}
