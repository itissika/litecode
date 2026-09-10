//! Subagent as a tool series: launch detaches to an OS thread; wait/stop consume the hub.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;
use std::time::Duration;

use common::bindings::binding_safe_for;
use common::scripted_provider::HangProvider;
use common::{ScriptedProvider, test_resolved, test_workspace};
use litecode::config::resolved::resolve;
use litecode::config::schema::{AgentProfile, AgentRole};
use litecode::config::{TurnGuard, workspace::set_runtime_paths};
use litecode::context_pipeline::Context;
use litecode::engines::WorkspaceEngines;
use litecode::llm::{LlmProvider, ModelRequest};
use litecode::optional::EngineManager;
use litecode::permission::{PermissionEngine, deny_permission_sink};
use litecode::session::manager::SessionManager;
use litecode::tool::Tool;
use litecode::tool::ToolPipeline;
use litecode::tool::output::DEFAULT_SPILL_THRESHOLD;
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tool::write_lock::process_write_lock;
use litecode::tools::subagent::{MAX_SUBAGENTS_PER_PARENT, SubagentHub, SubagentLaunchTool};
use litecode::types::{
    FunctionToolCall, Item, Result, StreamEvents, ToolSignalLevel, item_text_preview,
};
use tokio_util::sync::CancellationToken;

fn reviewer_resolved(cwd: &std::path::Path) -> litecode::config::ResolvedConfig {
    let workspace = test_workspace(cwd);
    set_runtime_paths(workspace.paths.clone());
    let base = test_resolved("default", &["subagent_launch".into()]);
    let mut global = base.global().clone();
    if let Some(default) = global.agents.get_mut("default") {
        default.allowed_subagents = vec!["reviewer".into()];
        default.max_steps = 4;
    }
    global.agents.insert(
        "reviewer".into(),
        AgentProfile {
            role: AgentRole::Subagent,
            model_ref: "default".into(),
            system_prompt: "builtin:general".into(),
            tools: HashMap::from([("read".into(), binding_safe_for("read"))]),
            max_steps: 2,
            ..Default::default()
        },
    );
    resolve(global, workspace)
}

fn launch_tool(
    resolved: litecode::config::ResolvedConfig,
    sessions: Arc<SessionManager>,
    parent_session_id: &str,
    provider: Box<dyn LlmProvider>,
) -> SubagentLaunchTool {
    let workspace =
        litecode::workspace::WorkspaceService::new(resolved.workspace_root().to_path_buf())
            .expect("workspace");
    let engines = WorkspaceEngines::new();
    let ide = litecode::ide_base::IdeBaseHandle::new(
        workspace,
        Arc::new(engines.clone()),
        Arc::new(litecode::terminal::TerminalHub::new()),
    );
    let hub = Arc::new(SubagentHub::new());
    hub.attach_sessions(Arc::clone(&sessions));
    SubagentLaunchTool::new(
        resolved,
        "default",
        provider,
        "test-key".into(),
        0,
        CancellationToken::new(),
        EngineManager::new(),
        engines,
        ide,
        sessions,
        parent_session_id,
        Arc::new(litecode::mcp::McpConnectionPool::new()),
        hub,
    )
}

fn exec_ctx(call_id: &str, cancel: CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::All,
        workspace_root: std::path::PathBuf::from("."),
        call_id: call_id.to_string(),
        cancel,
        output_limit: 8_000,
        session_id: String::new(),
        session: None,
    }
}

fn function_call(call_id: &str, prompt: &str) -> FunctionToolCall {
    FunctionToolCall {
        arguments: serde_json::json!({
            "agent": "reviewer",
            "prompt": prompt
        })
        .to_string(),
        call_id: call_id.into(),
        name: "subagent_launch".into(),
        namespace: None,
        id: Some(format!("fc_{call_id}")),
        status: None,
    }
}

#[derive(Clone)]
struct ThreadProbeProvider {
    inner: ScriptedProvider,
    thread_id: Arc<Mutex<Option<ThreadId>>>,
    saw_runtime: Arc<AtomicBool>,
}

impl LlmProvider for ThreadProbeProvider {
    fn endpoint(&self) -> &str {
        self.inner.endpoint()
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        self.inner.complete(request, api_key)
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        request: &'a ModelRequest,
        api_key: &'a str,
        on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        cancel: &'a CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        *self.thread_id.lock().unwrap() = Some(std::thread::current().id());
        self.saw_runtime.store(
            tokio::runtime::Handle::try_current().is_ok(),
            Ordering::SeqCst,
        );
        self.inner
            .complete_with_stream_events(request, api_key, on_event, cancel)
    }
}

