//! Phase 1: per-tool surface, one projection, and one call pipeline.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
use litecode::context_pipeline::Context;
use litecode::permission::{
    PermissionAction, PermissionEngine, RecordingPermissionSink, ToolPolicy,
};
use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use litecode::tool::write_lock::process_write_lock;
use litecode::tool::{Tool, ToolSurface, project_tools};
use litecode::tools::edit::EditTool;
use litecode::tools::glob::GlobTool;
use litecode::tools::grep::GrepTool;
use litecode::tools::read::ReadTool;
use litecode::tools::write::WriteTool;
use litecode::types::{FunctionToolCall, ToolCallResult, ToolErrorKind};
use serde_json::{Value, json};

fn ctx(root: &std::path::Path) -> Context {
    Context {
        cwd: root.to_path_buf(),
        workspace_paths: WorkspacePaths::for_legacy_root(root),
        agents_md: None,
        claude_md: None,
    }
}

fn engine(root: &std::path::Path, tool: &str, policy: ToolPolicy) -> PermissionEngine {
    let mut global = GlobalSettings::default();
    global.agents.insert(
        "default".into(),
        litecode::config::schema::AgentProfile {
            tools: [(
                tool.to_string(),
                litecode::config::schema::AgentToolBinding {
                    enabled: true,
                    policy,
                    path_mode: litecode::permission::BindingPathMode::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            )]
            .into(),
            ..Default::default()
        },
    );
    let resolved = ConfigManager::resolve_without_catalog(global, WorkspaceState::new(root));
    PermissionEngine::resolver(resolved, "default", 0)
}

fn call(name: &str, args: Value) -> FunctionToolCall {
    FunctionToolCall {
        arguments: args.to_string(),
        call_id: "call-1".into(),
        namespace: None,
        name: name.into(),
        id: None,
        status: None,
    }
}

struct CountTool {
    name: &'static str,
    surface: ToolSurface,
    hits: Arc<AtomicUsize>,
    with_data: bool,
    schema: Value,
    data_schema: Option<Value>,
}

impl CountTool {
    fn code(name: &'static str, hits: Arc<AtomicUsize>) -> Self {
        Self {
            name,
            surface: ToolSurface::Code,
            hits,
            with_data: true,
            schema: json!({"type": "object", "properties": {}}),
            data_schema: Some(json!({
                "type": "object",
                "properties": {"n": {"type": "integer"}},
                "required": ["n"]
            })),
        }
    }
}

impl Tool for CountTool {
    fn name(&self) -> &str {
        self.name
    }

    fn surface(&self) -> ToolSurface {
        self.surface
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    fn data_schema(&self) -> Option<Value> {
        self.data_schema.clone()
    }

    fn description(&self, _ctx: &Context) -> String {
        format!("count tool {}", self.name)
    }

    fn resource_keys(
        &self,
        _input: &Value,
        _path_mode: litecode::workspace::ToolPathMode,
        _workspace_root: &std::path::Path,
    ) -> Vec<litecode::tool::ResourceKey> {
        vec![litecode::tool::ResourceKey::File("/noted/file.txt".into())]
    }

    fn execute(
        &self,
        _input: Value,
        _execution: litecode::tool::trait_::ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        let result = if self.with_data {
            ToolCallResult::ok("ran").with_data(json!({"n": 1}))
        } else {
            ToolCallResult::ok("ran")
        };
        Box::pin(std::future::ready(result))
    }
}

struct BrokenCode;

impl Tool for BrokenCode {
    fn name(&self) -> &str {
        "broken"
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {"item": {"$ref": "#/Item"}}})
    }

    fn data_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}))
    }

    fn description(&self, _ctx: &Context) -> String {
        "Broken code tool.".into()
    }
}

struct Colliding {
    name: &'static str,
}

impl Tool for Colliding {
    fn name(&self) -> &str {
        self.name
    }

    fn surface(&self) -> ToolSurface {
        ToolSurface::Code
    }

    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn data_schema(&self) -> Option<Value> {
        Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}}}))
    }

    fn description(&self, _ctx: &Context) -> String {
        format!("colliding {}", self.name)
    }
}

fn names(tools: &[Arc<dyn Tool>]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name()).collect()
}

