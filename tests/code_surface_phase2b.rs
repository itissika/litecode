//! Phase 2b: `run_code` is the only way to call the code surface.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use litecode::config::schema::{AgentProfile, AgentToolBinding};
use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{
    AskOutcome, AskPrompt, AskReply, PermissionAction, PermissionEngine, PermissionSink,
    RecordingPermissionSink, ToolPolicy,
};
use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use litecode::tool::trait_::ToolExecutionContext;
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::{Tool, ToolSurface};
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
use litecode::tools::read::ReadTool;
use litecode::types::{FunctionToolCall, ToolCallResult, ToolErrorKind};
use serde_json::{Value, json};

fn run_code_result(content: &str) -> Value {
    let rest = content
        .split_once("result:\n")
        .unwrap_or_else(|| panic!("missing result section in {content}"))
        .1;
    serde_json::from_str(rest.trim()).unwrap_or_else(|_| Value::String(rest.trim().to_string()))
}
use tokio_util::sync::CancellationToken;

const SECRET: &str = "secret-file-body-zz";

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
        path_mode: litecode::permission::BindingPathMode::default(),
        last_applied_preset: None,
        allowed_tools: None,
    };
    let mut tools = std::collections::HashMap::new();
    for name in ["read", "glob", "grep", "block", "fast", "hold", "late"] {
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

fn origin(code_names: Arc<[String]>) -> ToolCallOrigin {
    ToolCallOrigin {
        caller: ToolCaller::Direct,
        code_names,
        file_note: None,
        permission_sink: None,
        plan_touch: None,
    }
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
    code_names: Arc<[String]>,
}

fn hosted(
    root: &Path,
    bundle: Vec<Arc<dyn Tool>>,
    policy: PermissionAction,
    sink: Arc<dyn PermissionSink>,
) -> Hosted {
    let permission = engine(root, policy);
    let ctx = ctx(root);
    let data_root = root.join(".litecode");
    std::fs::create_dir_all(&data_root).unwrap();
    let host = Arc::new(CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        sink,
        data_root.clone(),
        process_write_lock(),
        None,
    ));
    let code = Arc::new(CodeTool::new(bundle, host));
    let code_names = code.code_bindings().unwrap_or_else(|| Arc::from([]));
    Hosted {
        tools: vec![code],
        permission,
        ctx,
        data_root,
        code_names,
    }
}

async fn execute(
    hosted: &Hosted,
    name: &str,
    args: Value,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
) -> ToolCallResult {
    execute_tool_call(
        &call(name, args),
        &hosted.tools,
        &hosted.permission,
        &hosted.ctx,
        "phase2b",
        "default",
        sink,
        cancel,
        &hosted.data_root,
        32_768,
        None,
        process_write_lock(),
        None,
        origin(Arc::clone(&hosted.code_names)),
    )
    .await
}

fn object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn flag_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "ok": { "type": "boolean" } },
        "required": ["ok"]
    })
}

struct GateTool {
    name: &'static str,
    started: Arc<AtomicBool>,
    sealed: Arc<AtomicBool>,
    hold: bool,
}

impl GateTool {
    fn hold(name: &'static str, started: Arc<AtomicBool>, sealed: Arc<AtomicBool>) -> Self {
        Self {
            name,
            started,
            sealed,
            hold: true,
        }
    }

    fn fast(name: &'static str) -> Self {
        Self {
            name,
            started: Arc::new(AtomicBool::new(false)),
            sealed: Arc::new(AtomicBool::new(false)),
            hold: false,
        }
    }
}

impl Tool for GateTool {
    fn name(&self) -> &str {
        self.name
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        object_schema()
    }

    fn data_schema(&self) -> Option<Value> {
        Some(flag_schema())
    }

    fn description(&self, _: &litecode::context_pipeline::Context) -> String {
        "phase 2b gate tool".into()
    }

    fn is_concurrency_safe(&self, _: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        _: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let started = Arc::clone(&self.started);
        let sealed = Arc::clone(&self.sealed);
        let hold = self.hold;
        Box::pin(async move {
            if !hold {
                return ToolCallResult::ok("fast").with_data(json!({ "ok": true }));
            }
            started.store(true, Ordering::SeqCst);
            execution.cancel.cancelled().await;
            sealed.store(true, Ordering::SeqCst);
            ToolCallResult::error_with_kind(ToolErrorKind::Cancelled, "sealed", false)
        })
    }
}

