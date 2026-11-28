use std::collections::HashMap;
use std::sync::Arc;

use crate::config::global_db::tools::{
    non_transport_name_rejection, reserved_command_name_rejection,
};
use crate::config::resolved::ResolvedConfig;
use crate::config::schema::AgentProfile;
use crate::engines::WorkspaceEngines;
use crate::ide_base::IdeBaseHandle;
use crate::mcp::McpToolSchema;
use crate::optional::EngineManager;
use crate::runtime::RuntimeHandle;
use crate::session::SessionService;
use crate::session::manager::SessionManager;
use crate::tool::capability::{CapabilitySource, MemberSpec, capability_catalog};
use crate::tool::project::project_tools;
use crate::tool::trait_::Tool;
#[cfg(test)]
use crate::tools::bot::{BotCreateTool, BotListTool, BotSendTool};
use crate::tools::{
    apply_edits::ApplyEditsTool,
    ask_user::AskUserTool,
    browser,
    code::{CodeTool, CodeToolHost},
    code_search::CodeSearchTool,
    custom::CustomTool,
    exec_command::ExecCommandTool,
    glob::GlobTool,
    grep::GrepTool,
    knowledge::{KnowledgeCheckTool, KnowledgeCreateTool, KnowledgeListTool, KnowledgeRefsTool},
    litecode_workspace::{WorkspaceHelpTool, WorkspaceRefreshTool, WorkspaceStatusTool},
    lsp,
    mcp_tool::McpTool,
    memory::{MemoryDeleteTool, MemoryListTool, MemoryUpsertTool},
    plan::{PlanCreateTool, PlanFinishTool},
    read::ReadTool,
    session_search::SessionSearchTool,
    subagent::{
        SubagentLaunchTool, SubagentListTool, SubagentSendTool, SubagentStopTool,
        SubagentWaitTool,
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
        "exec_command" => Arc::new(ExecCommandTool::new(Arc::clone(&ide.terminal.jobs))),
        "read" => Arc::new(ReadTool::with_ide(Arc::clone(&ide))),
        "write" => Arc::new(WriteTool::with_ide(Arc::clone(&ide))),
        "apply_edits" => Arc::new(ApplyEditsTool::with_ide(Arc::clone(&ide))),
        "grep" => Arc::new(GrepTool),
        "knowledge.list" => Arc::new(KnowledgeListTool::new()),
        "knowledge.check" => Arc::new(KnowledgeCheckTool::new()),
        "knowledge.refs" => Arc::new(KnowledgeRefsTool::new()),
        "knowledge.create" => Arc::new(KnowledgeCreateTool::new(Arc::clone(&ide))),
        "glob" => Arc::new(GlobTool),
        "todo.write" => Arc::new(TodoWriteTool::new(Arc::clone(&sessions))),
        "memory.list" => Arc::new(MemoryListTool::new(Arc::clone(&sessions))),
        "memory.upsert" => Arc::new(MemoryUpsertTool::new(Arc::clone(&sessions))),
        "memory.delete" => Arc::new(MemoryDeleteTool::new(Arc::clone(&sessions))),
        "ask_user" => Arc::new(AskUserTool::new()),
        "plan.create" => Arc::new(PlanCreateTool::new(Arc::clone(&sessions))),
        "plan.finish" => Arc::new(PlanFinishTool::new(Arc::clone(&sessions))),
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
        "webfetch" => Arc::new(
            WebFetchTool::new(engines.webfetch_client())
                .with_warm_hook(engines.warm_hook("webfetch")),
        ),
        "websearch" => Arc::new(
            WebSearchTool::new(engines.websearch_client(), engines.websearch_endpoint())
                .with_warm_hook(engines.warm_hook("websearch")),
        ),
        "code_search" => Arc::new(CodeSearchTool::new(workspace_engines.clone())),
        "session_search" => Arc::new(SessionSearchTool::with_owners(
            workspace_engines.clone(),
            Arc::new(SessionService::new(Arc::clone(sessions))),
        )),
        "workspace.help" => Arc::new(WorkspaceHelpTool::new()),
        "workspace.refresh" => Arc::new(WorkspaceRefreshTool::new(runtime.clone())),
        "workspace.status" => {
            Arc::new(WorkspaceStatusTool::new(runtime.clone(), agent_id.to_string()))
        }
        _ => return None,
    };
    Some(tool)
}

