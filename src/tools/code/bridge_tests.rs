//! Bridge-level probes: the production `run_code` path with a real CodeSurface.
//!
//! The engine harness in `src/ptc/tests.rs` cannot judge a contract: only
//! `CodeSurface` (built from the generated contract) decides whether a call
//! holds. These probes therefore go through `CodeTool` exactly as production
//! does — real bundle, real surface, real host — and assert what the model sees.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::GlobalSettings;
use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, deny_permission_sink};
use crate::tool::Tool;
use crate::tool::trait_::{ToolExecutionContext, ToolSurface};
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tools::code::{CodeTool, CodeToolHost};
use crate::tools::apply_edits::ApplyEditsTool;
use crate::types::ToolCallResult;

fn allow_engine(root: &Path) -> PermissionEngine {
    let resolved = resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

fn context(root: &Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

fn execution(root: &Path) -> ToolExecutionContext {
    ToolExecutionContext {
        path_mode: crate::workspace::ToolPathMode::All,
        workspace_root: root.to_path_buf(),
        call_id: "probe-run".into(),
        cancel: CancellationToken::new(),
        output_limit: usize::MAX,
        session_id: "probe-session".into(),
        session: None,
    }
}

#[test]
fn run_code_schema_has_no_timeout_knob() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let host = Arc::new(CodeToolHost::new(
        allow_engine(root),
        context(root),
        "default",
        deny_permission_sink(),
        root.join(".litecode"),
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ));
    let tool = CodeTool::new(
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        host,
    );
    let spec = tool.direct_spec(&context(root)).expect("run_code spec");
    let properties = spec.input_schema["properties"]
        .as_object()
        .expect("properties");
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        ["code"]
    );
    assert_eq!(spec.input_schema["required"], json!(["code"]));
    assert_eq!(spec.input_schema["additionalProperties"], json!(false));
    assert!(!spec.input_schema.to_string().contains("timeout"));
}

async fn run_program(root: &Path, bundle: Vec<Arc<dyn Tool>>, code: &str) -> ToolCallResult {
    let host = Arc::new(CodeToolHost::new(
        allow_engine(root),
        context(root),
        "default",
        deny_permission_sink(),
        root.join(".litecode"),
        Arc::new(WorkspaceWriteLock::new()),
        None,
    ));
    let tool = CodeTool::new(bundle, host);
    tool.execute(json!({ "code": code }), execution(root)).await
}

/// Claims to be `read`, counts executions, and returns a shape the contract
/// allows. A count above zero means the bridge dispatched the call.
struct CountingRead {
    runs: Arc<AtomicUsize>,
}

impl Tool for CountingRead {
    fn name(&self) -> &str {
        "read"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        let mut result = ToolCallResult::ok(String::new());
        result.data = Some(json!({
            "path": "a.txt",
            "start_line": 1,
            "lines": ["hello"],
            "total_lines": 1,
            "has_more": false,
            "skipped_in_context": 0
        }));
        Box::pin(std::future::ready(result))
    }
}

/// Claims to be `read` and returns a shape the contract does not allow:
/// `lines` must be an array of strings.
struct BadRead;

impl Tool for BadRead {
    fn name(&self) -> &str {
        "read"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let mut result = ToolCallResult::ok(String::new());
        // `message` is present so the only violation is `lines`: a number where
        // the contract says an array of strings.
        result.data = Some(json!({ "lines": 42 }));
        Box::pin(std::future::ready(result))
    }
}

/// S2: an unknown parameter rejects the call, not the run. The program catches
/// the ordinary `Error`, keeps going, and the manifest row is a failure.
#[tokio::test]
async fn s2_unknown_parameter_rejects_and_the_run_continues() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        try {
          await tools.read({ path: "a.txt", nope: 1 });
          return "resolved";
        } catch (e) {
          return e.code + " " + e.message;
        }
        "#,
    )
    .await;
    assert!(
        result.error.is_none(),
        "the run must survive a bad argument: {}",
        result.content
    );
    assert!(result.content.contains("invalid_args"), "{}", result.content);
    assert!(result.content.contains("unknown parameter"), "{}", result.content);
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "an invalid call must not reach the tool: {}",
        result.content
    );
}