#[test]
fn builtin_code_tools_pass_admission_and_edit_write_join_the_code_surface() {
    for tool in [
        Arc::new(ReadTool::default()) as Arc<dyn Tool>,
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(EditTool::new()),
        Arc::new(WriteTool::new()),
    ] {
        assert_eq!(tool.surface(), ToolSurface::Code, "{}", tool.name());
    }

    let projected = project_tools(vec![
        Arc::new(ReadTool::default()),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(EditTool::new()),
        Arc::new(WriteTool::new()),
    ]);
    assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
    assert_eq!(
        names(&projected.code_tools),
        ["edit", "glob", "grep", "read", "write"]
    );
    assert_eq!(
        projected.whitelist,
        vec!["edit", "glob", "grep", "read", "write"]
    );
    assert!(projected.direct.is_empty());
    assert!(projected.sdk.contains("[\"read\"]"));
    assert!(projected.sdk.contains("[\"edit\"]"));
    assert!(projected.sdk.contains("[\"write\"]"));
    assert!(projected.sdk.contains("path:"));
    assert!(!projected.sdk.contains("file_path"));
    assert!(projected.sdk.contains("type Integer = number;"));
    assert!(!projected.sdk.contains(": unknown"));
}

#[test]
fn code_surface_is_projected_without_an_env_gate() {
    let projected = project_tools(vec![Arc::new(ReadTool::default()), Arc::new(GlobTool)]);
    assert_eq!(names(&projected.code_tools), ["glob", "read"]);
    assert!(projected.direct.is_empty());
    assert!(projected.rejected.is_empty());
    assert!(!projected.sdk.is_empty());
}

#[test]
fn fake_code_tool_joins_the_same_projection_as_builtins() {
    let hits = Arc::new(AtomicUsize::new(0));
    let projected = project_tools(vec![
        Arc::new(ReadTool::default()),
        Arc::new(CountTool::code("marker", hits)),
        Arc::new(EditTool::new()),
    ]);
    assert!(projected.rejected.is_empty(), "{:?}", projected.rejected);
    assert_eq!(projected.whitelist, vec!["edit", "marker", "read"]);
    assert!(!names(&projected.direct).contains(&"edit"));
    assert!(!names(&projected.direct).contains(&"marker"));
    assert!(projected.sdk.contains("[\"marker\"]"));
    assert!(projected.sdk.contains("[\"read\"]"));
    assert!(projected.sdk.contains("[\"edit\"]"));
}

#[test]
fn missing_or_unrenderable_code_tool_leaves_both_surfaces() {
    let hits = Arc::new(AtomicUsize::new(0));
    let mut stripped = CountTool::code("stripped", hits);
    stripped.data_schema = None;
    let projected = project_tools(vec![
        Arc::new(stripped),
        Arc::new(BrokenCode),
        Arc::new(ReadTool::default()),
        Arc::new(Colliding { name: "foo-bar" }),
        Arc::new(Colliding { name: "foo_bar" }),
    ]);
    let rejected: Vec<&str> = projected
        .rejected
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert!(rejected.contains(&"stripped"), "{rejected:?}");
    assert!(rejected.contains(&"broken"), "{rejected:?}");
    assert!(rejected.contains(&"foo-bar"), "{rejected:?}");
    assert!(rejected.contains(&"foo_bar"), "{rejected:?}");
    assert!(
        projected
            .rejected
            .iter()
            .any(|item| item.name == "stripped" && item.reason.contains("data schema"))
    );
    assert!(
        projected
            .rejected
            .iter()
            .any(|item| item.name == "broken" && item.reason.contains("$ref"))
    );
    assert_eq!(projected.whitelist, vec!["read"]);
    assert!(
        !names(&projected.direct)
            .iter()
            .any(|name| { ["stripped", "broken", "foo-bar", "foo_bar", "read"].contains(name) })
    );
    assert!(!projected.sdk.contains("stripped"));
    assert!(!projected.sdk.contains("broken"));
    assert!(!projected.sdk.contains("foo-bar"));
    assert!(projected.sdk.contains("[\"read\"]"));
}

