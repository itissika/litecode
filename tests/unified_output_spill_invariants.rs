//! Unified output spill, terminal `.stdout` contract, and JS resource invariants.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::test_auto_approve_sink;
use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::engines::WorkspaceEngines;
use litecode::ide_base::IdeBaseHandle;
use litecode::permission::PermissionEngine;
use litecode::ptc::Limits;
use litecode::terminal::{AgentJobRegistry, shell_command};
use litecode::tool::Tool;
use litecode::tool::executor::run_tool;
use litecode::tool::write_lock::process_write_lock;
use litecode::tools::apply_edits::ApplyEditsTool;
use litecode::tools::code::{CodeTool, CodeToolHost, MAX_CODE_OUTPUT_BYTES, RUN_CODE_VIEW_MAX_BYTES};
use litecode::tools::exec_command::ExecCommandTool;
use litecode::tools::knowledge::KnowledgeListTool;
use litecode::tools::read::ReadTool;
use litecode::types::{FunctionToolCall, ToolCallResult, ToolSignalLevel};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

fn walk_ext(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "node_modules" | "target" | "dist" | ".git") {
                continue;
            }
            walk_ext(&path, exts, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path);
        }
    }
}

fn non_test_prefix(src: &str) -> &str {
    src.split("#[cfg(test)]").next().unwrap_or(src)
}

fn make_ctx(root: &Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

fn make_permission(root: &Path) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        litecode::config::schema::AgentProfile::default(),
    );
    let resolved = ConfigManager::resolve_without_catalog(global, WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

async fn run_code_with_bundle(
    root: &Path,
    bundle: Vec<Arc<dyn Tool>>,
    code: &str,
) -> ToolCallResult {
    let data_root = root.join(".litecode");
    fs::create_dir_all(&data_root).unwrap();
    let permission = make_permission(root);
    let ctx = make_ctx(root);
    let sink = test_auto_approve_sink();
    let lock = process_write_lock();
    let host = Arc::new(CodeToolHost::new(
        permission.clone(),
        ctx.clone(),
        "default",
        Arc::clone(&sink),
        data_root.clone(),
        Arc::clone(&lock),
        None,
    ));
    let code_tool: Arc<dyn Tool> = Arc::new(CodeTool::new(bundle, host));
    let call = FunctionToolCall {
        arguments: json!({ "code": code }).to_string(),
        call_id: "inv_code_1".into(),
        namespace: None,
        name: "run_code".into(),
        id: None,
        status: None,
    };
    run_tool(
        &call,
        &[code_tool],
        &permission,
        &ctx,
        "inv-session",
        "default",
        sink.as_ref(),
        CancellationToken::new(),
        &data_root,
        65_536,
        None,
        lock,
        None,
    )
    .await
}

fn read_file_cmd(rel_path: &str) -> String {
    let uses_powershell = cfg!(windows)
        && shell_command("true")
            .program
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("powershell") || s.eq_ignore_ascii_case("pwsh"));
    if uses_powershell {
        format!("[Console]::Out.Write([System.IO.File]::ReadAllText('{rel_path}'))")
    } else {
        format!("cat {rel_path}")
    }
}

fn extract_full_output_path(text: &str) -> Option<String> {
    let (_, rest) = text.split_once("full output: ")?;
    let path = rest
        .lines()
        .next()?
        .split_whitespace()
        .next()?
        .trim_end_matches(&['.', ',', ';', ')', ']', '`', '\'', '"'][..]);
    Some(path.to_string())
}

#[test]
fn death_list_no_bespoke_spill_or_bypass_in_src() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut rs_files = Vec::new();
    walk_ext(&manifest.join("src"), &["rs"], &mut rs_files);

    let forbidden = [
        ".litecode/bash",
        "apply_emergency_cap",
        "<persisted-output>",
        "truncated_tool_result",
        "spill_content_if_needed",
        "spill_tool_results",
        "extract_degrade_omitted",
        "--- [omitted chunk",
        "budget_value",
        "water_fill_allowances",
        "INLINE_HEAD",
        "... (truncated)\n",
        "output was clipped by max_output_tokens",
    ];
    let mut hits = Vec::new();
    for path in rs_files {
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let prod = non_test_prefix(&raw);
        for needle in forbidden {
            if prod.contains(needle) {
                hits.push(format!("{}: forbidden `{needle}`", path.display()));
            }
        }
        if path.ends_with("knowledge.rs")
            && (prod.contains("fn paginate(") || prod.contains("fn spill("))
        {
            hits.push(format!("{}: knowledge must not paginate/spill", path.display()));
        }
    }
    assert!(hits.is_empty(), "production spill/bypass hits:\n{}", hits.join("\n"));
}

