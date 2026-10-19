use std::collections::HashMap;
use std::sync::Arc;

use crate::config::global_db::tools::{mcp_catalog_id, non_transport_name_rejection};
use crate::config::resolved::ResolvedConfig;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::mcp::McpToolSchema;
use crate::optional::EngineManager;
use crate::runtime::RuntimeHandle;
use crate::session::manager::SessionManager;
use crate::tool::availability::{available_tools, should_include_in_llm_list};
use crate::tool::project::project_tools;
use crate::tool::trait_::Tool;
use crate::tools::{
    ask_user::AskUserTool,
    bash::BashTool,
    code::{CodeTool, CodeToolHost},
    code_search::CodeSearchTool,
    custom::CustomTool,
    edit::EditTool,
    glob::GlobTool,
    grep::GrepTool,
    kill_shell::KillShellTool,
    knowledge::KnowledgeTool,
    litecode_workspace::LitecodeWorkspaceTool,
    lsp::LspTool,
    mcp_tool::McpTool,
    plan::PlanTool,
    read::ReadTool,
    session_search::SessionSearchTool,
    subagent::{
        SUBAGENT_MAX_DEPTH, SubagentDeleteTool, SubagentLaunchTool, SubagentListTool,
        SubagentSendTool, SubagentStopTool, SubagentWaitTool,
    },
    todo::TodoWriteTool,
    wait_shell::WaitShellTool,
    webfetch::WebFetchTool,
    websearch::WebSearchTool,
    write::WriteTool,
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
        "knowledge" => Arc::new(KnowledgeTool::new(Arc::clone(&ide))),
        "glob" => Arc::new(GlobTool),
        "todo" => Arc::new(TodoWriteTool::new(Arc::clone(&sessions))),
        "ask_user" => Arc::new(AskUserTool::new()),
        "plan" => Arc::new(PlanTool::new(Arc::clone(&sessions))),
        _ => return None,
    };
    Some(tool)
}

fn instantiate_tool(
    runtime: &RuntimeHandle,
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
        return vec![Arc::new(CustomTool::new(
            custom.clone(),
            Arc::clone(&runtime.custom_tool_hub),
        ))];
    }

    if let Some(server_id) = tool_id.strip_prefix("mcp_")
        && let Some(mcp) = resolved.mcp_servers().get(server_id)
    {
        // Handshake must have succeeded this turn (`mcp_schemas` is filled only
        // after start). Do not advertise a dummy catalog-id tool when the
        // process is down —the model would see it and try to call it.
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
                    server_id,
                    if tool.description.is_empty() {
                        format!("MCP tool {} from server '{server_id}'", tool.name)
                    } else {
                        tool.description.clone()
                    },
                    tool.input_schema.clone(),
                    crate::tools::mcp_tool::McpServerConnection {
                        tool_name: tool.name.clone(),
                        server_name: resolved.mcp_pool_key(server_id),
                        command: mcp.command.clone(),
                        args: mcp.args.clone(),
                        env: mcp.env.clone(),
                        cwd: Some(resolved.workspace_root().to_path_buf()),
                        pool: Arc::clone(&mcp_pool),
                        timeout_secs: mcp.call_timeout_secs(),
                    },
                )) as Arc<dyn Tool>
            })
            .collect();
    }

    if tool_id == "webfetch" {
        return vec![Arc::new(
            WebFetchTool::new(engines.webfetch_client())
                .with_warm_hook(engines.warm_hook("webfetch")),
        )];
    }

    if tool_id == "websearch" {
        return vec![Arc::new(
            WebSearchTool::new(engines.websearch_client(), engines.websearch_endpoint())
                .with_warm_hook(engines.warm_hook("websearch")),
        )];
    }

    if tool_id == "code_search" {
        return vec![Arc::new(CodeSearchTool::new(workspace_engines.clone()))];
    }

    if tool_id == "session_search" {
        return vec![Arc::new(SessionSearchTool::new(workspace_engines.clone()))];
    }

    if tool_id == "litecode_workspace" {
        return vec![Arc::new(LitecodeWorkspaceTool::new(
            runtime.clone(),
            Arc::clone(sessions),
            agent_id.to_string(),
        ))];
    }

    if tool_id == "lsp" {
        return vec![Arc::new(LspTool::new(
            workspace_engines.clone(),
            resolved.workspace_root().to_path_buf(),
        ))];
    }

    vec![]
}

