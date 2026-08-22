//! Permission cutover: authorize pipeline + evaluate/floor/grant/subagent coverage.

mod common;

use std::collections::HashMap;

use common::bindings::{binding_all_for, binding_safe_for};
use common::permission::recording_sink;
use litecode::config::resolved::{WorkspaceState, resolve};
use litecode::config::schema::{AgentProfile, AgentRole, GlobalSettings};
use litecode::config::WorkspacePaths;
use litecode::context_pipeline::Context;
use litecode::permission::{
    BindingPathMode, DEFAULT_RULE_ID, PermissionAction, PermissionEngine, clear_runtime_grants,
    clear_runtime_grants_for, grant_runtime,
};
use litecode::tool::authorize::{AuthResult, authorize};
use litecode::tools::write::WriteTool;
use litecode::types::FunctionToolCall;
use tempfile::TempDir;

fn tool_call(name: &str, args: serde_json::Value) -> FunctionToolCall {
    FunctionToolCall {
        arguments: args.to_string(),
        call_id: "call_1".into(),
        name: name.into(),
        namespace: None,
        id: Some("fc_1".into()),
        status: None,
    }
}

fn ctx_for(dir: &TempDir) -> Context {
    Context {
        cwd: dir.path().to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(dir.path()),
        agents_md: None,
        claude_md: None,
    }
}

fn engine_for(
    agent_id: &str,
    role: AgentRole,
    depth: u32,
    tools: HashMap<String, litecode::config::schema::AgentToolBinding>,
) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    global.agents.insert(
        agent_id.into(),
        AgentProfile {
            role,
            model_ref: "default".into(),
            tools,
            ..Default::default()
        },
    );
    let resolved = resolve(global, WorkspaceState::new("/tmp/test"));
    PermissionEngine::resolver(resolved, agent_id, depth)
}

fn safe_tools(ids: &[&str]) -> HashMap<String, litecode::config::schema::AgentToolBinding> {
    ids.iter()
        .map(|id| ((*id).to_string(), binding_safe_for(id)))
        .collect()
}

fn all_tools(ids: &[&str]) -> HashMap<String, litecode::config::schema::AgentToolBinding> {
    ids.iter()
        .map(|id| ((*id).to_string(), binding_all_for(id)))
        .collect()
}

#[test]
fn safe_bash_matrix_readonly_allow_others_deny_floor_hard_deny() {
    let dir = TempDir::new().unwrap();
    let engine = engine_for("default", AgentRole::Primary, 0, safe_tools(&["bash"]));

    let ls = engine.evaluate_tool("bash", &serde_json::json!({"command": "ls"}), dir.path());
    assert_eq!(ls.action, PermissionAction::Allow);
    assert_eq!(ls.rule_id, "readonly_command");

    let rm = engine.evaluate_tool(
        "bash",
        &serde_json::json!({"command": "rm -rf ./node_modules"}),
        dir.path(),
    );
    assert_eq!(rm.action, PermissionAction::Deny);
    assert_eq!(rm.rule_id, DEFAULT_RULE_ID);

    let root = engine.evaluate_tool(
        "bash",
        &serde_json::json!({"command": "rm -rf /"}),
        dir.path(),
    );
    assert_eq!(root.action, PermissionAction::Deny);
    assert_eq!(root.rule_id, "floor_dangerous_command");
}

#[test]
fn floor_sensitive_write_blocks_even_under_all_and_always_grant() {
    clear_runtime_grants();
    let dir = TempDir::new().unwrap();
    let engine = engine_for("default", AgentRole::Primary, 0, all_tools(&["write"]));

    grant_runtime(
        "default",
        "write",
        "floor_sensitive_write",
        PermissionAction::Allow,
    );

    let eval = engine.evaluate_tool(
        "write",
        &serde_json::json!({"file_path": "/etc/passwd", "content": "x"}),
        dir.path(),
    );
    assert_eq!(eval.action, PermissionAction::Deny);
    assert_eq!(eval.rule_id, "floor_sensitive_write");
    clear_runtime_grants();
}

