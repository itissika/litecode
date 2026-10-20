//! Phase 4: effect locks are taken after approval, cancellation does not write
//! afterwards, and a tool binding accepts one argument.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use litecode::config::schema::{AgentProfile, AgentToolBinding};
use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{
    AskOutcome, AskPrompt, AskReply, PermissionAction, PermissionEngine, PermissionSink, ToolPolicy,
};
use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::Tool;
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::read::ReadTool;
use litecode::tools::write::WriteTool;
use litecode::types::FunctionToolCall;
use serde_json::{Value, json};

fn run_code_result(content: &str) -> Value {
    let rest = content
        .split_once("result:\n")
        .unwrap_or_else(|| panic!("missing result section in {content}"))
        .1;
    serde_json::from_str(rest.trim()).unwrap_or_else(|_| Value::String(rest.trim().to_string()))
}
use tokio_util::sync::CancellationToken;

fn ctx(root: &Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

fn engine(root: &Path, policy: PermissionAction) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    let binding = AgentToolBinding {
        enabled: true,
        policy: ToolPolicy::with_default(policy),
        path_mode: Default::default(),
        last_applied_preset: None,
        allowed_tools: None,
    };
    let mut tools = std::collections::HashMap::new();
    for name in ["read", "write"] {
        tools.insert(name.to_string(), binding.clone());
    }
    global.agents.insert(
        "default".into(),
        AgentProfile {
            tools,
            ..Default::default()
        },
    );
    let resolved = ConfigManager::resolve_without_catalog(global, WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

fn call(name: &str, args: Value) -> FunctionToolCall {
    FunctionToolCall {
        arguments: args.to_string(),
        call_id: format!("call-{name}"),
        namespace: None,
        name: name.into(),
        id: None,
        status: None,
    }
}

struct Hosted {
    tools: Vec<Arc<dyn Tool>>,
    permission: PermissionEngine,
    ctx: Context,
    data_root: PathBuf,
}

fn hosted(root: &Path, policy: PermissionAction, sink: Arc<dyn PermissionSink>) -> Hosted {
    let permission = engine(root, policy);
    let ctx = ctx(root);
    let data_root = root.join(".litecode");
    std::fs::create_dir_all(&data_root).unwrap();
    let host = CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        sink,
        data_root.clone(),
        32_768,
        process_write_lock(),
        None,
    );
    let code = Arc::new(CodeTool::new(
        vec![
            Arc::new(ReadTool::default()) as Arc<dyn Tool>,
            Arc::new(WriteTool::new()),
        ],
        Arc::new(host),
    ));
    Hosted {
        tools: vec![code],
        permission,
        ctx,
        data_root,
    }
}

async fn execute(
    hosted: &Hosted,
    session_id: &str,
    name: &str,
    args: Value,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
) -> litecode::types::ToolCallResult {
    execute_tool_call(
        &call(name, args),
        &hosted.tools,
        &hosted.permission,
        &hosted.ctx,
        session_id,
        "default",
        sink,
        cancel,
        &hosted.data_root,
        32_768,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Direct,
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await
}

struct BlockingAsk {
    entered: Arc<AtomicBool>,
    release: Arc<(Mutex<Option<AskOutcome>>, Condvar)>,
}

impl BlockingAsk {
    fn new() -> Self {
        Self {
            entered: Arc::new(AtomicBool::new(false)),
            release: Arc::new((Mutex::new(None), Condvar::new())),
        }
    }

    fn release(&self, outcome: AskOutcome) {
        let (lock, cv) = &*self.release;
        *lock.lock().unwrap() = Some(outcome);
        cv.notify_all();
    }
}

impl PermissionSink for BlockingAsk {
    fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        self.entered.store(true, Ordering::SeqCst);
        let (lock, cv) = &*self.release;
        let mut slot = lock.lock().unwrap();
        loop {
            if let Some(outcome) = *slot {
                return AskReply::from_outcome(outcome);
            }
            if cancel.is_cancelled() {
                return AskReply::from_outcome(AskOutcome::Aborted);
            }
            let (next, _) = cv
                .wait_timeout(slot, Duration::from_millis(20))
                .unwrap_or_else(|error| error.into_inner());
            slot = next;
        }
    }
}

struct SlowDeny {
    entered: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl PermissionSink for SlowDeny {
    fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
        self.entered.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(5));
        self.finished.store(true, Ordering::SeqCst);
        AskReply::from_outcome(AskOutcome::Deny)
    }
}