#[test]
fn death_list_frontend_has_no_legacy_bash_parser() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut fe_files = Vec::new();
    walk_ext(&manifest.join("web/src"), &["ts", "tsx"], &mut fe_files);

    let forbidden = [
        "parseBashOutput",
        "ParsedBashOutput",
        "--- head ---",
        "--- tail ---",
        "parseBashId",
        "isRunningStatusText",
        "isBackgroundBashResult",
        "headExitCode",
        "BashToolView",
    ];
    let mut hits = Vec::new();
    for path in fe_files {
        let s = path.to_string_lossy();
        if s.contains(".test.") || s.ends_with("bashLive.test.ts") {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        for needle in forbidden {
            if raw.contains(needle) {
                hits.push(format!("{}: forbidden `{needle}`", path.display()));
            }
        }
    }
    assert!(hits.is_empty(), "legacy frontend bash parser hits:\n{}", hits.join("\n"));
}

#[test]
fn ptc_limits_give_generous_js_memory_and_8mb_output_cap() {
    let limits = Limits::default();
    assert!(
        limits.memory_bytes >= 256 * 1024 * 1024,
        "memory_bytes={}",
        limits.memory_bytes
    );
    assert!(
        limits.stack_bytes >= 4 * 1024 * 1024,
        "stack_bytes={}",
        limits.stack_bytes
    );
    assert!(
        MAX_CODE_OUTPUT_BYTES >= 8 * 1024 * 1024,
        "MAX_CODE_OUTPUT_BYTES={MAX_CODE_OUTPUT_BYTES}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn run_code_2mb_stream_succeeds_and_spills_full_output_to_litecode_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let read_tool: Arc<dyn Tool> = Arc::new(ReadTool::default());

    let script = r#"
const chunk = "MID_RUN_2MB_ROW alpha beta gamma delta\n".repeat(48000);
inspect("HEAD_RUN_2MB");
inspect(chunk);
inspect("TAIL_RUN_2MB");
"#;
    let first = run_code_with_bundle(root, vec![Arc::clone(&read_tool)], script).await;
    assert_eq!(first.level, ToolSignalLevel::Ok, "{}", first.content);
    assert!(first.error.is_none(), "{}", first.content);
    assert!(first.content.starts_with("run_code ok"), "{}", first.content);
    assert!(
        first.content.len() <= RUN_CODE_VIEW_MAX_BYTES + 256,
        "over cap: {}",
        first.content.len()
    );
    assert!(first.content.contains("HEAD_RUN_2MB"), "{}", first.content);
    assert!(first.content.contains("TAIL_RUN_2MB"), "{}", first.content);
    assert!(!first.content.contains(".litecode/bash/"), "{}", first.content);

    let full_rel = extract_full_output_path(&first.content)
        .unwrap_or_else(|| panic!("expected `full output: <path>` in:\n{}", first.content));
    assert!(
        full_rel.starts_with(".litecode/run/run_code_"),
        "unexpected spill path: {full_rel}"
    );

    let on_disk = fs::read_to_string(root.join(&full_rel)).unwrap();
    assert!(on_disk.contains("HEAD_RUN_2MB"), "spilled file must keep head");
    assert!(on_disk.contains("MID_RUN_2MB_ROW"), "spilled file must keep middle");
    assert!(on_disk.contains("TAIL_RUN_2MB"), "spilled file must keep tail");

    let verify_read = format!(
        "const r = await tools.read({{ path: {} }}); inspect(r.lines.slice(0, 3));",
        serde_json::to_string(&full_rel).unwrap()
    );
    let second = run_code_with_bundle(root, vec![read_tool], &verify_read).await;
    assert!(second.error.is_none(), "{}", second.content);
    assert!(second.content.contains("HEAD_RUN_2MB"), "{}", second.content);
}

#[tokio::test(flavor = "current_thread")]
async fn run_code_failure_with_huge_stream_pins_header_and_error_above_cut() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("t.txt"), "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n").unwrap();

    let script = r#"
inspect("HEAD_FAIL_LOG");
inspect("NOISE ".repeat(30000));
inspect("TAIL_FAIL_LOG");
await tools.apply_edits({
  edits: [{ path: "t.txt", old_string: "fn beeta() {}", new_string: "fn BETA() {}" }]
});
"#;
    let res = run_code_with_bundle(root, vec![Arc::new(ApplyEditsTool::new())], script).await;
    assert!(res.error.is_some(), "{}", res.content);
    assert!(res.content.starts_with("Error: run_code failed:"), "{}", res.content);
    assert!(res.content.contains("closest match is at line 2: fn beta() {}"), "{}", res.content);
    assert!(res.content.contains("HEAD_FAIL_LOG"), "{}", res.content);
    assert!(res.content.contains("TAIL_FAIL_LOG"), "{}", res.content);
    let full_rel = extract_full_output_path(&res.content)
        .unwrap_or_else(|| panic!("expected `full output:` on failed huge stream:\n{}", res.content));
    assert!(full_rel.starts_with(".litecode/run/run_code_"), "{full_rel}");
    let disk = fs::read_to_string(root.join(&full_rel)).unwrap();
    assert!(disk.contains("HEAD_FAIL_LOG") && disk.contains("TAIL_FAIL_LOG"));
}

