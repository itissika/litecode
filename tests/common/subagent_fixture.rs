//! Shared harness for subagent integration tests: one workspace, one durable
//! `SessionManager`, and the subagent tool series bound to the runtime hub.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use litecode::config::ResolvedConfig;
use litecode::config::TurnGuard;
use litecode::config::resolved::resolve;
use litecode::config::schema::{AgentProfile, AgentRole};
use litecode::config::workspace::set_runtime_paths;
use litecode::engines::WorkspaceEngines;
use litecode::llm::LlmProvider;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::session::manager::SessionManager;
use litecode::tool::Tool;
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tools::subagent::{
    SubagentLaunchTool, SubagentListTool, SubagentSendTool, SubagentStopTool, SubagentWaitTool,
};
use litecode::types::ToolCallResult;

use super::bindings::binding_safe_for;
use super::{test_resolved, test_workspace};

pub const PARENT_AGENT: &str = "default";
pub const SUBAGENT_AGENT: &str = "reviewer";

/// One initialized workspace with a durable parent session and the subagent
/// tool series bound to the runtime hub. Keep the harness alive for the whole
/// test: dropping it removes the temp workspace.
pub struct SubagentHarness {
    pub dir: tempfile::TempDir,
    pub resolved: ResolvedConfig,
    pub sessions: Arc<SessionManager>,
    pub runtime: RuntimeHandle,
    pub parent_id: String,
}

impl SubagentHarness {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self::with_subagent_max_steps(provider, 2)
    }

    pub fn with_subagent_max_steps(
        provider: Arc<dyn LlmProvider>,
        subagent_max_steps: u32,
    ) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = test_workspace(dir.path());
        set_runtime_paths(workspace.paths.clone());

        let base = test_resolved(PARENT_AGENT, &["subagent_launch".into()]);
        let mut global = base.global().clone();
        if let Some(default) = global.agents.get_mut(PARENT_AGENT) {
            default.allowed_subagents = vec![SUBAGENT_AGENT.into()];
            default.max_steps = 4;
        }
        global.agents.insert(
            SUBAGENT_AGENT.into(),
            AgentProfile {
                role: AgentRole::Subagent,
                model_ref: "default".into(),
                system_prompt: "builtin:general".into(),
                tools: HashMap::from([("read".into(), binding_safe_for("read"))]),
                max_steps: subagent_max_steps,
                ..Default::default()
            },
        );
        let resolved = resolve(global, workspace.clone());

        let project = workspace.workspace_root.to_string_lossy().to_string();
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            workspace.paths.sessions_db.to_string_lossy().to_string(),
        ));
        let parent_id = sessions
            .open_session_sync(&project, PARENT_AGENT, Some("default"))
            .expect("parent session");

        let workspace_service =
            litecode::workspace::WorkspaceService::new(workspace.workspace_root.clone())
                .expect("workspace service");
        let engines = WorkspaceEngines::new();
        let ide = litecode::ide_base::IdeBaseHandle::new(
            workspace_service,
            Arc::new(engines.clone()),
            Arc::new(litecode::terminal::TerminalHub::new()),
        );
        let runtime = RuntimeHandle::new(
            resolved.clone(),
            PARENT_AGENT.into(),
            workspace.clone(),
            Arc::new(EngineManager::new()),
            Arc::new(engines),
            ide,
            Arc::new(AtomicU64::new(0)),
            workspace.paths.sessions_db.with_file_name("global.db"),
        )
        .with_test_llm_override(provider);
        runtime.subagent_hub.attach_sessions(Arc::clone(&sessions));

        Self {
            dir,
            resolved,
            sessions,
            runtime,
            parent_id,
        }
    }

    pub fn project(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    pub fn launch_tool(&self) -> SubagentLaunchTool {
        SubagentLaunchTool::new(
            self.runtime.clone(),
            PARENT_AGENT,
            0,
            tokio_util::sync::CancellationToken::new(),
            Arc::clone(&self.sessions),
            self.parent_id.clone(),
        )
    }

    pub fn wait_tool(&self) -> SubagentWaitTool {
        SubagentWaitTool::new(Arc::clone(&self.sessions))
    }

    pub fn stop_tool(&self) -> SubagentStopTool {
        SubagentStopTool::new(Arc::clone(&self.sessions))
    }

    pub fn list_tool(&self) -> SubagentListTool {
        SubagentListTool::new(Arc::clone(&self.sessions))
    }

    pub fn send_tool(&self) -> SubagentSendTool {
        SubagentSendTool::new(self.runtime.clone(), Arc::clone(&self.sessions), 0)
    }

    pub async fn send(&self, call_id: &str, child_id: &str, message: &str) -> ToolCallResult {
        let tool = self.send_tool();
        tool.execute(
            serde_json::json!({ "id": child_id, "message": message }),
            self.exec_ctx(call_id),
        )
        .await
    }

    pub fn exec_ctx(&self, call_id: &str) -> ToolExecutionContext {
        ToolExecutionContext {
            path_mode: litecode::workspace::ToolPathMode::All,
            workspace_root: std::path::PathBuf::from("."),
            call_id: call_id.to_string(),
            cancel: tokio_util::sync::CancellationToken::new(),
            output_limit: 8_000,
            session_id: self.parent_id.clone(),
            session: None,
        }
    }

    pub async fn launch(&self, call_id: &str, input: serde_json::Value) -> ToolCallResult {
        let tool = self.launch_tool();
        tool.execute(input, self.exec_ctx(call_id)).await
    }

    pub async fn wait(&self, call_id: &str, input: serde_json::Value) -> ToolCallResult {
        let tool = self.wait_tool();
        tool.execute(input, self.exec_ctx(call_id)).await
    }

    pub async fn stop(&self, call_id: &str, child_id: &str) -> ToolCallResult {
        let tool = self.stop_tool();
        tool.execute(
            serde_json::json!({ "id": child_id }),
            self.exec_ctx(call_id),
        )
        .await
    }

    pub async fn list(&self, call_id: &str) -> ToolCallResult {
        let tool = self.list_tool();
        tool.execute(serde_json::json!({}), self.exec_ctx(call_id))
            .await
    }

    /// Launch `reviewer` and return the child session id.
    pub async fn launch_reviewer(&self, call_id: &str, prompt: &str) -> String {
        let result = self
            .launch(
                call_id,
                serde_json::json!({
                    "agent": SUBAGENT_AGENT,
                    "responsibility": "test",
                    "prompt": prompt
                }),
            )
            .await;
        assert_eq!(
            result.level,
            litecode::types::ToolSignalLevel::Ok,
            "launch failed: {}",
            result.content
        );
        Self::child_id(&result)
    }

    pub fn child_id(result: &ToolCallResult) -> String {
        result
            .metadata
            .as_ref()
            .and_then(|m| m.get("child_session_id"))
            .and_then(|v| v.as_str())
            .expect("launch result carries child_session_id metadata")
            .to_string()
    }

    pub fn wait_child_turn(&self, child_id: &str) {
        for _ in 0..500 {
            if !self.sessions.is_turn_running_blocking(child_id) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("child turn {child_id} did not finish");
    }

    pub fn wait_child_settled(&self, child_id: &str) {
        self.wait_child_turn(child_id);
    }

    pub fn wait_mailbox(&self, parent_id: &str) {
        self.wait_for(
            || self.runtime.subagent_hub.has_pending(parent_id),
            "completion reference",
        );
    }

    pub fn wait_for(&self, mut predicate: impl FnMut() -> bool, what: &str) {
        for _ in 0..500 {
            if predicate() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }
}
