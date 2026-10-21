//! Contract checks for the first code-mode slice.
//!
//! The model-facing page is rendered from the tool contracts. Execution goes
//! through `run_code` on the embedded runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{PermissionEngine, deny_permission_sink};
use litecode::tool::executor::run_tool;
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::{Tool, render_code_tool_description};
use litecode::tools::code::{CodeTool, CodeToolHost};
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
use litecode::tools::read::ReadTool;
use litecode::types::FunctionToolCall;
use serde_json::{Value, json};

const PTC_SESSION_ID: &str = "ptc-contract";

struct PtcHarness {
    tools: Vec<Arc<dyn Tool>>,
    permission: PermissionEngine,
    ctx: Context,
    data_root: PathBuf,
}

impl PtcHarness {
    fn new(root: &Path) -> Self {
        let bundle: Vec<Arc<dyn Tool>> = vec![
            Arc::new(GlobTool),
            Arc::new(GrepTool),
            Arc::new(ReadTool::default()),
        ];
        let resolved = ConfigManager::resolve_without_catalog(
            GlobalSettings::default(),
            WorkspaceState::new(root),
        );
        let permission = PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: root.to_path_buf(),
            workspace_paths: WorkspacePaths::for_legacy_root(root),
            agents_md: None,
            claude_md: None,
        };
        let data_root = root.join(".litecode");
        std::fs::create_dir_all(&data_root).unwrap();
        let host = Arc::new(CodeToolHost::new(
            permission.clone(),
            ctx.clone(),
            "default",
            deny_permission_sink(),
            data_root.clone(),
            process_write_lock(),
            None,
        ));
        let tools = vec![Arc::new(CodeTool::new(bundle, host)) as Arc<dyn Tool>];
        Self {
            tools,
            permission,
            ctx,
            data_root,
        }
    }

    fn code_description(&self) -> String {
        render_code_tool_description(&[
            Arc::new(GlobTool),
            Arc::new(GrepTool),
            Arc::new(ReadTool::default()),
        ])
        .expect("render code interface")
    }

    async fn run_code(&self, user_script: &str) -> Result<Value, String> {
        let call = FunctionToolCall {
            arguments: json!({ "code": user_script }).to_string(),
            call_id: "ptc-call".into(),
            namespace: None,
            name: "run_code".into(),
            id: None,
            status: None,
        };
        let result = run_tool(
            &call,
            &self.tools,
            &self.permission,
            &self.ctx,
            PTC_SESSION_ID,
            "default",
            deny_permission_sink().as_ref(),
            tokio_util::sync::CancellationToken::new(),
            &self.data_root,
            32_768,
            None,
            process_write_lock(),
            None,
        )
        .await;
        if result.error.is_some() {
            return Err(result.content);
        }
        model_view_result(&result.content)
    }
}

fn model_view_result(content: &str) -> Result<Value, String> {
    let body = content
        .split_once("\nresult:\n")
        .map(|(_, body)| body)
        .or_else(|| content.strip_prefix("result:\n"))
        .ok_or_else(|| format!("run_code returned no result: {content}"))?;
    serde_json::from_str(body.trim()).map_err(|error| format!("{error}: {content}"))
}

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

