//! Subagent first-class session isolation —end-to-end coverage.
//!
//! Asserts durable child sessions, parent linkage, no parent-channel pollution,
//! lifecycle filtering, and cascade delete.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use common::bindings::binding_safe_for;
use common::{
    ScriptedProvider, TEST_PRIMARY_MODEL_REF, default_test_catalog, test_resolved, test_workspace,
};
use litecode::config::resolved::resolve;
use litecode::config::schema::{AgentProfile, AgentRole};
use litecode::config::{TurnGuard, workspace::set_runtime_paths};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;
use litecode::runtime::RuntimeHandle;
use litecode::runtime::observer::InternalEvent;
use litecode::runtime::subagent_auto_turn::install_subagent_auto_turn;
use litecode::session::event::EventType;
use litecode::session::manager::SessionManager;
use litecode::tool::Tool;
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tools::subagent::SubagentLaunchTool;
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
            model_ref: TEST_PRIMARY_MODEL_REF.into(),
            system_prompt: "builtin:general".into(),
            tools: HashMap::from([("read".into(), binding_safe_for("read"))]),
            max_steps: 2,
            ..Default::default()
        },
    );
    resolve(global, workspace, default_test_catalog())
}

fn launch_tool_on_sessions(
    resolved: litecode::config::ResolvedConfig,
    sessions: Arc<SessionManager>,
    parent_session_id: &str,
    provider: ScriptedProvider,
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
    let ws_state = test_workspace(resolved.workspace_root());
    let db_path = resolved.workspace_root().join(".litecode/global.db");
    let runtime = RuntimeHandle::new(
        resolved,
        "default".into(),
        ws_state,
        Arc::new(EngineManager::new()),
        Arc::new(engines),
        ide,
        Arc::new(AtomicU64::new(0)),
        // Dummy global DB path: never applied in these tests (revision stays 0).
        db_path,
    )
    .with_test_llm_override(Arc::new(provider));
    runtime.subagent_hub.attach_sessions(Arc::clone(&sessions));
    SubagentLaunchTool::new(
        runtime,
        "default",
        0,
        CancellationToken::new(),
        sessions,
        parent_session_id,
    )
}

fn launch_tool(
    resolved: litecode::config::ResolvedConfig,
    sessions: Arc<SessionManager>,
    parent_session_id: &str,
    provider: ScriptedProvider,
) -> SubagentLaunchTool {
    launch_tool_on_sessions(resolved, sessions, parent_session_id, provider)
}

/// Run the subagent tool through `execute` with an explicit parent call_id
/// (REV-9: no TLS; the call_id travels in the execution context).
async fn run_subagent(
    tool: &SubagentLaunchTool,
    call_id: &str,
    input: serde_json::Value,
) -> litecode::types::ToolCallResult {
    let ctx = ToolExecutionContext {
        path_mode: litecode::workspace::ToolPathMode::All,
        workspace_root: std::path::PathBuf::from("."),
        call_id: call_id.to_string(),
        cancel: CancellationToken::new(),
        output_limit: 8_000,
        session_id: String::new(),
        session: None,
    };
    tool.execute(input, ctx).await
}

fn wait_child_turn(sessions: &SessionManager, child_id: &str) {
    for _ in 0..500 {
        if !sessions.is_turn_running_blocking(child_id) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("child turn {child_id} did not finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_launch_creates_durable_child_with_parent_link() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.clone(),
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some(TEST_PRIMARY_MODEL_REF))
        .await
        .expect("parent");

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("review complete"),
    );

    let result = run_subagent(
        &tool,
        "call_launch_1",
        serde_json::json!({
            "agent": "reviewer",
            "responsibility": "test",
            "prompt": "review this"
        }),
    )
    .await;

    assert!(
        !result.content.starts_with("Error:"),
        "launch failed: {}",
        result.content
    );
    assert!(
        result.content.contains("status: running"),
        "launch must detach immediately, got: {}",
        result.content
    );

    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .expect("child_session_id in metadata")
        .to_string();
    wait_child_turn(&sessions, &child_id);

    let child_meta = sessions.data().meta_blocking(&child_id).expect("child row");
    assert_eq!(
        child_meta.parent_session_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(child_meta.parent_call_id.as_deref(), Some("call_launch_1"));
    assert_eq!(child_meta.responsibility, "test");

    let listed = sessions.data().list_sessions_blocking().unwrap();
    assert!(
        listed.iter().any(|row| row.id == parent_id),
        "parent remains in session/list"
    );
    let child_row = listed
        .iter()
        .find(|row| row.id == child_id)
        .expect("child belongs in session/list");
    assert_eq!(
        child_row.parent_session_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(child_row.parent_call_id.as_deref(), Some("call_launch_1"));
    assert_eq!(child_row.agent_id, "reviewer");

    let transcript = sessions
        .data()
        .transcript_blocking(&child_id)
        .expect("child transcript");
    assert!(
        transcript
            .iter()
            .any(|item| litecode::types::item_text_preview(item).contains("review complete")),
        "child transcript must contain the subagent output"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_event_channel_does_not_receive_child_turn_events() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.clone(),
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some(TEST_PRIMARY_MODEL_REF))
        .await
        .expect("parent");

    // Subscribe to parent session broadcast before launching child.
    let mut parent_rx = sessions.subscribe(&parent_id).expect("parent subscribe");

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("child done"),
    );

    let result = run_subagent(
        &tool,
        "call_iso_1",
        serde_json::json!({
            "agent": "reviewer",
            "responsibility": "test",
            "prompt": "go"
        }),
    )
    .await;
    assert!(
        !result.content.starts_with("Error:"),
        "launch failed: {}",
        result.content
    );
    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .expect("child_session_id")
        .to_string();
    wait_child_turn(&sessions, &child_id);

    // Drain any pending parent envelopes —must not include child TurnStarted.
    let mut leaked = Vec::new();
    while let Ok(env) = parent_rx.try_recv() {
        if matches!(env.event, InternalEvent::TurnStarted { .. }) {
            leaked.push(env);
        }
    }
    assert!(
        leaked.is_empty(),
        "parent channel must not receive child TurnStarted: {leaked:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_binding_aborts_orphan_child_session() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let workspace = test_workspace(cwd);
    set_runtime_paths(workspace.paths.clone());
    let mut global = reviewer_resolved(cwd).global().clone();
    if let Some(reviewer) = global.agents.get_mut("reviewer") {
        reviewer.model_ref.clear();
    }
    // Nothing is selectable at all: no provider holds a credential, so neither the
    // agent's own ref nor the runnable fallback can seed the child. (An empty or
    // stale ref on its own would be auto-healed before the turn reads it.)
    global.provider_credentials.clear();
    let resolved = resolve(global, workspace, default_test_catalog());
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.clone(),
    ));
    let parent_id = sessions
        .open_session(&project, "default", Some(TEST_PRIMARY_MODEL_REF))
        .await
        .expect("parent");

    // The child cannot resolve a model, so its turn fails before it ever runs.
    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("unused"),
    );
    let result = run_subagent(
        &tool,
        "call_fail_1",
        serde_json::json!({
            "agent": "reviewer",
            "responsibility": "test",
            "prompt": "x"
        }),
    )
    .await;

    assert!(
        result.content.contains("Error:"),
        "expected error, got: {}",
        result.content
    );

    let children = sessions.data().list_child_ids_blocking(&parent_id).unwrap();
    assert!(
        children.is_empty(),
        "failed launch must not leave orphan child rows: {children:?}"
    );
}