/// Assemble LLM tool list from available cards + per-agent DB bindings.
///
/// Tools consume the process-owned editor-service graph; callers must pass the
/// shared `IdeBaseHandle` instead of minting a parallel workspace service.
///
/// `parent_session_id` is the owning session id used by `subagent_launch` to open child sessions.
pub async fn build_tool_list(
    runtime: &RuntimeHandle,
    agent_id: &str,
    depth: u32,
    parent_cancel: tokio_util::sync::CancellationToken,
    parent_session_id: &str,
    sessions: Arc<SessionManager>,
    code_host: Option<Arc<CodeToolHost>>,
) -> Vec<Arc<dyn Tool>> {
    let resolved = &runtime.resolved;
    let engines = &runtime.engine_manager;
    let workspace_engines = &runtime.workspace_engines;
    let mcp_pool = &runtime.mcp_pool;
    let mut mcp_schemas: HashMap<String, Vec<McpToolSchema>> = HashMap::new();
    let servers = resolved.mcp_servers();
    for (server_id, mcp_def) in &servers {
        let catalog_id = mcp_catalog_id(server_id);
        if !should_include_in_llm_list(resolved, agent_id, &catalog_id, engines, workspace_engines)
        {
            continue;
        }
        let pool_key = resolved.mcp_pool_key(server_id);
        match mcp_pool
            .start(
                &pool_key,
                mcp_def,
                Some(resolved.workspace_root().to_path_buf()),
            )
            .await
        {
            Ok(schemas) => {
                mcp_schemas.insert(server_id.clone(), schemas);
            }
            Err(e) => {
                tracing::warn!("Failed to start MCP server '{}': {}", server_id, e);
            }
        }
    }

    let mut catalog_ids: Vec<String> = available_tools(resolved)
        .into_iter()
        .map(|t| t.id)
        .collect();
    catalog_ids.sort();

    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();

    for tool_id in catalog_ids {
        if !should_include_in_llm_list(resolved, agent_id, &tool_id, engines, workspace_engines) {
            continue;
        }

        // Product lock: subagent tools are primary-turn only (depth lock = 1).
        if depth >= SUBAGENT_MAX_DEPTH && tool_id.starts_with("subagent_") {
            continue;
        }
        // Tool-set gate: primary-only tools (plan / todo).
        if depth >= SUBAGENT_MAX_DEPTH
            && crate::config::schema::PRIMARY_ONLY_TOOL_IDS.contains(&tool_id.as_str())
        {
            continue;
        }

        if depth == 0 && tool_id.starts_with("subagent_") {
            match tool_id.as_str() {
                "subagent_launch" => {
                    tools.push(Arc::new(SubagentLaunchTool::new(
                        runtime.clone(),
                        agent_id,
                        depth,
                        parent_cancel.clone(),
                        Arc::clone(&sessions),
                        parent_session_id.to_string(),
                    )));
                }
                "subagent_wait" => {
                    tools.push(Arc::new(SubagentWaitTool::new(Arc::clone(&sessions))));
                }
                "subagent_stop" => {
                    tools.push(Arc::new(SubagentStopTool::new(Arc::clone(&sessions))));
                }
                "subagent_send" => {
                    tools.push(Arc::new(SubagentSendTool::new(
                        runtime.clone(),
                        Arc::clone(&sessions),
                        depth,
                    )));
                }
                "subagent_delete" => {
                    tools.push(Arc::new(SubagentDeleteTool::new(
                        Arc::clone(&sessions),
                        Arc::clone(&runtime.subagent_hub),
                    )));
                }
                "subagent_list" => {
                    tools.push(Arc::new(SubagentListTool::new(Arc::clone(&sessions))));
                }
                _ => {}
            }
            continue;
        }

        let new_tools = instantiate_tool(
            runtime,
            resolved,
            agent_id,
            &tool_id,
            engines,
            workspace_engines,
            Arc::clone(&runtime.ide),
            &sessions,
            &mcp_schemas,
            Arc::clone(&runtime.mcp_pool),
        );
        tools.extend(new_tools);
    }

    // One projection from this caller's visible set. A missing host cannot
    // publish `run_code` or drop the code tools.
    tools = assemble_projected_tools(tools, code_host);

    tools
}