#[test]
fn launch_declares_pipeline_parallel_and_cancellable() {
    let dir = tempfile::tempdir().unwrap();
    let resolved = reviewer_resolved(dir.path());
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        resolved.paths().sessions_db.to_string_lossy().to_string(),
    ));
    let tool = launch_tool(
        resolved,
        sessions,
        "parent",
        Box::new(ScriptedProvider::with_text("x")),
    );
    let input = serde_json::json!({"agent": "reviewer", "prompt": "go"});
    assert!(
        tool.is_concurrency_safe(&input),
        "subagent_launch must join the concurrent ToolPipeline batch"
    );
    assert!(
        tool.is_cancellable(),
        "subagent_launch must be joinable on turn cancel"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn execute_returns_while_child_llm_runs_on_other_thread() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    let probe = ThreadProbeProvider {
        inner: ScriptedProvider::with_text("other-thread"),
        thread_id: Arc::new(Mutex::new(None)),
        saw_runtime: Arc::new(AtomicBool::new(false)),
    };
    let recorded = Arc::clone(&probe.thread_id);
    let saw_runtime = Arc::clone(&probe.saw_runtime);
    let caller_thread = std::thread::current().id();

    let tool = launch_tool(resolved, sessions, &parent_id, Box::new(probe));
    let result = tool
        .execute(
            serde_json::json!({"agent": "reviewer", "prompt": "go"}),
            exec_ctx("call_thread", CancellationToken::new()),
        )
        .await;

    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    assert!(
        saw_runtime.load(Ordering::SeqCst),
        "child LLM call must run inside a Tokio runtime"
    );
    assert_ne!(
        *recorded.lock().unwrap(),
        Some(caller_thread),
        "child loop must hop to a dedicated OS thread"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_during_foreground_wait_stops_that_child() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    let hang = HangProvider::until_cancel();
    let dropped = Arc::clone(&hang.dropped);
    let started = Arc::clone(&hang.started);
    let tool = launch_tool(resolved, sessions, &parent_id, Box::new(hang));
    let cancel = CancellationToken::new();
    let cancel_watch = cancel.clone();
    tokio::spawn(async move {
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        cancel_watch.cancel();
    });

    let result = tokio::time::timeout(
        Duration::from_secs(8),
        tool.execute(
            serde_json::json!({"agent": "reviewer", "prompt": "hang"}),
            exec_ctx("call_cancel", cancel),
        ),
    )
    .await
    .expect("execute must return after cancel");
    assert!(
        result.content.contains("cancel")
            || result.content.contains("Stopped")
            || result.level == ToolSignalLevel::Error,
        "expected cancelled tool result, got: {}",
        result.content
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stopping the child must drop its LLM future");
}

#[tokio::test(flavor = "current_thread")]
async fn background_launch_returns_while_child_keeps_running() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    let hang = HangProvider::ignore_cancel();
    let dropped = Arc::clone(&hang.dropped);
    let started = Arc::clone(&hang.started);
    let tool = launch_tool(resolved, Arc::clone(&sessions), &parent_id, Box::new(hang));
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tool.execute(
            serde_json::json!({
                "agent": "reviewer",
                "prompt": "hang",
                "run_in_background": true
            }),
            exec_ctx("call_bg", CancellationToken::new()),
        ),
    )
    .await
    .expect("background launch must return without waiting for the child");
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    assert!(result.content.contains("status: running"), "{}", result.content);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child LLM must start after launch returns");
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(
        !dropped.load(Ordering::SeqCst),
        "background child must keep running after launch returns"
    );
    let children = sessions
        .data()
        .list_child_ids_blocking(&parent_id)
        .expect("children");
    assert_eq!(children.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn pipeline_runs_two_launches_concurrently() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    assert!(
        MAX_SUBAGENTS_PER_PARENT >= 2,
        "parent slot cap must allow parallel launches"
    );

    let tool = Arc::new(launch_tool(
        resolved.clone(),
        Arc::clone(&sessions),
        &parent_id,
        Box::new(ScriptedProvider::with_texts(&["one", "two"])),
    ));
    let input = serde_json::json!({"agent": "reviewer", "prompt": "go"});
    assert!(tool.is_concurrency_safe(&input));

    let permission = PermissionEngine::resolver(resolved.clone(), "default", 0);
    let ctx = Context {
        cwd: cwd.to_path_buf(),
        workspace_paths: resolved.paths().clone(),
        agents_md: None,
        claude_md: None,
    };
    let runtime_ctx = Arc::new(litecode::runtime::RuntimeContext::new(
        vec![tool.clone() as Arc<dyn Tool>],
        permission,
        ctx,
        "default",
        deny_permission_sink(),
        CancellationToken::new(),
        sessions.data_root_path(),
        DEFAULT_SPILL_THRESHOLD,
        process_write_lock(),
        Some(sessions.reader()),
    ));
    let mut pipeline = ToolPipeline::new(Arc::clone(&runtime_ctx));
    pipeline.bind_session(parent_id.clone());

    let calls = vec![
        function_call("call_a", "task a"),
        function_call("call_b", "task b"),
    ];
    let mut transcript: Vec<Item> = calls.iter().cloned().map(Item::FunctionCall).collect();
    pipeline
        .execute_batch(&calls, &mut transcript)
        .await
        .expect("pipeline batch");

    let outputs = transcript
        .iter()
        .filter(|item| matches!(item, Item::FunctionCallOutput(_)))
        .count();
    assert_eq!(outputs, 2, "both launches must produce a tool result");
    for item in &transcript {
        if matches!(item, Item::FunctionCallOutput(_)) {
            let text = item_text_preview(item);
            assert!(!text.starts_with("Error:"), "launch failed: {text}");
        }
    }

    let children = sessions
        .data()
        .list_child_ids_blocking(&parent_id)
        .expect("children");
    assert_eq!(
        children.len(),
        2,
        "parallel launches must each own a child session, got {children:?}"
    );
}