#[test]
fn safe_glob_outside_workspace_denies_via_path_arg() {
    let dir = TempDir::new().unwrap();
    let engine = engine_for("default", AgentRole::Primary, 0, safe_tools(&["glob"]));
    let eval = engine.evaluate_tool(
        "glob",
        &serde_json::json!({"pattern": "*.rs", "path": "/etc"}),
        dir.path(),
    );
    assert_eq!(eval.action, PermissionAction::Deny);
    assert_eq!(eval.rule_id, "outside_workspace");
    assert_eq!(engine.path_mode("glob"), BindingPathMode::WorkspaceOnly);
}

#[test]
fn floor_blocks_sensitive_commands_under_unrestricted_all_preset() {
    clear_runtime_grants();
    let dir = TempDir::new().unwrap();
    // All preset → Unrestricted path mode (presets.rs ALL→Unrestricted). The floor
    // (G4) must still hard-deny sensitive commands unconditionally — a user
    // configurable preset must never weaken it.
    let engine = engine_for("default", AgentRole::Primary, 0, all_tools(&["bash"]));
    assert_eq!(engine.path_mode("bash"), BindingPathMode::Unrestricted);

    let eval = engine.evaluate_tool(
        "bash",
        &serde_json::json!({"command": "rm -rf /"}),
        dir.path(),
    );
    assert_eq!(eval.action, PermissionAction::Deny);
    assert_eq!(eval.rule_id, "floor_dangerous_command");

    // Preset switch: even an explicit runtime grant cannot lift the floor.
    grant_runtime(
        "default",
        "bash",
        "floor_preset_switch",
        PermissionAction::Allow,
    );
    let eval2 = engine.evaluate_tool(
        "bash",
        &serde_json::json!({"command": "rm -rf /"}),
        dir.path(),
    );
    assert_eq!(eval2.action, PermissionAction::Deny);
    assert_eq!(eval2.rule_id, "floor_dangerous_command");

    // Sensitive system write under All preset is also unconditionally denied.
    let engine_w = engine_for("default", AgentRole::Primary, 0, all_tools(&["write"]));
    assert_eq!(engine_w.path_mode("write"), BindingPathMode::Unrestricted);
    let eval_w = engine_w.evaluate_tool(
        "write",
        &serde_json::json!({"file_path": "/etc/passwd", "content": "x"}),
        dir.path(),
    );
    assert_eq!(eval_w.action, PermissionAction::Deny);
    assert_eq!(eval_w.rule_id, "floor_sensitive_write");
    clear_runtime_grants();
}

#[test]
fn subagent_ask_becomes_deny() {
    let dir = TempDir::new().unwrap();
    let engine = engine_for("reviewer", AgentRole::Subagent, 1, safe_tools(&["write"]));
    let eval = engine.evaluate_tool(
        "write",
        &serde_json::json!({"file_path": "a.txt", "content": "x"}),
        dir.path(),
    );
    assert_eq!(eval.action, PermissionAction::Deny);
}

#[test]
fn none_tools_always_allow() {
    let dir = TempDir::new().unwrap();
    let engine = engine_for("default", AgentRole::Primary, 0, HashMap::new());
    let eval = engine.evaluate_tool("plan", &serde_json::json!({}), dir.path());
    assert_eq!(eval.action, PermissionAction::Allow);
}