#[test]
fn bundle_interface_is_generated_from_tool_contracts() {
    let dir = tempfile::tempdir().unwrap();
    let harness = PtcHarness::new(dir.path());
    let page = harness.code_description();

    assert!(page.contains("do not copy type annotations"));
    assert!(page.contains("declare class ToolError"));
    assert!(!page.contains("ToolResult<"));
    assert!(!page.contains("type Diagnostic = {"));
    assert!(!page.contains("type ToolCallOptions"));
    assert!(
        page.lines()
            .filter(|line| line.contains("readonly ["))
            .all(|line| !line.contains("options")),
        "{page}"
    );
    assert!(page.contains("type GlobInput = {"));
    assert!(page.contains("type GlobData = {"));
    assert!(page.contains("type GrepInput = {"));
    assert!(page.contains("type GrepData = {"));
    assert!(page.contains("type ReadInput = {"));
    assert!(page.contains("type ReadData ="));
    assert!(page.contains("declare const libs:"));
    assert!(page.contains("stats: {"));
    assert!(page.contains("csv: {"));
    assert!(page.contains("groupBy<T>"));

    assert!(page.contains("include_ignored?: boolean;"));
    assert!(page.contains("Tool timeouts are internal"));
    assert!(!page.contains("timeout_ms?:"));
    assert!(!page.contains("-u?:"));
    assert!(page.contains("literal?: boolean;"));
    assert!(page.contains("path: string;"));
    assert!(!page.contains("file_path"));
    assert!(!page.contains("context_lines"));
    assert!(!page.contains("view?:"));
    assert!(!page.contains("next_cursor"));
    assert!(!page.contains("next_start_line"));

    assert!(page.contains("stale_cursor"));
    assert!(page.contains("has_more: boolean;"));
    assert!(page.contains("next: string | null;"));
    assert!(page.contains("type Integer = number;"));

    assert!(page.contains("readonly [\"glob\"]: (args: GlobInput) => Promise<GlobData>;"));
    assert!(page.contains("readonly [\"grep\"]: (args: GrepInput) => Promise<GrepData>;"));
    assert!(page.contains("readonly [\"read\"]: (args: ReadInput) => Promise<ReadData>;"));
    assert!(!page.contains("options?: ToolCallOptions"));
    assert!(!page.replace("details?: unknown", "").contains(": unknown"));
    assert!(!page.contains("tools.call"));

    let glob = page.find("[\"glob\"]").unwrap();
    let grep = page.find("[\"grep\"]").unwrap();
    let read = page.find("[\"read\"]").unwrap();
    assert!(glob < grep && grep < read);
}

#[tokio::test(flavor = "current_thread")]
async fn embedded_runtime_runs_read_glob_grep() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "fn needle_a() {}\nsecond\n");
    write(dir.path(), "src/b.rs", "fn needle_b() {}\n");
    let harness = PtcHarness::new(dir.path());

    let script = r#"
const files = await tools.glob({ pattern: "src/**/*.rs" });
if (!Array.isArray(files.paths)) {
  throw new Error("glob contract failed");
}

const first = await tools.grep({ pattern: "needle", path: "src", limit: 1 });
if (!first.matches || first.matches.length !== 1 || !first.page.has_more) {
  throw new Error("grep first page contract failed: " + JSON.stringify(first));
}
if (!first.page.next) {
  throw new Error("grep must expose page.next");
}

const second = await tools.grep({
  pattern: "needle",
  path: "src",
  limit: 1,
  cursor: first.page.next,
});
if (!second.matches || second.matches.length !== 1 || second.page.has_more) {
  throw new Error("grep continuation contract failed: " + JSON.stringify(second));
}

const read = await tools.read({
  path: first.matches[0].path,
  start_line: 1,
  end_line: 1,
});
if (!read.file || !read.file.hash || read.lines.length !== 1 || !read.page.has_more) {
  throw new Error("read contract failed: " + JSON.stringify(read));
}

return {
  file_paths: files.paths,
  second_has_more: second.page.has_more,
  read_has_more: read.page.has_more,
  read_path: read.file.path,
  first_path: first.matches[0].path,
  first_line: read.lines[0].text,
};
"#;

    let value = harness
        .run_code(script)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let file_paths = value["file_paths"].as_array().unwrap();
    assert!(file_paths.iter().any(|path| path == "src/a.rs"));
    assert!(file_paths.iter().any(|path| path == "src/b.rs"));
    assert_eq!(value["second_has_more"], false);
    assert_eq!(value["read_has_more"], true);
    assert_eq!(value["read_path"], "src/a.rs");
    assert_eq!(value["first_path"], "src/a.rs");
    assert_eq!(value["first_line"], "fn needle_a() {}");
}

#[tokio::test(flavor = "current_thread")]
async fn literal_true_searches_the_pattern_as_text() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "a.b\naxb\n");
    let harness = PtcHarness::new(dir.path());

    let script = r#"
const hit = await tools.grep({ pattern: "a.b", path: "src/a.rs", literal: true });
if (!hit.matches || hit.matches.length !== 1 || !hit.matches[0].line_text.includes("a.b")) {
  throw new Error("literal search failed: " + JSON.stringify(hit));
}
return { hits: hit.matches.length };
"#;

    let value = harness
        .run_code(script)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value["hits"], 1);
}
