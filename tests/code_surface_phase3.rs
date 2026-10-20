//! Phase 3: `edit` and `write` join the code surface.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use litecode::config::schema::{AgentProfile, AgentToolBinding};
use litecode::config::{ConfigManager, GlobalSettings, TurnGuard, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{
    AskOutcome, AskPrompt, AskReply, BindingPathMode, PermissionAction, PermissionEngine,
    PermissionSink, RecordingPermissionSink, ToolPolicy,
};
use litecode::session::manager::SessionManager;
use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use litecode::tool::project::project_tools;
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::{Tool, ToolSurface};
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::edit::EditTool;
use litecode::tools::read::ReadTool;
use litecode::tools::write::WriteTool;
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

fn ctx(root: &Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

fn engine(root: &Path, policy: PermissionAction) -> PermissionEngine {
    engine_with_mode(root, policy, BindingPathMode::default())
}

fn engine_with_mode(
    root: &Path,
    policy: PermissionAction,
    path_mode: BindingPathMode,
) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    let binding = AgentToolBinding {
        enabled: true,
        policy: ToolPolicy::with_default(policy),
        path_mode,
        last_applied_preset: None,
        allowed_tools: None,
    };
    let mut tools = std::collections::HashMap::new();
    for name in ["read", "edit", "write"] {
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
    code_names: Arc<[String]>,
}

type FileNote = Arc<dyn Fn(&[PathBuf]) + Send + Sync>;

fn hosted(root: &Path, policy: PermissionAction, sink: Arc<dyn PermissionSink>) -> Hosted {
    hosted_with(root, policy, sink, None, None)
}

fn hosted_with(
    root: &Path,
    policy: PermissionAction,
    sink: Arc<dyn PermissionSink>,
    file_note: Option<FileNote>,
    plan_touch: Option<litecode::tool::executor::PlanTouch>,
) -> Hosted {
    let permission = engine(root, policy);
    let ctx = ctx(root);
    let data_root = root.join(".litecode");
    std::fs::create_dir_all(&data_root).unwrap();
    let mut host = CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        sink,
        data_root.clone(),
        32_768,
        process_write_lock(),
        None,
    );
    if let Some(file_note) = file_note {
        host = host.with_file_note(file_note);
    }
    if let Some(plan_touch) = plan_touch {
        host = host.with_plan_touch(plan_touch);
    }
    let code = Arc::new(CodeTool::new(
        vec![
            Arc::new(ReadTool::default()) as Arc<dyn Tool>,
            Arc::new(EditTool::new()),
            Arc::new(WriteTool::new()),
        ],
        Arc::new(host),
    ));
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
    session_id: &str,
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
            code_names: Arc::clone(&hosted.code_names),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await
}

#[test]
fn edit_and_write_advertise_path_on_the_code_surface() {
    assert_eq!(EditTool::new().surface(), ToolSurface::Code);
    assert_eq!(WriteTool::new().surface(), ToolSurface::Code);
    for tool in [
        Arc::new(EditTool::new()) as Arc<dyn Tool>,
        Arc::new(WriteTool::new()),
    ] {
        let schema = tool.schema();
        let required = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(required.contains(&"path"), "{required:?}");
        assert!(!required.contains(&"file_path"), "{required:?}");
        assert!(schema["properties"].get("path").is_some());
        assert!(schema["properties"].get("file_path").is_none());
    }
    let projected = project_tools(vec![
        Arc::new(EditTool::new()),
        Arc::new(WriteTool::new()),
        Arc::new(ReadTool::default()),
    ]);
    assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
    assert!(projected.direct.is_empty());
    assert!(projected.sdk.contains("[\"edit\"]"));
    assert!(projected.sdk.contains("[\"write\"]"));
    assert!(projected.sdk.contains("path:"));
    assert!(!projected.sdk.contains("file_path"));
}

#[tokio::test(flavor = "current_thread")]
async fn direct_edit_and_write_are_refused_before_authorization() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Ask,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let asked = Arc::clone(&sink.calls);
    for name in ["edit", "write"] {
        let result = execute(
            &hosted,
            "phase3",
            name,
            json!({ "path": "note.txt", "content": "nope", "edits": [] }),
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
        asked.lock().unwrap().is_empty(),
        "direct routing happens before authorization: {:?}",
        asked.lock().unwrap()
    );
    assert!(!dir.path().join("note.txt").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn edit_and_write_succeed_through_run_code_with_path() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
const written = await tools.write({ path: "note.txt", content: "alpha\n" });
const edited = await tools.edit({
  path: "note.txt",
  edits: [{ old_string: "alpha", new_string: "beta" }]
});
return { written: written.file ? "ok" : "missing", edited: edited.applied >= 1 ? "ok" : "missing" };
"#;
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["written"], "ok", "{}", result.content);
    assert_eq!(value["edited"], "ok", "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("note.txt")).unwrap(),
        "beta\n"
    );
    assert!(
        result.content.contains("1. write path=note.txt ok"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("2. edit path=note.txt ok"),
        "{}",
        result.content
    );
}

#[tokio::test(flavor = "current_thread")]
async fn historical_file_path_still_writes_through_run_code() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
const written = await tools.write({ file_path: "legacy.txt", content: "kept" });
const edited = await tools.edit({
  file_path: "legacy.txt",
  edits: [{ old_string: "kept", new_string: "replayed" }]
});
return { written: written.status, edited: edited.status };
"#;
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("legacy.txt")).unwrap(),
        "replayed"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn nested_write_approval_denial_does_not_apply_the_edit() {
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingPermissionSink::from_reply(false, false);
    let hosted = hosted(dir.path(), PermissionAction::Ask, Arc::new(sink));
    let outer = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
let status = "ok";
try {
  await tools.write({ path: "nope.txt", content: "secret-nope" });
} catch (e) {
  status = "error";
}
return { status };
"#;
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &outer,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.error.is_none(),
        "the program continues: {}",
        result.content
    );
    let value = run_code_result(&result.content);
    assert_eq!(value["status"], "error");
    assert!(
        result.content.contains("1. write path=nope.txt error"),
        "{}",
        result.content
    );
    let ledger = result
        .content
        .lines()
        .find(|line| line.contains("1. write path=nope.txt"))
        .unwrap_or("");
    assert!(
        ledger.contains("error:permission_denied"),
        "the ledger line prints the error code: {ledger}"
    );
    assert!(
        result.content.contains("1 rejected (permission_denied×1), handled by the program"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("#1 write path=nope.txt: permission_denied:"),
        "{}",
        result.content
    );
    assert!(
        !result.content.contains("secret-nope"),
        "{}",
        result.content
    );
    assert!(!dir.path().join("nope.txt").exists());
}

#[tokio::test(flavor = "current_thread")]
async fn nested_write_sensitive_path_is_denied_by_the_same_floor() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let target = r"C:\Windows\litecode-phase3-no-write.txt";
    let script = format!(
        r#"
let status = "ok";
try {{
  await tools.write({{ path: {target:?}, content: "no" }});
}} catch (e) {{
  status = "error";
}}
return {{ status }};
"#
    );
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    assert_eq!(value["status"], "error");
    let ledger = result
        .content
        .lines()
        .find(|line| line.starts_with("1. write"))
        .unwrap_or("");
    assert!(
        ledger.contains("error:permission_denied"),
        "the ledger line prints the error code: {ledger}"
    );
    assert!(result.content.contains("permission_denied"), "{}", result.content);
    assert!(!Path::new(target).exists());
}