macro_rules! delegate_tool_inner {
    () => {
        fn dynamic_code_spec(&self) -> Option<crate::tool::DynamicCodeSpec> {
            self.inner.dynamic_code_spec()
        }

        fn dynamic_sdk_note(&self) -> Option<String> {
            self.inner.dynamic_sdk_note()
        }

        fn prepare_input(&self, input: &mut serde_json::Value) {
            self.inner.prepare_input(input);
        }

        fn schema_arguments<'a>(
            &self,
            input: &'a serde_json::Value,
        ) -> std::borrow::Cow<'a, serde_json::Value> {
            self.inner.schema_arguments(input)
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
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::types::ToolCallResult> + Send + '_>,
        > {
            self.inner.call_async(input)
        }

        fn execute(
            &self,
            input: serde_json::Value,
            execution: crate::tool::trait_::ToolExecutionContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::types::ToolCallResult> + Send + '_>,
        > {
            self.inner.execute(input, execution)
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

        fn releases_lock_after_spawn(&self) -> bool {
            self.inner.releases_lock_after_spawn()
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
    };
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

    fn code_bindings(&self) -> Option<Arc<[String]>> {
        self.inner.code_bindings()
    }

    fn is_code_transport(&self) -> bool {
        self.inner.is_code_transport()
    }

    fn transport_spec(
        &self,
        ctx: &crate::context_pipeline::Context,
    ) -> Option<crate::tool::TransportToolSpec> {
        self.inner.transport_spec(ctx)
    }

    fn max_result_size(&self) -> usize {
        self.inner.max_result_size()
    }

    delegate_tool_inner!();
}

fn custom_member(
    resolved: &ResolvedConfig,
    member: &str,
) -> Option<Arc<dyn Tool>> {
    resolved
        .custom_tools()
        .iter()
        .find(|definition| definition.name == member)
        .map(|definition| Arc::new(CustomTool::new(definition.clone())) as Arc<dyn Tool>)
}

fn mcp_members(
    runtime: &RuntimeHandle,
    resolved: &ResolvedConfig,
    profile: &crate::config::schema::AgentProfile,
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
    let allowed_tools = profile
        .tools
        .get(capability_id)
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
    parent_profile: &AgentProfile,
    members: &[MemberSpec],
) -> Vec<Arc<dyn Tool>> {
    let service = Arc::new(SessionService::new(Arc::clone(sessions)));
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for member in members {
        match member.id.as_str() {
            "subagent.launch" => tools.push(Arc::new(
                SubagentLaunchTool::new(
                    runtime.clone(),
                    agent_id,
                    depth,
                    parent_cancel.clone(),
                    Arc::clone(&service),
                    parent_session_id.to_string(),
                )
                .with_profile(parent_profile.clone()),
            )),
            "subagent.wait" => tools.push(Arc::new(SubagentWaitTool::new(Arc::clone(&service)))),
            "subagent.stop" => tools.push(Arc::new(SubagentStopTool::new(Arc::clone(&service)))),
            "subagent.send" => tools.push(Arc::new(SubagentSendTool::new(
                runtime.clone(),
                Arc::clone(&service),
                depth,
            ))),
            "subagent.list" => tools.push(Arc::new(SubagentListTool::new(Arc::clone(&service)))),
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
    // One resolution feeds both capability selection and bot-tool injection.
    let resolution = crate::bot::resolve_agent_for_session(
        &runtime.resolved,
        runtime.bots().map(Arc::as_ref),
        parent_session_id,
        agent_id,
    );
    build_tool_list_with_resolution(
        runtime,
        resolution,
        agent_id,
        depth,
        parent_cancel,
        parent_session_id,
        sessions,
        code_host,
    )
    .await
}

/// Build from an already-captured turn snapshot. A bot body edited after the
/// turn started must not change the current tool set.
#[allow(clippy::too_many_arguments)]
pub async fn build_tool_list_with_resolution(
    runtime: &RuntimeHandle,
    resolution: Option<crate::bot::AgentResolution>,
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
    let (profile, bot) = match resolution {
        Some(resolution) => (resolution.profile, resolution.bot),
        None => (crate::config::schema::AgentProfile::default(), None),
    };
    let selected = catalog.selected_for_profile(&profile, depth);

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
                    &profile,
                    &capability.members,
                ));
            }
            CapabilitySource::Custom => {
                let code_suite = capability.id.strip_prefix("suite:");
                for member in capability.member_ids() {
                    if let Some(tool) = custom_member(resolved, member) {
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
                    &profile,
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
                    } else {
                        // A card member with no constructor is a catalog/registry
                        // drift: the model would silently lose the tool. The
                        // contract test in `tool::code_contract` pins the same set.
                        tracing::warn!(
                            capability = %capability.id,
                            member = %member,
                            "builtin capability member has no runtime constructor; it is missing from the tool surface"
                        );
                    }
                }
            }
        }
    }

    // Bot tools are a product-layer capability, not part of the configurable
    // catalog: they exist only for a session that a bot owns, and are projected
    // into run_code so bots obey the same code-only surface.
    if let Some(_bot) = bot
        && let Some(bots) = runtime.bots()
    {
        for tool in crate::tools::bot::bot_tools(Arc::clone(bots)) {
            if let Some(index) = tools
                .iter()
                .position(|existing| existing.name() == tool.name())
            {
                // Bot capability is product-owned inside a bot session; a
                // same-named custom tool must not shadow the real one.
                tracing::warn!(
                    tool = tool.name(),
                    "bot tool name is already bound; replacing the duplicate"
                );
                tools[index] = tool;
                continue;
            }
            tools.push(tool);
        }
    }

    // One projection from this caller's visible set into `run_code`. A missing
    // host cannot publish `run_code` or drop the code tools.
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
            "code tool removed from run_code surface"
        );
    }
    let names = projected.whitelist();
    if projected.code_tools.is_empty() {
        return Vec::new();
    }
    let Some(host) = host else {
        tracing::error!("code tools unavailable: no code host to bind run_code");
        return Vec::new();
    };
    let Some(surface) = projected.code_surface else {
        tracing::error!("code tools unavailable: JS contract surface was not assembled");
        return Vec::new();
    };
    tracing::info!("code mode active: bundling {names:?}");
    vec![Arc::new(CodeTool::from_surface(
        projected.code_tools,
        surface,
        host,
    ))]
}

