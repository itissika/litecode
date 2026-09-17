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
use litecode::session::manager::{SessionManager, SessionStatus};
use litecode::tool::Tool;
use litecode::tool::ToolPipeline;
use litecode::tool::output::DEFAULT_SPILL_THRESHOLD;
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tool::write_lock::process_write_lock;
use litecode::tools::subagent::{SubagentHub, SubagentLaunchTool, SubagentStopTool};
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

fn launch_tool_with_hub(
    resolved: litecode::config::ResolvedConfig,
    sessions: Arc<SessionManager>,
    parent_session_id: &str,
    provider: Box<dyn LlmProvider>,
) -> (SubagentLaunchTool, Arc<SubagentHub>) {
    let workspace =
        litecode::workspace::WorkspaceService::new(resolved.workspace_root().to_path_buf())
            .expect("workspace");
    let engines = WorkspaceEngines::new();
    let ide = litecode::ide_base::IdeBaseHandle::new(
        workspace,
        Arc::new(engines.clone()),
        Arc::new(litecode::terminal::TerminalHub::new()),
    );
    let runtime = litecode::runtime::RuntimeHandle::new(
        resolved.clone(),
        "default".into(),
        test_workspace(resolved.workspace_root()),
        Arc::new(EngineManager::new()),
        Arc::new(engines),
        ide,
        Arc::new(std::sync::atomic::AtomicU64::new(0)),
        // Dummy global DB path: never applied (revision stays 0).
        resolved.workspace_root().join(".litecode/global.db"),
    )
    .with_test_llm_override(Arc::from(provider));
    runtime.subagent_hub.attach_sessions(Arc::clone(&sessions));
    let hub = Arc::clone(&runtime.subagent_hub);
    let tool = SubagentLaunchTool::new(
        runtime,
        "default",
        0,
        CancellationToken::new(),
        Arc::clone(&sessions),
        parent_session_id,
    );
    (tool, hub)
}

fn launch_tool(
    resolved: litecode::config::ResolvedConfig,
    sessions: Arc<SessionManager>,
    parent_session_id: &str,
    provider: Box<dyn LlmProvider>,
) -> SubagentLaunchTool {
    launch_tool_with_hub(resolved, sessions, parent_session_id, provider).0
}

fn exec_ctx_for(
    session_id: &str,
    call_id: &str,
    cancel: CancellationToken,
) -> ToolExecutionContext {
    ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::All,
        workspace_root: std::path::PathBuf::from("."),
        call_id: call_id.to_string(),
        cancel,
        output_limit: 8_000,
        session_id: session_id.to_string(),
        session: None,
    }
}

fn exec_ctx(call_id: &str, cancel: CancellationToken) -> ToolExecutionContext {
    exec_ctx_for("", call_id, cancel)
}

fn function_call(call_id: &str, prompt: &str) -> FunctionToolCall {
    FunctionToolCall {
        arguments: serde_json::json!({
            "agent": "reviewer",
            "responsibility": "test",
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

#[derive(Clone)]
struct PanicProvider;

impl LlmProvider for PanicProvider {
    fn endpoint(&self) -> &str {
        "panic://"
    }

    fn box_clone(&self) -> Box<dyn LlmProvider> {
        Box::new(self.clone())
    }

    fn complete_with_stream_events<'a>(
        &'a self,
        _request: &'a ModelRequest,
        _api_key: &'a str,
        _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
        _cancel: &'a CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Item>>> + Send + 'a>> {
        Box::pin(async { panic!("provider boom") })
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
    let input = serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "go"});
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
            serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "go"}),
            exec_ctx("call_thread", CancellationToken::new()),
        )
        .await;

    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    assert!(
        result.content.contains("status: running"),
        "launch must detach immediately, got: {}",
        result.content
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while !saw_runtime.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child LLM call must run inside a Tokio runtime");
    assert_ne!(
        *recorded.lock().unwrap(),
        Some(caller_thread),
        "child loop must hop to a dedicated OS thread"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn parent_cancel_does_not_stop_background_child_and_stop_tool_can() {
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
    let (tool, _hub) =
        launch_tool_with_hub(resolved, Arc::clone(&sessions), &parent_id, Box::new(hang));
    let parent_cancel = CancellationToken::new();
    let result = tool
        .execute(
            serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "hang"}),
            exec_ctx("call_bg_cancel", parent_cancel.clone()),
        )
        .await;
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    tokio::time::timeout(Duration::from_secs(10), async {
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child LLM must start");

    // Cancelling the parent turn signal must not cancel the background child.
    parent_cancel.cancel();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(
        sessions.is_turn_running_blocking(&child_id),
        "parent cancel must not stop a background subagent"
    );
    assert!(!dropped.load(Ordering::SeqCst));

    // The first-class stop operation cancels the child's current turn.
    let stop = SubagentStopTool::new(Arc::clone(&sessions));
    let stop_result = stop
        .execute(
            serde_json::json!({"id": child_id}),
            exec_ctx_for(&parent_id, "call_bg_stop", CancellationToken::new()),
        )
        .await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stop must cancel the child turn");
    assert!(
        stop_result.content.contains("stop_requested")
            || stop_result.content.contains("already ended"),
        "stop result: {}",
        stop_result.content
    );
    tokio::time::timeout(Duration::from_secs(10), async {
        while sessions.is_turn_running_blocking(&child_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child hub record must settle after stop");
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
        Duration::from_secs(10),
        tool.execute(
            serde_json::json!({
                "agent": "reviewer",
                "responsibility": "test",
                "prompt": "hang"
            }),
            exec_ctx("call_bg", CancellationToken::new()),
        ),
    )
    .await
    .expect("background launch must return without waiting for the child");
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    assert!(
        result.content.contains("status: running"),
        "{}",
        result.content
    );
    tokio::time::timeout(Duration::from_secs(10), async {
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
async fn child_exit_fires_hub_exit_handler() {
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

    let (tool, hub) = launch_tool_with_hub(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        Box::new(ScriptedProvider::with_text("done")),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    hub.set_exit_handler(Arc::new(move |notice| {
        let _ = tx.send(notice.child_session_id);
    }));

    let result = tool
        .execute(
            serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "go"}),
            exec_ctx("call_exit_handler", CancellationToken::new()),
        )
        .await;
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    let notified = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(15)))
        .await
        .expect("join")
        .expect("child exit notice must fire the configured exit handler");
    assert_eq!(notified, child_id);
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

    let tool = Arc::new(launch_tool(
        resolved.clone(),
        Arc::clone(&sessions),
        &parent_id,
        Box::new(ScriptedProvider::with_texts(&["one", "two"])),
    ));
    let input = serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "go"});
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_panic_finishes_job_and_releases_session() {
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

    let (tool, _hub) = launch_tool_with_hub(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        Box::new(PanicProvider),
    );
    let result = tool
        .execute(
            serde_json::json!({"agent": "reviewer", "responsibility": "test", "prompt": "go"}),
            exec_ctx("call_panic", CancellationToken::new()),
        )
        .await;
    assert_eq!(result.level, ToolSignalLevel::Ok, "{}", result.content);
    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    tokio::time::timeout(Duration::from_secs(10), async {
        while sessions.is_turn_running_blocking(&child_id) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("panicking worker must still finalize its job");
    assert_eq!(
        sessions.session_status(&child_id),
        Some(SessionStatus::Idle),
        "panicking worker must release the child session"
    );
}