#[tokio::test(flavor = "current_thread")]
async fn two_nested_writes_leave_one_complete_file_and_sequential_writes_both_apply() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
await Promise.allSettled([
  tools.write({ path: "same.txt", content: "AAAA" }),
  tools.write({ path: "same.txt", content: "BBBB" }),
  tools.write({ path: "other.txt", content: "CCCC" })
]);
await tools.write({ path: "seq.txt", content: "one" });
await tools.write({ path: "seq.txt", content: "two" });
return "done";
"#;
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let same = std::fs::read_to_string(dir.path().join("same.txt")).unwrap();
    assert!(
        same == "AAAA" || same == "BBBB",
        "concurrent writes must leave one complete body, got {same:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("seq.txt")).unwrap(),
        "two",
        "sequential writes stay re-entrant"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("other.txt")).unwrap(),
        "CCCC"
    );
    assert!(
        result.content.contains("write path=same.txt ok"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("write path=other.txt ok"),
        "{}",
        result.content
    );
}

#[tokio::test(flavor = "current_thread")]
async fn partial_program_keeps_one_ledger_line_per_started_write() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
await tools.write({ path: "a.txt", content: "BODY_A" });
await tools.write({ path: "b.txt", content: "BODY_B" });
throw new Error("stop-after-two");
"#;
    let result = execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(
        result.error.is_some(),
        "the program failed: {}",
        result.content
    );
    assert!(
        result.content.contains("1. write path=a.txt ok"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("2. write path=b.txt ok"),
        "{}",
        result.content
    );
    assert!(!result.content.contains("BODY_A"), "{}", result.content);
    assert!(!result.content.contains("BODY_B"), "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "BODY_A"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "BODY_B"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn nested_write_is_visible_to_the_same_turn_level_effects() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join(".litecode").join("sessions.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let sessions = Arc::new(SessionManager::new_for_test(
        Arc::new(TurnGuard::new()),
        db.to_str().unwrap().to_string(),
    ));
    let sid = sessions
        .open_session_sync(dir.path().to_str().unwrap(), "default", None)
        .unwrap();
    let touched = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let plans = Arc::new(Mutex::new(Vec::<String>::new()));
    let file_note = {
        let sessions = Arc::clone(&sessions);
        let sid = sid.clone();
        let touched = Arc::clone(&touched);
        Arc::new(move |paths: &[PathBuf]| {
            touched.lock().unwrap().extend(paths.iter().cloned());
            sessions.note_touched_files(&sid, paths);
        }) as FileNote
    };
    let plan_touch = {
        let plans = Arc::clone(&plans);
        Arc::new(move |call: &FunctionToolCall| {
            plans
                .lock()
                .unwrap()
                .push(format!("{} {}", call.name, call.arguments));
        }) as litecode::tool::executor::PlanTouch
    };
    let hosted = hosted_with(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
        Some(file_note),
        Some(plan_touch),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
const written = await tools.write({ path: "noted.txt", content: "v1" });
return written.file ? "ok" : "missing";
"#;
    let result = execute(
        &hosted,
        &sid,
        "run_code",
        json!({ "code": script }),
        &sink,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let noted = touched.lock().unwrap().clone();
    assert!(
        noted.iter().any(|path| path.ends_with("noted.txt")),
        "file note (absorb input) missed the nested write: {noted:?}"
    );
    let planned = plans.lock().unwrap().clone();
    assert!(
        planned
            .iter()
            .any(|line| line.starts_with("write ") && line.contains("noted.txt")),
        "plan touch missed the nested write: {planned:?}"
    );

    sessions.absorb_touched_files(&sid);
    assert!(
        sessions.take_changed_files(&sid).is_empty(),
        "absorb hides the nested write the same way it hides a direct write"
    );
    let path = dir.path().join("noted.txt");
    std::fs::write(&path, "v1\nexternal").unwrap();
    let changed = sessions.take_changed_files(&sid);
    assert!(
        changed.iter().any(|path| path.ends_with("noted.txt")),
        "snapshot sees the nested write's path: {changed:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_run_code_seals_an_in_flight_write_and_edit() {
    let dir = tempfile::tempdir().unwrap();
    let hosted = hosted(
        dir.path(),
        PermissionAction::Allow,
        litecode::permission::deny_permission_sink(),
    );
    let cancel = CancellationToken::new();
    let sink = RecordingPermissionSink::from_reply(true, false);
    let script = r#"
await tools.write({ path: "seal.txt", content: "x".repeat(80000) });
await tools.edit({
  path: "seal.txt",
  edits: [{ old_string: "x", new_string: "y", replace_all: true }]
});
return "done";
"#;
    let path = dir.path().join("seal.txt");
    let mut run = std::pin::pin!(execute(
        &hosted,
        "phase3",
        "run_code",
        json!({ "code": script, "timeout_ms": 5000 }),
        &sink,
        cancel.clone(),
    ));
    let result = loop {
        tokio::select! {
            result = &mut run => break result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {
                if path.exists() && !cancel.is_cancelled() {
                    cancel.cancel();
                }
            }
        }
    };
    assert!(
        result.content.contains("1. write"),
        "the started write stays in the ledger: {}",
        result.content
    );
    if path.exists() {
        let body = std::fs::read(&path).unwrap();
        assert!(
            body.iter().all(|byte| *byte == b'x') || body.iter().all(|byte| *byte == b'y'),
            "an in-flight edit must not leave a torn file (len {})",
            body.len()
        );
        assert_eq!(body.len(), 80_000);
    }
    if result.content.contains("2. edit") {
        assert!(
            result.content.contains("2. edit path=seal.txt ok")
                || result.content.contains("2. edit path=seal.txt cancelled")
                || result.content.contains("2. edit path=seal.txt unsettled")
                || result.content.contains("2. edit path=seal.txt timeout"),
            "{}",
            result.content
        );
    }
}

/// Publish a marker when approval starts, then pause. The path is not locked
/// during that wait. A read of the same file in the window must return before
/// the write is approved.
struct HoldWriteSink {
    root: PathBuf,
    seq: AtomicU64,
}

fn hold_marker(summary: &str) -> Option<&'static str> {
    match summary {
        "./held.txt" => Some("held.ready"),
        "trail.txt" | "trail.txt." => Some("trail.ready"),
        "ä.txt" | "Ä.txt" => Some("auml.ready"),
        "parented.txt" => Some("parent.ready"),
        _ if summary.ends_with(r"\gone\..\parented.txt") => Some("parent.ready"),
        _ => None,
    }
}

impl PermissionSink for HoldWriteSink {
    fn ask(&self, prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
        if prompt.tool == "write"
            && let Some(name) = hold_marker(prompt.summary)
        {
            let generation = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
            std::fs::write(self.root.join(name), generation.to_string()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
        AskReply::from_outcome(AskOutcome::Allow { always: false })
    }
}

fn hosted_unrestricted(root: &Path, sink: Arc<dyn PermissionSink>) -> Hosted {
    let permission = engine_with_mode(root, PermissionAction::Ask, BindingPathMode::Unrestricted);
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
            Arc::new(EditTool::new()),
            Arc::new(WriteTool::new()),
        ],
        Arc::new(host),
    ));
    let code_names = code.code_bindings().unwrap_or_else(|| Arc::from([]));
    Hosted {
        tools: vec![code],
        permission,
        ctx,
        data_root,
        code_names,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn conflicting_spellings_have_one_create_only_winner_and_a_read_waits_for_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let sink = Arc::new(HoldWriteSink {
        root: root.to_path_buf(),
        seq: AtomicU64::new(0),
    });
    let hosted = hosted_unrestricted(root, Arc::clone(&sink) as Arc<dyn PermissionSink>);
    let outer = RecordingPermissionSink::from_reply(true, false);

    let mut pairs = vec![
        (
            "same",
            "same.txt".to_string(),
            "same.txt".to_string(),
            vec!["same.txt"],
            false,
        ),
        (
            "dot",
            "dot.txt".to_string(),
            "./dot.txt".to_string(),
            vec!["dot.txt"],
            false,
        ),
    ];
    if cfg!(windows) {
        let parented = format!(r"{}\gone\..\parented.txt", root.display());
        pairs.extend([
            (
                "case",
                "case.txt".to_string(),
                "Case.txt".to_string(),
                vec!["case.txt"],
                false,
            ),
            (
                "slash",
                "slash.txt".to_string(),
                r".\slash.txt".to_string(),
                vec!["slash.txt"],
                false,
            ),
            (
                "nested",
                "sub/nested.txt".to_string(),
                r"sub\nested.txt".to_string(),
                vec!["sub", "nested.txt"],
                false,
            ),
            (
                "parented",
                "parented.txt".to_string(),
                parented,
                vec!["parented.txt"],
                false,
            ),
            (
                "trail",
                "trail.txt".to_string(),
                "trail.txt.".to_string(),
                vec!["trail.txt"],
                true,
            ),
            (
                "auml",
                "ä.txt".to_string(),
                "Ä.txt".to_string(),
                vec!["ä.txt"],
                true,
            ),
        ]);
    }
    let race_calls = pairs
        .iter()
        .map(|(name, left, right, _, _)| {
            format!(
                "await race({}, {}, {})",
                serde_json::to_string(name).unwrap(),
                serde_json::to_string(left).unwrap(),
                serde_json::to_string(right).unwrap()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n  ");
    let read_paths = if cfg!(windows) {
        r#"["held.txt", "Held.txt", ".\\held.txt"]"#
    } else {
        r#"["held.txt"]"#
    };
    let spelled_calls = if cfg!(windows) {
        r#"
const trailPrior = await markerText("trail.ready");
const aumlPrior = await markerText("auml.ready");
const spelled = [
  await readAfterWrite("trail.txt", "trail.txt.", "trail.ready", trailPrior),
  await readAfterWrite("ä.txt", "Ä.txt", "auml.ready", aumlPrior)
];"#
    } else {
        "const spelled = [];"
    };
    let script = r#"
const bodyX = "x".repeat(512 * 1024);
const bodyY = "y".repeat(512 * 1024);
async function race(name, a, b) {
  const results = await Promise.allSettled([
    tools.write({ path: a, content: bodyX, create_only: true }),
    tools.write({ path: b, content: bodyY, create_only: true })
  ]);
  return {
    name,
    statuses: results.map(result => result.status === "fulfilled" ? "ok" : "error"),
    messages: results.map(result => result.status === "rejected" ? ((result.reason && result.reason.message) || "") : ""),
    bytes: results.map(result => result.status === "fulfilled" && result.value.file ? result.value.file.bytes : 0)
  };
}
async function lineText(result) {
  const line = result && result.lines && result.lines[0];
  return line ? line.text : "";
}
async function markerText(path) {
  try {
    return await lineText(await tools.read({ path }));
  } catch (e) {
    return "";
  }
}
async function readsDuringWrite(paths) {
  const writing = tools.write({
    path: "./held.txt",
    content: "exclusive-body\n",
    create_only: true
  });
  let saw = false;
  for (let i = 0; i < 40; i++) {
    if (await markerText("held.ready")) {
      saw = true;
      break;
    }
  }
  const reads = await Promise.all(paths.map(async (path) => {
    try {
      const result = await tools.read({ path });
      return { path, status: "ok", text: await lineText(result), message: "" };
    } catch (e) {
      return { path, status: "error", text: "", message: (e && e.message) || "" };
    }
  }));
  let write = "ok";
  let writeMessage = "";
  try {
    await writing;
  } catch (e) {
    write = "error";
    writeMessage = (e && e.message) || "";
  }
  let after = { status: "error", text: "" };
  try {
    after = { status: "ok", text: await lineText(await tools.read({ path: "held.txt" })) };
  } catch (e) {}
  return { saw, write, writeMessage, reads, after };
}
async function readAfterWrite(writePath, readPath, marker, prior) {
  const writing = tools.write({ path: writePath, content: "exclusive-body\n" });
  let saw = "";
  for (let i = 0; i < 80; i++) {
    const text = await markerText(marker);
    if (text && text !== prior) {
      saw = text;
      break;
    }
  }
  let during = { status: "error", text: "" };
  try {
    during = { status: "ok", text: await lineText(await tools.read({ path: readPath })) };
  } catch (e) {}
  let write = "ok";
  let writeMessage = "";
  try {
    await writing;
  } catch (e) {
    write = "error";
    writeMessage = (e && e.message) || "";
  }
  let after = { status: "error", text: "" };
  try {
    after = { status: "ok", text: await lineText(await tools.read({ path: readPath })) };
  } catch (e) {}
  return {
    saw: saw !== "",
    write,
    writeMessage,
    during: { path: readPath, status: during.status, text: during.text },
    after
  };
}
const races = [
  RACE_CALLS
];
const held = await readsDuringWrite(READ_PATHS);
SPELLED_CALLS
return { races, held, spelled };
"#
    .replace("RACE_CALLS", &race_calls)
    .replace("READ_PATHS", read_paths)
    .replace("SPELLED_CALLS", spelled_calls);

    let result = execute(
        &hosted,
        "phase3-spellings",
        "run_code",
        json!({ "code": script }),
        &outer,
        CancellationToken::new(),
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    let value = run_code_result(&result.content);
    let races = value["races"].as_array().expect("races");
    assert_eq!(races.len(), pairs.len(), "{races:?}");
    for (pair, race) in pairs.iter().zip(races) {
        let statuses: Vec<&str> = race["statuses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        let ok = statuses.iter().filter(|status| **status == "ok").count();
        assert_eq!(ok, 1, "{} must have one create_only winner: {race}", pair.0);
        let messages: Vec<&str> = race["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("already exists")),
            "{} loser should report the existing file: {race}",
            pair.0
        );
        let written: Vec<u64> = race["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_u64().unwrap())
            .collect();
        assert!(
            written.contains(&(512 * 1024)),
            "{} winner did not write the full body: {race}",
            pair.0
        );
        if pair.4 {
            continue;
        }
        let mut disk = root.to_path_buf();
        for part in &pair.3 {
            disk.push(part);
        }
        let bytes = std::fs::read(&disk)
            .unwrap_or_else(|error| panic!("{} missing {}: {error}", pair.0, disk.display()));
        assert_eq!(bytes.len(), 512 * 1024, "{}", pair.0);
        assert!(
            bytes.iter().all(|byte| *byte == b'x') || bytes.iter().all(|byte| *byte == b'y'),
            "{} left a torn body",
            pair.0
        );
    }

    let held = &value["held"];
    assert_eq!(held["saw"], true, "write never reached the gate: {held}");
    assert_eq!(held["write"], "ok", "{held}");
    let reads = held["reads"].as_array().expect("reads");
    assert!(!reads.is_empty());
    for read in reads {
        assert_ne!(
            read["status"], "ok",
            "a read of {} ran while approval was pending and must not wait for the write: {read}",
            read["path"]
        );
    }
    assert_eq!(held["after"]["status"], "ok", "{held}");
    assert_eq!(held["after"]["text"], "exclusive-body", "{held}");
    assert_eq!(
        std::fs::read_to_string(root.join("held.txt")).unwrap(),
        "exclusive-body\n"
    );

    let spelled = value["spelled"].as_array().expect("spelled");
    if cfg!(windows) {
        assert_eq!(spelled.len(), 2, "{spelled:?}");
        for read in spelled {
            assert_eq!(read["saw"], true, "write never reached the gate: {read}");
            assert_eq!(read["write"], "ok", "{read}");
            assert_ne!(
                read["during"]["text"], "exclusive-body",
                "a read of {} waited for the unapproved write: {read}",
                read["during"]["path"]
            );
            assert_eq!(read["after"]["status"], "ok", "{read}");
            assert_eq!(read["after"]["text"], "exclusive-body", "{read}");
        }
        assert_eq!(
            std::fs::read_to_string(root.join("trail.txt")).unwrap(),
            "exclusive-body\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("ä.txt")).unwrap(),
            "exclusive-body\n"
        );
    } else {
        assert!(spelled.is_empty(), "{spelled:?}");
    }
}