/// S2: a missing required parameter fails the call, not the run, and names
/// the parameter.
#[tokio::test]
async fn s2_missing_required_parameter_fails_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        try {
          await tools.read({});
          return "resolved";
        } catch (e) {
          return e.code + " " + e.message;
        }
        "#,
    )
    .await;
    assert!(
        result.error.is_none(),
        "the run must survive a missing argument: {}",
        result.content
    );
    assert!(result.content.contains("invalid_args"), "{}", result.content);
    assert!(result.content.contains("path"), "{}", result.content);
    assert_eq!(runs.load(Ordering::SeqCst), 0, "{}", result.content);
}

/// S2: a rejected call does not stop the next one: only the call failed.
#[tokio::test]
async fn s2_a_rejected_call_does_not_stop_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        let first;
        try {
          await tools.read({ path: "a.txt", nope: 1 });
        } catch (e) {
          first = e.code;
        }
        const second = await tools.read({ path: "a.txt" });
        return first + " " + second.lines.length;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("invalid_args 1"), "{}", result.content);
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the valid call must still run: {}",
        result.content
    );
}

/// A failing edit item rejects `apply_edits` with an Error carrying the public
/// reason code and details, while earlier valid edits in the batch stay saved.
#[tokio::test]
async fn edit_input_failure_rejects_and_keeps_valid_edits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "world\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(ApplyEditsTool::new())],
        r#"
        try {
          await tools.apply_edits({
            edits: [
              { path: "b.txt", old_string: "world", new_string: "WORLD" },
              { path: "a.txt", old_string: "hello", new_string: "hello" },
            ],
          });
          return "unexpected_ok";
        } catch (e) {
          return {
            code: e.code,
            applied: e.details && e.details.applied,
            total: e.details && e.details.total,
            r0: e.details && e.details.results[0].ok,
            r1: e.details && e.details.results[1].reason,
          };
        }
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert_eq!(std::fs::read_to_string(dir.path().join("b.txt")).unwrap(), "WORLD\n");
    assert!(
        result.content.contains("\"code\":\"invalid_args\"")
            && result.content.contains("\"applied\":1")
            && result.content.contains("\"total\":2")
            && result.content.contains("\"r0\":true")
            && result.content.contains("\"r1\":\"invalid_args\""),
        "caught error must report saved count and item reason: {}",
        result.content
    );
}

/// An edit that is refused for permission rejects with `code: "permission"`
/// and stops subsequent statements unless caught.
#[tokio::test]
async fn edit_on_a_session_record_rejects_with_permission_code() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![
            Arc::new(ApplyEditsTool::new()),
            Arc::new(crate::tools::write::WriteTool::new()),
        ],
        r#"
        await tools.apply_edits({ edits: [{
            path: ".litecode/sessions/01M6MH2CWCE5DJDAZBE0E3THT5.md",
            old_string: "a",
            new_string: "b",
        }] });
        await tools.write({ path: "after.txt", content: "should_not_run" });
        return "unreachable";
        "#,
    )
    .await;
    assert_eq!(result.error_code.as_deref(), Some("permission"), "{}", result.content);
    assert!(!dir.path().join("after.txt").exists(), "downstream call must not run");
}

/// Claims to be `grep` and returns grep-shaped data, so a read can be checked
/// against a *different* field list than `read`'s.
struct GrepLike;

impl Tool for GrepLike {
    fn name(&self) -> &str {
        "grep"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let mut result = ToolCallResult::ok(String::new());
        result.data = Some(json!({
            "hits": [{"path": "a", "line": 1, "text": "a"}],
            "at_limit_cap": false,
            "lines_truncated": false,
            "context_omitted": false,
            "output_capped": false
        }));
        Box::pin(std::future::ready(result))
    }
}

/// Claims to be `read` and returns a resolved failure envelope.
struct FailingRead;

impl Tool for FailingRead {
    fn name(&self) -> &str {
        "read"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        // A real failure result: the host envelope, not ok-data that says false.
        let mut result = ToolCallResult::error_with_kind(
            crate::types::ToolErrorKind::NotFound,
            "p does not exist",
            false,
        );
        result.error_code = Some("not_found".into());
        Box::pin(std::future::ready(result))
    }
}