fn assemble_projected_tools(
    tools: Vec<Arc<dyn Tool>>,
    host: Option<Arc<CodeToolHost>>,
) -> Vec<Arc<dyn Tool>> {
    let tools = strip_reserved_transport_names(tools);
    let projected = project_tools(tools);
    for rejection in &projected.rejected {
        tracing::warn!(
            tool = %rejection.name,
            reason = %rejection.reason,
            "code tool removed from both surfaces"
        );
    }
    let mut direct = projected.direct;
    if projected.code_tools.is_empty() {
        return direct;
    }
    let Some(host) = host else {
        tracing::error!("code surface kept direct: no code host to bind run_code");
        direct.extend(projected.code_tools);
        return direct;
    };
    let names = projected.whitelist.clone();
    tracing::info!("code mode active: bundling {names:?}");
    direct.push(Arc::new(CodeTool::from_surface(
        projected.code_tools,
        projected.sdk,
        names,
        host,
    )));
    direct
}

/// A non-transport tool must not keep the transport's name. Drop the impostor
/// and keep the code surface; the transport is appended afterwards.
fn strip_reserved_transport_names(tools: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
    tools
        .into_iter()
        .filter(|tool| {
            if let Some(reason) = non_transport_name_rejection(tool.name()) {
                tracing::warn!(
                    tool = tool.name(),
                    reason,
                    "dropping non-transport tool that claims the code transport name"
                );
                false
            } else {
                true
            }
        })
        .collect()
}