/// MCP publishes `mcp_{server}.{inner}`. The inner name still must not be a
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

#[cfg(test)]
pub fn builtin_code_surface_tools() -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadTool::default()),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(ApplyEditsTool::new()),
        Arc::new(WriteTool::new()),
        Arc::new(ExecCommandTool::new(Arc::new(
            crate::terminal::AgentJobRegistry::new(),
        ))),
        // Network tools are Code-only (cold stubs for settings admission).
        Arc::new(WebFetchTool::new(Arc::new(std::sync::RwLock::new(None)))),
        Arc::new(WebSearchTool::new(
            Arc::new(std::sync::RwLock::new(None)),
            Arc::new(std::sync::RwLock::new(None)),
        )),
        // Session-scoped memory is Code-only; cold tools carry contracts for
        // settings admission and the generated SDK.
        Arc::new(MemoryListTool::cold()),
        Arc::new(MemoryUpsertTool::cold()),
        Arc::new(MemoryDeleteTool::cold()),
        Arc::new(TodoWriteTool::cold()),
        Arc::new(PlanCreateTool::cold()),
        Arc::new(PlanFinishTool::cold()),
        Arc::new(AskUserTool::new()),
        Arc::new(KnowledgeListTool::new()),
        Arc::new(KnowledgeCheckTool::new()),
        Arc::new(KnowledgeRefsTool::new()),
        Arc::new(KnowledgeCreateTool::cold()),
        Arc::new(WorkspaceHelpTool::new()),
        Arc::new(WorkspaceRefreshTool::cold()),
        Arc::new(WorkspaceStatusTool::cold()),
        Arc::new(SubagentLaunchTool::cold()),
        Arc::new(SubagentWaitTool::cold()),
        Arc::new(SubagentSendTool::cold()),
        Arc::new(SubagentStopTool::cold()),
        Arc::new(SubagentListTool::cold()),
        Arc::new(BotCreateTool::cold()),
        Arc::new(BotListTool::cold()),
        Arc::new(BotSendTool::cold()),
        // Engine-backed, but a code-surface tool first. Its cold adapter is
        // used only for admission/schema rendering; live calls get the bound
        // WorkspaceEngines adapter from capability instantiation.
        Arc::new(CodeSearchTool::new(WorkspaceEngines::new())),
        // Engine-backed and Code-only; its cold adapter is used for admission
        // and for the generated SDK contract.
        Arc::new(SessionSearchTool::new(WorkspaceEngines::new())),
    ];
    // The seven LSP navigation tools are Code-surface too (cold
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

    fn test_host(resolved: &ResolvedConfig, depth: u32) -> Arc<crate::tools::code::CodeToolHost> {
        let permission =
            crate::permission::PermissionEngine::resolver(resolved.clone(), "default", depth);
        Arc::new(crate::tools::code::CodeToolHost::new(
            permission,
            dummy_ctx(),
            "default",
            crate::permission::deny_permission_sink(),
            std::env::temp_dir().join("litecode-registry-code-host"),
            crate::tool::write_lock::process_write_lock(),
            None,
        ))
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
    fn bot_tools_are_injected_only_for_a_bot_session() {
        let resolved = test_resolved(default_bindings());
        let sessions = dummy_sessions();
        let service = Arc::new(crate::session::SessionService::new(Arc::clone(&sessions)));
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = Arc::new(crate::config::SettingsWriter::with_path(
            dir.path().join("settings.db"),
            Arc::new(crate::config::TurnGuard::new()),
        ));
        settings
            .write_agent(
                "default",
                crate::config::schema::AgentProfile {
                    role: crate::config::schema::AgentRole::Primary,
                    ..crate::config::schema::AgentProfile::default()
                },
            )
            .expect("template");
        let bots = Arc::new(
            crate::bot::BotService::new(
                Arc::clone(&service),
                Arc::clone(&settings),
                None,
                "/tmp",
                dir.path().join("bots.json"),
            )
            .expect("bot service"),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let bot = rt
            .block_on(bots.create_user_bot("Helper", "", crate::bot::BotBody::default()))
            .expect("bot");

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
        runtime.set_bots(Arc::clone(&bots));

        let bot_tools = rt
            .block_on(build_tool_list(
                &runtime,
                "default",
                0,
                tokio_util::sync::CancellationToken::new(),
                &bot.session_id,
                Arc::clone(&sessions),
                Some(test_host(&resolved, 0)),
            ))
            .expect("bot tool list");
        let bot_bindings: Vec<String> = bot_tools[0]
            .code_bindings()
            .expect("bot code_bindings")
            .iter()
            .cloned()
            .collect();
        for name in ["bot.create", "bot.list", "bot.send"] {
            assert!(
                bot_bindings.iter().any(|tool| tool == name),
                "bot session is missing {name} in {bot_bindings:?}"
            );
        }

        // A bot-created bot is not depth-limited: it gets all three tools too.
        let child = rt
            .block_on(bots.create_from_bot(&bot.session_id, "Child", ""))
            .expect("child bot");
        let child_tools = rt
            .block_on(build_tool_list(
                &runtime,
                &child.id,
                0,
                tokio_util::sync::CancellationToken::new(),
                &child.session_id,
                Arc::clone(&sessions),
                Some(test_host(&resolved, 0)),
            ))
            .expect("child bot tool list");
        let child_bindings: Vec<String> = child_tools[0]
            .code_bindings()
            .expect("child code_bindings")
            .iter()
            .cloned()
            .collect();
        for name in ["bot.create", "bot.list", "bot.send"] {
            assert!(
                child_bindings.iter().any(|tool| tool == name),
                "a bot-created bot is missing {name} in {child_bindings:?}"
            );
        }

        let normal_tools = rt
            .block_on(build_tool_list(
                &runtime,
                "default",
                0,
                tokio_util::sync::CancellationToken::new(),
                "normal-session",
                sessions,
                Some(test_host(&resolved, 0)),
            ))
            .expect("normal tool list");
        let normal_bindings = normal_tools[0]
            .code_bindings()
            .expect("normal code_bindings");
        assert!(
            normal_bindings
                .iter()
                .all(|tool| !tool.starts_with("bot.")),
            "normal session received a bot tool: {normal_bindings:?}"
        );
    }

    #[test]
    fn turn_resolution_snapshot_keeps_the_old_bot_tools() {
        let resolved = test_resolved(default_bindings());
        let sessions = dummy_sessions();
        let service = Arc::new(crate::session::SessionService::new(Arc::clone(&sessions)));
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = Arc::new(crate::config::SettingsWriter::with_path(
            dir.path().join("settings.db"),
            Arc::new(crate::config::TurnGuard::new()),
        ));
        settings
            .write_agent(
                "default",
                AgentProfile {
                    role: crate::config::schema::AgentRole::Primary,
                    ..AgentProfile::default()
                },
            )
            .expect("template");
        let bots = Arc::new(
            crate::bot::BotService::new(
                Arc::clone(&service),
                Arc::clone(&settings),
                None,
                "/tmp",
                dir.path().join("bots.json"),
            )
            .expect("bot service"),
        );
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let bot = rt
            .block_on(bots.create_user_bot("Snapshot", "", crate::bot::BotBody::default()))
            .expect("bot");
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
        runtime.set_bots(Arc::clone(&bots));
        let old = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("old resolution");
        let old_tools = rt
            .block_on(build_tool_list_with_resolution(
                &runtime,
                Some(old.clone()),
                &bot.id,
                0,
                tokio_util::sync::CancellationToken::new(),
                &bot.session_id,
                Arc::clone(&sessions),
                Some(test_host(&resolved, 0)),
            ))
            .expect("old tool list");
        let has_binding = |tools: &[Arc<dyn Tool>], target: &str| {
            tools
                .iter()
                .filter_map(|tool| tool.code_bindings())
                .any(|bindings| bindings.iter().any(|name| name == target))
        };
        assert!(!has_binding(&old_tools, "hover"));

        let mut body = crate::bot::BotBody::default();
        body.tools.insert(
            "lsp".into(),
            AgentToolBinding {
                enabled: true,
                preset: Some(ToolPreset::All),
                allowed_tools: None,
            },
        );
        rt.block_on(bots.update(
            &bot.id,
            crate::bot::BotUpdate {
                body: Some(body),
                ..crate::bot::BotUpdate::default()
            },
        ))
        .expect("update");
        let new = crate::bot::resolve_agent_for_session(
            &resolved,
            Some(bots.as_ref()),
            &bot.session_id,
            &bot.id,
        )
        .expect("new resolution");
        assert!(
            new.profile.tools.contains_key("lsp"),
            "new profile tools: {:?}",
            new.profile.tools
        );
        let old_again = rt
            .block_on(build_tool_list_with_resolution(
                &runtime,
                Some(old),
                &bot.id,
                0,
                tokio_util::sync::CancellationToken::new(),
                &bot.session_id,
                Arc::clone(&sessions),
                Some(test_host(&resolved, 0)),
            ))
            .expect("old rebuilt");
        assert!(!has_binding(&old_again, "hover"));
        let new_tools = rt
            .block_on(build_tool_list_with_resolution(
                &runtime,
                Some(new),
                &bot.id,
                0,
                tokio_util::sync::CancellationToken::new(),
                &bot.session_id,
                sessions,
                Some(test_host(&resolved, 0)),
            ))
            .expect("new tool list");
        assert!(
            has_binding(&new_tools, "hover"),
            "new tools: {:?}",
            new_tools.iter().map(|tool| tool.name()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn catalog_definition_collision_does_not_block_other_tools() {
        let mut global = GlobalSettings::default();
        global
            .custom_tools
            .push(crate::config::schema::CustomToolDefinition {
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
                suite: None,
            });
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: default_bindings(),
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let tools = list_tools_with(&resolved, 0, Some(test_host(&resolved, 0)));
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(names, ["run_code"]);
        let sdk = tools[0]
            .transport_spec(&dummy_ctx())
            .unwrap()
            .description;
        assert!(sdk.contains("read(args: {"));
    }

    #[test]
    fn colliding_custom_name_cannot_drop_the_builtin_tool() {
        let mut global = GlobalSettings::default();
        global
            .custom_tools
            .push(crate::config::schema::CustomToolDefinition {
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
                suite: None,
            });
        let mut bindings = default_bindings();
        for id in ["lsp", "hover"] {
            bindings.insert(
                id.into(),
                AgentToolBinding {
                    enabled: true,
                    preset: None,
                    allowed_tools: None,
                },
            );
        }
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let tools = list_tools_with(&resolved, 0, Some(test_host(&resolved, 0)));
        let sdk = tools[0]
            .transport_spec(&dummy_ctx())
            .unwrap()
            .description;
        assert!(
            sdk.contains("hover(args: RangeInput)"),
            "builtin LSP hover must keep its name: {sdk}"
        );
        assert!(
            !sdk.contains("External tool: hover"),
            "the colliding custom tool must not reach the surface: {sdk}"
        );
    }

    #[test]
    fn core_bindings_produce_full_code_only_transport() {
        let resolved = test_resolved(default_bindings());
        let host = test_host(&resolved, 0);
        let tools = list_tools_with(&resolved, 0, Some(host));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["run_code"], "only run_code on transport: {names:?}");

        let bindings: Vec<String> = tools[0]
            .code_bindings()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        for absent in [
            "bash",
            "wait_shell",
            "kill_shell",
            "subagent_delete",
            "subagent.delete",
        ] {
            assert!(!bindings.iter().any(|b| b == absent), "{absent} present");
        }
        for code in [
            "exec_command",
            "read",
            "glob",
            "grep",
            "memory.list",
            "todo.write",
            "plan.create",
            "plan.finish",
            "ask_user",
            "knowledge.list",
            "knowledge.check",
            "workspace.help",
            "workspace.refresh",
            "subagent.launch",
            "subagent.send",
            "subagent.wait",
            "subagent.stop",
            "subagent.list",
        ] {
            assert!(
                bindings.iter().any(|b| b == code),
                "missing code binding {code} in {bindings:?}"
            );
        }

        let sdk = tools[0]
            .transport_spec(&dummy_ctx())
            .unwrap()
            .description;
        for needle in ["exec_command(args: {", "read(args: {", "upsert(args: {"] {
            assert!(sdk.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn builtin_code_surface_contains_expected_tools() {
        let all = builtin_code_surface_tools();
        let code_names: std::collections::HashSet<String> =
            all.iter().map(|tool| tool.name().to_string()).collect();
        assert!(
            code_names.contains("code_search"),
            "code_search must be a code tool"
        );
        assert!(
            code_names.contains("session_search"),
            "session_search must be a code tool"
        );
        let projected = project_tools(all);
        assert!(
            projected
                .code_tools
                .iter()
                .any(|tool| tool.name() == "code_search"),
            "code_search must survive code projection"
        );
        assert!(
            projected
                .code_tools
                .iter()
                .any(|tool| tool.name() == "session_search"),
            "session_search must survive code projection"
        );
    }

    #[test]
    fn memory_tools_are_on_the_code_surface() {
        let tools = builtin_code_surface_tools();
        for name in ["memory.list", "memory.upsert", "memory.delete"] {
            assert!(
                tools.iter().any(|tool| tool.name() == name),
                "missing code tool {name}"
            );
        }
    }

    #[test]
    fn scope_table_controls_subagent_and_depth() {
        let primary = test_resolved(default_bindings());
        let primary_tools = list_tools_with(&primary, 0, Some(test_host(&primary, 0)));
        assert_eq!(primary_tools.len(), 1);
        let primary_bindings = primary_tools[0].code_bindings().unwrap();
        assert!(primary_bindings.iter().any(|b| b == "subagent.launch"));
        assert!(primary_bindings.iter().any(|b| b == "plan.create"));

        let subagent = test_resolved_role(
            default_bindings(),
            crate::config::schema::AgentRole::Subagent,
        );
        let subagent_tools = list_tools_with(&subagent, 0, Some(test_host(&subagent, 0)));
        let subagent_names: Vec<&str> = subagent_tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(subagent_names, ["run_code"]);
        let subagent_bindings = subagent_tools[0].code_bindings().unwrap();
        assert!(!subagent_bindings.iter().any(|b| b == "subagent.launch"));
        assert!(!subagent_bindings.iter().any(|b| b == "plan.create"));
        assert!(!subagent_bindings.iter().any(|b| b == "memory.list"));
        assert!(subagent_bindings.iter().any(|b| b == "read"));

        let deep_primary_tools = list_tools_with(&primary, 1, Some(test_host(&primary, 1)));
        assert_eq!(deep_primary_tools.len(), 1);
        let deep_bindings = deep_primary_tools[0].code_bindings().unwrap();
        assert!(!deep_bindings.iter().any(|b| b == "subagent.launch"));
        assert!(!deep_bindings.iter().any(|b| b == "plan.create"));
        assert!(deep_bindings.iter().any(|b| b == "read"));
    }

    #[test]
    fn custom_tools_cannot_take_command_or_retired_shell_names() {
        use crate::config::schema::{CustomToolDefinition, ToolSchema};
        use crate::config::settings_writer::validate_custom_definition;

        for name in [
            "run",
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
                suite: None,
            }
        }
    }

    #[test]
    fn custom_suite_is_one_binding_and_instantiates_namespaced_members() {
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
                suite: Some("deck".into()),
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
        let tools = list_tools_with(&resolved, 0, Some(test_host(&resolved, 0)));
        assert_eq!(tools.len(), 1);
        let code_bindings: Vec<String> = tools[0].code_bindings().unwrap().iter().cloned().collect();
        assert!(code_bindings.contains(&"deck.suite_a".to_string()));
        assert!(code_bindings.contains(&"deck.suite_b".to_string()));
        assert!(!code_bindings.contains(&"suite:deck".to_string()));
    }

    #[test]
    fn code_suite_exposes_namespaced_members_even_with_one_member() {
        use crate::config::schema::{
            CustomSuiteDefinition, CustomToolDefinition, ToolSchema,
        };

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
            suite: Some("deck".into()),
        });
        global.agents.insert(
            "default".into(),
            AgentProfile {
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = resolve_without_catalog(global, WorkspaceState::new("/tmp"));
        let tools = list_tools_with(&resolved, 0, Some(test_host(&resolved, 0)));
        let names: Vec<String> = tools.iter().map(|tool| tool.name().to_string()).collect();
        assert_eq!(names, ["run_code"], "names: {names:?}");
        let transport = &tools[0];
        let bindings: Vec<String> = transport.code_bindings().unwrap().iter().cloned().collect();
        assert!(
            bindings.contains(&"deck.only".to_string()),
            "bindings: {bindings:?}"
        );
        let sdk = transport.transport_spec(&dummy_ctx()).unwrap().description;
        assert!(sdk.contains("interface DeckTools"), "{sdk}");
        assert!(sdk.contains("deck: DeckTools"), "{sdk}");
        assert!(sdk.contains("only(args:"), "{sdk}");
    }

    #[test]
    fn mcp_inner_names_cannot_use_reserved_command_names() {
        for name in [
            "run",
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

    /// In extreme code-only mode, `run_code` is the sole outer transport tool.
    const TRANSPORT_TOOLS: &[&str] = &["run_code"];

    #[test]
    fn transport_spec_is_only_run_code_and_has_object_schema() {
        let resolved = test_resolved(default_bindings());
        let ctx = dummy_ctx();
        let tools = list_tools_with(&resolved, 0, Some(test_host(&resolved, 0)));
        let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
        assert_eq!(names, TRANSPORT_TOOLS);
        let spec = tools[0].transport_spec(&ctx).expect("run_code transport spec");
        assert_eq!(spec.input_schema["type"], "object");
        assert_eq!(spec.input_schema["additionalProperties"], serde_json::json!(false));
    }

    #[test]
    fn code_gate_projects_every_depth_from_that_callers_visible_set() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tools::code::CodeToolHost;

        let resolved = test_resolved(default_bindings());
        let closed = list_tools(&resolved, 1);
        assert!(closed.is_empty(), "without host, code tools must not publish");

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
        assert_eq!(child_names, ["run_code"]);
        let child_sdk = child[0]
            .transport_spec(&dummy_ctx())
            .unwrap()
            .description;
        assert!(child_sdk.contains("declare const tools"));
        for needle in ["exec_command(args: {", "apply_edits(args: {"] {
            assert!(child_sdk.contains(needle), "missing {needle}");
        }
        assert!(!child_sdk.contains("write_stdin"));
        assert!(!child_sdk.contains("bash"));
        assert!(!child_sdk.contains("wait_shell"));
        assert!(!child_sdk.contains("kill_shell"));

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
        let narrow_sdk = narrow_tools[0]
            .transport_spec(&dummy_ctx())
            .unwrap()
            .description;
        for needle in [
            "read(args: {",
            "glob(args: {",
            "write(args: {",
        ] {
            assert!(narrow_sdk.contains(needle), "missing {needle}");
        }
        assert!(!narrow_sdk.contains("grep(args: "));
        assert_ne!(child_sdk, narrow_sdk);
        let whitelist = narrow_tools[0].code_bindings().unwrap();
        let names: Vec<&str> = whitelist.iter().map(String::as_str).collect();
        assert_eq!(
            names,
            [
                "apply_edits",
                "exec_command",
                "glob",
                "knowledge.check",
                "knowledge.create",
                "knowledge.list",
                "knowledge.refs",
                "read",
                "session_search",
                "workspace.help",
                "workspace.refresh",
                "workspace.status",
                "write"
            ]
        );
        assert!(narrow_sdk.contains("exec_command(args: {"));
        assert!(!narrow_sdk.contains("write_stdin"));
        assert!(narrow_sdk.contains("apply_edits(args: {"));
    }

    #[test]
    fn webfetch_websearch_project_to_code_only() {
        use crate::permission::{PermissionEngine, deny_permission_sink};
        use crate::tool::project::project_tools;
        use crate::tools::code::CodeToolHost;

        assert!(!WebFetchTool::new(Arc::new(std::sync::RwLock::new(None))).is_code_transport());
        assert!(!WebSearchTool::new(
            Arc::new(std::sync::RwLock::new(None)),
            Arc::new(std::sync::RwLock::new(None)),
        )
        .is_code_transport());

        let projected = project_tools(vec![
            Arc::new(WebFetchTool::new(Arc::new(std::sync::RwLock::new(None)))) as Arc<dyn Tool>,
            Arc::new(WebSearchTool::new(
                Arc::new(std::sync::RwLock::new(None)),
                Arc::new(std::sync::RwLock::new(None)),
            )) as Arc<dyn Tool>,
        ]);
        assert!(
            projected.whitelist().iter().any(|n| n == "webfetch")
                && projected.whitelist().iter().any(|n| n == "websearch"),
            "Code whitelist must admit both: {:?}",
            projected.whitelist()
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
        assert_eq!(top, ["run_code"], "only transport exposed: {top:?}");
        let bindings = assembled[0].code_bindings().expect("run_code bindings");
        assert!(bindings.iter().any(|n| n == "webfetch"));
        assert!(bindings.iter().any(|n| n == "websearch"));
    }
}
