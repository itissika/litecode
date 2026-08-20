pub mod bash_auto_turn;
pub mod context;
pub mod exec;
pub mod llm_resolve;
pub mod observer;
pub(crate) mod phase;
pub mod provider_registry;

pub use context::RuntimeContext;
pub use llm_resolve::{
    TurnLlmBinding, project_llm_input_for_model, resolve_session_llm,
    validate_llm_input_capabilities,
};
pub use provider_registry::ProviderRegistry;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use tokio::sync::mpsc;

use crate::agent::{AgentDeps, TurnOutcome};
use crate::config::bridge::agent_config_for;
use crate::config::schema::ADAPTER_DEEPSEEK_RESPONSES;
use crate::config::workspace::set_runtime_paths;
use crate::config::{AgentConfig, ConfigManager, ResolvedConfig, WorkspaceState, log_filter};
use crate::context_pipeline::{Context, build_context};
use crate::context_pipeline::{ContextPipeline, ProviderPromptBaseline};
use crate::engines::WorkspaceEngines;
use crate::hook::{HookDispatcher, HookRegistry, apply_hook_output};
use crate::ide_base::IdeBaseHandle;
use crate::llm::LlmProvider;
use crate::mcp::McpConnectionPool;
use crate::optional::EngineManager;
use crate::permission::{CancellingPermissionSink, PermissionEngine, PermissionSink};
use crate::runtime::observer::{
    ChannelObserver, InternalEnvelope, InternalEvent, RuntimeObserver, TurnEndReason, TurnPhase,
    TurnTokenStats,
};
use crate::session::manager::SessionManager;
use crate::session::snapshot;
use crate::session::working::{WorkingRow, align_working, project_items};
use crate::tool::ToolPipeline;
use crate::tool::output;
use crate::tool::registry::build_tool_list;
use crate::types::{LitecodeError, Result, item_text_preview, user_text};

/// Shared runtime configuration for CLI and serve (Phase 4 R4.4).
pub struct RuntimeHandle {
    pub resolved: ResolvedConfig,
    provider_registry: Mutex<ProviderRegistry>,
    pub agent_name: String,
    desired_primary_agent: String,
    pub workspace: WorkspaceState,
    pub engine_manager: Arc<EngineManager>,
    pub workspace_engines: Arc<WorkspaceEngines>,
    /// Shared editor-facing services consumed by workspace-scoped Agent tools.
    pub ide: Arc<IdeBaseHandle>,
    /// Process-level MCP stdio connections (start / restart / stop).
    pub mcp_pool: Arc<McpConnectionPool>,
    global_db_path: PathBuf,
    settings_revision: Arc<AtomicU64>,
    loaded_revision: Arc<AtomicU64>,
    /// Test-only: when set, `build_runtime` uses this provider instead of registry resolution.
    test_llm_override: Option<Arc<dyn LlmProvider>>,
}