#[test]
fn run_code_description_is_a_pure_function_of_the_code_surface() {
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(GrepTool), Arc::new(ReadTool::default())];
    let left = project_tools(tools.clone());
    let right = project_tools(vec![Arc::new(ReadTool::default()), Arc::new(GrepTool)]);
    assert_eq!(left.sdk, right.sdk);
    let host_ctx = ctx(std::path::Path::new("/var/workspaces/one"));
    let other = ctx(std::path::Path::new("/tmp/other-depth"));
    let code = litecode::tools::code::CodeTool::from_surface(
        left.code_tools,
        left.sdk.clone(),
        left.whitelist,
        Arc::new(litecode::tools::code::CodeToolHost::new(
            engine(std::path::Path::new("."), "read", ToolPolicy::allow_all()),
            host_ctx.clone(),
            "default",
            litecode::permission::deny_permission_sink(),
            PathBuf::from("."),
            1024,
            process_write_lock(),
            None,
        )),
    );
    assert_eq!(code.description(&host_ctx), code.description(&other));
    assert_eq!(code.description(&other), left.sdk);
    assert!(!code.description(&other).contains("/var/workspaces/one"));
    assert!(!code.description(&other).contains("/tmp/other-depth"));
}

#[tokio::test(flavor = "current_thread")]
async fn direct_call_of_code_tool_is_routed_before_authorization() {
    let hits = Arc::new(AtomicUsize::new(0));
    let tool = Arc::new(CountTool::code("read", Arc::clone(&hits)));
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(
        dir.path(),
        "read",
        ToolPolicy::with_default(PermissionAction::Ask),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let context = ctx(dir.path());
    let leaked = execute_tool_call(
        &call("read", json!({})),
        &[tool],
        &permission,
        &context,
        "s",
        "default",
        &sink,
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
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
    .await;
    assert_eq!(
        leaked.error.as_ref().map(|error| error.kind),
        Some(ToolErrorKind::NotFound)
    );
    assert!(
        leaked
            .content
            .contains("`read` is only available as tools.read inside run_code"),
        "{}",
        leaked.content
    );
    assert!(sink.calls.lock().unwrap().is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    let hidden = execute_tool_call(
        &call("read", json!({})),
        &[],
        &permission,
        &context,
        "s",
        "default",
        &sink,
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Direct,
            code_names: Arc::from(["read".to_string()]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;
    assert!(hidden.content.contains("tools.read"));
    assert!(sink.calls.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn code_call_of_invisible_tool_is_not_found_and_does_not_run() {
    let hits = Arc::new(AtomicUsize::new(0));
    let hidden = Arc::new(CountTool::code("hidden", Arc::clone(&hits)));
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(
        dir.path(),
        "hidden",
        ToolPolicy::with_default(PermissionAction::Ask),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let result = execute_tool_call(
        &call("hidden", json!({})),
        &[],
        &permission,
        &ctx(dir.path()),
        "s",
        "default",
        &sink,
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "outer".into(),
            },
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;
    assert!(
        result.content.contains("tool 'hidden' not found"),
        "{}",
        result.content
    );
    assert!(sink.calls.lock().unwrap().is_empty());
    assert_eq!(hidden.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn code_call_keeps_the_tools_own_permission_and_does_not_execute_a_deny() {
    let hits = Arc::new(AtomicUsize::new(0));
    let tool = Arc::new(CountTool::code("marker", Arc::clone(&hits)));
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(
        dir.path(),
        "marker",
        ToolPolicy::with_default(PermissionAction::Deny),
    );
    let sink = RecordingPermissionSink::from_reply(true, false);
    let result = execute_tool_call(
        &call("marker", json!({})),
        &[tool],
        &permission,
        &ctx(dir.path()),
        "s",
        "default",
        &sink,
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "outer-1".into(),
            },
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;
    assert_eq!(
        result.error.as_ref().map(|error| error.kind),
        Some(ToolErrorKind::PermissionDenied)
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn code_success_without_matching_data_is_an_internal_error() {
    let hits = Arc::new(AtomicUsize::new(0));
    let mut tool = CountTool::code("marker", hits);
    tool.with_data = false;
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(dir.path(), "marker", ToolPolicy::allow_all());
    let result = execute_tool_call(
        &call("marker", json!({})),
        &[Arc::new(tool)],
        &permission,
        &ctx(dir.path()),
        "s",
        "default",
        &RecordingPermissionSink::from_reply(true, false),
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "outer".into(),
            },
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;
    assert_eq!(
        result.error.as_ref().map(|error| error.kind),
        Some(ToolErrorKind::Internal)
    );
    assert!(
        result.content.contains("without data"),
        "{}",
        result.content
    );
}

#[tokio::test(flavor = "current_thread")]
async fn nested_and_direct_calls_note_the_same_paths() {
    let notes = Arc::new(Mutex::new(Vec::<Vec<PathBuf>>::new()));
    let hook = {
        let notes = Arc::clone(&notes);
        Arc::new(move |paths: &[PathBuf]| {
            notes.lock().unwrap().push(paths.to_vec());
        }) as Arc<dyn Fn(&[PathBuf]) + Send + Sync>
    };
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(dir.path(), "touch", ToolPolicy::allow_all());
    let context = ctx(dir.path());

    let direct_hits = Arc::new(AtomicUsize::new(0));
    let mut direct_tool = CountTool::code("touch", direct_hits);
    direct_tool.surface = ToolSurface::Direct;
    execute_tool_call(
        &call("touch", json!({})),
        &[Arc::new(direct_tool)],
        &permission,
        &context,
        "s",
        "default",
        &RecordingPermissionSink::from_reply(true, false),
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Direct,
            code_names: Arc::from([]),
            file_note: Some(Arc::clone(&hook)),
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;

    let nested_hits = Arc::new(AtomicUsize::new(0));
    let nested = CountTool::code("touch", nested_hits);
    execute_tool_call(
        &call("touch", json!({})),
        &[Arc::new(nested)],
        &permission,
        &context,
        "s",
        "default",
        &RecordingPermissionSink::from_reply(true, false),
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "outer".into(),
            },
            code_names: Arc::from([]),
            file_note: Some(hook),
            permission_sink: None,
            plan_touch: None,
        },
    )
    .await;

    let noted = notes.lock().unwrap().clone();
    assert_eq!(noted.len(), 2);
    assert_eq!(noted[0], noted[1]);
    assert_eq!(noted[0], vec![PathBuf::from("/noted/file.txt")]);
}

#[tokio::test(flavor = "current_thread")]
async fn code_transport_is_allowed_by_identity_not_by_name() {
    use litecode::config::schema::{
        AgentProfile, AgentToolBinding, CustomToolDefinition, ToolPreset, ToolSchema,
    };
    use litecode::permission::{ArgMatcher, PermissionAction, PolicyRule};
    use litecode::tool::{AuthResult, authorize};

    let dir = tempfile::tempdir().unwrap();
    let mut global = GlobalSettings::default();
    let mut profile = AgentProfile::default();
    profile.tools.insert(
        "run_code".into(),
        AgentToolBinding {
            enabled: true,
            policy: ToolPolicy::allow_all(),
            path_mode: litecode::permission::BindingPathMode::Unrestricted,
            last_applied_preset: Some(ToolPreset::Safe),
            allowed_tools: None,
        },
    );
    global.agents.insert("default".into(), profile);
    global.custom_tools.push(CustomToolDefinition {
        name: "run_code".into(),
        description: String::new(),
        schema: ToolSchema {
            schema_type: "object".into(),
            properties: json!({}),
            required: Vec::new(),
        },
        command: "echo".into(),
        args: Vec::new(),
        timeout: 120,
        rules: vec![PolicyRule {
            id: "block".into(),
            when: ArgMatcher::Any,
            action: PermissionAction::Deny,
        }],
        suite: None,
        suite_label: None,
    });
    let resolved = ConfigManager::resolve_without_catalog(global, WorkspaceState::new(dir.path()));
    let permission = PermissionEngine::resolver(resolved, "default", 0);
    let denied = permission.evaluate_tool("run_code", &json!({"code": "return 1"}), dir.path());
    assert_eq!(denied.action, PermissionAction::Deny);

    let context = ctx(dir.path());
    let sink = RecordingPermissionSink::from_reply(false, false);
    let host = Arc::new(litecode::tools::code::CodeToolHost::new(
        permission.clone(),
        context.clone(),
        "default",
        litecode::permission::deny_permission_sink(),
        dir.path().to_path_buf(),
        1024,
        process_write_lock(),
        None,
    ));
    let transport =
        litecode::tools::code::CodeTool::from_surface(Vec::new(), "sdk".into(), Vec::new(), host);
    assert!(transport.is_code_transport());
    let allowed = authorize(
        &call("run_code", json!({"code": "return 1"})),
        &transport,
        &permission,
        &context,
        "default",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    assert!(matches!(allowed, AuthResult::Proceed { .. }));

    let mut impostor = CountTool::code("run_code", Arc::new(AtomicUsize::new(0)));
    impostor.surface = ToolSurface::Direct;
    let denied_call = authorize(
        &call("run_code", json!({})),
        &impostor,
        &permission,
        &context,
        "default",
        &sink,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await;
    match denied_call {
        AuthResult::Denied(message) => assert!(message.contains("permission"), "{message}"),
        other => panic!("same name without the transport identity must be denied, got {other:?}"),
    }
}

#[test]
fn code_admission_rejections_are_in_the_catalog_payload() {
    use litecode::config::schema::GlobalSettings;
    use litecode::tool::availability::available_tools_payload;

    let resolved = ConfigManager::resolve_without_catalog(
        GlobalSettings::default(),
        WorkspaceState::new("/tmp"),
    );
    let payload = available_tools_payload(
        &resolved,
        vec![
            Arc::new(BrokenCode) as Arc<dyn Tool>,
            Arc::new(ReadTool::default()),
        ],
    );
    let broken = payload
        .code_rejections
        .iter()
        .find(|item| item.name == "broken")
        .expect("broken tool is reported");
    assert!(broken.reason.contains("$ref"), "{}", broken.reason);
    assert!(
        payload
            .code_rejections
            .iter()
            .all(|item| item.name != "read")
    );
    let body = serde_json::to_value(&payload).unwrap();
    assert!(
        body["code_rejections"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("$ref")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn direct_and_nested_calls_share_plan_touch() {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let hook = {
        let seen = Arc::clone(&seen);
        Arc::new(move |call: &litecode::types::FunctionToolCall| {
            seen.lock().unwrap().push(call.name.clone());
        }) as litecode::tool::executor::PlanTouch
    };
    let dir = tempfile::tempdir().unwrap();
    let permission = engine(dir.path(), "touch", ToolPolicy::allow_all());
    let context = ctx(dir.path());

    let mut direct_tool = CountTool::code("touch", Arc::new(AtomicUsize::new(0)));
    direct_tool.surface = ToolSurface::Direct;
    execute_tool_call(
        &call("touch", json!({})),
        &[Arc::new(direct_tool)],
        &permission,
        &context,
        "s",
        "default",
        &RecordingPermissionSink::from_reply(true, false),
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Direct,
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: Some(Arc::clone(&hook)),
        },
    )
    .await;

    execute_tool_call(
        &call("touch", json!({})),
        &[Arc::new(CountTool::code(
            "touch",
            Arc::new(AtomicUsize::new(0)),
        ))],
        &permission,
        &context,
        "s",
        "default",
        &RecordingPermissionSink::from_reply(true, false),
        tokio_util::sync::CancellationToken::new(),
        dir.path(),
        4096,
        None,
        process_write_lock(),
        None,
        ToolCallOrigin {
            caller: ToolCaller::Code {
                parent_call_id: "outer".into(),
            },
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: Some(hook),
        },
    )
    .await;

    assert_eq!(seen.lock().unwrap().as_slice(), ["touch", "touch"]);
}

#[tokio::test(flavor = "current_thread")]
async fn pipeline_direct_call_uses_the_shared_file_note() {
    let notes = Arc::new(Mutex::new(Vec::<Vec<PathBuf>>::new()));
    let hook = {
        let notes = Arc::clone(&notes);
        Arc::new(move |paths: &[PathBuf]| {
            notes.lock().unwrap().push(paths.to_vec());
        }) as Arc<dyn Fn(&[PathBuf]) + Send + Sync>
    };
    let dir = tempfile::tempdir().unwrap();
    let mut tool = CountTool::code("touch", Arc::new(AtomicUsize::new(0)));
    tool.surface = ToolSurface::Direct;
    let permission = engine(dir.path(), "touch", ToolPolicy::allow_all());
    let context = ctx(dir.path());
    let sink: Arc<dyn litecode::permission::PermissionSink> =
        Arc::new(RecordingPermissionSink::from_reply(true, false));
    let runtime = Arc::new(litecode::runtime::RuntimeContext::new(
        vec![Arc::new(tool)],
        permission,
        context,
        "default",
        Arc::clone(&sink),
        tokio_util::sync::CancellationToken::new(),
        dir.path().to_path_buf(),
        4096,
        process_write_lock(),
        None,
    ));
    let mut pipeline = litecode::tool::ToolPipeline::new(runtime);
    pipeline.bind_session("s");
    pipeline.set_file_note(hook);
    let mut transcript = Vec::new();
    pipeline
        .execute_batch(&[call("touch", json!({}))], &mut transcript)
        .await
        .unwrap();
    let noted = notes.lock().unwrap().clone();
    assert_eq!(noted, vec![vec![PathBuf::from("/noted/file.txt")]]);
    assert_eq!(transcript.len(), 1);
}
