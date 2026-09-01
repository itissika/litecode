//! Subagent first-class session isolation —end-to-end coverage.
//!
//! Asserts durable child sessions, parent linkage, no parent-channel pollution,
//! lifecycle filtering, and cascade delete.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::bindings::binding_safe_for;
use common::{ScriptedProvider, test_resolved, test_workspace};
use litecode::config::resolved::resolve;
use litecode::config::schema::{AgentProfile, AgentRole};
use litecode::config::{TurnGuard, workspace::set_runtime_paths};
use litecode::engines::WorkspaceEngines;
use litecode::optional::EngineManager;
use litecode::runtime::observer::InternalEvent;
use litecode::session::live::LifecycleEvent;
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
    SubagentLaunchTool::new(
        resolved,
        "default",
        Box::new(provider),
        "test-key".into(),
        0,
        CancellationToken::new(),
        EngineManager::new(),
        engines,
        ide,
        sessions,
        parent_session_id,
        Arc::new(litecode::mcp::McpConnectionPool::new()),
    )
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
        .open_session(&project, "default", Some("default"))
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
        result.content.contains("review complete"),
        "expected subagent text, got: {}",
        result.content
    );

    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .expect("child_session_id in metadata")
        .to_string();

    let child_meta = sessions.data().meta_blocking(&child_id).expect("child row");
    assert_eq!(
        child_meta.parent_session_id.as_deref(),
        Some(parent_id.as_str())
    );
    assert_eq!(child_meta.parent_call_id.as_deref(), Some("call_launch_1"));

    let listed = sessions.data().list_sessions_blocking().unwrap();
    assert_eq!(listed.len(), 1, "child must not appear in top-level list");
    assert_eq!(listed[0].0, parent_id);

    let transcript = sessions
        .data()
        .transcript_blocking(&child_id)
        .expect("child transcript");
    assert!(
        !transcript.is_empty(),
        "child transcript must persist after tool returns"
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
        .open_session(&project, "default", Some("default"))
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
            "prompt": "go"
        }),
    )
    .await;
    assert!(
        !result.content.starts_with("Error:"),
        "launch failed: {}",
        result.content
    );

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
async fn subagent_bound_arrives_on_parent_before_tool_returns() {
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
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    // Waiter must subscribe before launch; keep call_async on this task so TLS
    // `call_id` is visible (thread-local is not inherited by tokio::spawn workers).
    let wait_sessions = Arc::clone(&sessions);
    let wait_parent = parent_id.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let waiter = tokio::spawn(async move {
        let mut rx = wait_sessions
            .subscribe(&wait_parent)
            .expect("parent subscribe");
        let _ = ready_tx.send(());
        loop {
            let env = rx.recv().await.expect("parent channel closed");
            if let InternalEvent::SubagentBound {
                call_id,
                child_session_id,
            } = env.event
            {
                return (call_id, child_session_id);
            }
        }
    });
    ready_rx.await.expect("waiter ready");

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("child done"),
    );

    let result = run_subagent(
        &tool,
        "call_bind_1",
        serde_json::json!({
            "agent": "reviewer",
            "prompt": "go"
        }),
    )
    .await;

    let bound = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("SubagentBound waiter timed out")
        .expect("SubagentBound waiter joined");

    assert_eq!(bound.0, "call_bind_1");
    assert!(
        !bound.1.is_empty(),
        "child_session_id must be non-empty on bind"
    );
    assert!(
        sessions.data().meta_blocking(&bound.1).is_ok(),
        "child session must exist when SubagentBound arrives"
    );
    assert!(
        !result.content.starts_with("Error:"),
        "launch failed: {}",
        result.content
    );
    assert_eq!(
        result
            .metadata
            .as_ref()
            .and_then(|m| m.get("child_session_id"))
            .and_then(|v| v.as_str()),
        Some(bound.1.as_str()),
        "metadata child id must match immediate bind"
    );

    let bindings = sessions.child_bindings_for_parent(&parent_id);
    assert_eq!(
        bindings.get("call_bind_1").map(String::as_str),
        Some(bound.1.as_str()),
        "rebuild path must resolve call_id →child via bindings"
    );
    assert_eq!(
        sessions
            .child_session_id_for_call(&parent_id, "call_bind_1")
            .as_deref(),
        Some(bound.1.as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_lifecycle_is_not_broadcast_to_workspace() {
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

    let mut life_rx = sessions.subscribe_lifecycle();

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("ok"),
    );
    let result = run_subagent(
        &tool,
        "call_life_1",
        serde_json::json!({
            "agent": "reviewer",
            "prompt": "x"
        }),
    )
    .await;
    assert!(!result.content.starts_with("Error:"), "{}", result.content);

    let child_id = result
        .metadata
        .as_ref()
        .and_then(|m| m.get("child_session_id"))
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Give fanout a moment to emit (if it incorrectly would).
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut child_turn_events = 0;
    while let Ok(ev) = life_rx.try_recv() {
        let sid = match &ev {
            LifecycleEvent::TurnStarted { session_id, .. }
            | LifecycleEvent::TurnProgress { session_id, .. }
            | LifecycleEvent::TurnFinished { session_id, .. }
            | LifecycleEvent::TurnStep { session_id, .. }
            | LifecycleEvent::SessionPreviewUpdated { session_id, .. } => session_id.as_str(),
            LifecycleEvent::SessionRemoved { .. } => continue,
        };
        if sid == child_id {
            child_turn_events += 1;
        }
    }
    assert_eq!(
        child_turn_events, 0,
        "workspace lifecycle must not carry child turn events"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_binding_aborts_orphan_child_session() {
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
        .open_session(&project, "default", Some("default"))
        .await
        .expect("parent");

    // Force LLM binding failure via unknown model override.
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
            "prompt": "x",
            "model": "does-not-exist-model-id"
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
        .block_on(sessions.open_session(&project, "default", Some("default")))
        .expect("parent");

    let tool = launch_tool(
        resolved,
        Arc::clone(&sessions),
        &parent_id,
        ScriptedProvider::with_text("unused"),
    );
    let result = tool.call(serde_json::json!({
        "agent": "reviewer",
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
async fn gc_skips_empty_child_while_parent_exists() {
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
    // Keep parent non-empty so GC cannot cascade-delete via the parent row.
    sessions
        .insert_detail_rows(&parent_id, &[litecode::types::user_text("keep")])
        .expect("seed parent transcript");
    let child_id = sessions
        .open_child_session(&project, "reviewer", None, &parent_id, "call_gc")
        .unwrap();
    assert!(
        sessions.is_child_session(&child_id),
        "child must be recognized before GC"
    );

    sessions
        .gc_stale_empty_sessions(Duration::from_secs(0))
        .await;

    assert!(
        sessions.data().meta_blocking(&child_id).is_ok(),
        "GC must not delete empty child while parent still exists"
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
