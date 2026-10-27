use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::config::global_db::tools::{
    non_transport_name_rejection, reserved_command_name_rejection,
};
use crate::config::resolved::ResolvedConfig;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::mcp::McpToolSchema;
use crate::optional::EngineManager;
use crate::runtime::RuntimeHandle;
use crate::session::manager::SessionManager;
use crate::tool::capability::{CapabilitySource, MemberSpec, capability_catalog};
use crate::tool::project::project_tools;
use crate::tool::spec::{CodeFunctionSpec, FunctionSpec};
use crate::tool::trait_::Tool;
use crate::tools::{
    ask_user::AskUserTool,
    browser,
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
    lsp,
    mcp_tool::McpTool,
    plan::PlanTool,
    read::ReadTool,
    run::RunTool,
    wait::WaitTool,
    session_search::SessionSearchTool,
    subagent::{
        SubagentDeleteTool, SubagentLaunchTool, SubagentListTool, SubagentSendTool,
        SubagentStopTool, SubagentWaitTool,
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

fn instantiate_builtin_member(
    runtime: &RuntimeHandle,
    agent_id: &str,
    member: &str,
    engines: &EngineManager,
    workspace_engines: &WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
    sessions: &Arc<SessionManager>,
) -> Option<Arc<dyn Tool>> {
    if let Some(tool) = builtin_tool(member, Arc::clone(sessions), workspace_engines, ide) {
        return Some(tool);
    }

    if let Some(tool) = browser::BrowserTool::tool_for_name(member) {
        return Some(tool);
    }

    let tool: Arc<dyn Tool> = match member {
        "wait" => Arc::new(WaitTool::new(Arc::clone(&runtime.ide.terminal.jobs))),
        "kill" => Arc::new(KillTool::new(Arc::clone(&runtime.ide.terminal.jobs))),
        "webfetch" => Arc::new(
            WebFetchTool::new(engines.webfetch_client())
                .with_warm_hook(engines.warm_hook("webfetch")),
        ),
        "websearch" => Arc::new(
            WebSearchTool::new(engines.websearch_client(), engines.websearch_endpoint())
                .with_warm_hook(engines.warm_hook("websearch")),
        ),
        "code_search" => Arc::new(CodeSearchTool::new(workspace_engines.clone())),
        "session_search" => Arc::new(SessionSearchTool::new(workspace_engines.clone())),
        "litecode_workspace" => Arc::new(LitecodeWorkspaceTool::new(
            runtime.clone(),
            Arc::clone(sessions),
            agent_id.to_string(),
        )),
        _ => return None,
    };
    Some(tool)
}

/// Code-suite members keep their Tool identity for bindings/permissions, but
/// the code surface exposes them as `suite.member`. The wrapper changes only
/// the model-facing name; execution and all cross-cutting facts delegate.
struct ExposedCodeTool {
    inner: Arc<dyn Tool>,
    name: String,
}

impl ExposedCodeTool {
    fn new(inner: Arc<dyn Tool>, name: String) -> Self {
        Self { inner, name }
    }
}

impl Tool for ExposedCodeTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn surface(&self) -> crate::tool::trait_::ToolSurface {
        self.inner.surface()
    }

    fn code_bindings(&self) -> Option<Arc<[String]>> {
        self.inner.code_bindings()
    }

    fn is_code_transport(&self) -> bool {
        self.inner.is_code_transport()
    }

    fn schema(&self) -> serde_json::Value {
        self.inner.schema()
    }

    fn prepare_input(&self, input: &mut serde_json::Value) {
        self.inner.prepare_input(input);
    }

    fn schema_arguments<'a>(&self, input: &'a serde_json::Value) -> std::borrow::Cow<'a, serde_json::Value> {
        self.inner.schema_arguments(input)
    }

    fn data_schema(&self) -> Option<serde_json::Value> {
        self.inner.data_schema()
    }

    fn call(&self, input: serde_json::Value) -> crate::types::ToolCallResult {
        self.inner.call(input)
    }

    fn call_inner(&self, input: serde_json::Value) -> crate::types::ToolCallResult {
        self.inner.call_inner(input)
    }

    fn call_async(
        &self,
        input: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::types::ToolCallResult> + Send + '_>> {
        self.inner.call_async(input)
    }

    fn execute(
        &self,
        input: serde_json::Value,
        execution: crate::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::types::ToolCallResult> + Send + '_>> {
        self.inner.execute(input, execution)
    }

    fn description(&self, ctx: &crate::context_pipeline::Context) -> String {
        self.inner.description(ctx)
    }

    fn timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeout()
    }

    fn effects(
        &self,
        input: &serde_json::Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> crate::tool::resource_gate::ToolEffects {
        self.inner.effects(input, path_mode, workspace_root)
    }

    fn is_concurrency_safe(&self, input: &serde_json::Value) -> bool {
        self.inner.is_concurrency_safe(input)
    }

    fn resource_keys(
        &self,
        input: &serde_json::Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> Vec<crate::tool::write_lock::ResourceKey> {
        self.inner.resource_keys(input, path_mode, workspace_root)
    }

    fn is_cancellable(&self) -> bool {
        self.inner.is_cancellable()
    }

    fn releases_lock_after_spawn(&self) -> bool {
        self.inner.releases_lock_after_spawn()
    }

    fn max_result_size(&self) -> usize {
        self.inner.max_result_size()
    }

    fn validate_input(&self, input: &serde_json::Value) -> std::result::Result<(), String> {
        self.inner.validate_input(input)
    }

    fn is_destructive(
        &self,
        input: &serde_json::Value,
        path_mode: crate::workspace::ToolPathMode,
        workspace_root: &std::path::Path,
    ) -> bool {
        self.inner.is_destructive(input, path_mode, workspace_root)
    }

    fn set_active_session(&self, session_id: String) {
        self.inner.set_active_session(session_id);
    }

    fn set_permission_sink(&self, sink: Arc<dyn crate::permission::PermissionSink>) {
        self.inner.set_permission_sink(sink);
    }
}

fn custom_member(resolved: &ResolvedConfig, runtime: &RuntimeHandle, member: &str) -> Option<Arc<dyn Tool>> {
    resolved
        .custom_tools()
        .iter()
        .find(|definition| definition.name == member)
        .map(|definition| {
            Arc::new(CustomTool::new(
                definition.clone(),
                Arc::clone(&runtime.custom_tool_hub),
            )) as Arc<dyn Tool>
        })
}

fn mcp_members(
    runtime: &RuntimeHandle,
    resolved: &ResolvedConfig,
    agent_id: &str,
    capability_id: &str,
    server_id: &str,
    mcp_schemas: &HashMap<String, Vec<McpToolSchema>>,
    mcp_pool: Arc<crate::mcp::McpConnectionPool>,
) -> crate::types::Result<Vec<Arc<dyn Tool>>> {
    let Some(mcp) = resolved.mcp_servers().get(server_id).cloned() else {
        return Ok(Vec::new());
    };
    let Some(tool_schemas) = mcp_schemas.get(server_id) else {
        return Ok(Vec::new());
    };
    let allowed_tools = resolved
        .agents()
        .get(agent_id)
        .and_then(|agent| agent.tools.get(capability_id))
        .and_then(|binding| binding.allowed_tools.as_ref());
    let _ = runtime;
    let mut seen = std::collections::HashSet::new();
    let mut tools = Vec::new();
    for tool in tool_schemas {
        if !mcp_inner_name_allowed(&tool.name) {
            tracing::warn!(
                tool = %tool.name,
                "dropping MCP tool that uses a reserved name"
            );
            continue;
        }
        if !seen.insert(tool.name.clone()) {
            tracing::warn!(
                server = %server_id,
                tool = %tool.name,
                "dropping duplicate MCP tool name"
            );
            continue;
        }
        if allowed_tools.is_some_and(|allowed| !allowed.contains(&tool.name)) {
            continue;
        }
        tools.push(Arc::new(McpTool::new(
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
        )) as Arc<dyn Tool>);
    }
    Ok(tools)
}

fn subagent_tools(
    runtime: &RuntimeHandle,
    agent_id: &str,
    depth: u32,
    parent_cancel: tokio_util::sync::CancellationToken,
    sessions: &Arc<SessionManager>,
    parent_session_id: &str,
    members: &[MemberSpec],
) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for member in members {
        match member.id.as_str() {
            "subagent_launch" => tools.push(Arc::new(SubagentLaunchTool::new(
                runtime.clone(),
                agent_id,
                depth,
                parent_cancel.clone(),
                Arc::clone(sessions),
                parent_session_id.to_string(),
            ))),
            "subagent_wait" => {
                tools.push(Arc::new(SubagentWaitTool::new(Arc::clone(sessions))))
            }
            "subagent_stop" => {
                tools.push(Arc::new(SubagentStopTool::new(Arc::clone(sessions))))
            }
            "subagent_send" => tools.push(Arc::new(SubagentSendTool::new(
                runtime.clone(),
                Arc::clone(sessions),
                depth,
            ))),
            "subagent_delete" => tools.push(Arc::new(SubagentDeleteTool::new(
                Arc::clone(sessions),
                Arc::clone(&runtime.subagent_hub),
            ))),
            "subagent_list" => {
                tools.push(Arc::new(SubagentListTool::new(Arc::clone(sessions))))
            }
            _ => {}
        }
    }
    tools
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
) -> crate::types::Result<Vec<Arc<dyn Tool>>> {
    let resolved = &runtime.resolved;
    let engines = &runtime.engine_manager;
    let workspace_engines = &runtime.workspace_engines;
    let mcp_pool = &runtime.mcp_pool;

    let catalog = capability_catalog(resolved);
    // A malformed custom definition must not make every turn unusable. The
    // catalog keeps the first owner for each member; log the diagnostics here
    // so the turn still builds with the remaining tools.
    for error in &catalog.errors {
        tracing::error!(error = %error, "capability catalog rejected one definition");
    }
    let selected = catalog.selected_for_agent(resolved, agent_id, depth);

    let mut mcp_schemas: HashMap<String, Vec<McpToolSchema>> = HashMap::new();
    let mcp_defs = resolved.mcp_servers();
    for capability in &selected {
        let CapabilitySource::Mcp { server_id } = &capability.source else {
            continue;
        };
        let Some(mcp_def) = mcp_defs.get(server_id) else {
            continue;
        };
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
            Err(error) => {
                tracing::warn!("Failed to start MCP server '{}': {}", server_id, error);
            }
        }
    }

    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for capability in selected {
        match &capability.source {
            CapabilitySource::Lsp => {
                tools.extend(lsp::all_code_tools(
                    workspace_engines.as_ref().clone(),
                    resolved.workspace_root().to_path_buf(),
                ));
            }
            CapabilitySource::Subagent => {
                tools.extend(subagent_tools(
                    runtime,
                    agent_id,
                    depth,
                    parent_cancel.clone(),
                    &sessions,
                    parent_session_id,
                    &capability.members,
                ));
            }
            CapabilitySource::Custom => {
                let code_suite = (capability.surface == crate::config::schema::CallSurface::Code)
                    .then(|| capability.id.strip_prefix("suite:"))
                    .flatten();
                for member in capability.member_ids() {
                    if let Some(tool) = custom_member(resolved, runtime, member) {
                        if let Some(suite) = code_suite {
                            tools.push(Arc::new(ExposedCodeTool::new(
                                tool,
                                format!("{suite}.{member}"),
                            )));
                        } else {
                            tools.push(tool);
                        }
                    }
                }
            }
            CapabilitySource::Mcp { server_id } => {
                tools.extend(mcp_members(
                    runtime,
                    resolved,
                    agent_id,
                    &capability.id,
                    server_id,
                    &mcp_schemas,
                    Arc::clone(&runtime.mcp_pool),
                )?);
            }
            CapabilitySource::Builtin => {
                for member in capability.member_ids() {
                    if let Some(tool) = instantiate_builtin_member(
                        runtime,
                        agent_id,
                        member,
                        engines,
                        workspace_engines,
                        Arc::clone(&runtime.ide),
                        &sessions,
                    ) {
                        tools.push(tool);
                    }
                }
            }
        }
    }

    // One projection from this caller's visible set. A missing host cannot
    // publish run_code or drop the code tools.
    Ok(assemble_projected_tools(tools, code_host))
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
pub fn builtin_code_specs() -> Vec<CodeFunctionSpec> {
    static SPECS: OnceLock<Vec<CodeFunctionSpec>> = OnceLock::new();
    SPECS
        .get_or_init(|| {
            builtin_code_surface_tools()
                .into_iter()
                .filter_map(|tool| FunctionSpec::from_tool(tool.as_ref()).ok()?.into_code())
                .collect()
        })
        .clone()
}

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
    // Browser tools are code-surface only; the client reads the loopback bridge
    // from the sidecar environment when a call is actually made.
    tools.extend(browser::all_code_tools());
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
            tools.insert(
                (*id).to_string(),
                AgentToolBinding {
                    enabled: true,
                    preset: Some(ToolPreset::All),
                    allowed_tools: None,
                },
            );
        }
        for id in core_none_tools() {
            tools.insert(
                (*id).to_string(),
                AgentToolBinding {
                    enabled: true,
                    preset: None,
                    allowed_tools: None,
                },
            );
        }
        tools
    }

    fn test_resolved(agent_tools: HashMap<String, AgentToolBinding>) -> ResolvedConfig {
        test_resolved_role(agent_tools, Default::default())
    }

    fn test_resolved_role(
        agent_tools: HashMap<String, AgentToolBinding>,
        role: crate::config::schema::AgentRole,
    ) -> ResolvedConfig {
        let mut global = GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            AgentProfile {
                role,
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
        .expect("build tool list")
    }

    #[test]
    fn catalog_definition_collision_does_not_block_other_tools() {
        let mut global = GlobalSettings::default();
        global.custom_tools.push(crate::config::schema::CustomToolDefinition {
            name: "hover".into(),
            description: String::new(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 30,
            rules: Vec::new(),
            surface: Some(crate::config::schema::CallSurface::Direct),
            suite: None,
            output_schema: None,
        });
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: default_bindings(),
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let tools = list_tools(&resolved, 0);
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert!(names.contains(&"read"));
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
        assert!(names.contains(&"subagent_delete"));
        // D2: session_search is bound and listed even when the semantic
        // engine is cold; the tool itself falls back to lexical search.
        assert!(names.contains(&"session_search"));
        assert!(names.contains(&"litecode_workspace"));
    }

    #[test]
    fn scope_table_controls_subagent_and_depth() {
        let primary = test_resolved(default_bindings());
        let primary_tools = list_tools(&primary, 0);
        let primary_names: Vec<&str> = primary_tools.iter().map(|tool| tool.name()).collect();
        assert!(primary_names.contains(&"subagent_launch"));
        assert!(primary_names.contains(&"plan"));

        let subagent = test_resolved_role(
            default_bindings(),
            crate::config::schema::AgentRole::Subagent,
        );
        let subagent_tools = list_tools(&subagent, 0);
        let subagent_names: Vec<&str> = subagent_tools
            .iter()
            .map(|tool| tool.name())
            .collect();
        assert!(!subagent_names.contains(&"subagent_launch"));
        assert!(!subagent_names.contains(&"plan"));
        assert!(subagent_names.contains(&"read"));

        let deep_primary_tools = list_tools(&primary, 1);
        let deep_primary_names: Vec<&str> = deep_primary_tools
            .iter()
            .map(|tool| tool.name())
            .collect();
        assert!(!deep_primary_names.contains(&"subagent_launch"));
        assert!(!deep_primary_names.contains(&"plan"));
        assert!(deep_primary_names.contains(&"read"));

        let hidden = test_resolved_role(
            default_bindings(),
            crate::config::schema::AgentRole::Hidden,
        );
        assert!(list_tools(&hidden, 0).is_empty());
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
                output_schema: None,
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
                surface: Some(crate::config::schema::CallSurface::Direct),
                suite: None,
            }
        }
    }

    #[test]
    fn custom_suite_is_one_binding_and_instantiates_all_members() {
        use crate::config::schema::{CustomToolDefinition, ToolSchema};

        fn custom(name: &str) -> CustomToolDefinition {
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
                surface: Some(crate::config::schema::CallSurface::Direct),
                suite: Some("deck".into()),
                output_schema: None,
            }
        }

        let mut bindings = default_bindings();
        bindings.insert(
            "suite:deck".into(),
            AgentToolBinding {
                enabled: true,
                preset: None,
                allowed_tools: None,
            },
        );
        let mut global = GlobalSettings::default();
        global.custom_suites.insert(
            "deck".into(),
            crate::config::schema::CustomSuiteDefinition {
                surface: crate::config::schema::CallSurface::Direct,
                label: "Deck".into(),
                rules: Vec::new(),
            },
        );
        global.custom_tools = vec![custom("suite_a"), custom("suite_b")];
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let suite_tools = list_tools(&resolved, 0);
        let names: Vec<&str> = suite_tools.iter().map(|tool| tool.name()).collect();
        assert!(names.contains(&"suite_a"));
        assert!(names.contains(&"suite_b"));
        assert!(!names.contains(&"suite:deck"));
    }

    #[test]
    fn code_suite_exposes_namespaced_members_even_with_one_member() {
        use crate::config::schema::{CallSurface, CustomSuiteDefinition, CustomToolDefinition, ToolSchema};

        let mut bindings = default_bindings();
        bindings.insert(
            "suite:deck".into(),
            AgentToolBinding {
                enabled: true,
                preset: None,
                allowed_tools: None,
            },
        );
        let mut global = GlobalSettings::default();
        global.custom_suites.insert(
            "deck".into(),
            CustomSuiteDefinition {
                surface: CallSurface::Code,
                label: "Deck".into(),
                rules: Vec::new(),
            },
        );
        global.custom_tools.push(CustomToolDefinition {
            name: "only".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 30,
            rules: Vec::new(),
            surface: None,
            suite: Some("deck".into()),
            output_schema: Some(serde_json::json!({
                "type": "object", "properties": {}, "required": []
            })),
        });
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let names: Vec<String> = list_tools(&resolved, 0)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert!(
            names.contains(&"deck.only".to_string()),
            "names: {names:?}"
        );
        assert!(!names.contains(&"only".to_string()), "names: {names:?}");
        assert!(!names.contains(&"deck".to_string()), "names: {names:?}");
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
                preset: None,
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