/// S4 (was P4): reading a field the result does not have is reported at the
/// end, lists the real fields, and the read yields `undefined`.
#[tokio::test]
async fn s4_unknown_result_field_is_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::new(AtomicUsize::new(0)),
        })],
        r#"
        const r = await tools.read({ path: "a.txt" });
        const data = r.path;
        return typeof data;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "the run must survive a wrong field: {}", result.content);
    assert!(result.content.contains("'path'"), "{}", result.content);
    assert!(result.content.contains("lines"), "{}", result.content);
    assert!(result.content.contains("undefined"), "{}", result.content);
}

/// S4 (was P9): a failed call reaches the program as an ordinary rejection,
/// with the real code and message; the run continues after the catch.
#[tokio::test]
async fn s4_failed_call_is_caught_with_code_and_message() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(FailingRead)],
        r#"
        try {
          await tools.read({ path: "p" });
          return "resolved";
        } catch (e) {
          console.log("saw", e.code, e.message);
        }
        return "done";
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(
        result.content.contains("[log] saw not_found p does not exist"),
        "{}",
        result.content
    );
}

/// The field list belongs to the tool's own contract: reading `lines` on a
/// `grep` result names grep's fields, never read's.
#[tokio::test]
async fn s4_field_list_comes_from_each_tools_contract() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(GrepLike), Arc::new(CountingRead {
            runs: Arc::new(AtomicUsize::new(0)),
        })],
        r#"
        const g = await tools.grep({ pattern: "a" });
        const lines = g.lines;
        return g.hits.length;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("hits"), "{}", result.content);
    assert!(
        !result.content.contains("start_line"),
        "the message must not leak another tool's fields: {}",
        result.content
    );
}

/// The pre-execution check reaches the model, and it does not stop the run:
/// a field the result does not have is one warning line, reported once.
#[tokio::test]
async fn a_static_typo_is_reported_once_and_the_run_still_happens() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        const r = await tools.read({ path: "a.txt" });
        const data = r.data;
        return "ran";
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(
        result.content.contains("no field 'data'"),
        "{}",
        result.content
    );
    assert_eq!(
        result.content.matches("no field 'data'").count(),
        1,
        "one typo must read as one line: {}",
        result.content
    );
    assert_eq!(runs.load(Ordering::SeqCst), 1, "{}", result.content);
}

/// A tool result reaches the model as fields plus exactly one reminder line.
///
/// Unit tests cover the renderer in isolation; this probe proves the wiring:
/// the real glob tool, the real contract, the real run_code view.
#[tokio::test]
async fn a_tool_result_reaches_the_model_with_fields_and_a_reminder_line() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..3 {
        std::fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
    }
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::glob::GlobTool)],
        r#"const r = await tools.glob({ pattern: "*.txt", limit: 2 }); return JSON.parse(JSON.stringify(r));"#,
    )
    .await;
    let content = result.content;
    assert!(content.contains("\"paths\""), "{content}");
    assert!(!content.contains("\"total\""), "{content}");
    assert!(!content.contains("\"truncated\""), "{content}");
    assert!(
        content.contains("tools.glob: showing first 2 of 3 paths"),
        "{content}"
    );
    assert!(
        !content.contains("\"message\""),
        "the prose field must be gone: {content}"
    );
}

/// Facts must not depend on the program handing the result back.
///
/// The program returns only the lines array; the read facts still have to
/// reach the model, because they describe the call and not the returned value.
#[tokio::test]
async fn a_slice_of_a_result_still_reaches_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (0..5).map(|i| format!("line{i}\n")).collect();
    std::fs::write(dir.path().join("a.txt"), body).unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"const r = await tools.read({ path: "a.txt", limit: 2 }); return r.lines;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains("tools.read: showing lines 1-2 of 5; continue with start_line 3"),
        "{content}"
    );
}

/// The most natural program shape: hand a tool result straight back.
///
/// The result arrives behind the contract proxy, so the output walker has to
/// read through it; calling its own result "unsupported object" was the bug.
#[tokio::test]
async fn a_returned_tool_result_is_presented() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::glob::GlobTool)],
        r#"return await tools.glob({ pattern: "*.txt" });"#,
    )
    .await;
    let content = result.content;
    assert!(result.error.is_none(), "{content}");
    assert!(content.contains("a.txt"), "{content}");
    assert!(!content.contains("unsupported object"), "{content}");
    assert!(!content.contains("not JSON"), "{content}");
}

