//! Phase 5 manual procedure: structured discovery, paging, hash, partial edit,
//! and calls that stay runnable while another approval is pending.

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
use litecode::tools::edit::EditTool;
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
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
    for name in ["read", "glob", "grep", "edit", "write"] {
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

fn surface() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool::default()),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(EditTool::new()),
        Arc::new(WriteTool::new()),
    ]
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
        process_write_lock(),
        None,
    );
    let code = Arc::new(CodeTool::new(surface(), Arc::new(host)));
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
    code: &str,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
) -> litecode::types::ToolCallResult {
    let call = FunctionToolCall {
        arguments: json!({ "code": code }).to_string(),
        call_id: format!("call-{session_id}"),
        namespace: None,
        name: "run_code".into(),
        id: None,
        status: None,
    };
    execute_tool_call(
        &call,
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

async fn wait_flag(flag: &AtomicBool) {
    let start = std::time::Instant::now();
    while !flag.load(Ordering::SeqCst) {
        if start.elapsed() > Duration::from_secs(5) {
            panic!("approval was never entered");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_procedure_chains_glob_read_grep_edit_and_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "chain.txt", "one\ntwo\n");
    write(root, "notes/hit.txt", "NEEDLE\n");
    let blocks: String = (0..10).map(|index| format!("line{index}\n")).collect();
    write(root, "blocks.txt", &blocks);
    let hosted = hosted(
        root,
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let script = r#"
const listed = await tools.glob({ pattern: "chain.txt", limit: 5 });
const window = await tools.read({ path: listed.paths[0], end_line: 1 });
const rest = await tools.read({
  path: listed.paths[0],
  start_line: Number(window.page.next)
});
const found = await tools.grep({ pattern: "NEEDLE", path: "notes" });
const hitPath = found.matches[0].path;
const body = await tools.read({ path: hitPath });
const hash = body.file.hash;
const edited = await tools.edit({
  path: hitPath,
  expected_hash: hash,
  edits: [{ old_string: "NEEDLE", new_string: "DONE" }]
});
let staleCode = "";
try {
  await tools.edit({
    path: hitPath,
    expected_hash: hash,
    edits: [{ old_string: "DONE", new_string: "NOPE" }]
  });
} catch (e) {
  staleCode = e.code;
}
const edits = [];
for (let i = 0; i < 9; i++) {
  edits.push({ old_string: "line" + i, new_string: "done" + i });
}
edits.push({ old_string: "missing-block", new_string: "nope" });
const partial = await tools.edit({ path: "blocks.txt", edits });
const created = await tools.write({ path: "fresh.txt", content: "fresh\n", create_only: true });
const checked = await tools.read({ path: "fresh.txt" });
const failed = partial.blocks.find((block) => block.status === "failed");
return {
  glob_path: listed.paths[0],
  window_line: window.lines[0].text,
  window_next: window.page.next,
  rest_line: rest.lines[0].text,
  hit_path: hitPath,
  edit_status: edited.applied >= 1 ? "ok" : "missing",
  stale_code: staleCode,
  partial_status: "partial",
  partial_applied: partial.applied,
  partial_failed: partial.failed,
  failed_message: failed && failed.reason && failed.reason.message,
  created_hash: created.file.hash,
  checked_hash: checked.file.hash
};
"#;
    let result = execute(
        &hosted,
        "phase5-chain",
        script,
        &litecode::permission::DenyPermissionSink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["glob_path"], "chain.txt", "{value}");
    assert_eq!(value["window_line"], "one");
    assert_eq!(value["window_next"], "2");
    assert_eq!(value["rest_line"], "two");
    assert!(
        value["hit_path"].as_str().unwrap().replace('\\', "/").ends_with("notes/hit.txt"),
        "{value}"
    );
    assert_eq!(value["edit_status"], "ok");
    assert_eq!(value["stale_code"], "stale");
    assert_eq!(value["partial_status"], "partial");
    assert_eq!(value["partial_applied"], 9);
    assert_eq!(value["partial_failed"], 1);
    assert!(value["failed_message"].as_str().unwrap().len() > 0, "{value}");
    assert_eq!(value["created_hash"], value["checked_hash"]);
    assert_eq!(std::fs::read_to_string(root.join("notes/hit.txt")).unwrap(), "DONE\n");
    let written = std::fs::read_to_string(root.join("blocks.txt")).unwrap();
    assert!(written.contains("done0") && written.contains("line9"), "{written}");
    assert!(!written.contains("nope"), "{written}");
    assert_eq!(std::fs::read_to_string(root.join("fresh.txt")).unwrap(), "fresh\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_pending_does_not_lock_other_calls() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "other.txt", "OTHER_NEEDLE\n");
    let ask = Arc::new(BlockingAsk::new());
    let pending = hosted(
        root,
        PermissionAction::Ask,
        Arc::clone(&ask) as Arc<dyn PermissionSink>,
    );
    let cancel = CancellationToken::new();
    let run = tokio::spawn(async move {
        execute(
            &pending,
            "phase5-pending",
            r#"try { await tools.write({ path: "pending.txt", content: "wait" }); } catch (e) {} return "done";"#,
            &litecode::permission::DenyPermissionSink,
            cancel,
        )
        .await
    });
    wait_flag(&ask.entered).await;

    let allow = hosted(
        root,
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let other = tokio::time::timeout(
        Duration::from_secs(3),
        execute(
            &allow,
            "phase5-other",
            r#"
const page = await tools.read({ path: "other.txt" });
const found = await tools.grep({ pattern: "OTHER_NEEDLE", path: "other.txt" });
return { line: page.lines[0].text, hits: found.matches.length };
"#,
            &litecode::permission::DenyPermissionSink,
            CancellationToken::new(),
        ),
    )
    .await
    .expect("another call stayed locked while approval was pending");
    assert!(other.error.is_none(), "{}", other.content);
    let value = run_code_result(&other.content);
    assert_eq!(value["line"], "OTHER_NEEDLE");
    assert_eq!(value["hits"], 1);

    ask.release(AskOutcome::Deny);
    let pending_result = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("pending write did not finish")
        .unwrap();
    assert!(pending_result.error.is_none(), "{}", pending_result.content);
    assert!(!root.join("pending.txt").exists());
    assert_eq!(std::fs::read_to_string(root.join("other.txt")).unwrap(), "OTHER_NEEDLE\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_before_approval_does_not_write_later() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ask = Arc::new(BlockingAsk::new());
    let hosted = hosted(
        root,
        PermissionAction::Ask,
        Arc::clone(&ask) as Arc<dyn PermissionSink>,
    );
    let cancel = CancellationToken::new();
    let cancel_run = cancel.clone();
    let run = tokio::spawn(async move {
        execute(
            &hosted,
            "phase5-cancel",
            r#"await tools.write({ path: "gone.txt", content: "later" }); return "done";"#,
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
        result.content.contains(" cancelled") || result.content.contains(" unsettled"),
        "{}",
        result.content
    );
    assert!(!root.join("gone.txt").exists());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!root.join("gone.txt").exists(), "a write landed after cancellation");
}
