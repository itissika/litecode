//! Phase 0 baseline for `read` / `glob` / `grep` / `edit` / `write`.
//!
//! Each fixture locks the current `data` and `error` seen through the nested
//! code path (`execute_tool_call` with [`ToolCaller::Code`]). `content_historical`
//! is stored beside them and is not an assertion target.
//!
//! Scenarios the tools cannot express are JSON gaps. They are not skipped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::resolved::{WorkspaceState, resolve_without_catalog};
use crate::config::schema::{AgentProfile, AgentToolBinding, GlobalSettings};
use crate::context_pipeline::Context;
use crate::permission::{BindingPathMode, PermissionAction, PermissionEngine, ToolPolicy};
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::Tool;
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::types::{FunctionToolCall, ToolCallResult, ToolErrorKind};

const SCENARIOS: &[&str] = &[
    "success",
    "empty",
    "truncation",
    "invalid_input",
    "permission_denied",
    "cancelled",
    "partial_failure",
];

#[derive(Clone, Copy)]
enum Lane {
    Allow,
    Deny,
    Cancel,
}

#[derive(Clone, Copy)]
enum Bulk {
    None,
    LongLine,
    ManyTxt,
}

struct Live {
    tool: &'static str,
    scenario: &'static str,
    lane: Lane,
    files: &'static [(&'static str, &'static str)],
    bulk: Bulk,
    args: Value,
}

fn catalog() -> (Vec<Live>, Vec<(&'static str, &'static str)>) {
    let live = vec![
        live(
            "read",
            "success",
            Lane::Allow,
            &[("note.txt", "alpha\nbeta\n")],
            Bulk::None,
            json!({"path":"note.txt"}),
        ),
        live(
            "read",
            "empty",
            Lane::Allow,
            &[("empty.txt", "")],
            Bulk::None,
            json!({"path":"empty.txt"}),
        ),
        live(
            "read",
            "truncation",
            Lane::Allow,
            &[],
            Bulk::LongLine,
            json!({"path":"long.txt"}),
        ),
        live(
            "read",
            "invalid_input",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({}),
        ),
        live(
            "read",
            "permission_denied",
            Lane::Deny,
            &[("note.txt", "alpha\n")],
            Bulk::None,
            json!({"path":"note.txt"}),
        ),
        live(
            "read",
            "cancelled",
            Lane::Cancel,
            &[("note.txt", "alpha\n")],
            Bulk::None,
            json!({"path":"note.txt"}),
        ),
        live(
            "glob",
            "success",
            Lane::Allow,
            &[("keep.txt", "a")],
            Bulk::None,
            json!({"pattern":"**/*.txt"}),
        ),
        live(
            "glob",
            "empty",
            Lane::Allow,
            &[("keep.txt", "a")],
            Bulk::None,
            json!({"pattern":"**/zz-nope-*.qq"}),
        ),
        live(
            "glob",
            "truncation",
            Lane::Allow,
            &[],
            Bulk::ManyTxt,
            json!({"pattern":"**/*.txt"}),
        ),
        live(
            "glob",
            "invalid_input",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({}),
        ),
        live(
            "glob",
            "permission_denied",
            Lane::Deny,
            &[("keep.txt", "a")],
            Bulk::None,
            json!({"pattern":"**/*.txt"}),
        ),
        live(
            "glob",
            "cancelled",
            Lane::Cancel,
            &[("keep.txt", "a")],
            Bulk::None,
            json!({"pattern":"**/*.txt"}),
        ),
        live(
            "grep",
            "success",
            Lane::Allow,
            &[("a.txt", "alpha needle\n")],
            Bulk::None,
            json!({"pattern":"needle","path":"a.txt"}),
        ),
        live(
            "grep",
            "empty",
            Lane::Allow,
            &[("a.txt", "alpha\n")],
            Bulk::None,
            json!({"pattern":"missing-needle","path":"a.txt"}),
        ),
        live(
            "grep",
            "truncation",
            Lane::Allow,
            &[("hits.txt", "hit one\nhit two\n")],
            Bulk::None,
            json!({"pattern":"hit","path":"hits.txt","limit":1}),
        ),
        live(
            "grep",
            "invalid_input",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({}),
        ),
        live(
            "grep",
            "permission_denied",
            Lane::Deny,
            &[("a.txt", "alpha needle\n")],
            Bulk::None,
            json!({"pattern":"needle","path":"a.txt"}),
        ),
        live(
            "grep",
            "cancelled",
            Lane::Cancel,
            &[("a.txt", "alpha needle\n")],
            Bulk::None,
            json!({"pattern":"needle","path":"a.txt"}),
        ),
        live(
            "edit",
            "success",
            Lane::Allow,
            &[("a.txt", "alpha\n")],
            Bulk::None,
            json!({"path":"a.txt","old_string":"alpha","new_string":"beta"}),
        ),
        live(
            "edit",
            "empty",
            Lane::Allow,
            &[("empty.txt", "")],
            Bulk::None,
            json!({"path":"empty.txt","old_string":"alpha","new_string":"beta"}),
        ),
        live(
            "edit",
            "invalid_input",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({}),
        ),
        live(
            "edit",
            "permission_denied",
            Lane::Deny,
            &[("a.txt", "alpha\n")],
            Bulk::None,
            json!({"path":"a.txt","old_string":"alpha","new_string":"beta"}),
        ),
        live(
            "edit",
            "cancelled",
            Lane::Cancel,
            &[("a.txt", "alpha\n")],
            Bulk::None,
            json!({"path":"a.txt","old_string":"alpha","new_string":"beta"}),
        ),
        live(
            "edit",
            "partial_failure",
            Lane::Allow,
            &[("a.txt", "alpha\nbeta\n")],
            Bulk::None,
            json!({"path":"a.txt","old_string":"missing_token_zz","new_string":"nope"}),
        ),
        live(
            "write",
            "success",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({"path":"new.txt","content":"hello"}),
        ),
        live(
            "write",
            "empty",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({"path":"blank.txt","content":""}),
        ),
        live(
            "write",
            "invalid_input",
            Lane::Allow,
            &[],
            Bulk::None,
            json!({}),
        ),
        live(
            "write",
            "permission_denied",
            Lane::Deny,
            &[],
            Bulk::None,
            json!({"path":"new.txt","content":"hello"}),
        ),
        live(
            "write",
            "cancelled",
            Lane::Cancel,
            &[],
            Bulk::None,
            json!({"path":"new.txt","content":"hello"}),
        ),
    ];
    let gaps = vec![
        ("read", "partial_failure"),
        ("glob", "partial_failure"),
        ("grep", "partial_failure"),
        ("edit", "truncation"),
        ("write", "truncation"),
        ("write", "partial_failure"),
    ];
    (live, gaps)
}

