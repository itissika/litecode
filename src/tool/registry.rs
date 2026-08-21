use std::collections::HashMap;
use std::sync::Arc;

use crate::config::global_db::tools::mcp_catalog_id;
use crate::config::resolved::ResolvedConfig;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::llm::LlmProvider;
use crate::mcp::{McpConnectionPool, McpToolSchema};
use crate::optional::EngineManager;
use crate::session::manager::SessionManager;
use crate::tool::catalog::should_include_in_llm_list;
use crate::tool::trait_::Tool;
use crate::tools::{
    bash::BashTool, code_search::CodeSearchTool, custom::CustomTool, edit::EditTool,
    glob::GlobTool, grep::GrepTool, kill_shell::KillShellTool, lsp::LspTool, mcp_tool::McpTool,
    plan::PlanTool, read::ReadTool, session_search::SessionSearchTool,
    subagent::SubagentLaunchTool, todo::TodoWriteTool, wait_shell::WaitShellTool,
    webfetch::WebFetchTool, websearch::WebSearchTool, write::WriteTool,
};
fn builtin_tool(
    id: &str,
    sessions: Arc<SessionManager>,
    _workspace_engines: &WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
) -> Option<Arc<dyn Tool>> {
    let tool: Arc<dyn Tool> = match id {
        "bash" => Arc::new(BashTool::new(Arc::clone(&ide.terminal))),
        "kill_shell" => Arc::new(KillShellTool::new(Arc::clone(&ide.terminal))),
        "wait_shell" => Arc::new(WaitShellTool::new(Arc::clone(&ide.terminal))),
        "read" => Arc::new(ReadTool::with_ide(Arc::clone(&ide))),
        "write" => Arc::new(WriteTool::with_ide(Arc::clone(&ide))),
        "edit" => Arc::new(EditTool::with_ide(Arc::clone(&ide))),
        "grep" => Arc::new(GrepTool),
        "glob" => Arc::new(GlobTool),
        "todo" => Arc::new(TodoWriteTool::new(Arc::clone(&sessions))),
        "plan" => Arc::new(PlanTool::new(Arc::clone(&sessions))),
        _ => return None,
    };
    Some(tool)
}

fn instantiate_tool(
    resolved: &ResolvedConfig,
    agent_id: &str,
    tool_id: &str,
    engines: &EngineManager,
    workspace_engines: &WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
    sessions: &Arc<SessionManager>,
    mcp_schemas: &HashMap<String, Vec<McpToolSchema>>,
    mcp_pool: Arc<crate::mcp::McpConnectionPool>,
) -> Vec<Arc<dyn Tool>> {
    if let Some(tool) = builtin_tool(tool_id, Arc::clone(sessions), workspace_engines, ide) {
        return vec![tool];
    }

    if let Some(custom) = resolved.custom_tools().iter().find(|ct| ct.name == tool_id) {
        return vec![Arc::new(CustomTool::new(custom.clone()))];
    }

    if let Some(server_id) = tool_id.strip_prefix("mcp_")
        && let Some(mcp) = resolved.mcp_servers().get(server_id)
    {
        // Handshake must have succeeded this turn (`mcp_schemas` is filled only
        // after start). Do not advertise a dummy catalog-id tool when the
        // process is down — the model would see it and try to call it.
        let Some(tool_schemas) = mcp_schemas.get(server_id) else {
            return vec![];
        };
        let allowed_tools = resolved
            .agents()
            .get(agent_id)
            .and_then(|agent| agent.tools.get(tool_id))
            .and_then(|binding| binding.allowed_tools.as_ref());
        return tool_schemas
            .iter()
            .filter(|tool| allowed_tools.map_or(true, |allowed| allowed.contains(&tool.name)))
            .map(|tool| {
                Arc::new(McpTool::new(
                    if tool.description.is_empty() {
                        format!("MCP tool {} from server '{server_id}'", tool.name)
                    } else {
                        tool.description.clone()
                    },
                    tool.input_schema.clone(),
                    crate::tools::mcp_tool::McpServerConnection {
                        tool_name: tool.name.clone(),
                        server_name: server_id.to_string(),
                        command: mcp.command.clone(),
                        args: mcp.args.clone(),
                        env: mcp.env.clone(),
                        pool: Arc::clone(&mcp_pool),
                    },
                )) as Arc<dyn Tool>
            })
            .collect();
    }

    if tool_id == "webfetch" {
        return vec![Arc::new(WebFetchTool::new(engines.webfetch_client()))];
    }

    if tool_id == "websearch" {
        return vec![Arc::new(WebSearchTool::new(
            engines.websearch_client(),
            engines.websearch_endpoint(),
        ))];
    }

    if tool_id == "code_search" {
        return vec![Arc::new(CodeSearchTool::new(workspace_engines.clone()))];
    }

    if tool_id == "session_search" {
        return vec![Arc::new(SessionSearchTool::new(workspace_engines.clone()))];
    }

    if tool_id == "lsp" {
        return vec![Arc::new(LspTool::new(
            workspace_engines.clone(),
            resolved.workspace_root().to_path_buf(),
        ))];
    }

    vec![]
}

