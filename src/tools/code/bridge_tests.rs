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
use crate::tools::edit::EditTool;
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
        result.data = Some(json!({ "path": "a.txt", "start_line": 1, "lines": 42, "message": "" }));
        Box::pin(std::future::ready(result))
    }
}

/// S2: an unknown parameter fails the call, not the run. The program gets a
/// failure envelope, keeps going, and the manifest row is a failure.
#[tokio::test]
async fn s2_unknown_parameter_fails_the_call_and_the_run_continues() {
    let dir = tempfile::tempdir().unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let result = run_program(
        dir.path(),
        vec![Arc::new(CountingRead {
            runs: Arc::clone(&runs),
        })],
        r#"
        const r = await tools.file.read({ path: "a.txt", nope: 1 });
        return r.ok + " " + r.reason + " " + r.message;
        "#,
    )
    .await;
    assert!(
        result.error.is_none(),
        "the run must survive a bad argument: {}",
        result.content
    );
    assert!(result.content.contains("false invalid_args"), "{}", result.content);
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
        const r = await tools.file.read({});
        return r.ok + " " + r.reason + " " + r.message;
        "#,
    )
    .await;
    assert!(
        result.error.is_none(),
        "the run must survive a missing argument: {}",
        result.content
    );
    assert!(result.content.contains("false invalid_args"), "{}", result.content);
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
        const first = await tools.file.read({ path: "a.txt", nope: 1 });
        const second = await tools.file.read({ path: "a.txt" });
        return first.ok + " " + second.ok;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("false true"), "{}", result.content);
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "the valid call must still run: {}",
        result.content
    );
}

/// The edit-input cut: a replacement the tool refuses for its input is a
/// *value*, so the program keeps running and can branch on it.
#[tokio::test]
async fn edit_input_failure_resolves_as_a_value() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(EditTool::new())],
        r#"
        const r = await tools.file.edit({ path: "a.txt", old_string: "hello", new_string: "hello" });
        console.log("continued", r.ok, r.reason);
        throw new Error("after-edit");
        "#,
    )
    .await;
    // Reaching the throw proves the edit did not end the run.
    assert!(result.content.contains("after-edit"), "{}", result.content);
    assert!(
        result.content.contains("[log] continued false invalid_args"),
        "the program must see ok:false / reason invalid_args: {}",
        result.content
    );
    assert!(
        result.content.contains("failed(invalid_args)"),
        "the ledger row keeps the JS-facing label: {}",
        result.content
    );
}

/// An edit that is refused for permission is a value too: the run continues and
/// the program can branch, while the ledger row carries the real code.
#[tokio::test]
async fn edit_on_a_session_record_resolves_as_a_permission_value() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(EditTool::new())],
        r#"
        const r = await tools.file.edit({
            path: ".litecode/sessions/01M6MH2CWCE5DJDAZBE0E3THT5.md",
            old_string: "a",
            new_string: "b",
        });
        console.log("continued", r.ok, r.reason);
        throw new Error("after-edit");
        "#,
    )
    .await;
    assert!(result.content.contains("after-edit"), "{}", result.content);
    assert!(
        result.content.contains("[log] continued false permission"),
        "the program must see ok:false / reason permission: {}",
        result.content
    );
    assert!(
        result.content.contains("failed(permission_denied)"),
        "the ledger row carries the table key: {}",
        result.content
    );
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
        const r = await tools.file.read({ path: "a.txt" });
        const data = r.data;
        return typeof data;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "the run must survive a wrong field: {}", result.content);
    assert!(result.content.contains("'data'"), "{}", result.content);
    assert!(result.content.contains("start_line"), "{}", result.content);
    assert!(result.content.contains("undefined"), "{}", result.content);
}