impl RuntimeHandle {
    /// Construct a runtime against the process-owned shared editor-service graph.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resolved: ResolvedConfig,
        agent_name: String,
        workspace: WorkspaceState,
        engine_manager: Arc<EngineManager>,
        workspace_engines: Arc<WorkspaceEngines>,
        ide: Arc<IdeBaseHandle>,
        settings_revision: Arc<AtomicU64>,
        global_db_path: impl Into<PathBuf>,
    ) -> Self {
        set_runtime_paths(workspace.paths.clone());
        let loaded = settings_revision.load(Ordering::Acquire);
        let desired_primary_agent = agent_name.clone();
        Self {
            resolved,
            provider_registry: Mutex::new(ProviderRegistry::new()),
            agent_name: desired_primary_agent.clone(),
            desired_primary_agent,
            workspace,
            engine_manager,
            workspace_engines,
            ide,
            mcp_pool: Arc::new(McpConnectionPool::new()),
            global_db_path: global_db_path.into(),
            settings_revision,
            loaded_revision: Arc::new(AtomicU64::new(loaded)),
            test_llm_override: None,
        }
    }

    #[doc(hidden)]
    pub fn with_test_llm_override(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.test_llm_override = Some(provider);
        self
    }

    pub fn settings_revision(&self) -> u64 {
        self.settings_revision.load(Ordering::Acquire)
    }

    /// Resolve the hidden compaction agent for a standalone compact operation.
    pub fn resolve_compaction_binding(&self) -> Result<TurnLlmBinding> {
        let mut binding = llm_resolve::binding_for_agent(
            &self.resolved,
            &mut self.provider_registry.lock().unwrap(),
            "compaction",
            None,
            self.settings_revision(),
        )?;
        if let Some(provider) = &self.test_llm_override {
            binding.provider = Arc::clone(provider);
        }
        Ok(binding)
    }

    pub fn compaction_context(&self) -> Context {
        build_context(
            &self.resolved,
            &self.workspace.workspace_root,
            &self.workspace.paths,
        )
    }

    pub fn compaction_system_prompt(&self, ctx: &Context) -> String {
        self.resolved
            .agents()
            .get("compaction")
            .map(crate::config::bridge::agent_config_from_profile)
            .map(|agent| crate::context_pipeline::build_system_prompt(&agent, ctx))
            .unwrap_or_else(|| crate::context_pipeline::BUILTIN_COMPACTION.to_string())
    }

    /// Reload global settings into this handle when the writer revision advanced.
    pub fn reload_if_needed(&mut self) -> Result<()> {
        let current = self.settings_revision.load(Ordering::Acquire);
        let loaded = self.loaded_revision.load(Ordering::Acquire);
        if current <= loaded {
            return Ok(());
        }
        let global = ConfigManager::load_global_from(&self.global_db_path)?;
        ConfigManager::validate(&global)?;
        let workspace = crate::config::workspace::workspace_with_disk_readiness(&self.workspace);
        self.resolved = ConfigManager::resolve(global.clone(), workspace.clone());
        crate::tool::catalog::init(&mut self.resolved, crate::config::schema::InitScope::Global);
        self.workspace = workspace;
        log_filter::reload_from_path(&self.global_db_path);
        self.engine_manager.reconcile(&self.resolved);
        self.workspace_engines.reconcile(&self.resolved);
        self.ensure_valid_desired_primary();
        self.provider_registry
            .lock()
            .unwrap()
            .invalidate_if_stale(current);
        self.loaded_revision.store(current, Ordering::Release);
        Ok(())
    }

    pub fn desired_primary_agent(&self) -> &str {
        &self.desired_primary_agent
    }

    pub fn set_desired_primary_agent(&mut self, agent_id: String) -> Result<()> {
        Self::validate_primary_agent(&self.resolved, &agent_id)?;
        self.desired_primary_agent = agent_id;
        Ok(())
    }

    /// Apply process-level primary selection at turn boundary.
    pub fn apply_primary_for_turn(&mut self) -> Result<()> {
        Self::validate_primary_agent(&self.resolved, &self.desired_primary_agent)?;
        self.agent_name = self.desired_primary_agent.clone();
        Ok(())
    }

    fn ensure_valid_desired_primary(&mut self) {
        if Self::validate_primary_agent(&self.resolved, &self.desired_primary_agent).is_err() {
            self.desired_primary_agent = "default".to_string();
            if Self::validate_primary_agent(&self.resolved, &self.desired_primary_agent).is_ok() {
                self.agent_name = self.desired_primary_agent.clone();
            }
        }
    }

    pub fn validate_primary_agent(resolved: &ResolvedConfig, agent_id: &str) -> Result<()> {
        use crate::config::schema::AgentRole;
        let profile = resolved.agents().get(agent_id).ok_or_else(|| {
            LitecodeError::Config(format!("primary agent '{agent_id}' not found"))
        })?;
        if profile.role != AgentRole::Primary {
            return Err(LitecodeError::Config(format!(
                "agent '{agent_id}' is not a primary agent"
            )));
        }
        Ok(())
    }

    /// Refresh workspace-scoped optional tool readiness from `.litecode/engines.json`.
    pub fn sync_workspace_tool_readiness(&mut self) {
        let workspace = crate::config::workspace::workspace_with_disk_readiness(&self.workspace);
        self.workspace = workspace.clone();
        self.resolved.workspace_mut().workspace_tool_readiness =
            workspace.workspace_tool_readiness.clone();
    }

    pub fn db_path(&self) -> String {
        self.workspace
            .paths
            .sessions_db
            .to_string_lossy()
            .into_owned()
    }

    pub fn workspace_root(&self) -> &std::path::Path {
        &self.workspace.workspace_root
    }

    pub fn llm_ecosystem(&self) -> &'static str {
        let adapter_id = self
            .resolved
            .agents()
            .get(&self.desired_primary_agent)
            .and_then(|agent| {
                if agent.model_ref.is_empty() {
                    return None;
                }
                self.resolved.models().get(&agent.model_ref)
            })
            .and_then(|model| self.resolved.providers().get(&model.provider_ref))
            .map(|provider| provider.adapter_id.as_str());
        match adapter_id {
            Some(id) if id == ADAPTER_DEEPSEEK_RESPONSES => "deepseek",
            _ => "openai",
        }
    }

    pub fn build_runtime(
        &self,
        session_id: String,
        sessions: Arc<SessionManager>,
        agent_name: &str,
        depth: u32,
        sink: Arc<dyn PermissionSink>,
        observer: Arc<dyn RuntimeObserver>,
    ) -> Result<AgentRuntime> {
        let revision = self.settings_revision();
        let mut binding = {
            let mut registry = self.provider_registry.lock().unwrap();
            resolve_session_llm(
                &self.resolved,
                &mut registry,
                &sessions,
                &session_id,
                revision,
            )?
        };
        if let Some(provider) = &self.test_llm_override {
            binding.provider = Arc::clone(provider);
        }

        AgentRuntime::with_mcp_pool(
            self.resolved.clone(),
            session_id,
            sessions,
            binding,
            agent_name,
            depth,
            sink,
            observer,
            None,
            None,
            (*self.engine_manager).clone(),
            (*self.workspace_engines).clone(),
            Arc::clone(&self.ide),
            Arc::clone(&self.mcp_pool),
        )
    }
}