/// Assemble LLM tool list from catalog gate + per-agent DB bindings (CONFIG §2.5, §3.6).
///
/// Tools consume the process-owned editor-service graph; callers must pass the
/// shared `IdeBaseHandle` instead of minting a parallel workspace service.
///
/// `parent_session_id` is the owning session id used by `subagent_launch` to open child sessions.
pub async fn build_tool_list(
    resolved: &ResolvedConfig,
    agent_id: &str,
    provider: Box<dyn LlmProvider>,
    api_key: &str,
    depth: u32,
    parent_cancel: tokio_util::sync::CancellationToken,
    engines: EngineManager,
    workspace_engines: WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
    parent_session_id: &str,
    sessions: Arc<SessionManager>,
    mcp_pool: Arc<McpConnectionPool>,
) -> Vec<Arc<dyn Tool>> {
    let mut mcp_schemas: HashMap<String, Vec<McpToolSchema>> = HashMap::new();
    for (server_id, mcp_def) in resolved.mcp_servers() {
        let catalog_id = mcp_catalog_id(server_id);
        if !should_include_in_llm_list(
            resolved,
            agent_id,
            &catalog_id,
            &engines,
            &workspace_engines,
        ) {
            continue;
        }
        match mcp_pool.start(server_id, mcp_def).await {
            Ok(schemas) => {
                mcp_schemas.insert(server_id.clone(), schemas);
            }
            Err(e) => {
                tracing::warn!("Failed to start MCP server '{}': {}", server_id, e);
            }
        }
    }

    let mut catalog_ids: Vec<String> = resolved.tool_catalog().keys().cloned().collect();
    catalog_ids.sort();

    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    for tool_id in catalog_ids {
        if !should_include_in_llm_list(resolved, agent_id, &tool_id, &engines, &workspace_engines) {
            continue;
        }

        if depth > 0 && tool_id.starts_with("subagent_") {
            continue;
        }

        if depth == 0 && tool_id == "subagent_launch" {
            let subagent_tool = Arc::new(SubagentLaunchTool::new(
                resolved.clone(),
                agent_id,
                provider.box_clone(),
                api_key.to_string(),
                depth,
                parent_cancel.clone(),
                engines.clone(),
                workspace_engines.clone(),
                Arc::clone(&ide),
                Arc::clone(&sessions),
                parent_session_id.to_string(),
                Arc::clone(&mcp_pool),
            ));
            tools.push(subagent_tool);
            continue;
        }

        let new_tools = instantiate_tool(
            resolved,
            agent_id,
            &tool_id,
            &engines,
            &workspace_engines,
            Arc::clone(&ide),
            &sessions,
            &mcp_schemas,
            Arc::clone(&mcp_pool),
        );
        tools.extend(new_tools);
    }

    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::global_db::tools::{core_configurable_tools, core_none_tools};
    use crate::config::resolved::{WorkspaceState, resolve};
    use crate::config::schema::{
        AgentProfile, AgentToolBinding, CustomToolDefinition, GlobalSettings, InitScope,
        McpServerDefinition, ProviderAuth, ProviderDefinition, ToolCatalogEntry, ToolPreset,
        ToolSchema, ToolTier,
    };
    use crate::context_pipeline::Context;
    use crate::llm::{LlmProvider, provider_from_definition};
    use crate::optional::EngineManager;
    use crate::tool::catalog::init;
    use std::collections::HashMap;

    fn dummy_ctx() -> Context {
        Context {
            cwd: std::path::PathBuf::from("/tmp"),
            workspace_paths: WorkspacePaths::for_legacy_root(&std::path::PathBuf::from("/tmp")),
            agents_md: None,
            claude_md: None,
        }
    }

    fn dummy_provider() -> Box<dyn LlmProvider> {
        use crate::config::schema::{ADAPTER_OPENAI_RESPONSES, ProviderConnectionConfig};
        let def = ProviderDefinition {
            id: "test".into(),
            adapter_id: ADAPTER_OPENAI_RESPONSES.into(),
            label: "test".into(),
            config: ProviderConnectionConfig {
                endpoint: "http://localhost:11434/v1".into(),
                api_key: "sk-test".into(),
                auth: ProviderAuth::Bearer,
            },
        };
        provider_from_definition(&def).unwrap()
    }

    fn seed_core_catalog(global: &mut GlobalSettings) {
        for id in core_configurable_tools() {
            global.tool_catalog.insert(
                (*id).to_string(),
                ToolCatalogEntry {
                    id: (*id).to_string(),
                    tier: ToolTier::Core,
                    init_scope: InitScope::None,
                    catalog_enabled: true,
                },
            );
        }
        for id in core_none_tools() {
            global.tool_catalog.insert(
                (*id).to_string(),
                ToolCatalogEntry {
                    id: (*id).to_string(),
                    tier: ToolTier::Core,
                    init_scope: InitScope::None,
                    catalog_enabled: true,
                },
            );
        }
    }

    fn default_bindings() -> HashMap<String, AgentToolBinding> {
        let mut tools = HashMap::new();
        for id in core_configurable_tools() {
            let (policy, path_mode) =
                crate::permission::presets::binding_for_tool(*id, ToolPreset::All);
            tools.insert(
                (*id).to_string(),
                AgentToolBinding {
                    enabled: true,
                    policy,
                    path_mode,
                    last_applied_preset: Some(ToolPreset::All),
                    allowed_tools: None,
                },
            );
        }
        for id in core_none_tools() {
            tools.insert(
                (*id).to_string(),
                AgentToolBinding {
                    enabled: true,
                    policy: crate::permission::ToolPolicy::allow_all(),
                    path_mode: crate::permission::BindingPathMode::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            );
        }
        tools
    }

    fn test_resolved(agent_tools: HashMap<String, AgentToolBinding>) -> ResolvedConfig {
        let mut global = GlobalSettings::default();
        seed_core_catalog(&mut global);
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: agent_tools,
                ..Default::default()
            },
        );
        resolve(global, WorkspaceState::new("/tmp"))
    }

    fn dummy_sessions() -> Arc<SessionManager> {
        Arc::new(SessionManager::new(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ))
    }

    fn list_tools(resolved: &ResolvedConfig, depth: u32) -> Vec<Arc<dyn Tool>> {
        let engines = WorkspaceEngines::new();
        let ide = IdeBaseHandle::open(resolved.workspace_root(), Arc::new(engines.clone()))
            .expect("ide base");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(build_tool_list(
            resolved,
            "default",
            dummy_provider(),
            "test",
            depth,
            tokio_util::sync::CancellationToken::new(),
            EngineManager::new(),
            engines,
            ide,
            "test-parent-session",
            dummy_sessions(),
            Arc::new(McpConnectionPool::new()),
        ))
    }

    #[test]
    fn core_bindings_produce_full_builtin_list() {
        let resolved = test_resolved(default_bindings());
        let tools = list_tools(&resolved, 0);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(
            tools.len() >= 9,
            "expected all core tools from DB bindings, got {}",
            tools.len()
        );
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"wait_shell"));
        assert!(names.contains(&"todo"));
        assert!(names.contains(&"plan"));
        assert!(names.contains(&"subagent_launch"));
        assert!(names.contains(&"session_search"));
    }

    #[test]
    fn binding_filter_excludes_disabled_tools() {
        let mut bindings = default_bindings();
        bindings.get_mut("todo").unwrap().enabled = false;
        bindings.get_mut("plan").unwrap().enabled = false;
        let resolved = test_resolved(bindings);
        let tools = list_tools(&resolved, 0);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"todo"));
        assert!(!names.contains(&"plan"));
        assert!(names.contains(&"bash"));
    }

    #[test]
    fn catalog_gate_excludes_not_ready_custom() {
        let mut bindings = default_bindings();
        bindings.insert(
            "webfetch".into(),
            AgentToolBinding {
                enabled: true,
                policy: crate::permission::presets::binding_for_tool("webfetch", ToolPreset::Safe)
                    .0,
                path_mode: crate::permission::presets::binding_for_tool(
                    "webfetch",
                    ToolPreset::Safe,
                )
                .1,
                last_applied_preset: Some(ToolPreset::Safe),
                allowed_tools: None,
            },
        );
        let mut global = GlobalSettings::default();
        seed_core_catalog(&mut global);
        global.tool_catalog.insert(
            "webfetch".into(),
            ToolCatalogEntry {
                id: "webfetch".into(),
                tier: ToolTier::Custom,
                init_scope: InitScope::Global,
                catalog_enabled: false,
            },
        );
        global.custom_tools.push(CustomToolDefinition {
            name: "webfetch".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: vec![],
            },
            command: "echo".into(),
            args: vec![],
            timeout: 120,
        });
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve(global, WorkspaceState::new("/tmp"));
        let tools = list_tools(&resolved, 0);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(!names.iter().any(|n| n == "webfetch"));

        // Construct new ResolvedConfig with webfetch catalog_enabled=true and init global.
        let mut ready_global = resolved.global().clone();
        ready_global
            .tool_catalog
            .get_mut("webfetch")
            .unwrap()
            .catalog_enabled = true;
        let mut ready = resolve(ready_global, WorkspaceState::new("/tmp"));
        init(&mut ready, InitScope::Global);
        let tools = list_tools(&ready, 0);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(names.iter().any(|n| n == "webfetch"));
    }

    #[test]
    fn does_not_read_agent_config_tools_whitelist() {
        let resolved = test_resolved({
            let mut bindings = default_bindings();
            bindings.get_mut("bash").unwrap().enabled = false;
            bindings
        });
        let tools = list_tools(&resolved, 0);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(
            !names.contains(&"bash"),
            "AgentConfig must not override DB tool binding"
        );
    }

    #[test]
    fn tool_schemas_are_valid_json() {
        let resolved = test_resolved(default_bindings());
        let tools = list_tools(&resolved, 0);
        let ctx = dummy_ctx();
        for tool in &tools {
            let name = tool.name();
            let schema = tool.schema();
            let desc = tool.description(&ctx);
            assert!(!name.is_empty(), "tool {} has empty name", name);
            assert!(schema.is_object(), "tool {} schema is not an object", name);
            assert!(!desc.is_empty(), "tool {} has empty description", name);
        }
    }

    #[test]
    fn depth_gt_zero_excludes_subagent_tools() {
        let resolved = test_resolved(default_bindings());
        let tools = list_tools(&resolved, 1);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"subagent_launch"));
    }

    #[test]
    fn failed_mcp_start_does_not_advertise_dummy_tool() {
        let mut bindings = default_bindings();
        bindings.insert(
            "mcp_dead".into(),
            AgentToolBinding {
                enabled: true,
                policy: crate::permission::ToolPolicy::allow_all(),
                path_mode: crate::permission::BindingPathMode::default(),
                last_applied_preset: None,
                allowed_tools: None,
            },
        );
        let mut global = GlobalSettings::default();
        seed_core_catalog(&mut global);
        global.mcp_servers.insert(
            "dead".into(),
            McpServerDefinition {
                command: "__litecode_no_such_mcp__".into(),
                args: vec![],
                env: HashMap::new(),
                transport: crate::config::schema::McpTransport::Stdio,
            },
        );
        global.tool_catalog.insert(
            "mcp_dead".into(),
            ToolCatalogEntry {
                id: "mcp_dead".into(),
                tier: ToolTier::Mcp,
                init_scope: InitScope::Global,
                catalog_enabled: true,
            },
        );
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let mut resolved = resolve(global, WorkspaceState::new("/tmp"));
        init(&mut resolved, InitScope::Global);
        let tools = list_tools(&resolved, 0);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(
            !names.iter().any(|n| n == "mcp_dead" || n.contains("dead")),
            "unusable MCP must not enter the model tool list, got {names:?}"
        );
    }
}