#[tokio::test(flavor = "current_thread")]
async fn exec_command_in_code_mode_passes_120kb_untruncated_and_exposes_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let payload = format!("HEAD_EXEC_120K\n{}TAIL_EXEC_120K\n", "x".repeat(120 * 1024));
    fs::write(root.join("big.txt"), &payload).unwrap();

    let jobs = Arc::new(AgentJobRegistry::new());
    let exec_tool: Arc<dyn Tool> = Arc::new(ExecCommandTool::new(jobs));
    let cmd = read_file_cmd("big.txt");
    let script = format!(
        r#"
const r = await tools.exec_command({{ cmd: {} }});
inspect({{
  has_head: r.output.includes("HEAD_EXEC_120K"),
  has_tail: r.output.includes("TAIL_EXEC_120K"),
  len: r.output.length,
  output_file: r.output_file,
}});
"#,
        serde_json::to_string(&cmd).unwrap()
    );
    let res = run_code_with_bundle(root, vec![exec_tool], &script).await;
    assert!(res.error.is_none(), "{}", res.content);
    assert!(res.content.contains("\"has_head\":true"), "{}", res.content);
    assert!(res.content.contains("\"has_tail\":true"), "{}", res.content);
    assert!(res.content.contains(".litecode/run/bg_"), "{}", res.content);
    assert!(res.content.contains(".stdout"), "{}", res.content);
    assert!(!res.content.contains("full output:"), "no spill needed when JS filters small return: {}", res.content);
}

#[tokio::test(flavor = "current_thread")]
async fn exec_command_explicit_small_cap_uses_unified_notice_and_clean_utf8() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let body = format!("中{}\nTAIL_UTF8_OK\n", "中".repeat(6000));
    fs::write(root.join("utf8.txt"), &body).unwrap();

    let jobs = Arc::new(AgentJobRegistry::new());
    let exec_tool: Arc<dyn Tool> = Arc::new(ExecCommandTool::new(jobs));
    let cmd = read_file_cmd("utf8.txt");
    let script = format!(
        r#"
const r = await tools.exec_command({{ cmd: {}, max_output_tokens: 256 }});
inspect({{
  dirty_prefix: r.output.startsWith("... (truncated)"),
  has_replacement: r.output.includes("\uFFFD"),
  has_tail: r.output.includes("TAIL_UTF8_OK"),
  output_file: r.output_file,
}});
"#,
        serde_json::to_string(&cmd).unwrap()
    );
    let res = run_code_with_bundle(root, vec![exec_tool], &script).await;
    assert!(res.error.is_none(), "{}", res.content);
    assert!(res.content.contains("\"dirty_prefix\":false"), "{}", res.content);
    assert!(res.content.contains("\"has_replacement\":false"), "{}", res.content);
    assert!(res.content.contains("\"has_tail\":true"), "{}", res.content);
    assert!(
        res.content.contains("bytes total; full output: .litecode/run/bg_"),
        "{}",
        res.content
    );
    assert!(
        !res.content.contains("output was clipped by max_output_tokens"),
        "{}",
        res.content
    );
}

#[tokio::test(flavor = "current_thread")]
async fn knowledge_tool_never_spills_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let kdir = root.join(".litecode").join("knowledge");
    fs::create_dir_all(&kdir).unwrap();
    for i in 0..150 {
        let key = format!("node_{i:03}");
        let body = format!(
            "```node\nnode : {key}\nstatus : enabled\nsummary : {}\n```\n\nBody {i}.\n",
            "detail ".repeat(12)
        );
        fs::write(kdir.join(format!("{key}.md")), body).unwrap();
    }

    let _ide = IdeBaseHandle::open(root.to_path_buf(), Arc::new(WorkspaceEngines::new())).unwrap();
    let tool: Arc<dyn Tool> = Arc::new(KnowledgeListTool::new());
    let script = "const r = await tools.knowledge.list(); inspect(r.nodes.map(n => n.key));";
    let res = run_code_with_bundle(root, vec![tool], script).await;
    assert!(res.error.is_none(), "{}", res.content);
    assert!(res.content.contains("node_149"), "all nodes kept without spill: {}", res.content);
    assert!(!root.join(".litecode/bash").exists(), ".litecode/bash must not exist");
    if let Ok(entries) = fs::read_dir(root.join(".litecode/run")) {
        for entry in entries.flatten() {
            assert!(
                !entry.file_name().to_string_lossy().contains("knowledge"),
                "knowledge must never spill to disk"
            );
        }
    }
}