impl Clone for RuntimeHandle {
    fn clone(&self) -> Self {
        Self {
            resolved: self.resolved.clone(),
            provider_registry: Mutex::new(ProviderRegistry::new()),
            agent_name: self.agent_name.clone(),
            desired_primary_agent: self.desired_primary_agent.clone(),
            workspace: self.workspace.clone(),
            engine_manager: Arc::clone(&self.engine_manager),
            workspace_engines: Arc::clone(&self.workspace_engines),
            ide: Arc::clone(&self.ide),
            mcp_pool: Arc::clone(&self.mcp_pool),
            global_db_path: self.global_db_path.clone(),
            settings_revision: Arc::clone(&self.settings_revision),
            loaded_revision: Arc::new(AtomicU64::new(0)),
            test_llm_override: self.test_llm_override.clone(),
        }
    }
}

/// Handle for an agent turn running on a background thread.
pub struct TurnHandle {
    /// The spawned turn thread. `None` in paths that drive the turn inline (the
    /// subagent parent) where reclamation is handled by the caller's own join.
    pub handle: Option<JoinHandle<std::result::Result<String, LitecodeError>>>,
    pub rx: mpsc::UnboundedReceiver<InternalEnvelope>,
    pub cancel: tokio_util::sync::CancellationToken,
    /// Identifier of the turn spawned by this handle (used by lifecycle events).
    pub turn_id: String,
    /// Maximum number of steps the turn is allowed to run (from agent config).
    pub step_max: u32,
}

impl TurnHandle {
    pub fn is_finished(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.is_finished())
    }
}

/// Spawn an agent turn on a background thread (CLI / SessionController path).
pub fn spawn_turn(
    runtime: &RuntimeHandle,
    session_id: String,
    sessions: Arc<SessionManager>,
    input: String,
    permission_sink: Arc<dyn PermissionSink>,
    turn_id: String,
) -> anyhow::Result<TurnHandle> {
    let default_primary = runtime.desired_primary_agent();
    let primary_agent = sessions
        .resolve_primary_agent(&session_id, default_primary, &runtime.resolved)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let (tx, rx) = mpsc::unbounded_channel::<InternalEnvelope>();
    let observer = ChannelObserver::new(tx);
    let mut agent_loop = runtime.build_runtime(
        session_id,
        sessions,
        &primary_agent,
        0,
        permission_sink,
        observer,
    )?;

    let cancel = agent_loop.cancel_token();
    let step_max = agent_loop.agent_config.max_steps;
    let workspace_paths = runtime.workspace.paths.clone();
    let turn_id_for_thread = turn_id.clone();

    let handle = std::thread::spawn(move || {
        set_runtime_paths(workspace_paths);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(agent_loop.run_with_turn(&input, &turn_id_for_thread, step_max))
    });

    Ok(TurnHandle {
        handle: Some(handle),
        rx,
        cancel,
        turn_id,
        step_max,
    })
}

pub struct AgentRuntime {
    pub resolved: ResolvedConfig,
    pub session_id: String,
    sessions: Arc<SessionManager>,
    pub(crate) runtime_ctx: Option<Arc<RuntimeContext>>,
    pub turn_llm: TurnLlmBinding,
    pub agent_config: AgentConfig,
    pub(crate) tool_pipeline: Option<ToolPipeline>,
    pub(crate) context_pipeline: ContextPipeline,
    /// Cached context for access before lazy tool init (mirrors what runtime_ctx.ctx would hold).
    base_ctx: Context,
    observer: Arc<dyn RuntimeObserver>,
    cancel: tokio_util::sync::CancellationToken,
    pub(crate) prompt_usage_baseline: ProviderPromptBaseline,
    pub(crate) current_step: Arc<AtomicU64>,
    /// Last LLM request usage only (replaced each call — never turn-summed).
    pub(crate) turn_token_stats: TurnTokenStats,
    /// Σ every LLM request's usage in the current turn (feeds session cum_* meter).
    pub(crate) turn_usage_totals: TurnTokenStats,
    /// Stored parameters for deferred build_tool_list (async MCP schema fetch).
    build_tool_params: Option<Arc<BuildToolParams>>,
}