/// A program that asks the language server twice gets the state it was given,
/// and `partial` and `unknown` read differently.
#[tokio::test]
async fn lsp_state_reaches_the_model_as_a_reminder_line() {
    struct DiagnosticsStub;
    impl Tool for DiagnosticsStub {
        fn name(&self) -> &str {
            "diagnostics"
        }
        fn surface(&self) -> ToolSurface {
            ToolSurface::Code
        }
        fn execute(
            &self,
            input: Value,
            _execution: ToolExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
            Box::pin(async move {
                let state = if input["path"] == "b.rs" { "unknown" } else { "partial" };
                let mut result = ToolCallResult::ok(String::new());
                result.data = Some(json!({ "items": [], "state": state, "truncated": false }));
                result
            })
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(DiagnosticsStub)],
        r#"const a = await tools.diagnostics({ path: "a.rs" });
const b = await tools.diagnostics({ path: "b.rs" }); return a.state + b.state;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains(
            "↳ #1 tools.diagnostics: no diagnostics reported; the index is still building, "
        ),
        "{content}"
    );
    assert!(
        content.contains(
            "↳ #2 tools.diagnostics: no diagnostics reported; the index state is unknown, "
        ),
        "{content}"
    );
    assert_ne!(
        content
            .lines()
            .find(|line| line.starts_with("↳ #1 "))
            .map(str::to_string),
        content
            .lines()
            .find(|line| line.starts_with("↳ #2 "))
            .map(str::to_string),
        "{content}"
    );
}
/// The defect this guards: a hit limit cut the result, but the reminder only
/// fired when a known total exceeded the shown count, so the commonest cut
/// said nothing at all.
#[tokio::test]
async fn a_capped_grep_says_more_exist() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (0..45).map(|i| format!("fn f{i}() {{}}\n")).collect();
    std::fs::write(dir.path().join("a.rs"), body).unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::grep::GrepTool)],
        r#"const r = await tools.grep({ pattern: "fn ", path: "a.rs", limit: 5 });
return r.hits.length;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains(
            "↳ #1 tools.grep: showing the first 5 matches; more exist — narrow the pattern/path or raise limit"
        ),
        "{content}"
    );
}
/// The path risk is reminder metadata: the program only sees the path, while
/// the model gets one line naming the resolved target.
#[tokio::test]
async fn an_outside_write_reaches_the_model_as_a_field_and_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("note.txt");
    let literal = serde_json::to_string(&target.to_string_lossy().to_string()).unwrap();
    let code = format!(
        "const r = await tools.write({{ path: {literal}, content: \"hi\" }});\nreturn {{ path: r.path, hasRisk: Object.hasOwn(r, 'path_risk') }};"
    );
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::write::WriteTool::new())],
        &code,
    )
    .await;
    let content = result.content;
    assert!(content.contains("\"hasRisk\":false"), "{content}");
    let expected = format!(
        "↳ #1 tools.write: this write lands outside the workspace: {}",
        target.to_string_lossy()
    );
    assert!(content.contains(&expected), "{content}\n{expected}");
}

/// Claims to be `workspace_symbol` and fails like a dead language server: the
/// call never ran, so it must add no line of its own.
struct UnavailableSymbols;