#[test]
fn missing_call_id_scope_errors_without_creating_child() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.clone(),
    ));
    // Sync open via block_on
    let parent_id = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(sessions.open_session(&project, "default", Some(TEST_PRIMARY_MODEL_REF)))
        .expect("parent");

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("unused"),
    );
    let result = tool.call(serde_json::json!({
        "agent": "reviewer",
        "responsibility": "test",
        "prompt": "x"
    }));
    assert!(
        result.content.contains("call_id"),
        "expected call_id scope error: {}",
        result.content
    );
    assert!(
        sessions
            .data()
            .list_child_ids_blocking(&parent_id)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_parent_cascades_child() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let workspace = test_workspace(cwd);
    let db_path = workspace.paths.sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path.clone(),
    ));
    let parent_id = sessions
        .open_session(&project, "default", None)
        .await
        .unwrap();
    let child_id = sessions
        .open_child_session(&project, "reviewer", None, &parent_id, "call_del")
        .unwrap();

    sessions.remove_session(&parent_id).unwrap();
    assert!(sessions.data().meta_blocking(&parent_id).is_err());
    assert!(sessions.data().meta_blocking(&child_id).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_exit_triggers_parent_auto_turn_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let resolved = reviewer_resolved(cwd);
    let workspace_state = test_workspace(cwd);
    let db_path = resolved.paths().sessions_db.to_string_lossy().to_string();
    let project = cwd.to_string_lossy().to_string();

    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db_path,
    ));
    let parent = sessions
        .open_session(&project, "default", Some(TEST_PRIMARY_MODEL_REF))
        .await
        .expect("parent");
    // Attach acts as the UI subscriber that makes idle completion wake real.
    let _ = sessions.attach(&parent);

    let workspace = litecode::workspace::WorkspaceService::new(cwd.to_path_buf()).unwrap();
    let engines = Arc::new(WorkspaceEngines::new());
    let ide = litecode::ide_base::IdeBaseHandle::new(
        workspace,
        Arc::clone(&engines),
        Arc::new(litecode::terminal::TerminalHub::new()),
    );
    let runtime = RuntimeHandle::new(
        resolved.clone(),
        "default".into(),
        workspace_state,
        Arc::new(EngineManager::new()),
        engines,
        ide,
        Arc::new(AtomicU64::new(0)),
        cwd.join("global.db"),
    )
    .with_test_llm_override(Arc::new(ScriptedProvider::with_text("done")));
    runtime.subagent_hub.attach_sessions(Arc::clone(&sessions));
    let hub = Arc::clone(&runtime.subagent_hub);
    install_subagent_auto_turn(
        hub,
        Arc::new(RwLock::new(runtime.clone())),
        Arc::clone(&sessions),
        cwd.to_path_buf(),
    );

    let tool = SubagentLaunchTool::new(
        runtime,
        "default",
        0,
        CancellationToken::new(),
        Arc::clone(&sessions),
        parent.clone(),
    );
    let result = run_subagent(
        &tool,
        "call_auto_turn",
        serde_json::json!({
            "agent": "reviewer",
            "responsibility": "test",
            "prompt": "go"
        }),
    )
    .await;
    assert!(
        result.content.contains("status: running"),
        "launch must detach: {}",
        result.content
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let events = sessions
            .data()
            .events_blocking(&parent)
            .expect("parent events");
        if events.iter().any(|event| {
            matches!(
                event.event_type,
                EventType::Reminder(litecode::reminder::ReminderKind::SubagentSettled)
            )
        }) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "child exit must trigger the parent auto-turn reminder"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