#[test]
fn grant_is_rule_and_agent_scoped() {
    clear_runtime_grants();
    grant_runtime("a", "write", DEFAULT_RULE_ID, PermissionAction::Allow);
    assert!(litecode::permission::check_runtime_grant("a", "write", DEFAULT_RULE_ID).is_some());
    assert!(litecode::permission::check_runtime_grant("b", "write", DEFAULT_RULE_ID).is_none());
    assert!(litecode::permission::check_runtime_grant("a", "write", "other").is_none());
    clear_runtime_grants_for("a");
    assert!(litecode::permission::check_runtime_grant("a", "write", DEFAULT_RULE_ID).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn authorize_ask_denied_by_sink() {
    clear_runtime_grants_for("ask_deny");
    let dir = TempDir::new().unwrap();
    let ctx = ctx_for(&dir);
    let engine = engine_for("ask_deny", AgentRole::Primary, 0, safe_tools(&["write"]));
    let sink = recording_sink((false, false));
    let inv = tool_call(
        "write",
        serde_json::json!({"file_path": "out.txt", "content": "x"}),
    );
    let result = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "ask_deny",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, AuthResult::Denied(_)));
    assert_eq!(sink.calls.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn authorize_ask_always_grants_same_rule_only() {
    clear_runtime_grants_for("ask_always");
    let dir = TempDir::new().unwrap();
    let ctx = ctx_for(&dir);
    let engine = engine_for("ask_always", AgentRole::Primary, 0, safe_tools(&["write"]));
    let sink = recording_sink((true, true));
    let inv = tool_call(
        "write",
        serde_json::json!({"file_path": "out.txt", "content": "x"}),
    );
    let result = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "ask_always",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, AuthResult::Proceed { .. }));
    assert!(
        litecode::permission::check_runtime_grant("ask_always", "write", DEFAULT_RULE_ID).is_some()
    );

    let sink2 = recording_sink((false, false));
    let result2 = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "ask_always",
        &sink2,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(result2, AuthResult::Proceed { .. }));
    assert!(sink2.calls.lock().unwrap().is_empty());
    clear_runtime_grants_for("ask_always");
}

#[tokio::test(flavor = "current_thread")]
async fn authorize_floor_deny_ignores_always_grant() {
    clear_runtime_grants_for("floor_agent");
    let dir = TempDir::new().unwrap();
    let ctx = ctx_for(&dir);
    let engine = engine_for("floor_agent", AgentRole::Primary, 0, all_tools(&["write"]));
    grant_runtime(
        "floor_agent",
        "write",
        "floor_sensitive_write",
        PermissionAction::Allow,
    );
    let sink = recording_sink((true, true));
    let inv = tool_call(
        "write",
        serde_json::json!({"file_path": "/etc/passwd", "content": "x"}),
    );
    let result = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "floor_agent",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, AuthResult::Denied(_)));
    assert!(sink.calls.lock().unwrap().is_empty());
    clear_runtime_grants_for("floor_agent");
}

#[tokio::test(flavor = "current_thread")]
async fn authorize_subagent_ask_denied_without_sink() {
    clear_runtime_grants_for("reviewer");
    let dir = TempDir::new().unwrap();
    let ctx = ctx_for(&dir);
    let engine = engine_for("reviewer", AgentRole::Subagent, 1, safe_tools(&["write"]));
    let sink = recording_sink((true, true));
    let inv = tool_call(
        "write",
        serde_json::json!({"file_path": "out.txt", "content": "x"}),
    );
    let result = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "reviewer",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(result, AuthResult::Denied(_)));
    assert!(sink.calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn authorize_ask_aborted_is_not_deny() {
    clear_runtime_grants_for("ask_abort");
    let dir = TempDir::new().unwrap();
    let ctx = ctx_for(&dir);
    let engine = engine_for("ask_abort", AgentRole::Primary, 0, safe_tools(&["write"]));
    let sink = litecode::permission::RecordingPermissionSink::new(
        litecode::permission::AskOutcome::Aborted,
    );
    let inv = tool_call(
        "write",
        serde_json::json!({"file_path": "out.txt", "content": "x"}),
    );
    let result = authorize(
        &inv,
        &WriteTool::new(),
        &engine,
        &ctx,
        "ask_abort",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(
        matches!(result, AuthResult::Aborted),
        "cancel during Ask must abort, not permission-denied, got {result:?}"
    );
}