/// S4 (was P9): reading a success-only field on a failed result is reported
/// with the real reason and message, and the run continues.
#[tokio::test]
async fn s4_failed_result_read_is_a_warning() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(FailingRead)],
        r#"
        const r = await tools.file.read({ path: "p" });
        const lines = r.lines;
        return r.reason;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("not_found"), "{}", result.content);
    assert!(result.content.contains("p does not exist"), "{}", result.content);
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
        const g = await tools.file.grep({ pattern: "a" });
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
        const r = await tools.file.read({ path: "a.txt" });
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
        r#"const r = await tools.file.glob({ pattern: "*.txt", limit: 2 }); return JSON.parse(JSON.stringify(r));"#,
    )
    .await;
    let content = result.content;
    assert!(content.contains("\"total\""), "{content}");
    assert!(content.contains("\"truncated\""), "{content}");
    assert!(
        content.contains("file.glob: showing first 2 of 3 paths"),
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
        r#"const r = await tools.file.read({ path: "a.txt", limit: 2 }); return r.lines;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains("file.read: showing lines 1-2 of 5; continue with start_line 3"),
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
        r#"return await tools.file.glob({ pattern: "*.txt" });"#,
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
        r#"const a = await tools.lsp.diagnostics({ path: "a.rs" });
const b = await tools.lsp.diagnostics({ path: "b.rs" }); return a.state + b.state;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains(
            "↳ #1 lsp.diagnostics: no diagnostics reported; the index is still building, "
        ),
        "{content}"
    );
    assert!(
        content.contains(
            "↳ #2 lsp.diagnostics: no diagnostics reported; the index state is unknown, "
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
        r#"const r = await tools.file.grep({ pattern: "fn ", path: "a.rs", limit: 5 });
return r.hits.length;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains(
            "↳ #1 file.grep: showing the first 5 matches; more exist — narrow the pattern/path or raise limit"
        ),
        "{content}"
    );
}
/// A POSIX-looking argument reaches the model as a flag and one line, not as
/// prose glued onto the command output.
#[tokio::test]
async fn a_path_like_argument_reaches_the_model_as_a_field_and_a_line() {
    if !crate::terminal::decode::shell_identity().contains("MINGW") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let tool = crate::tools::run::RunTool::with_window(
        Arc::new(crate::terminal::AgentJobRegistry::new()),
        std::time::Duration::from_secs(5),
    );
    let result = run_program(
        dir.path(),
        vec![Arc::new(tool)],
        r#"const r = await tools.process.run({ command: "cmd /c echo hi" });
return r.path_conversion;"#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains(
            "↳ #1 process.run: the shell may convert POSIX-style paths; write `cmd //c` or set `MSYS_NO_PATHCONV=1`"
        ),
        "{content}"
    );
}

/// The path risk used to hide in a side channel; a program sees the field and
/// the model sees one line that names the resolved target.
#[tokio::test]
async fn an_outside_write_reaches_the_model_as_a_field_and_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("note.txt");
    let literal = serde_json::to_string(&target.to_string_lossy().to_string()).unwrap();
    let code = format!(
        "const r = await tools.file.write({{ path: {literal}, content: \"hi\" }});\nreturn r.path_risk;"
    );
    let result = run_program(
        dir.path(),
        vec![Arc::new(crate::tools::write::WriteTool::new())],
        &code,
    )
    .await;
    let content = result.content;
    assert!(content.contains("outside_workspace"), "{content}");
    let expected = format!(
        "↳ #1 file.write: this write lands outside the workspace: {}",
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
/// "no symbols reported; the index state is unknown", as if it had run.
#[tokio::test]
async fn a_failed_lsp_call_adds_no_line_to_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_program(
        dir.path(),
        vec![Arc::new(UnavailableSymbols)],
        r#"
        const r = await tools.lsp.workspace_symbol({ query: "x" });
        console.log("saw", r.ok, r.reason);
        return 1;
        "#,
    )
    .await;
    let content = result.content;
    assert!(
        content.contains("[log] saw false unavailable"),
        "the failure must reach the program as a value: {content}"
    );
    assert!(
        !content.contains("no symbols reported"),
        "a failed call guessed a result: {content}"
    );
    // The ledger row stays; no renderer line may be added.
    assert!(!content.contains("↳"), "a failed call added a line: {content}");
}
/// Item failures stay item-level: an unknown or internal reason must not end
/// the program, and every reason a program sees is one of the six values.
#[tokio::test]
async fn a_bad_edit_item_never_terminates_the_run() {
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
        const r = await tools.file.apply_edits({ edits: bad });
        return r.results.map((item) => item.reason).join(",");
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
        r#"const r = await tools.search.code_search({ query: "x" }); return r.hits.length;"#,
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
        r#"const r = await tools.file.read({ path: "a.txt", limit: 2 }); return r.lines.length;"#,
    )
    .await;
    let content = result.content;
    assert!(content.contains("file.read: showing lines 1-2 of 5"), "{content}");
}

/// The failure field list is the same one the program is handed.
#[test]
fn failure_fields_match_the_program_envelope() {
    let mut result = ToolCallResult::error_with_kind(
        crate::types::ToolErrorKind::NotFound,
        "nope",
        false,
    );
    result.error_code = Some("not_found".into());
    let value = crate::tool::outcome::program_result(&result).expect("a soft failure resolves");
    let mut keys: Vec<String> = value
        .as_object()
        .expect("object envelope")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    let mut expected: Vec<String> = crate::tool::outcome::FAILURE_FIELDS
        .iter()
        .map(|field| (*field).to_string())
        .collect();
    expected.sort();
    assert_eq!(keys, expected);
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
        try { await tools.file.readx({ path: "a.txt" }); } catch (e) { return "swallowed"; }
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
        result.content.contains("file.readx"),
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
        const r = await tools.file.read({ path: "a.txt" }, { path: "b.txt" });
        return r.reason + " / " + r.message;
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
        const r = await tools.file.read(root);
        return r.ok + " " + r.reason;
        "#,
    )
    .await;
    assert!(result.error.is_none(), "{}", result.content);
    assert!(result.content.contains("false invalid_args"), "{}", result.content);
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
        try { await tools.file.read({ path: "a.txt" }); } catch (e) { return "swallowed"; }
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