async fn wait_flag(flag: &AtomicBool) {
    let start = std::time::Instant::now();
    while !flag.load(Ordering::SeqCst) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("approval was never entered");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_pending_does_not_preclaim_the_same_path() {
    let dir = tempfile::tempdir().unwrap();
    let ask = Arc::new(BlockingAsk::new());
    let nested = hosted(
        dir.path(),
        PermissionAction::Ask,
        Arc::clone(&ask) as Arc<dyn PermissionSink>,
    );
    let cancel = CancellationToken::new();
    let script = r#"
await tools.write({ path: "same.txt", content: "from-nested" });
return "done";
"#;
    let run = tokio::spawn(async move {
        execute(
            &nested,
            "phase4-nested",
            "run_code",
            json!({ "code": script }),
            &litecode::permission::DenyPermissionSink,
            cancel,
        )
        .await
    });
    wait_flag(&ask.entered).await;

    let allow = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let other = r#"
const read = await tools.read({ path: "same.txt" });
if (read.status === "ok") {
  throw new Error("read saw a file before the pending write was approved");
}
const write = await tools.write({ path: "same.txt", content: "from-direct" });
if (write.status !== "ok") {
  throw new Error("write failed while approval was pending: " + JSON.stringify(write));
}
return "wrote";
"#;
    let other = tokio::time::timeout(
        Duration::from_secs(3),
        execute(
            &allow,
            "phase4-direct",
            "run_code",
            json!({ "code": other }),
            &litecode::permission::DenyPermissionSink,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("a same-path read/write was blocked while approval was still pending");
    assert!(other.error.is_none(), "{}", other.content);
    assert_eq!(run_code_result(&other.content), json!("wrote"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("same.txt")).unwrap(),
        "from-direct"
    );

    ask.release(AskOutcome::Deny);
    let result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("nested write stayed pending after the approval was denied")
        .unwrap();
    assert!(result.error.is_none(), "{}", result.content);
    let ledger = result
        .content
        .lines()
        .find(|line| line.starts_with("1. write"))
        .unwrap_or("");
    assert!(ledger.ends_with(" error:permission_denied"), "{ledger}");
    assert!(result.content.contains("permission_denied"), "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("same.txt")).unwrap(),
        "from-direct"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_before_approval_does_not_write_later() {
    let dir = tempfile::tempdir().unwrap();
    let ask = Arc::new(BlockingAsk::new());
    let hosted = hosted(
        dir.path(),
        PermissionAction::Ask,
        Arc::clone(&ask) as Arc<dyn PermissionSink>,
    );
    let cancel = CancellationToken::new();
    let cancel_run = cancel.clone();
    let script = r#"
await tools.write({ path: "gone.txt", content: "later" });
return "done";
"#;
    let run = tokio::spawn(async move {
        execute(
            &hosted,
            "phase4-cancel",
            "run_code",
            json!({ "code": script }),
            &litecode::permission::DenyPermissionSink,
            cancel_run,
        )
        .await
    });
    wait_flag(&ask.entered).await;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(8), run)
        .await
        .expect("cancelled run did not return")
        .unwrap();
    assert!(
        result.content.contains("1. write path=gone.txt cancelled"),
        "{}",
        result.content
    );
    assert!(!dir.path().join("gone.txt").exists());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !dir.path().join("gone.txt").exists(),
        "a write landed after cancellation"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn grace_expiry_is_unsettled_and_the_later_denial_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let entered = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(SlowDeny {
        entered: Arc::clone(&entered),
        finished: Arc::clone(&finished),
    });
    let hosted = hosted(
        dir.path(),
        PermissionAction::Ask,
        Arc::clone(&sink) as Arc<dyn PermissionSink>,
    );
    let cancel = CancellationToken::new();
    let cancel_run = cancel.clone();
    let script = r#"
await tools.write({ path: "late.txt", content: "maybe" });
return "done";
"#;
    let run = tokio::spawn(async move {
        execute(
            &hosted,
            "phase4-unsettled",
            "run_code",
            json!({ "code": script }),
            &litecode::permission::DenyPermissionSink,
            cancel_run,
        )
        .await
    });
    wait_flag(&entered).await;
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(8), run)
        .await
        .expect("grace did not seal the running call")
        .unwrap();
    assert!(
        result.content.contains("1. write path=late.txt unsettled"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("1. write path=late.txt cancelled"),
        "{}",
        result.content
    );
    assert!(!dir.path().join("late.txt").exists());
    let start = std::time::Instant::now();
    while !finished.load(Ordering::SeqCst) {
        if start.elapsed() > Duration::from_secs(8) {
            panic!("the approval future did not finish");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !dir.path().join("late.txt").exists(),
        "the call wrote after it had already been reported unsettled"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_second_binding_argument_is_invalid_input_and_does_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let script = r#"
const denied = await tools.write({ path: "extra.txt", content: "no" }, { timeout_ms: 1 });
return { status: denied.status, code: denied.error && denied.error.code };
"#;
    let result = execute(
        &hosted,
        "phase4-args",
        "run_code",
        json!({ "code": script }),
        &litecode::permission::DenyPermissionSink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["status"], "error");
    assert_eq!(value["code"], "invalid_input");
    let ledger = result
        .content
        .lines()
        .find(|line| line.starts_with("1. write"))
        .unwrap_or("");
    assert!(ledger.ends_with(" error:invalid_input"), "{ledger}");
    assert!(
        result.content.contains("1. write path=extra.txt error"),
        "{}",
        result.content
    );
    assert!(!dir.path().join("extra.txt").exists());
}
