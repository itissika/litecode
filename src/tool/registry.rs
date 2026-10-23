use std::collections::HashMap;
use std::sync::Arc;

use crate::config::global_db::tools::{
    mcp_catalog_id, non_transport_name_rejection, reserved_command_name_rejection,
};
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
    code::{CodeTool, CodeToolHost},
    code_search::CodeSearchTool,
    kill::KillTool,
    custom::CustomTool,
    apply_edits::ApplyEditsTool,
    edit::EditTool,
    glob::GlobTool,
    grep::GrepTool,
    knowledge::KnowledgeTool,
    litecode_workspace::LitecodeWorkspaceTool,
    lsp::{self, LSP_CARD_ID},
    mcp_tool::McpTool,
    plan::PlanTool,
    read::ReadTool,
    run::RunTool,
    wait::WaitTool,
    session_search::SessionSearchTool,
    subagent::{
        SUBAGENT_MAX_DEPTH, SubagentDeleteTool, SubagentLaunchTool, SubagentListTool,
        SubagentSendTool, SubagentStopTool, SubagentWaitTool,
    },
    todo::TodoWriteTool,
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
        "run" => Arc::new(RunTool::new(Arc::clone(&ide.terminal.jobs))),
        "read" => Arc::new(ReadTool::with_ide(Arc::clone(&ide))),
        "write" => Arc::new(WriteTool::with_ide(Arc::clone(&ide))),
        "edit" => Arc::new(EditTool::with_ide(Arc::clone(&ide))),
        "apply_edits" => Arc::new(ApplyEditsTool::with_ide(Arc::clone(&ide))),
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
            .filter(|tool| {
                if !mcp_inner_name_allowed(&tool.name) {
                    tracing::warn!(
                        tool = %tool.name,
                        "dropping MCP tool that uses a reserved name"
                    );
                    return false;
                }
                allowed_tools.map_or(true, |allowed| allowed.contains(&tool.name))
            })
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

    if tool_id == LSP_CARD_ID {
        // One settings card → seven Code tools (Direct never sees name "lsp").
        return lsp::all_code_tools(
            workspace_engines.clone(),
            resolved.workspace_root().to_path_buf(),
        );
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

    // Code companions of run. They are not settings cards. A caller
    // that cannot see run does not get either tool.
    if tools.iter().any(|tool| tool.name() == "run") {
        let jobs = Arc::clone(&runtime.ide.terminal.jobs);
        tools.push(Arc::new(WaitTool::new(Arc::clone(&jobs))));
        tools.push(Arc::new(KillTool::new(jobs)));
    }

    // Code companion of edit. Not a settings card; shares edit's dial via
    // permission engine alias. Hidden when edit is unbound/disabled.
    if tools.iter().any(|tool| tool.name() == "edit") {
        tools.push(Arc::new(ApplyEditsTool::with_ide(Arc::clone(&runtime.ide))));
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

/// MCP publishes `mcp_{server}_{inner}`. The inner name still must not be a
/// command-job name, a retired shell name, or the code transport.
fn mcp_inner_name_allowed(name: &str) -> bool {
    non_transport_name_rejection(name).is_none() && reserved_command_name_rejection(name).is_none()
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
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadTool::default()),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(EditTool::new()),
        Arc::new(ApplyEditsTool::new()),
        Arc::new(WriteTool::new()),
        Arc::new(RunTool::new(Arc::new(
            crate::terminal::AgentJobRegistry::new(),
        ))),
        Arc::new(WaitTool::new(Arc::new(
            crate::terminal::AgentJobRegistry::new(),
        ))),
        Arc::new(KillTool::new(Arc::new(
            crate::terminal::AgentJobRegistry::new(),
        ))),
        // WP-203: network tools are Code-only (cold stubs for settings admission).
        Arc::new(WebFetchTool::new(Arc::new(std::sync::RwLock::new(None)))),
        Arc::new(WebSearchTool::new(
            Arc::new(std::sync::RwLock::new(None)),
            Arc::new(std::sync::RwLock::new(None)),
        )),
    ];
    // WP-210 I: the seven LSP navigation tools are Code-surface too (cold
    // engines; admission only needs their contracts).
    tools.extend(lsp::all_code_tools(
        crate::engines::WorkspaceEngines::new(),
        std::path::PathBuf::from("."),
    ));
    tools
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
        // WP-197: 默认绑定无旧 shell；有 run / wait / kill。
        assert!(!names.contains(&"bash"));
        assert!(!names.contains(&"wait_shell"));
        assert!(!names.contains(&"kill_shell"));
        assert!(names.contains(&"run"));
        // Injected with run. Without a code host they stay on this list;
        // a host bundles them into run_code and they are not Direct names.
        assert!(names.contains(&"wait"));
        assert!(names.contains(&"kill"));
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
    fn custom_tools_cannot_take_command_or_retired_shell_names() {
        use crate::config::schema::{CustomToolDefinition, ToolSchema};
        use crate::config::settings_writer::validate_custom_definition;

        for name in [
            "wait",
            "kill",
            "bash",
            "wait_shell",
            "kill_shell",
            "run_command",
            "command_output",
            "command_kill",
        ] {
            let mut def = custom_def(name);
            let err = validate_custom_definition(name, &mut def).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("reserved"), "{name}: {message}");
        }
        let mut ok = custom_def("my_echo");
        assert!(validate_custom_definition("my_echo", &mut ok).is_ok());

        fn custom_def(name: &str) -> CustomToolDefinition {
            CustomToolDefinition {
                name: name.into(),
                description: String::new(),
                schema: ToolSchema {
                    schema_type: "object".into(),
                    properties: serde_json::json!({}),
                    required: Vec::new(),
                },
                command: "echo hi".into(),
                args: Vec::new(),
                timeout: 30,
                rules: Vec::new(),
                surface: Default::default(),
                suite: None,
                suite_label: None,
            }
        }
    }

    #[test]
    fn mcp_inner_names_cannot_use_reserved_command_names() {
        for name in [
            "wait",
            "kill",
            "bash",
            "wait_shell",
            "kill_shell",
            "run_command",
            "command_output",
            "command_kill",
            "run_code",
        ] {
            assert!(!mcp_inner_name_allowed(name), "{name}");
        }
        assert!(mcp_inner_name_allowed("list_files"));
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
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let child = list_tools_with(&resolved, 1, Some(Arc::clone(&host)));
        let child_names: Vec<&str> = child.iter().map(|tool| tool.name()).collect();
        assert!(!child_names.contains(&"read"));
        assert!(!child_names.contains(&"glob"));
        assert!(!child_names.contains(&"grep"));
        assert!(!child_names.contains(&"edit"));
        assert!(!child_names.contains(&"apply_edits"));
        assert!(!child_names.contains(&"write"));
        assert!(child_names.contains(&"run_code"));
        assert!(!child_names.contains(&"bash"));
        assert!(!child_names.contains(&"wait_shell"));
        assert!(!child_names.contains(&"kill_shell"));
        assert!(!child_names.contains(&"run"));
        assert!(!child_names.contains(&"wait"));
        assert!(!child_names.contains(&"kill"));
        let child_sdk = child
            .iter()
            .find(|tool| tool.name() == "run_code")
            .unwrap()
            .description(&dummy_ctx());
        assert!(child_sdk.contains("declare const tools"));
        assert!(child_sdk.contains("[\"run\"]"));
        assert!(child_sdk.contains("[\"wait\"]"));
        assert!(child_sdk.contains("[\"kill\"]"));
        assert!(child_sdk.contains("[\"edit\"]"));
        assert!(child_sdk.contains("[\"apply_edits\"]"));
        assert!(!child_sdk.contains("[\"bash\"]"));
        assert!(!child_sdk.contains("[\"wait_shell\"]"));
        assert!(!child_sdk.contains("[\"kill_shell\"]"));

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
        assert!(narrow_sdk.contains("[\"edit\"]"));
        assert!(narrow_sdk.contains("[\"write\"]"));
        assert!(!narrow_sdk.contains("[\"grep\"]"));
        assert_ne!(child_sdk, narrow_sdk);
        let whitelist = narrow_tools
            .iter()
            .find(|tool| tool.name() == "run_code")
            .unwrap()
            .code_bindings()
            .unwrap();
        let names: Vec<&str> = whitelist.iter().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "apply_edits",
                "edit",
                "glob",
                "kill",
                "read",
                "run",
                "wait",
                "write"
            ]
        );
        assert!(narrow_sdk.contains("[\"run\"]"));
        assert!(narrow_sdk.contains("[\"wait\"]"));
        assert!(narrow_sdk.contains("[\"kill\"]"));
        assert!(narrow_sdk.contains("[\"apply_edits\"]"));
    }

    #[test]
    #[test]
    fn webfetch_websearch_project_to_code_not_direct() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tool::project::project_tools;
        use crate::tools::code::CodeToolHost;

        assert_eq!(
            WebFetchTool::new(Arc::new(std::sync::RwLock::new(None))).surface(),
            crate::tool::ToolSurface::Code
        );
        assert_eq!(
            WebSearchTool::new(
                Arc::new(std::sync::RwLock::new(None)),
                Arc::new(std::sync::RwLock::new(None)),
            )
            .surface(),
            crate::tool::ToolSurface::Code
        );

        let projected = project_tools(vec![
            Arc::new(WebFetchTool::new(Arc::new(std::sync::RwLock::new(None)))) as Arc<dyn Tool>,
            Arc::new(WebSearchTool::new(
                Arc::new(std::sync::RwLock::new(None)),
                Arc::new(std::sync::RwLock::new(None)),
            )) as Arc<dyn Tool>,
        ]);
        let direct_names: Vec<&str> = projected.direct.iter().map(|t| t.name()).collect();
        assert!(
            direct_names.is_empty(),
            "Direct must be empty for webfetch/websearch-only set: {direct_names:?}"
        );
        assert!(
            projected.whitelist.iter().any(|n| n == "webfetch")
                && projected.whitelist.iter().any(|n| n == "websearch"),
            "Code whitelist must admit both: {:?}",
            projected.whitelist
        );
        assert!(
            projected.rejected.is_empty(),
            "admission must pass: {:?}",
            projected.rejected
        );

        let dir = tempfile::tempdir().unwrap();
        let resolved = test_resolved(default_bindings());
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let host = Arc::new(CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            deny_permission_sink(),
            dir.path().join(".litecode"),
            crate::tool::write_lock::process_write_lock(),
            None,
        ));
        let assembled = assemble_projected_tools(
            vec![
                Arc::new(WebFetchTool::new(Arc::new(std::sync::RwLock::new(None))))
                    as Arc<dyn Tool>,
                Arc::new(WebSearchTool::new(
                    Arc::new(std::sync::RwLock::new(None)),
                    Arc::new(std::sync::RwLock::new(None)),
                )) as Arc<dyn Tool>,
            ],
            Some(host),
        );
        let top: Vec<&str> = assembled.iter().map(|t| t.name()).collect();
        assert_eq!(top, ["run_code"], "only transport on Direct: {top:?}");
        let bindings = assembled[0].code_bindings().expect("run_code bindings");
        assert!(bindings.iter().any(|n| n == "webfetch"));
        assert!(bindings.iter().any(|n| n == "websearch"));
    }
}