fn live(
    tool: &'static str,
    scenario: &'static str,
    lane: Lane,
    files: &'static [(&'static str, &'static str)],
    bulk: Bulk,
    args: Value,
) -> Live {
    Live {
        tool,
        scenario,
        lane,
        files,
        bulk,
        args,
    }
}

fn fixture_path(tool: &str, scenario: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tools/baseline/fixtures")
        .join(format!("{tool}__{scenario}.json"))
}

fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(crate::tools::read::ReadTool::default()),
        Arc::new(crate::tools::glob::GlobTool),
        Arc::new(crate::tools::grep::GrepTool),
        Arc::new(crate::tools::edit::EditTool::new()),
        Arc::new(crate::tools::write::WriteTool::new()),
    ]
}

fn allow_engine(root: &Path) -> PermissionEngine {
    let resolved = resolve_without_catalog(GlobalSettings::default(), WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

fn deny_engine(root: &Path) -> PermissionEngine {
    let mut profile = AgentProfile::default();
    for name in ["read", "glob", "grep", "edit", "write"] {
        profile.tools.insert(
            name.to_string(),
            AgentToolBinding {
                enabled: true,
                policy: ToolPolicy::with_default(PermissionAction::Deny),
                path_mode: BindingPathMode::Unrestricted,
                last_applied_preset: None,
                allowed_tools: None,
            },
        );
    }
    let mut global = GlobalSettings::default();
    global.agents.insert("default".into(), profile);
    let resolved = resolve_without_catalog(global, WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

fn code_origin() -> ToolCallOrigin {
    ToolCallOrigin {
        caller: ToolCaller::Code {
            parent_call_id: "p0".into(),
        },
        code_names: Arc::from([]),
        file_note: None,
        permission_sink: None,
        plan_touch: None,
    }
}

fn prepare(root: &Path, case: &Live) {
    for (rel, content) in case.files {
        write_rel(root, rel, content);
    }
    match case.bulk {
        Bulk::None => {}
        Bulk::LongLine => write_rel(root, "long.txt", &"a".repeat(1600)),
        Bulk::ManyTxt => {
            for index in 0..1001 {
                write_rel(root, &format!("g{index:04}.txt"), "x");
            }
        }
    }
}

fn write_rel(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

async fn run_case(case: &Live, root: &Path, data_root: &Path) -> ToolCallResult {
    let permission = match case.lane {
        Lane::Allow | Lane::Cancel => allow_engine(root),
        Lane::Deny => deny_engine(root),
    };
    let cancel = CancellationToken::new();
    if matches!(case.lane, Lane::Cancel) {
        cancel.cancel();
    }
    let ctx = Context {
        cwd: root.to_path_buf(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    };
    let call = FunctionToolCall {
        arguments: case.args.to_string(),
        call_id: format!("{}-{}", case.tool, case.scenario),
        name: case.tool.into(),
        namespace: None,
        id: None,
        status: None,
    };
    let sink = crate::permission::sinks::deny_permission_sink();
    execute_tool_call(
        &call,
        &tools(),
        &permission,
        &ctx,
        "p0-session",
        "default",
        sink.as_ref(),
        cancel,
        data_root,
        usize::MAX,
        None,
        Arc::new(WorkspaceWriteLock::new()),
        None,
        code_origin(),
    )
    .await
}

fn expect_shape(case: &Live, result: &ToolCallResult) {
    let label = format!("{}/{}", case.tool, case.scenario);
    match (case.tool, case.scenario) {
        ("edit", "empty") => {
            assert!(
                result.error.is_some() && result.data.is_none(),
                "{label} empty file is an error without data: {}",
                describe(result)
            );
        }
        (_, "invalid_input") => {
            assert_kind(result, ToolErrorKind::InvalidInput, &label);
            assert!(result.data.is_none(), "{label} data={}", describe(result));
        }
        (_, "permission_denied") => {
            assert_kind(result, ToolErrorKind::PermissionDenied, &label);
            assert!(result.data.is_none(), "{label} data={}", describe(result));
        }
        (_, "cancelled") => {
            assert_kind(result, ToolErrorKind::Cancelled, &label);
            assert!(result.data.is_none(), "{label} data={}", describe(result));
        }
        ("edit", "partial_failure") => {
            assert!(
                result.error.is_none() && result.data.is_some(),
                "{label} partial edit still returns data: {}",
                describe(result)
            );
        }
        (_, "success" | "empty" | "truncation") => {
            assert!(
                result.error.is_none() && result.data.is_some(),
                "{label} should succeed with data: {}",
                describe(result)
            );
        }
        _ => panic!("unclassified scenario {label}"),
    }
    if case.tool == "read" && case.scenario == "truncation" {
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["lines"][0]["line_truncated"], true, "{data}");
    }
    if case.tool == "glob" && case.scenario == "truncation" {
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["page"]["has_more"], true, "{data}");
        assert!(
            data["page"]["total"].as_u64().unwrap()
                > data["paths"].as_array().unwrap().len() as u64
        );
    }
    if case.tool == "grep" && case.scenario == "truncation" {
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["page"]["has_more"], true, "{data}");
    }
}

fn assert_kind(result: &ToolCallResult, kind: ToolErrorKind, label: &str) {
    let got = result.error.as_ref().map(|error| error.kind);
    assert_eq!(got, Some(kind), "{label}: {}", describe(result));
}

fn describe(result: &ToolCallResult) -> String {
    format!(
        "error={:?} data={}",
        result
            .error
            .as_ref()
            .map(|error| (&error.kind, &error.message)),
        result
            .data
            .as_ref()
            .map(|data| data.to_string())
            .unwrap_or_else(|| "null".into())
    )
}

fn observed(case: &Live, result: &ToolCallResult, root: &Path) -> Value {
    let mut data = normalize_value(&result.data.clone().unwrap_or(Value::Null), root);
    if case.tool == "grep"
        && let Some(next) = data.get_mut("page").and_then(|page| page.get_mut("next"))
        && next.is_string()
    {
        *next = json!("<cursor>");
    }
    if case.tool == "glob" && case.scenario == "truncation" {
        if let Some(paths) = data.get_mut("paths").and_then(Value::as_array_mut) {
            paths.sort_by(|left, right| {
                left.as_str()
                    .unwrap_or("")
                    .cmp(right.as_str().unwrap_or(""))
            });
        }
    }
    let mut value = json!({
        "tool": case.tool,
        "scenario": case.scenario,
        "covered": true,
        "gap": null,
        "data": data,
        "error": normalize_value(
            &match &result.error {
                Some(error) => serde_json::to_value(error).expect("serialize error"),
                None => Value::Null,
            },
            root,
        ),
        "content_historical": normalize_text(&result.content, root),
    });
    if case.tool == "glob" && case.scenario == "truncation" {
        value["snapshot_note"] = json!(
            "paths are sorted only in this snapshot; walk order is not pinned and the tool is unchanged"
        );
    }
    value
}

fn normalize_value(value: &Value, root: &Path) -> Value {
    match value {
        Value::String(text) => Value::String(normalize_text(text, root)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| normalize_value(item, root))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                out.insert(key.clone(), normalize_value(child, root));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn normalize_text(text: &str, root: &Path) -> String {
    let mut prefixes = Vec::new();
    push_prefix(&mut prefixes, &root.to_string_lossy());
    let lap = crate::config::path::canon_abs_lossy(root);
    push_prefix(&mut prefixes, &lap.to_string_lossy());
    prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
    prefixes.dedup();
    let mut out = text.to_string();
    for prefix in prefixes {
        if prefix.is_empty() {
            continue;
        }
        let mut next = String::new();
        let mut rest = out.as_str();
        while let Some(index) = rest.find(&prefix) {
            next.push_str(&rest[..index]);
            next.push_str("<ws>");
            rest = &rest[index + prefix.len()..];
            if rest.starts_with('\\') || rest.starts_with('/') {
                rest = &rest[1..];
            }
        }
        next.push_str(rest);
        out = next;
    }
    out.replace('\\', "/")
}

fn push_prefix(prefixes: &mut Vec<String>, raw: &str) {
    let trimmed = raw.trim_end_matches(['\\', '/']);
    if trimmed.is_empty() {
        return;
    }
    prefixes.push(trimmed.to_string());
    prefixes.push(trimmed.replace('\\', "/"));
}

fn load_fixture(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|error| {
        panic!("missing fixture {}: {error}", path.display());
    });
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!("fixture {} is not json: {error}", path.display());
    })
}

#[tokio::test]
async fn phase0_tool_fixtures_lock_data_and_error() {
    let (live, gaps) = catalog();
    let mut covered = std::collections::BTreeSet::new();
    for case in &live {
        covered.insert((case.tool, case.scenario));
        let workspace = tempfile::tempdir().unwrap();
        let data_root = tempfile::tempdir().unwrap();
        prepare(workspace.path(), case);
        let result = run_case(case, workspace.path(), data_root.path()).await;
        expect_shape(case, &result);
        let actual = observed(case, &result, workspace.path());
        let path = fixture_path(case.tool, case.scenario);
        let expected = load_fixture(&path);
        assert_eq!(expected["tool"], case.tool);
        assert_eq!(expected["scenario"], case.scenario);
        assert_eq!(expected["covered"], true);
        assert!(expected["content_historical"].is_string());
        assert_eq!(
            expected["data"], actual["data"],
            "{}/{} data drifted",
            case.tool, case.scenario
        );
        assert_eq!(
            expected["error"], actual["error"],
            "{}/{} error drifted",
            case.tool, case.scenario
        );
        let _historical = &expected["content_historical"];
    }
    for (tool, scenario) in &gaps {
        assert!(
            covered.insert((tool, scenario)),
            "duplicate {tool}/{scenario}"
        );
    }
    for tool in ["read", "glob", "grep", "edit", "write"] {
        for scenario in SCENARIOS {
            assert!(
                covered.contains(&(tool, scenario)),
                "missing {tool}/{scenario}"
            );
        }
    }
}

#[test]
fn phase0_uncovered_scenarios_are_recorded_as_gaps() {
    let (_live, gaps) = catalog();
    assert_eq!(gaps.len(), 6);
    for (tool, scenario) in gaps {
        let value = load_fixture(&fixture_path(tool, scenario));
        assert_eq!(value["tool"], tool);
        assert_eq!(value["scenario"], scenario);
        assert_eq!(value["covered"], false, "{tool}/{scenario} must stay a gap");
        let gap = value["gap"].as_str().unwrap_or("");
        assert!(
            gap.chars().count() > 20,
            "{tool}/{scenario} gap reason missing: {gap}"
        );
        assert!(value["data"].is_null());
        assert!(value["error"].is_null());
        assert!(value.get("content_historical").is_some());
    }
}