struct LateTool;

impl Tool for LateTool {
    fn name(&self) -> &str {
        "late"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        object_schema()
    }

    fn data_schema(&self) -> Option<Value> {
        Some(flag_schema())
    }

    fn description(&self, _: &litecode::context_pipeline::Context) -> String {
        "finishes after the cancel grace".into()
    }

    fn is_concurrency_safe(&self, _: &Value) -> bool {
        true
    }

    fn execute(
        &self,
        _: Value,
        _: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            ToolCallResult::ok("late").with_data(json!({ "ok": true }))
        })
    }
}

struct SlowAllow;

impl PermissionSink for SlowAllow {
    fn ask(&self, _: &AskPrompt<'_>, _: &CancellationToken) -> AskReply {
        std::thread::sleep(Duration::from_millis(700));
        AskReply::from_outcome(AskOutcome::Allow { always: false })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn direct_code_tools_are_refused_and_succeed_through_run_code() {
    unsafe {
        std::env::remove_var("LITECODE_CODE_MODE");
    }
    assert!(std::env::var("LITECODE_CODE_MODE").is_err());

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), SECRET).unwrap();
    let sink = RecordingPermissionSink::from_reply(true, false);
    let asked = Arc::clone(&sink.calls);
    let hosted = hosted(
        dir.path(),
        vec![
            Arc::new(ReadTool::default()),
            Arc::new(GlobTool),
            Arc::new(GrepTool),
        ],
        PermissionAction::Ask,
        Arc::new(sink),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    // The host keeps its own sink. This one is the direct-call sink.
    let direct_asked = Arc::clone(&sink.calls);

    for name in ["read", "glob", "grep"] {
        let result = execute(
            &hosted,
            name,
            json!({ "path": "probe.txt", "pattern": "zz" }),
            &sink,
            CancellationToken::new(),
        )
        .await;
        assert!(
            result.content.contains(&format!(
                "`{name}` is only available as tools.{name} inside run_code"
            )),
            "{name}: {}",
            result.content
        );
        assert_eq!(
            result.error.as_ref().map(|error| error.kind),
            Some(ToolErrorKind::NotFound)
        );
    }
    assert!(
        direct_asked.lock().unwrap().is_empty(),
        "direct routing must happen before authorization: {:?}",
        direct_asked.lock().unwrap()
    );

    let script = r#"
const read = await tools.read({ path: "probe.txt" });
const glob = await tools.glob({ pattern: "probe.txt" });
const grep = await tools.grep({ pattern: "zz", path: "probe.txt" });
return {
  read: read.file ? "ok" : "missing",
  glob: Array.isArray(glob.paths) ? "ok" : "missing",
  grep: Array.isArray(grep.matches) ? "ok" : "missing"
};
"#;
    let result = execute(
        &hosted,
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.error.is_none(),
        "run_code failed: {}",
        result.content
    );
    let value = run_code_result(&result.content);
    assert_eq!(value["read"], "ok", "{}", result.content);
    assert_eq!(value["glob"], "ok", "{}", result.content);
    assert_eq!(value["grep"], "ok", "{}", result.content);
    assert!(
        result.content.contains("3 succeeded, 0 failed"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("calls ("), "{}", result.content);
    assert!(!result.content.contains("diagnostics:"), "{}", result.content);
    assert!(
        !result.content.contains(SECRET),
        "ledger must not include the result body: {}",
        result.content
    );
    assert!(
        !asked.lock().unwrap().is_empty(),
        "nested calls still ask through the same permission sink"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_seals_in_flight_nested_calls() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let sealed = Arc::new(AtomicBool::new(false));
    let hosted = hosted(
        dir.path(),
        vec![Arc::new(GateTool::hold(
            "block",
            Arc::clone(&started),
            Arc::clone(&sealed),
        ))],
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let cancel = CancellationToken::new();
    let sink = RecordingPermissionSink::from_reply(true, false);
    let mut run = std::pin::pin!(execute(
        &hosted,
        "run_code",
        json!({
            "code": "await tools.block({}); return { done: true };",
            "timeout_ms": 5000
        }),
        &sink,
        cancel.clone(),
    ));
    let result = loop {
        tokio::select! {
            result = &mut run => break result,
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                if started.load(Ordering::SeqCst) && !cancel.is_cancelled() {
                    cancel.cancel();
                }
            }
        }
    };
    assert!(
        sealed.load(Ordering::SeqCst),
        "in-flight nested call was not joined"
    );
    assert!(
        result.content.contains("1. block cancelled"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("1/1 calls finished, 0 unsettled"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("calls were still running"),
        "{}",
        result.content
    );
}

#[tokio::test(flavor = "current_thread")]
async fn a_call_that_outlasts_the_grace_period_is_unsettled() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        vec![Arc::new(LateTool)],
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let result = execute(
        &hosted,
        "run_code",
        json!({
            "code": "await tools.late({}); return 'done';",
            "timeout_ms": 400
        }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.content.contains("may still land"),
        "{}",
        result.content
    );
    assert!(result.content.contains("unsettled"), "{}", result.content);
    assert!(
        result.content.contains("1. late") && result.content.contains("unsettled"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("retryable"), "{}", result.content);
}

#[tokio::test(flavor = "current_thread")]
async fn approval_wait_does_not_consume_the_program_budget() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("probe.txt"), "ok").unwrap();
    let sink = Arc::new(SlowAllow);
    let hosted = hosted(
        dir.path(),
        vec![Arc::new(ReadTool::default())],
        PermissionAction::Ask,
        Arc::clone(&sink) as Arc<dyn PermissionSink>,
    );
    let result = execute(
        &hosted,
        "run_code",
        json!({
            "code": "const read = await tools.read({ path: \"probe.txt\" }); return read.file ? \"ok\" : \"missing\";",
            "timeout_ms": 600
        }),
        sink.as_ref(),
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.error.is_none(),
        "approval wait consumed the program budget: {}",
        result.content
    );
    assert_eq!(run_code_result(&result.content), json!("ok"));
}

#[tokio::test(flavor = "current_thread")]
async fn outer_timeout_does_not_overwrite_a_completed_nested_result() {
    let dir = tempfile::tempdir().unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let sealed = Arc::new(AtomicBool::new(false));
    let hosted = hosted(
        dir.path(),
        vec![
            Arc::new(GateTool::fast("fast")),
            Arc::new(GateTool::hold("hold", started, Arc::clone(&sealed))),
        ],
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let result = execute(
        &hosted,
        "run_code",
        json!({
            "code": "await tools.fast({}); await tools.hold({}); return { done: true };",
            "timeout_ms": 1000
        }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        sealed.load(Ordering::SeqCst),
        "timed-out nested call was not joined"
    );
    assert!(result.content.contains("1. fast ok"), "{}", result.content);
    assert!(
        !result.content.contains("1. fast timeout")
            && !result.content.contains("1. fast cancelled"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("2. hold cancelled"),
        "{}",
        result.content
    );
    assert_eq!(
        result.error.as_ref().map(|error| error.kind),
        Some(ToolErrorKind::Timeout)
    );
}

#[test]
fn deep_run_code_return_survives_the_host_process() {
    if std::env::var_os("LITECODE_DEEP_RESULT_PROBE").is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(run_deep_return_probe());
        return;
    }

    let output = std::process::Command::new(std::env::current_exe().expect("test exe"))
        .arg("deep_run_code_return_survives_the_host_process")
        .arg("--exact")
        .arg("--nocapture")
        .env("LITECODE_DEEP_RESULT_PROBE", "1")
        .output()
        .expect("spawn deep-result probe");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "probe crashed ({:?})\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        stdout.contains("PROBE_RESULT_OK"),
        "probe did not return a result\nstdout: {stdout}\nstderr: {stderr}"
    );
}

async fn run_deep_return_probe() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        vec![Arc::new(ReadTool::default())],
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let result = execute(
        &hosted,
        "run_code",
        json!({
            "code": "let value = 0; for (let i = 0; i < 20000; i++) value = [value]; return value;"
        }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.error.is_none(),
        "{}",
        result.content.chars().take(180).collect::<String>()
    );
    let text = result
        .content
        .split_once("result:\n")
        .expect("deep result section")
        .1;
    assert_eq!(text.matches('[').count(), 20_000);
    assert!(text.starts_with("[[[") && text.ends_with("]]]") && text.contains('0'));
    assert!(result.content.contains("[[["));
    drop(result);
    println!("PROBE_RESULT_OK");
}