impl Tool for UnavailableSymbols {
    fn name(&self) -> &str {
        "workspace_symbol"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn execute(
        &self,
        _input: Value,
        _execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        let mut result = ToolCallResult::error_with_kind(
            crate::types::ToolErrorKind::Unavailable,
            "language server I/O failed",
            false,
        );
        result.error_code = Some("unavailable".into());
        Box::pin(std::future::ready(result))
    }
}

/// The reported defect: a failed `lsp.workspace_symbol` was followed by
/// "no symbols reported; the index state is unknown", as if it had run. Now the
/// call gets exactly one line of its own — the failure, anchored to the call —
/// and no guessed result.
#[tokio::test]
async fn a_caught_failure_reads_as_one_line() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(UnavailableSymbols)],
        r#"
        try {
          await tools.workspace_symbol({ query: "x" });
          console.log("resolved");
        } catch (e) {
          console.log("saw", e.code, e.message);
        }
        return 1;
        "#,
    )
    .await;
    let content = result.content;
    assert!(result.error.is_none(), "{content}");
    assert!(
        content.contains("[log] saw unavailable language server I/O failed"),
        "the failure must reach the program as an ordinary Error: {content}"
    );
    assert!(
        !content.contains("no symbols reported"),
        "a failed call guessed a result: {content}"
    );
    let lines: Vec<&str> = content.lines().collect();
    let reported: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("↳ #1 "))
        .collect();
    assert_eq!(
        reported,
        ["↳ #1 tools.workspace_symbol: failed(unavailable); the program may have caught the error"],
        "{content}"
    );
    // Above the stream, and the stream still ends the view.
    let anchor = lines
        .iter()
        .position(|line| line.starts_with("↳ #1 "))
        .unwrap();
    let stream = lines
        .iter()
        .position(|line| line.starts_with("[log] saw"))
        .unwrap();
    assert!(anchor < stream, "{content}");
    assert_eq!(lines.last().copied(), Some("1"), "{content}");
}
/// `Promise.allSettled` is the batch idiom: one rejected call is visible as a
/// rejected entry, the next call still resolves, and the program keeps running.
#[tokio::test]
async fn promise_allsettled_consumes_a_rejection_and_a_success() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"
        const results = await Promise.allSettled([
          tools.read({ path: "missing.txt" }),
          tools.read({ path: "a.txt" }),
        ]);
        return results.map((item) =>
          item.status === "fulfilled" ? item.value.lines.length : item.reason.code
        );
        "#,
    )
    .await;
    let content = result.content;
    assert!(result.error.is_none(), "{content}");
    assert!(content.contains("\"not_found\""), "{content}");
    assert!(content.contains("1"), "{content}");
}

/// A run that stopped on the failure says it in the manifest, so the block
/// stays out of it: one fact, one line.
#[tokio::test]
async fn a_failed_run_states_a_swallowed_failure_once() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(UnavailableSymbols)],
        r#"
        const r = await tools.workspace_symbol({ query: "x" });
        throw new Error("boom");
        "#,
    )
    .await;
    let content = result.content;
    assert!(result.error.is_some(), "{content}");
    assert_eq!(content.matches("failed(unavailable)").count(), 1, "{content}");
    assert!(!content.contains("the program kept going"), "{content}");
    assert!(
        content
            .lines()
            .any(|line| line.starts_with("#1 tools.workspace_symbol")),
        "the manifest keeps the row: {content}"
    );
}

/// The other half of the rule: a call that succeeded says no failure line.
#[tokio::test]
async fn a_successful_call_reports_no_failure_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"const r = await tools.read({ path: "a.txt" }); return r.lines.length;"#,
    )
    .await;
    let content = result.content;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "{content}");
    assert!(!content.contains("failed("), "{content}");
    assert!(!content.contains("invalid_args"), "{content}");
    assert!(!content.contains("↳"), "{content}");
}
/// The defect this batch closes: a key the precheck cannot prove (it lives in
/// a variable) reaches the host, the host refuses the call before running it,
/// and the view said nothing at all. It now says it once, above the stream,
/// with the reason code and the names the next call may use.
#[tokio::test]
async fn a_computed_unknown_parameter_is_one_line_above_the_stream() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"
        const key = "pth";
        const args = { path: "a.txt", [key]: "a.txt" };
        try {
          const r = await tools.read(args);
          console.log("saw", r.lines.length);
        } catch (e) {
          console.log("saw", e.code);
        }
        return "done";
        "#,
    )
    .await;
    let content = result.content;
    assert!(result.error.is_none(), "{content}");
    let lines: Vec<&str> = content.lines().collect();
    let reported: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|line| line.starts_with("↳ #1 "))
        .collect();
    assert_eq!(reported.len(), 1, "{content}");
    let line = reported[0];
    assert!(
        line.starts_with("↳ #1 tools.read: invalid_args"),
        "{content}"
    );
    assert!(line.contains("unknown parameter 'pth'"), "{content}");
    assert!(
        line.contains("parameters: limit, path, start_line"),
        "the legal names are the actionable half: {content}"
    );
    let anchor = lines
        .iter()
        .position(|line| line.starts_with("↳ #1 "))
        .unwrap();
    let stream = lines
        .iter()
        .position(|line| line.starts_with("[log] saw"))
        .unwrap();
    assert!(anchor < stream, "{content}");
    assert_eq!(lines.last().copied(), Some("done"), "{content}");
}

