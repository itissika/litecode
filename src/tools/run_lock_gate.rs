//! WP-206 group A: `run` foreground window + locks held only until spawn.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::GlobalSettings;
use crate::context_pipeline::Context;
use crate::permission::PermissionEngine;
use crate::permission::sinks::RecordingPermissionSink;
use crate::terminal::AgentJobRegistry;
use crate::tool::run_scope::{RunScope, SpawnLocks, scope};
use crate::tool::trait_::{Tool, ToolExecutionContext};
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tools::code::{CodeTool, CodeToolHost};
use crate::tools::command_job::test_execution;
use crate::tools::run::RunTool;
use crate::types::ToolCallResult;

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn guarded_scope(deadline: Option<tokio::time::Instant>) -> (RunScope, Arc<AtomicBool>) {
    let flag = Arc::new(AtomicBool::new(false));
    let locks = SpawnLocks::default();
    locks.hold(Box::new(DropFlag(Arc::clone(&flag))));
    (
        RunScope {
            deadline,
            spawn_locks: locks,
        },
        flag,
    )
}

async fn run_in(scope_: RunScope, tool: &RunTool, root: &Path, command: &str) -> ToolCallResult {
    scope(
        scope_,
        tool.execute(json!({ "command": command }), test_execution(root, "s-lock")),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn short_command_returns_exit_code_in_foreground() {
    let dir = tempfile::tempdir().unwrap();
    let tool = RunTool::new(Arc::new(AgentJobRegistry::new()));
    let (s, _) = guarded_scope(None);
    let result = run_in(s, &tool, dir.path(), "sleep 2 && echo done").await;
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.data.as_ref().unwrap()["exit_code"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_is_capped_by_run_code_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let jobs = Arc::new(AgentJobRegistry::new());
    let tool = RunTool::new(Arc::clone(&jobs));
    let (s, _) = guarded_scope(Some(tokio::time::Instant::now() + Duration::from_millis(1500)));
    let started = Instant::now();
    let result = run_in(s, &tool, dir.path(), "sleep 30").await;
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
    let data = result.data.as_ref().expect("data");
    assert!(data["exit_code"].is_null(), "{data}");
    let id = data["id"].as_str().unwrap().to_string();
    let _ = jobs.kill_registered(&id, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locks_release_after_spawn_while_command_still_runs() {
    let dir = tempfile::tempdir().unwrap();
    let tool = Arc::new(RunTool::new(Arc::new(AgentJobRegistry::new())));
    let (s, flag) = guarded_scope(None);
    let root = dir.path().to_path_buf();
    let t = Arc::clone(&tool);
    let task = tokio::spawn(async move { run_in(s, &t, &root, "sleep 2").await });
    let deadline = Instant::now() + Duration::from_millis(1500);
    while !flag.load(Ordering::SeqCst) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(flag.load(Ordering::SeqCst), "locks must drop once spawned");
    assert!(!task.is_finished(), "run must still be waiting in the foreground");
    let result = task.await.unwrap();
    assert_eq!(result.data.as_ref().unwrap()["exit_code"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_failure_releases_locks() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    let tool = RunTool::new(Arc::new(AgentJobRegistry::new()));
    let (s, flag) = guarded_scope(None);
    let keep = s.spawn_locks.clone();
    let result = run_in(s, &tool, &missing, "echo x").await;
    assert_eq!(result.error_code.as_deref(), Some("io"), "{result:?}");
    assert!(flag.load(Ordering::SeqCst));
    assert_eq!(keep.held(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_before_spawn_releases_when_the_call_ends() {
    let dir = tempfile::tempdir().unwrap();
    let tool = RunTool::new(Arc::new(AgentJobRegistry::new()));
    let (s, flag) = guarded_scope(None);
    let mut exec: ToolExecutionContext = test_execution(dir.path(), "s-lock");
    exec.cancel = CancellationToken::new();
    exec.cancel.cancel();
    // The executor's clone of the scope is the last owner; it ends with the call.
    let result = scope(s, tool.execute(json!({"command": "echo x"}), exec)).await;
    assert_eq!(result.error_code.as_deref(), Some("cancelled"));
    assert!(flag.load(Ordering::SeqCst), "dropped scope must release");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_in_flight_future_releases_and_panic_unwind_releases() {
    // Cancel mid-call: abort the task before spawn completes / during wait.
    let dir = tempfile::tempdir().unwrap();
    let tool = Arc::new(RunTool::new(Arc::new(AgentJobRegistry::new())));
    let (s, flag) = guarded_scope(None);
    let root = dir.path().to_path_buf();
    let t = Arc::clone(&tool);
    let task = tokio::spawn(async move { run_in(s, &t, &root, "sleep 5").await });
    task.abort();
    let _ = task.await;
    assert!(flag.load(Ordering::SeqCst));

    // Panic inside the scoped future: unwinding drops the guards.
    let (s, flag) = guarded_scope(None);
    let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(scope(
        s,
        async {
            panic!("boom");
        },
    )))
    .await;
    assert!(outcome.is_err());
    assert!(flag.load(Ordering::SeqCst));
}

fn context(root: &Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

/// Acceptance: 4 parallel ~1s non-read-only runs via Promise.all overlap.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_parallel_write_runs_finish_in_about_one_second() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let resolved = resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
    let host = Arc::new(CodeToolHost::new(
        PermissionEngine::resolver(resolved, "default", 0),
        context(root),
        "default",
        Arc::new(RecordingPermissionSink::from_reply(true, true)),
        root.join(".litecode"),
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ));
    let bundle: Vec<Arc<dyn Tool>> = vec![Arc::new(RunTool::new(Arc::new(AgentJobRegistry::new())))];
    let tool = CodeTool::new(bundle, host);
    let code = "const rs = await Promise.all([1,2,3,4].map(i => tools.process.run({command:`sleep 1 && touch f${i}`}))); return rs.map(r => r.exit_code);";
    let started = Instant::now();
    let result = tool
        .execute(
            json!({ "code": code }),
            ToolExecutionContext {
                path_mode: crate::workspace::ToolPathMode::All,
                workspace_root: root.to_path_buf(),
                call_id: "wp206-a".into(),
                cancel: CancellationToken::new(),
                output_limit: usize::MAX,
                session_id: "wp206-a".into(),
                session: None,
            },
        )
        .await;
    let elapsed = started.elapsed();
    assert!(result.error.is_none(), "{}", result.content);
    for i in 1..=4 {
        assert!(root.join(format!("f{i}")).exists(), "f{i}: {}", result.content);
    }
    assert!(result.content.contains("[0,0,0,0]") || result.content.contains("[\n  0,"), "{}", result.content);
    assert!(
        elapsed >= Duration::from_millis(950) && elapsed < Duration::from_millis(3500),
        "4x1s parallel runs took {elapsed:?} (serial would be ~4s)"
    );
}