/// Builtins that currently declare a code surface. Admission for the settings
/// catalog runs on this set; each tool's own `surface` is still the decision.
pub fn builtin_code_surface_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool::default()),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspacePaths;
    use crate::config::global_db::tools::{core_configurable_tools, core_none_tools};
    use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
    use crate::config::schema::{
        AgentProfile, AgentToolBinding, GlobalSettings, McpServerDefinition, ToolPreset,
    };
    use crate::context_pipeline::Context;
    use crate::optional::EngineManager;
    use std::collections::HashMap;

    fn dummy_ctx() -> Context {
        Context {
            cwd: std::path::PathBuf::from("/tmp"),
            workspace_paths: WorkspacePaths::for_legacy_root(&std::path::PathBuf::from("/tmp")),
            agents_md: None,
            claude_md: None,
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
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: agent_tools,
                ..Default::default()
            },
        );
        resolve_without_catalog(global, WorkspaceState::new("/tmp"))
    }

    fn dummy_sessions() -> Arc<SessionManager> {
        Arc::new(SessionManager::new_for_test(
            Arc::new(crate::config::TurnGuard::new()),
            String::new(),
        ))
    }

    fn list_tools(resolved: &ResolvedConfig, depth: u32) -> Vec<Arc<dyn Tool>> {
        list_tools_with(resolved, depth, None)
    }

    fn list_tools_with(
        resolved: &ResolvedConfig,
        depth: u32,
        host: Option<Arc<crate::tools::code::CodeToolHost>>,
    ) -> Vec<Arc<dyn Tool>> {
        let engines = WorkspaceEngines::new();
        let ide = IdeBaseHandle::open(resolved.workspace_root(), Arc::new(engines.clone()))
            .expect("ide base");
        let runtime = RuntimeHandle::new(
            resolved.clone(),
            "default".into(),
            WorkspaceState::new(resolved.workspace_root()),
            Arc::new(EngineManager::new()),
            Arc::new(engines),
            ide,
            Arc::new(std::sync::atomic::AtomicU64::new(0)),
            std::path::PathBuf::from("/tmp/litecode-test-global.db"),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(build_tool_list(
            &runtime,
            "default",
            depth,
            tokio_util::sync::CancellationToken::new(),
            "test-parent-session",
            dummy_sessions(),
            host,
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
        assert!(names.contains(&"subagent_send"));
        assert!(names.contains(&"subagent_wait"));
        assert!(names.contains(&"subagent_stop"));
        assert!(names.contains(&"subagent_list"));
        // WP-057 gate (tool/availability.rs: is_session_search_gated /
        // session_search_visible): session_search is bound but hidden until
        // code_search is desired + Warm. This fixture's WorkspaceEngines is never
        // warmed, so it must be absent. Visible-after-warm is covered by
        // tool::availability::tests::retrieval_visible_when_desired_and_warm.
        assert!(
            !names.contains(&"session_search"),
            "session_search must stay hidden while code_search is not Warm (WP-057)"
        );
        assert!(names.contains(&"litecode_workspace"));
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
    fn unbound_webfetch_is_not_in_llm_list() {
        let bindings = default_bindings();
        let resolved = test_resolved(bindings);
        let tools = list_tools(&resolved, 0);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(!names.iter().any(|n| n == "webfetch"));

        let mut bound = default_bindings();
        bound.insert(
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
        let resolved = test_resolved(bound);
        let tools = list_tools(&resolved, 0);
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
    fn subagent_delete_hidden_at_depth_ge_1() {
        let resolved = test_resolved(default_bindings());
        let primary: Vec<String> = list_tools(&resolved, 0)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert!(
            primary.iter().any(|n| n == "subagent_delete"),
            "{primary:?}"
        );
        let child: Vec<String> = list_tools(&resolved, 1)
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert!(!child.iter().any(|n| n == "subagent_delete"), "{child:?}");
    }

    #[test]
    fn depth_gt_zero_excludes_subagent_tools() {
        let resolved = test_resolved(default_bindings());
        let tools = list_tools(&resolved, 1);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"subagent_launch"));
        assert!(!names.contains(&"subagent_send"));
        assert!(!names.contains(&"subagent_wait"));
        assert!(!names.contains(&"subagent_stop"));
        assert!(!names.contains(&"subagent_list"));
        assert!(!names.contains(&"plan"));
        assert!(!names.contains(&"todo"));
        assert!(
            names.contains(&"knowledge"),
            "knowledge stays on the subagent tool set"
        );
        assert!(
            names.contains(&"litecode_workspace"),
            "the workspace tool stays on the subagent tool set"
        );
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
        global.mcp_servers.insert(
            "dead".into(),
            McpServerDefinition {
                command: "__litecode_no_such_mcp__".into(),
                args: vec![],
                env: HashMap::new(),
                transport: crate::config::schema::McpTransport::Stdio,
                ..Default::default()
            },
        );
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let tools = list_tools(&resolved, 0);
        let names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
        assert!(
            !names.iter().any(|n| n == "mcp_dead" || n.contains("dead")),
            "unusable MCP must not enter the model tool list, got {names:?}"
        );
    }

    #[test]
    fn code_mode_bundles_and_hides_first_slice_tools() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tools::code::CodeToolHost;
        use crate::tools::glob::GlobTool;
        use crate::tools::grep::GrepTool;
        use crate::tools::read::ReadTool;

        let resolved = test_resolved(default_bindings());
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let dir = tempfile::tempdir().unwrap();
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let host = Arc::new(CodeToolHost::new(
            permission,
            ctx.clone(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            32_768,
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let direct: Vec<Arc<dyn Tool>> = vec![
            Arc::new(GlobTool),
            Arc::new(GrepTool),
            Arc::new(ReadTool::default()),
        ];

        let tools = assemble_projected_tools(direct, Some(host));
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "run_code");
        assert!(tools[0].schema()["properties"].get("timeout_ms").is_some());
        let description = tools[0].description(&ctx);
        let other = Context {
            cwd: std::path::PathBuf::from("/somewhere/else"),
            ..ctx.clone()
        };
        assert_eq!(description, tools[0].description(&other));
        assert!(description.contains("declare const tools"));
        assert!(description.contains(
            "readonly [\"grep\"]: (args: GrepInput, options?: ToolCallOptions) => Promise<ToolResult<GrepData>>;"
        ));
        assert!(
            description.contains(
                "readonly [\"read\"]: (args: ReadInput) => Promise<ToolResult<ReadData>>;"
            )
        );
        assert!(!description.contains("timeout_ms") || description.contains("ToolCallOptions"));
        let bindings = tools[0].code_bindings().expect("whitelist");
        let names: Vec<&str> = bindings.iter().map(String::as_str).collect();
        assert_eq!(names, ["glob", "grep", "read"]);
    }

    #[test]
    fn code_gate_projects_every_depth_from_that_callers_visible_set() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tools::code::CodeToolHost;

        let resolved = test_resolved(default_bindings());
        let closed = list_tools(&resolved, 1);
        let closed_names: Vec<&str> = closed.iter().map(|tool| tool.name()).collect();
        assert!(closed_names.contains(&"read"));
        assert!(closed_names.contains(&"glob"));
        assert!(closed_names.contains(&"grep"));
        assert!(!closed_names.contains(&"run_code"));

        let dir = tempfile::tempdir().unwrap();
        let permission = PermissionEngine::resolver(resolved.clone(), "default", 1);
        let host = Arc::new(CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            32_768,
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let child = list_tools_with(&resolved, 1, Some(Arc::clone(&host)));
        let child_names: Vec<&str> = child.iter().map(|tool| tool.name()).collect();
        assert!(!child_names.contains(&"read"));
        assert!(!child_names.contains(&"glob"));
        assert!(!child_names.contains(&"grep"));
        assert!(child_names.contains(&"run_code"));
        assert!(child_names.contains(&"bash"));
        let child_sdk = child
            .iter()
            .find(|tool| tool.name() == "run_code")
            .unwrap()
            .description(&dummy_ctx());

        let mut bindings = default_bindings();
        bindings.get_mut("grep").unwrap().enabled = false;
        let narrower = test_resolved(bindings);
        let permission = PermissionEngine::resolver(narrower.clone(), "default", 1);
        let host = Arc::new(CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            32_768,
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let narrow_tools = list_tools_with(&narrower, 1, Some(host));
        let narrow_sdk = narrow_tools
            .iter()
            .find(|tool| tool.name() == "run_code")
            .expect("run_code")
            .description(&dummy_ctx());
        assert!(narrow_sdk.contains("[\"read\"]"));
        assert!(narrow_sdk.contains("[\"glob\"]"));
        assert!(!narrow_sdk.contains("[\"grep\"]"));
        assert_ne!(child_sdk, narrow_sdk);
        let whitelist = narrow_tools
            .iter()
            .find(|tool| tool.name() == "run_code")
            .unwrap()
            .code_bindings()
            .unwrap();
        let names: Vec<&str> = whitelist.iter().map(String::as_str).collect();
        assert_eq!(names, ["glob", "read"]);
    }

    struct Impostor;

    impl Tool for Impostor {
        fn name(&self) -> &str {
            "run_code"
        }

        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn description(&self, _: &Context) -> String {
            "not the transport".into()
        }
    }

    #[test]
    fn a_same_named_tool_does_not_drop_the_code_surface() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tool::ToolSurface;
        use crate::tools::code::CodeToolHost;

        let dir = tempfile::tempdir().unwrap();
        let resolved = test_resolved(default_bindings());
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let host = Arc::new(CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            32_768,
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let tools = assemble_projected_tools(
            vec![
                Arc::new(ReadTool::default()) as Arc<dyn Tool>,
                Arc::new(Impostor),
            ],
            Some(host),
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "run_code");
        assert_ne!(tools[0].description(&dummy_ctx()), "not the transport");
        let bindings = tools[0].code_bindings().expect("code surface kept");
        let names: Vec<&str> = bindings.iter().map(String::as_str).collect();
        assert_eq!(names, ["read"]);

        let closed = assemble_projected_tools(
            vec![
                Arc::new(ReadTool::default()) as Arc<dyn Tool>,
                Arc::new(Impostor),
            ],
            None,
        );
        let closed_names: Vec<&str> = closed.iter().map(|tool| tool.name()).collect();
        assert_eq!(closed_names, ["read"]);
        assert_eq!(ReadTool::default().surface(), ToolSurface::Code);
    }

    struct Unrenderable;

    impl Tool for Unrenderable {
        fn name(&self) -> &str {
            "unrenderable"
        }

        fn surface(&self) -> crate::tool::ToolSurface {
            crate::tool::ToolSurface::Code
        }

        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn description(&self, _: &Context) -> String {
            "missing data schema".into()
        }
    }

    #[test]
    fn unrenderable_code_tool_does_not_publish_run_code() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tools::code::CodeToolHost;

        let dir = tempfile::tempdir().unwrap();
        let resolved = test_resolved(default_bindings());
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let host = Arc::new(CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            32_768,
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let tools = assemble_projected_tools(vec![Arc::new(Unrenderable)], Some(host));
        assert!(
            tools.is_empty(),
            "a failed code surface must not leave run_code running: {:?}",
            tools.iter().map(|tool| tool.name()).collect::<Vec<_>>()
        );
    }
}