/// A literal key is said by the precheck and again by the host; one line is
/// enough, and the anchored one is the line that can be located.
#[tokio::test]
async fn a_literal_unknown_parameter_is_stated_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"
        try {
          await tools.read({ path: "a.txt", pth: "x" });
          return "resolved";
        } catch (e) {
          return e.code;
        }
        "#,
    )
    .await;
    let content = result.content;
    assert_eq!(content.matches("unknown parameter 'pth'").count(), 1, "{content}");
    assert!(
        content.contains("↳ #1 tools.read: invalid_args — unknown parameter 'pth'"),
        "{content}"
    );
    assert!(content.contains("parameters: limit, path, start_line"), "{content}");
    // The precheck sentence would be the second line for the same fact.
    assert!(!content.contains("invalid input for"), "{content}");
}


/// A value the schema refuses names the legal values: the validator complaint
/// is copied into the line verbatim, so the next call can be right.
#[tokio::test]
async fn an_illegal_value_names_the_legal_ones() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::webfetch::WebFetchTool::new(Arc::new(
            std::sync::RwLock::new(None),
        )))],
        r#"
        const format = "pdf";
        try {
          await tools.webfetch({ url: "https://example.com", format });
          console.log("resolved");
        } catch (e) {
          console.log("saw", e.code);
        }
        return "done";
        "#,
    )
    .await;
    let content = result.content;
    let reported: Vec<&str> = content
        .lines()
        .filter(|line| line.starts_with("↳ #1 "))
        .collect();
    assert_eq!(reported.len(), 1, "{content}");
    let line = reported[0];
    assert!(
        line.starts_with("↳ #1 tools.webfetch: invalid_args"),
        "{content}"
    );
    assert!(
        line.contains("parameter 'format' expected one of text, markdown, html, got string \"pdf\""),
        "{content}"
    );
    assert!(line.contains("parameters: format, url"), "{content}");
}
/// Wrong arity keeps exactly the line it had: the precheck names it, and no
/// second, host-side line is added for the same call.
#[tokio::test]
async fn two_arguments_keep_their_single_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hi\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"
        const r = await tools.read({ path: "a.txt" }, { path: "b.txt" });
        return r.reason;
        "#,
    )
    .await;
    let content = result.content;
    let lines: Vec<&str> = content.lines().filter(|line| line.starts_with("↳")).collect();
    assert_eq!(lines.len(), 1, "{content}");
    assert!(
        lines[0].contains("accepts exactly one argument, got 2"),
        "{content}"
    );
    assert!(!lines[0].contains("#1 "), "the arity line keeps no anchor: {content}");
}
/// Item failures reject with an Error carrying details, and every reason a
/// program sees is one of the seven public values (no internal keys leak).
#[tokio::test]
async fn a_bad_edit_item_projects_public_reasons_without_leaking_internals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::apply_edits::ApplyEditsTool::new())],
        r#"
        const bad = [
            { path: "missing.txt", old_string: "a", new_string: "b" },
            { path: "a.txt", old_string: "zzz", new_string: "b" },
            { path: "a.txt", old_string: "alpha", new_string: "alpha" },
            { path: "a.txt", old_string: "a", new_string: "a" },
        ];
        try {
          await tools.apply_edits({ edits: bad });
          return "unexpected_ok";
        } catch (e) {
          return e.details.results.map((item) => item.reason).join(",");
        }
        "#,
    )
    .await;
    let content = result.content;
    assert!(!content.contains("invalid_output"), "{content}");
    assert!(result.error.is_none(), "{content}");
    for reason in ["not_found", "no_match", "invalid_args"] {
        assert!(content.contains(reason), "{reason} missing: {content}");
    }
    for leaked in ["empty_old_string", "no_op", "permission_denied", "invalid_input"] {
        assert!(!content.contains(leaked), "{leaked} leaked: {content}");
    }
}
/// A hits shaped result from another tool must not borrow grep wording.
#[tokio::test]
async fn a_hits_shape_from_another_tool_produces_no_grep_line() {
    struct HitsLike;
    impl Tool for HitsLike {
        fn name(&self) -> &str {
            "code_search"
        }
        fn surface(&self) -> ToolSurface {
            ToolSurface::Code
        }
        fn execute(
            &self,
            _input: Value,
            _execution: ToolExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
            let mut result = ToolCallResult::ok(String::new());
            result.data = Some(json!({
                "hits": [{
                    "path": "a.rs",
                    "anchor_line": 1,
                    "start_line": 1,
                    "end_line": 2,
                    "via": "lex",
                    "dense_similarity": 0.9,
                    "text": "x",
                    "snippet": "x"
                }],
            }));
            Box::pin(std::future::ready(result))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(HitsLike)],
        r#"const r = await tools.code_search({ query: "x" }); return r.hits.length;"#,
    )
    .await;
    let content = result.content;
    assert!(!content.contains("grep"), "{content}");
    assert!(!content.contains("↳"), "{content}");
}

