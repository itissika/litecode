//! Test-only PTC contract harness.
//!
//! This file is intentionally not wired into the production tool registry. It
//! bundles the first code-mode slice (`read`, `glob`, `grep`), verifies the
//! model-facing manifest, and executes real tool calls from a real JavaScript
//! runtime (Node) over a JSON-lines host bridge.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{PermissionEngine, PermissionSink, deny_permission_sink};
use litecode::tool::executor::run_tool;
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::{Tool, render_code_tool_description};
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
use litecode::tools::read::ReadTool;
use litecode::types::{FunctionToolCall, ToolCallResult, ToolSignalLevel};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PTC_SESSION_ID: &str = "ptc-contract";

const NODE_RUNNER: &str = r####"
import fs from 'node:fs';
import readline from 'node:readline';

console.log = (...args) => process.stderr.write(args.map(String).join(' ') + '\n');
console.info = console.log;

const pending = new Map();
let nextId = 0;

const rl = readline.createInterface({ input: process.stdin, terminal: false });
rl.on('line', (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  if (msg.type !== 'tool_result') return;
  const pendingCall = pending.get(String(msg.id));
  if (!pendingCall) return;
  pending.delete(String(msg.id));
  if (msg.ok) pendingCall.resolve(msg.result);
  else pendingCall.reject(new Error(msg.error || 'tool host error'));
});

function callTool(name, args) {
  return new Promise((resolve, reject) => {
    const id = String(++nextId);
    pending.set(id, { resolve, reject });
    process.stdout.write(JSON.stringify({
      type: 'tool_call',
      id,
      name,
      args: args ?? {},
    }) + '\n');
  });
}

const tools = {
  call: (name, args) => callTool(String(name), args),
};
for (const name of (process.env.PTC_TOOL_NAMES || '').split(',').filter(Boolean)) {
  tools[name] = (args) => callTool(name, args);
}