/// Parameters needed to call build_tool_list lazily on first turn.
struct BuildToolParams {
    resolved: ResolvedConfig,
    agent_name: String,
    provider: Box<dyn LlmProvider>,
    api_key: String,
    depth: u32,
    cancel: tokio_util::sync::CancellationToken,
    engine_manager: EngineManager,
    workspace_engines: WorkspaceEngines,
    ide: Arc<IdeBaseHandle>,
    parent_session_id: String,
    sessions: Arc<SessionManager>,
    permission_sink: Arc<dyn PermissionSink>,
    mcp_pool: Arc<McpConnectionPool>,
}

impl AgentRuntime {
    pub fn provider(&self) -> &dyn LlmProvider {
        self.turn_llm.provider.as_ref()
    }

    pub fn api_key(&self) -> &str {
        &self.turn_llm.api_key
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resolved: ResolvedConfig,
        session_id: String,
        sessions: Arc<SessionManager>,
        turn_llm: TurnLlmBinding,
        agent_name: &str,
        depth: u32,
        permission_sink: Arc<dyn PermissionSink>,
        observer: Arc<dyn RuntimeObserver>,
        cancel: Option<tokio_util::sync::CancellationToken>,
        max_steps_override: Option<u32>,
        engine_manager: EngineManager,
        workspace_engines: WorkspaceEngines,
        ide: Arc<IdeBaseHandle>,
    ) -> Result<Self> {
        Self::with_mcp_pool(
            resolved,
            session_id,
            sessions,
            turn_llm,
            agent_name,
            depth,
            permission_sink,
            observer,
            cancel,
            max_steps_override,
            engine_manager,
            workspace_engines,
            ide,
            Arc::new(McpConnectionPool::new()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_mcp_pool(
        resolved: ResolvedConfig,
        session_id: String,
        sessions: Arc<SessionManager>,
        turn_llm: TurnLlmBinding,
        agent_name: &str,
        depth: u32,
        permission_sink: Arc<dyn PermissionSink>,
        observer: Arc<dyn RuntimeObserver>,
        cancel: Option<tokio_util::sync::CancellationToken>,
        max_steps_override: Option<u32>,
        engine_manager: EngineManager,
        workspace_engines: WorkspaceEngines,
        ide: Arc<IdeBaseHandle>,
        mcp_pool: Arc<McpConnectionPool>,
    ) -> Result<Self> {
        let cancel = cancel.unwrap_or_default();

        let mut agent_config = agent_config_for(&resolved, agent_name)?;
        if let Some(max_steps) = max_steps_override {
            agent_config.max_steps = max_steps;
        }

        let binding_count = resolved
            .agents()
            .get(agent_name)
            .map(|profile| profile.tools.values().filter(|b| b.enabled).count())
            .unwrap_or(0);

        tracing::info!(
            "agent init: name={} model_ref={} bindings_enabled={} provider={}",
            agent_name,
            agent_config.model_ref,
            binding_count,
            turn_llm.provider_id
        );

        // Process-owned workspace from ResolvedConfig. Do not use thread-local
        // `active_paths()`, which falls back to process cwd on tokio workers
        // (dev `cargo run` cwd is the source tree, not `--workspace`).
        let paths = resolved.paths().clone();
        let cwd = resolved.workspace_root().to_path_buf();
        let context = build_context(&resolved, &cwd, &paths);

        let context_window = turn_llm.context_window;
        let context_pipeline = sessions.with_entry_store(&session_id, |s| {
            let data_root = s.data_root().to_path_buf();
            Ok(ContextPipeline::new(
                s,
                context_window,
                context.clone(),
                data_root,
            ))
        })?;

        let build_tool_params = Arc::new(BuildToolParams {
            resolved: resolved.clone(),
            agent_name: agent_name.to_string(),
            provider: turn_llm.provider.box_clone(),
            api_key: turn_llm.api_key.clone(),
            depth,
            cancel: cancel.clone(),
            engine_manager,
            workspace_engines,
            ide,
            parent_session_id: session_id.clone(),
            sessions: Arc::clone(&sessions),
            permission_sink,
            mcp_pool,
        });

        let runtime = Self {
            resolved,
            session_id,
            sessions,
            runtime_ctx: None,
            turn_llm,
            agent_config,
            tool_pipeline: None,
            context_pipeline,
            base_ctx: context,
            observer,
            cancel,
            prompt_usage_baseline: ProviderPromptBaseline::default(),
            current_step: Arc::new(AtomicU64::new(0)),
            turn_token_stats: TurnTokenStats::default(),
            turn_usage_totals: TurnTokenStats::default(),
            build_tool_params: Some(build_tool_params),
        };
        Ok(runtime)
    }

    pub fn sessions(&self) -> &Arc<SessionManager> {
        &self.sessions
    }

    /// Access the runtime context — panics if called before first turn (logic error).
    fn rctx(&self) -> &RuntimeContext {
        self.runtime_ctx
            .as_ref()
            .expect("runtime_ctx not initialized: run_with_turn must be called first")
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn cancel_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancel.clone()
    }

    pub fn context(&self) -> &Context {
        &self.base_ctx
    }

    pub fn prepared_view(&self) -> Option<crate::context_pipeline::PreparedView> {
        self.context_pipeline.prepared_view()
    }

    /// Override the working directory used by tools, hooks, and context restoration.
    pub fn set_context_cwd(&mut self, cwd: std::path::PathBuf) {
        self.base_ctx.cwd = cwd.clone();
        if let Some(ref mut pipeline) = self.tool_pipeline
            && let Some(ref mut rctx) = self.runtime_ctx
        {
            let new_ctx = Arc::new(RuntimeContext {
                ctx: Context {
                    cwd,
                    ..rctx.ctx.clone()
                },
                ..(**rctx).clone()
            });
            *rctx = Arc::clone(&new_ctx);
            pipeline.set_runtime(new_ctx);
        }
        self.context_pipeline.sync_context(&self.base_ctx);
    }

    /// Test helper: replace hooks on the shared runtime context.
    pub fn set_hook_registry(&mut self, hooks: HookDispatcher) {
        if let Some(ref mut pipeline) = self.tool_pipeline
            && let Some(ref mut rctx) = self.runtime_ctx
        {
            let new_ctx = Arc::new(RuntimeContext {
                hook_dispatcher: hooks,
                ..(**rctx).clone()
            });
            *rctx = Arc::clone(&new_ctx);
            pipeline.set_runtime(new_ctx);
        }
    }

    /// Test helper: simulate provider-reported prompt token usage from the prior LLM call.
    pub fn set_last_prompt_tokens(&self, tokens: u64) {
        self.prompt_usage_baseline.record(tokens, 0);
    }

    /// Current provider-reported prompt token usage (set from real `response.completed`
    /// usage in `call_model_complete`). Drives the next compaction decision (2.4).
    pub fn last_prompt_tokens(&self) -> u64 {
        self.prompt_usage_baseline.prompt_tokens()
    }

    pub(crate) fn emit_internal(&self, event: InternalEvent) {
        self.observer.on_internal(event);
    }

    /// Single authority for `TurnCompleted`: Idle gate + emit + join result.
    ///
    /// Also drops the in-memory turn working set (`end_turn`). Prefer
    /// [`Self::emit_turn_completed`] on the main agent path so commit can run
    /// before `end_turn`.
    fn finalize_turn(
        &mut self,
        turn_id: &str,
        reason: TurnEndReason,
        final_text: Option<String>,
    ) -> Result<String> {
        self.context_pipeline.end_turn();
        self.emit_turn_completed(turn_id, reason, final_text)
    }

    fn emit_turn_completed(
        &mut self,
        turn_id: &str,
        reason: TurnEndReason,
        final_text: Option<String>,
    ) -> Result<String> {
        let step = self.current_step_value();
        if step > 0 {
            self.emit_phase(TurnPhase::Finalizing, step);
        }
        let text = final_text.clone().unwrap_or_default();
        // Last LLM request usage only (`turn_token_stats` — never turn-summed).
        // Persist provider truth only; never invent occupancy from local estimates.
        // No usage this turn → leave prior meter untouched (truth-or-absent).
        // Session-total accumulators (cum_*) add this turn's Σ (`turn_usage_totals`),
        // not just the last request — multi-step tool loops must not under-count.
        let stats = std::mem::take(&mut self.turn_token_stats);
        let turn_totals = std::mem::take(&mut self.turn_usage_totals);
        if stats.has_provider_usage() {
            let previous = self
                .sessions
                .with_entry_store(&self.session_id, |s| Ok(s.load_context_meter()?))
                .unwrap_or_default();
            // Prefer turn_totals (Σ every request); fall back to last-request stats
            // if totals were somehow empty while stats still hold truth.
            let add = if turn_totals.has_provider_usage() {
                &turn_totals
            } else {
                &stats
            };
            let meter = crate::session::SessionContextMeter {
                prompt_tokens: stats.prompt_tokens,
                completion_tokens: stats.completion_tokens,
                cache_hit_tokens: stats.cache_hit_tokens,
                cache_miss_tokens: stats.cache_miss_tokens,
                cum_prompt_tokens: previous.cum_prompt_tokens.saturating_add(add.prompt_tokens),
                cum_completion_tokens: previous
                    .cum_completion_tokens
                    .saturating_add(add.completion_tokens),
                cum_cache_hit_tokens: previous
                    .cum_cache_hit_tokens
                    .saturating_add(add.cache_hit_tokens),
                cum_cache_miss_tokens: previous
                    .cum_cache_miss_tokens
                    .saturating_add(add.cache_miss_tokens),
            };
            if let Err(e) = self
                .sessions
                .with_entry_store(&self.session_id, |s| Ok(s.save_context_meter(&meter)?))
            {
                tracing::warn!(
                    session_id = %self.session_id,
                    error = %e,
                    "failed to persist session context meter"
                );
            }
        }
        let _ = self.sessions.finish_turn(&self.session_id, turn_id);
        self.emit_internal(InternalEvent::TurnCompleted {
            turn_id: turn_id.to_string(),
            final_text,
            reason,
            turn_token_stats: stats,
            committed_next_seq: 0,
        });
        match reason {
            TurnEndReason::Error => Err(LitecodeError::Llm(if text.is_empty() {
                "turn failed".to_string()
            } else {
                text
            })),
            _ => Ok(text),
        }
    }

    fn finalize_agent_outcome(&mut self, turn_id: &str, outcome: TurnOutcome) -> Result<String> {
        match outcome {
            TurnOutcome::Completed { final_text } => {
                self.emit_turn_completed(turn_id, TurnEndReason::Completed, Some(final_text))
            }
            TurnOutcome::Cancelled { final_text } => self.emit_turn_completed(
                turn_id,
                TurnEndReason::Cancelled,
                (!final_text.is_empty()).then_some(final_text),
            ),
            TurnOutcome::MaxSteps { final_text } => {
                self.emit_turn_completed(
                    turn_id,
                    TurnEndReason::MaxSteps,
                    (!final_text.is_empty()).then_some(final_text),
                )?;
                Err(LitecodeError::MaxStepsReached)
            }
            TurnOutcome::Error(err) => {
                let msg = err.to_string();
                self.emit_turn_completed(turn_id, TurnEndReason::Error, Some(msg))
            }
        }
    }

    pub async fn run(&mut self, user_prompt: &str) -> Result<String> {
        let step_max = self.agent_config.max_steps;
        self.run_with_turn(user_prompt, "local-turn", step_max)
            .await
    }

    pub async fn run_with_turn(
        &mut self,
        user_prompt: &str,
        turn_id: &str,
        step_max: u32,
    ) -> Result<String> {
        // Lazy-init: build tool list (async MCP schema fetch) on first turn.
        if self.tool_pipeline.is_none() {
            let params = self.build_tool_params.take().unwrap();
            let tools = build_tool_list(
                &params.resolved,
                &params.agent_name,
                params.provider.box_clone(),
                &params.api_key,
                params.depth,
                params.cancel.clone(),
                params.engine_manager.clone(),
                params.workspace_engines.clone(),
                Arc::clone(&params.ide),
                &params.parent_session_id,
                Arc::clone(&params.sessions),
                Arc::clone(&params.mcp_pool),
            )
            .await;

            let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
            tracing::info!(
                target: "litecode.debug.startup",
                tools_count = tool_names.len(),
                tools = ?tool_names,
                agent = %params.agent_name,
                "agent initialized (lazy)"
            );

            let permission = PermissionEngine::resolver(
                params.resolved.clone(),
                &params.agent_name,
                params.depth,
            );
            let hook_dispatcher = HookDispatcher::from_registry(HookRegistry::default());
            let permission_sink = Arc::new(CancellingPermissionSink::new(
                Arc::clone(&params.permission_sink),
                params.cancel.clone(),
            ));
            let permission_sink = phase::PhasePermissionSink::wrap(
                permission_sink,
                Arc::clone(&self.observer),
                Arc::clone(&self.current_step),
            );
            let data_root = self
                .sessions
                .with_entry_store(&self.session_id, |s| Ok(s.data_root().to_path_buf()))?;
            let spill_threshold = output::DEFAULT_SPILL_THRESHOLD;
            let write_lock = crate::tool::write_lock::process_write_lock();
            let runtime_ctx = Arc::new(RuntimeContext::new(
                tools,
                permission,
                hook_dispatcher,
                self.base_ctx.clone(),
                params.agent_name.to_string(),
                permission_sink,
                params.cancel.clone(),
                data_root,
                spill_threshold,
                write_lock,
            ));
            let mut tool_pipeline = ToolPipeline::new(Arc::clone(&runtime_ctx));
            tool_pipeline.bind_session(self.session_id.clone());
            self.runtime_ctx = Some(runtime_ctx);
            self.tool_pipeline = Some(tool_pipeline);
        }

        tracing::info!(input = %user_prompt, "agent loop start");

        // Fresh per-turn meters (defensive if a prior turn exited without finalize).
        self.turn_token_stats = TurnTokenStats::default();
        self.turn_usage_totals = TurnTokenStats::default();

        self.emit_internal(InternalEvent::TurnStarted {
            turn_id: turn_id.to_string(),
            input: user_prompt.to_string(),
            step_max,
        });
        self.emit_internal(InternalEvent::PhaseChanged {
            phase: TurnPhase::Starting,
            step: 1,
        });

        let sid = self.session_id.clone();
        // Propagate begin_turn errors explicitly — a failed load must not start
        // the turn with an empty (silently truncated) transcript.
        let mut working = self.sessions.with_entry_store(&sid, |s| {
            Ok(self
                .context_pipeline
                .begin_turn_with_id(s, Some(turn_id.to_string()))?)
        })?;
        let mut items = project_items(&working);
        let resumed = !items.is_empty();
        let ts = chrono::Utc::now().timestamp_millis();

        let start_payload = crate::hook::HookPayload::new(
            "SessionStart",
            &self.session_id,
            &self.rctx().ctx.cwd.display().to_string(),
            serde_json::json!({
                "item_count": items.len(),
                "resumed": resumed,
            }),
        );
        let start_output = self
            .rctx()
            .hook_dispatcher
            .fire("SessionStart", &start_payload, &self.rctx().ctx)
            .await;
        self.emit_hook_fired("SessionStart", &format!("{:?}", start_output.action));
        if start_output.action == crate::hook::HookAction::Block {
            return self.finalize_turn(
                turn_id,
                TurnEndReason::HookBlocked,
                Some("Session blocked by SessionStart hook".into()),
            );
        }
        if self
            .rctx()
            .hook_dispatcher
            .phase_applies_inject("SessionStart")
        {
            apply_hook_output(&mut items, start_output, ts);
            align_working(&mut working, &items);
        }

        let prompt_payload = crate::hook::HookPayload::new(
            "UserPromptSubmit",
            &self.session_id,
            &self.rctx().ctx.cwd.display().to_string(),
            serde_json::json!({"prompt": user_prompt}),
        );
        let prompt_output = self
            .rctx()
            .hook_dispatcher
            .fire("UserPromptSubmit", &prompt_payload, &self.rctx().ctx)
            .await;
        self.emit_hook_fired("UserPromptSubmit", &format!("{:?}", prompt_output.action));
        if prompt_output.action == crate::hook::HookAction::Block {
            return self.finalize_turn(
                turn_id,
                TurnEndReason::HookBlocked,
                Some("Prompt blocked by UserPromptSubmit hook".into()),
            );
        }

        let already_last_user = items
            .last()
            .is_some_and(|m| item_text_preview(m) == user_prompt);
        if !already_last_user {
            items.push(user_text(user_prompt.to_string()));
            working.push(WorkingRow::pending(user_text(user_prompt.to_string())));
        }

        if self
            .rctx()
            .hook_dispatcher
            .phase_applies_inject("UserPromptSubmit")
        {
            apply_hook_output(&mut items, prompt_output, ts);
            align_working(&mut working, &items);
        }

        // User (and inject) Items are complete before any model stream. Persist
        // so the working set and disk agree before InFlight begins.
        {
            let commit_outcome = self.sessions.with_entry_store(&self.session_id, |s| {
                Ok(self.context_pipeline.commit_step(s, &mut working)?)
            })?;
            if commit_outcome.discarded {
                return self.finalize_agent_outcome(
                    turn_id,
                    crate::agent::TurnOutcome::Cancelled {
                        final_text: String::new(),
                    },
                );
            }
            if commit_outcome.committed {
                self.emit_internal(crate::runtime::observer::InternalEvent::StepCommitted);
            }
            if let Some((preview, updated_at)) = commit_outcome.preview {
                self.emit_internal(
                    crate::runtime::observer::InternalEvent::SessionPreviewUpdated {
                        preview,
                        updated_at,
                    },
                );
            }
        }

        items = project_items(&working);

        let (_last_seq, next_seq) = self.sessions.entry_wire_seq_cursor(&self.session_id);
        let anchor_k = next_seq as i64;
        self.rctx().set_turn_anchor_k(anchor_k);

        // Snapshot workspace before tools run (OpenCode-style git-based snapshot).
        // Must finish before agent::run so the tracked tree is the pre-tool workspace.
        // Git index I/O runs on the blocking pool; we await here before any tools execute.
        let ws = self.rctx().ctx.cwd.clone();
        let snaps = self.rctx().ctx.workspace_paths.snapshots_dir.clone();
        let track_ws = ws.clone();
        let track_snaps = snaps.clone();
        let track_sid = self.session_id.clone();
        let track_result = tokio::task::spawn_blocking(move || {
            snapshot::snapshot_track(&track_ws, &track_snaps, &track_sid, anchor_k)
        })
        .await;
        match track_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(%e, anchor_k, "snapshot_track failed, revert may be incomplete");
                self.emit_internal(InternalEvent::SnapshotNotice {
                    level: "warn".into(),
                    message: format!(
                        "Workspace snapshot track failed (anchor {anchor_k}): {e}; file revert may be unavailable"
                    ),
                });
            }
            Err(e) => {
                tracing::warn!(%e, anchor_k, "snapshot_track join failed");
                self.emit_internal(InternalEvent::SnapshotNotice {
                    level: "warn".into(),
                    message: format!(
                        "Workspace snapshot track failed (anchor {anchor_k}): {e}; file revert may be unavailable"
                    ),
                });
            }
        }

        tracing::info!(
            item_count = items.len(),
            anchor_k,
            "session transcript loaded"
        );

        self.emit_todo_progress();

        let outcome = crate::agent::run(self, &mut items).await;

        let should_commit = matches!(
            outcome,
            TurnOutcome::Completed { .. } | TurnOutcome::MaxSteps { .. }
        );
        let success = !matches!(outcome, TurnOutcome::Error(_));

        // Persist SessionEnd injects + final turn delta BEFORE TurnCompleted so the
        // DB already contains the whole turn when the event lands (2.3).
        let end_payload = crate::hook::HookPayload::new(
            "SessionEnd",
            &self.session_id,
            &self.rctx().ctx.cwd.display().to_string(),
            serde_json::json!({
                "item_count": items.len(),
                "success": success,
            }),
        );
        let end_ts = chrono::Utc::now().timestamp_millis();
        let end_output = self
            .rctx()
            .hook_dispatcher
            .fire_and_apply(
                "SessionEnd",
                &end_payload,
                &self.rctx().ctx,
                &mut items,
                end_ts,
            )
            .await;
        self.emit_hook_fired("SessionEnd", &format!("{:?}", end_output.action));
        if end_output.action == crate::hook::HookAction::Block {
            tracing::warn!("SessionEnd hook returned Block (logged only)");
        }

        // Persist SessionEnd injects only when the turn is kept (not cancel/error).
        if should_commit {
            align_working(&mut working, &items);
            let commit_outcome = self.sessions.with_entry_store(&self.session_id, |s| {
                Ok(self.context_pipeline.commit_step(s, &mut working)?)
            })?;
            if !commit_outcome.discarded {
                if commit_outcome.committed {
                    self.emit_internal(crate::runtime::observer::InternalEvent::StepCommitted);
                }
                if let Some((preview, updated_at)) = commit_outcome.preview {
                    self.emit_internal(
                        crate::runtime::observer::InternalEvent::SessionPreviewUpdated {
                            preview,
                            updated_at,
                        },
                    );
                }
            }
        }

        // Idle + TurnCompleted before workspace patch so Running never spans patch I/O.
        let finalize_result = self.finalize_agent_outcome(turn_id, outcome);

        // OpenCode-style: record which paths changed this turn (file-level revert).
        let patch_ws = ws.clone();
        let patch_snaps = snaps.clone();
        let patch_sid = self.session_id.clone();
        let patch_result = tokio::task::spawn_blocking(move || {
            snapshot::snapshot_record_patch(&patch_ws, &patch_snaps, &patch_sid, anchor_k)
        })
        .await;
        match patch_result {
            Ok(Ok(patch)) if patch.track_failed => {
                tracing::warn!(anchor_k, "snapshot_record_patch wrote track_failed marker");
                self.emit_internal(InternalEvent::SnapshotNotice {
                    level: "warn".into(),
                    message: format!(
                        "Workspace snapshot unavailable for this turn (anchor {anchor_k}); file revert will fail"
                    ),
                });
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::warn!(%e, anchor_k, "snapshot_record_patch failed");
                self.emit_internal(InternalEvent::SnapshotNotice {
                    level: "warn".into(),
                    message: format!(
                        "Workspace snapshot patch record failed (anchor {anchor_k}): {e}"
                    ),
                });
            }
            Err(e) => {
                tracing::warn!(%e, anchor_k, "snapshot_record_patch join failed");
                self.emit_internal(InternalEvent::SnapshotNotice {
                    level: "warn".into(),
                    message: format!(
                        "Workspace snapshot patch record failed (anchor {anchor_k}): {e}"
                    ),
                });
            }
        }

        let max_k = snapshot::max_file_revert_k(&snaps, &self.session_id);
        self.emit_internal(InternalEvent::FileRevertUpdated { max_k });

        tracing::info!(
            session_id = %self.session_id,
            total_items = items.len(),
            "agent loop complete"
        );
        self.context_pipeline.end_turn();
        finalize_result
    }
}