/// Reminders do not need console output or a returned object either.
#[tokio::test]
async fn a_numeric_return_does_not_hide_the_facts() {
    let dir = tempfile::tempdir().unwrap();
    let body: String = (0..5).map(|i| format!("line{i}\n")).collect();
    std::fs::write(dir.path().join("a.txt"), body).unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::read::ReadTool::default())],
        r#"const r = await tools.read({ path: "a.txt", limit: 2 }); return r.lines.length;"#,
    )
    .await;
    let content = result.content;
    assert!(content.contains("tools.read: showing lines 1-2 of 5"), "{content}");
}

/// A failed nested call rejects instead of producing a platform envelope.
#[test]
fn failures_reject_instead_of_producing_an_envelope() {
    let mut result = ToolCallResult::error_with_kind(
        crate::types::ToolErrorKind::NotFound,
        "nope",
        false,
    );
    result.error_code = Some("not_found".into());
    assert!(crate::tool::outcome::program_result(&result).is_err());
}

/// S1: a name that does not exist ends the run with `not_found`, and there is
/// no ledger row because no call was dispatched.
#[tokio::test]
async fn s1_unknown_member_ends_the_run_without_a_ledger_row() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        try { await tools.readx({ path: "a.txt" }); } catch (e) { return "swallowed"; }
        return "continued";
        "#,
    )
    .await;
    assert_eq!(
        result.error_code.as_deref(),
        Some("not_found"),
        "header code: {}",
        result.content
    );
    assert!(
        result.content.contains("tools.readx"),
        "the message names the missing member: {}",
        result.content
    );
    assert!(
        !result.content.contains("#1"),
        "no call happened, so no ledger row may appear: {}",
        result.content
    );
    assert_eq!(runs.load(Ordering::SeqCst), 0, "{}", result.content);
}

/// S2: two positional arguments fail the call under the same reason as any
/// other input contract failure, and the program reads the envelope.
#[tokio::test]
async fn s2_two_arguments_fail_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        try {
          await tools.read({ path: "a.txt" }, { path: "b.txt" });
          return "resolved";
        } catch (e) {
          return e.code + " / " + e.message;
        }
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("invalid_args"), "{}", result.content);
    assert!(result.content.contains("got 2"), "{}", result.content);
    assert_eq!(runs.load(Ordering::SeqCst), 0, "{}", result.content);
}

/// S2: an argument tree too deep to own fails the call under the same reason.
#[tokio::test]
async fn s2_over_deep_arguments_fail_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        const root = {};
        let node = root;
        for (let i = 0; i < 200; i++) { node.next = {}; node = node.next; }
        try {
          await tools.read(root);
          return "resolved";
        } catch (e) {
          return e.code + " " + e.message;
        }
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("invalid_args"), "{}", result.content);
    assert!(result.content.contains("nested too deeply"), "{}", result.content);
    assert_eq!(runs.load(Ordering::SeqCst), 0, "{}", result.content);
}

/// S3: a result that does not match its own contract ends the run.
/// (Expected red before B2: today it becomes an invalid_output value.)
#[tokio::test]
async fn s3_result_that_violates_the_contract_ends_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(BadRead)],
        r#"
        try { await tools.read({ path: "a.txt" }); } catch (e) { return "swallowed"; }
        return "continued";
        "#,
    )
    .await;
    assert!(
        result.error.is_some(),
        "an invalid result must end the run: {}",
        result.content
    );
    assert!(result.content.contains("lines"), "{}", result.content);
    // One code for the same failure in both places this layer can see: the
    // manifest row and the header. The third place, the ToolError identity, is
    // built from the same slot and asserted at the engine layer
    // (`contract_latch_tests::contract_failure_carries_the_tool_error_code`).
    assert_eq!(
        result.error_code.as_deref(),
        Some("invalid_output"),
        "header code: {}",
        result.content
    );
    assert!(
        result.content.contains("failed(invalid_output)"),
        "manifest row must be a failure, not ok: {}",
        result.content
    );
}