const userScript = fs.readFileSync(process.env.PTC_USER_SCRIPT, 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function finish(message) {
  process.stdout.write(JSON.stringify(message) + '\n', () => process.exit(0));
}

try {
  const body = '"use strict";\nconst require = undefined;\nconst global = undefined;\n' + userScript;
  const fn = new AsyncFunction('tools', 'process', body);
  const value = await fn(tools, undefined);
  rl.close();
  finish({ type: 'final', ok: true, value: value ?? null });
} catch (error) {
  rl.close();
  finish({
    type: 'final',
    ok: false,
    error: String(error && error.stack ? error.stack : error),
  });
}
"####;

struct PtcHarness {
    root: PathBuf,
    tools: Vec<Arc<dyn Tool>>,
    permission: PermissionEngine,
    ctx: Context,
    data_root: PathBuf,
    sink: Arc<dyn PermissionSink>,
}

impl PtcHarness {
    fn new(root: &Path) -> Self {
        let tools: Vec<Arc<dyn Tool>> = vec![
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

        Self {
            root: root.to_path_buf(),
            tools,
            permission,
            ctx,
            data_root,
            sink: deny_permission_sink(),
        }
    }

    fn code_description(&self) -> String {
        render_code_tool_description(&self.tools).expect("render code interface")
    }

    async fn execute_tool(&self, name: &str, args: Value, call_id: &str) -> ToolCallResult {
        let call = FunctionToolCall {
            arguments: args.to_string(),
            call_id: call_id.to_string(),
            namespace: None,
            name: name.to_string(),
            id: None,
            status: None,
        };
        run_tool(
            &call,
            &self.tools,
            &self.permission,
            &self.ctx,
            PTC_SESSION_ID,
            "default",
            self.sink.as_ref(),
            tokio_util::sync::CancellationToken::new(),
            &self.data_root,
            32_768,
            None,
            process_write_lock(),
            None,
        )
        .await
    }
}

fn tool_result_to_json(result: &ToolCallResult) -> Value {
    let status = match result.level {
        ToolSignalLevel::Ok => "ok",
        ToolSignalLevel::Warning => "warning",
        ToolSignalLevel::Error => "error",
    };
    json!({
        "status": status,
        "text": result.content,
        "data": result.data,
        "error": result.error,
    })
}

fn node_bin() -> String {
    std::env::var("LITECODE_NODE_BIN").unwrap_or_else(|_| "node".to_string())
}

fn node_available() -> bool {
    std::process::Command::new(node_bin())
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

async fn run_code(harness: &PtcHarness, user_script: &str) -> Result<Value, String> {
    let runner_path = harness.root.join(".litecode").join("ptc_runner.mjs");
    let script_path = harness.root.join(".litecode").join("ptc_user.mjs");
    std::fs::write(&runner_path, NODE_RUNNER).map_err(|error| error.to_string())?;
    std::fs::write(&script_path, user_script).map_err(|error| error.to_string())?;

    let tool_names = harness
        .tools
        .iter()
        .map(|tool| tool.name())
        .collect::<Vec<_>>()
        .join(",");
    let mut child = tokio::process::Command::new(node_bin())
        .arg(&runner_path)
        .env("PTC_USER_SCRIPT", &script_path)
        .env("PTC_TOOL_NAMES", tool_names)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("failed to spawn node: {error}"))?;

    let mut stdin = child.stdin.take().ok_or("missing node stdin")?;
    let stdout = child.stdout.take().ok_or("missing node stdout")?;
    let stderr = child.stderr.take().ok_or("missing node stderr")?;
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut text = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            text.push_str(&line);
            text.push('\n');
        }
        text
    });

    let exchange = async {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = lines
                .next_line()
                .await
                .map_err(|error| format!("node stdout read failed: {error}"))?
                .ok_or_else(|| "node exited before final result".to_string())?;
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)
                .map_err(|error| format!("invalid node protocol line: {error}: {line}"))?;
            match message["type"].as_str() {
                Some("tool_call") => {
                    let id = message["id"].as_str().ok_or("tool call missing id")?;
                    let name = message["name"].as_str().ok_or("tool call missing name")?;
                    let args = message["args"].clone();
                    let result = harness.execute_tool(name, args, &format!("ptc-{id}")).await;
                    let response = json!({
                        "type": "tool_result",
                        "id": id,
                        "ok": true,
                        "result": tool_result_to_json(&result),
                    });
                    let mut encoded = serde_json::to_vec(&response)
                        .map_err(|error| format!("encode tool response: {error}"))?;
                    encoded.push(b'\n');
                    stdin
                        .write_all(&encoded)
                        .await
                        .map_err(|error| format!("write node stdin: {error}"))?;
                    stdin
                        .flush()
                        .await
                        .map_err(|error| format!("flush node stdin: {error}"))?;
                }
                Some("final") => {
                    if message["ok"].as_bool() == Some(true) {
                        return Ok(message["value"].clone());
                    }
                    return Err(message["error"]
                        .as_str()
                        .unwrap_or("node script failed without an error")
                        .to_string());
                }
                other => return Err(format!("unexpected node protocol message: {other:?}")),
            }
        }
    };

    let result = tokio::time::timeout(Duration::from_secs(20), exchange).await;
    drop(stdin);
    match result {
        Ok(Ok(value)) => {
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            let stderr_text = stderr_task.await.unwrap_or_default();
            if !stderr_text.trim().is_empty() {
                eprintln!("ptc node stderr:\n{stderr_text}");
            }
            Ok(value)
        }
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let stderr_text = stderr_task.await.unwrap_or_default();
            Err(format!("{error}\nnode stderr:\n{stderr_text}"))
        }
        Err(_) => {
            let _ = child.kill().await;
            let stderr_text = stderr_text_if_ready(stderr_task).await;
            Err(format!(
                "node script timed out\nnode stderr:\n{stderr_text}"
            ))
        }
    }
}

async fn stderr_text_if_ready(task: tokio::task::JoinHandle<String>) -> String {
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .ok()
        .and_then(|result| result.ok())
        .unwrap_or_default()
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
    assert!(page.contains("type ToolResult<T> ="));
    assert!(page.contains("type ToolCallOptions = {"));
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
    assert!(page.contains("timeout_ms?: number;"));
    assert!(!page.contains("-u?:"));
    assert!(page.contains("view?: \"lines\" | \"context\" | \"expanded\";"));
    assert!(page.contains("path: string;"));
    assert!(!page.contains("file_path: string;"));

    assert!(page.contains("// Opaque continuation cursor from a previous grep result."));
    assert!(page.contains("next_cursor: string | null;"));
    assert!(page.contains("truncated: boolean;"));
    assert!(page.contains("next_start_line: Integer | null;"));
    assert!(page.contains("type Integer = number;"));

    assert!(page.contains(
        "readonly [\"glob\"]: (args: GlobInput, options?: ToolCallOptions) => Promise<ToolResult<GlobData>>;"
    ));
    assert!(page.contains(
        "readonly [\"grep\"]: (args: GrepInput, options?: ToolCallOptions) => Promise<ToolResult<GrepData>>;"
    ));
    assert!(page.contains(
        "readonly [\"read\"]: (args: ReadInput) => Promise<ToolResult<ReadData>>;"
    ));
    assert!(!page.contains("read(args: ReadInput, options?: ToolCallOptions)"));
    assert!(!page.contains(": unknown"));
    assert!(!page.contains("tools.call"));

    let glob = page.find("[\"glob\"]").unwrap();
    let grep = page.find("[\"grep\"]").unwrap();
    let read = page.find("[\"read\"]").unwrap();
    assert!(glob < grep && grep < read);
}

#[tokio::test(flavor = "current_thread")]
async fn node_runs_real_read_glob_grep_bridge() {
    if !node_available() {
        eprintln!("skipping PTC Node harness: node is not available");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "src/a.rs", "fn needle_a() {}\n");
    write(dir.path(), "src/b.rs", "fn needle_b() {}\n");
    let harness = PtcHarness::new(dir.path());

    let script = r#"
const files = await tools.glob({ pattern: "src/**/*.rs" });
if (files.status !== "ok" || !Array.isArray(files.data.paths)) {
  throw new Error("glob contract failed");
}

const first = await tools.grep({ pattern: "needle", path: "src", limit: 1 });
if (!first.data || first.data.total !== 2 || first.data.matches.length !== 1) {
  throw new Error("grep first page contract failed");
}
if (!first.data.next_cursor) {
  throw new Error("grep must expose next_cursor");
}

const second = await tools.grep({
  pattern: "needle",
  path: "src",
  limit: 1,
  cursor: first.data.next_cursor,
});
if (!second.data || second.data.matches.length !== 1 || second.data.has_more) {
  throw new Error("grep continuation contract failed");
}

const read = await tools.read({
  path: first.data.matches[0].path,
  start_line: 1,
  end_line: 1,
});
if (!read.data || read.data.kind !== "text" || read.data.lines.length !== 1) {
  throw new Error("read contract failed: " + JSON.stringify(read));
}

return {
  file_paths: files.data.paths,
  grep_total: first.data.total,
  second_has_more: second.data.has_more,
  read_kind: read.data.kind,
  read_path: read.data.path,
  first_path: first.data.matches[0].path,
  first_line: read.data.lines[0].text,
};
"#;

    let value = run_code(&harness, script)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let file_paths = value["file_paths"].as_array().unwrap();
    assert!(file_paths.iter().any(|path| path == "src/a.rs"));
    assert!(file_paths.iter().any(|path| path == "src/b.rs"));
    assert_eq!(value["grep_total"], 2);
    assert_eq!(value["second_has_more"], false);
    assert_eq!(value["read_kind"], "text");
    assert_eq!(value["read_path"], "src/a.rs");
    assert_eq!(value["first_path"], "src/a.rs");
    assert_eq!(value["first_line"], "fn needle_a() {}");
}

#[tokio::test(flavor = "current_thread")]
async fn node_view_switch_keeps_structured_data_identical() {
    if !node_available() {
        eprintln!("skipping PTC Node harness: node is not available");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "src/a.rs",
        "fn outer() {\n    if ready {\n        needle();\n    }\n}\n",
    );
    let harness = PtcHarness::new(dir.path());

    let script = r#"
const lines = await tools.grep({ pattern: "needle", path: "src", view: "lines" });
const expanded = await tools.grep({ pattern: "needle", path: "src", view: "expanded" });
if (JSON.stringify(lines.data) !== JSON.stringify(expanded.data)) {
  throw new Error("view changed structured grep data");
}
if (lines.text === expanded.text) {
  throw new Error("expanded view should change text presentation");
}
return { data_identical: true };
"#;

    let value = run_code(&harness, script)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(value["data_identical"], true);
}
