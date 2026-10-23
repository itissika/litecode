//! Code surface integration tests (merged from code_surface_phase1..5, WP-207).

/// Per-tool surface, one projection, one call pipeline.
mod projection {

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
    use litecode::context_pipeline::Context;
    use litecode::permission::{
        PermissionAction, PermissionEngine, RecordingPermissionSink, ToolPolicy,
    };
    use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
    use litecode::tool::write_lock::process_write_lock;
    use litecode::tool::{Tool, ToolSurface, project_tools};

    use litecode::tools::read::ReadTool;

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
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
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
            !names(&projected.direct).iter().any(|name| {
                ["stripped", "broken", "foo-bar", "foo_bar", "read"].contains(name)
            })
        );
        assert!(!projected.sdk.contains("stripped"));
        assert!(!projected.sdk.contains("broken"));
        assert!(!projected.sdk.contains("foo-bar"));
        assert!(projected.sdk.contains("[\"read\"]"));
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
                deadline: None,
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
                deadline: None,
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
                deadline: None,
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
            surface: Default::default(),
            suite: None,
            suite_label: None,
        });
        let resolved =
            ConfigManager::resolve_without_catalog(global, WorkspaceState::new(dir.path()));
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
            process_write_lock(),
            None,
        ));
        let transport = litecode::tools::code::CodeTool::from_surface(
            Vec::new(),
            "sdk".into(),
            Vec::new(),
            host,
        );
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
            other => {
                panic!("same name without the transport identity must be denied, got {other:?}")
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn custom_tool_on_code_surface_uses_the_shared_pipeline_and_opaque_text() {
        use litecode::config::schema::{CallSurface, CustomToolDefinition, ToolSchema};
        use litecode::tools::custom::CustomTool;
        use litecode::tools::custom_hub::CustomToolHub;

        let marker = "custom-code-marker";
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C".into(), format!("echo {marker}")]);
        #[cfg(unix)]
        let (command, args) = ("sh", vec!["-c".into(), format!("printf '%s' '{marker}'")]);
        let def = CustomToolDefinition {
            name: "custom_code".into(),
            description: "custom code probe".into(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: json!({}),
                required: Vec::new(),
            },
            command: command.into(),
            args,
            timeout: 10,
            rules: Vec::new(),
            surface: CallSurface::Code,
            suite: None,
            suite_label: None,
        };
        let tool: Arc<dyn Tool> = Arc::new(CustomTool::new(def, Arc::new(CustomToolHub::new())));
        let dir = tempfile::tempdir().unwrap();
        let permission = engine(dir.path(), "custom_code", ToolPolicy::allow_all());
        let sink = RecordingPermissionSink::from_reply(true, false);
        let result = execute_tool_call(
            &call("custom_code", json!({})),
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
                    parent_call_id: "outer".into(),
                },
                code_names: Arc::from([]),
                file_note: None,
                permission_sink: None,
                plan_touch: None,
                deadline: None,
            },
        )
        .await;
        assert!(result.error.is_none(), "{}", result.content);
        let text = result
            .data
            .as_ref()
            .and_then(|data| data["text"].as_str())
            .expect("opaque text data");
        assert!(text.contains(marker), "{text}");
    }
}

/// `run_code` is the only way to call the code surface; cancel/timeout/approval budget.
mod run_code_only {
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
        for name in [
            "read",
            "glob",
            "grep",
            "block",
            "fast",
            "hold",
            "late",
            "custom_code",
        ] {
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
            deadline: None,
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
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
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
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
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
      read: read.path && Array.isArray(read.lines) ? "ok" : "missing",
      glob: Array.isArray(glob.paths) ? "ok" : "missing",
      grep: Array.isArray(grep.hits) ? "ok" : "missing"
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
            result.content.contains("tool calls: 3 ok / 0 failed"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert!(
            !result.content.contains("diagnostics:"),
            "{}",
            result.content
        );
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
        assert_eq!(
            result.error.as_ref().map(|error| error.kind),
            Some(ToolErrorKind::Cancelled),
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
        assert!(
            result.content.contains("1 calls were still running"),
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
                "code": "const read = await tools.read({ path: \"probe.txt\" }); return read.path && read.lines ? \"ok\" : \"missing\";",
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
        // The completed fast call stays counted as a success; only hold is cut off.
        assert!(
            result.content.contains("tool calls: 1 ok"),
            "{}",
            result.content
        );
        assert_eq!(
            result.error.as_ref().map(|error| error.kind),
            Some(ToolErrorKind::Timeout)
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn custom_tool_code_surface_round_trips_through_run_code_sdk() {
        use litecode::config::schema::{
            CallSurface, CustomToolDefinition, ToolSchema,
        };
        use litecode::tools::custom::CustomTool;
        use litecode::tools::custom_hub::CustomToolHub;

        let marker = "custom-sdk-marker";
        #[cfg(windows)]
        let (command, args) = ("cmd", vec!["/C".into(), format!("echo {marker}")]);
        #[cfg(unix)]
        let (command, args) = ("sh", vec!["-c".into(), format!("printf '%s' '{marker}'")]);
        let def = CustomToolDefinition {
            name: "custom_code".into(),
            description: "custom code sdk probe".into(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: json!({}),
                required: Vec::new(),
            },
            command: command.into(),
            args,
            timeout: 10,
            rules: Vec::new(),
            surface: CallSurface::Code,
            suite: None,
            suite_label: None,
        };
        let custom = Arc::new(CustomTool::new(def, Arc::new(CustomToolHub::new())));
        let root = tempfile::tempdir().unwrap();
        let sink = litecode::permission::deny_permission_sink();
        let hosted = hosted(root.path(), vec![custom], PermissionAction::Allow, sink.clone());
        assert!(hosted.code_names.iter().any(|name| name == "custom_code"));

        let result = execute(
            &hosted,
            "run_code",
            json!({ "code": "const r = await tools.custom_code({}); return r;" }),
            sink.as_ref(),
            CancellationToken::new(),
        )
        .await;
        assert!(result.error.is_none(), "{}", result.content);
        let value = run_code_result(&result.content);
        assert_eq!(value["ok"], true);
        let text = value["text"].as_str().expect("text field");
        assert!(text.contains(marker), "{text}");
    }
}

/// `edit` and `write` on the code surface: approval, sealing, concurrent writes.
mod edit_write {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use litecode::config::schema::{AgentProfile, AgentToolBinding};
    use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
    use litecode::context_pipeline::Context;
    use litecode::permission::{
        AskOutcome, AskPrompt, AskReply, BindingPathMode, PermissionAction, PermissionEngine,
        PermissionSink, RecordingPermissionSink, ToolPolicy,
    };

    use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};

    use litecode::tool::Tool;
    use litecode::tool::write_lock::process_write_lock;
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
                deadline: None,
            },
        )
        .await
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
    async fn nested_write_approval_denial_does_not_apply_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        let sink = RecordingPermissionSink::from_reply(false, false);
        let hosted = hosted(dir.path(), PermissionAction::Ask, Arc::new(sink));
        let outer = RecordingPermissionSink::from_reply(true, false);
        let script = r#"
    let status = "ok";
    try {
      const written = await tools.write({ path: "nope.txt", content: "secret-nope" });
      if (written.ok === false) status = "error";
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
            result.content.contains("tool calls: 0 ok / 1 failed"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert!(!result.content.contains("#1 "), "{}", result.content);
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
      const written = await tools.write({{ path: {target:?}, content: "no" }});
      if (written.ok === false) status = "error";
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
        assert!(
            result.content.contains("tool calls: 0 ok / 1 failed"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
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
      old_string: "x",
      new_string: "y",
      replace_all: true
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
        // Behaviour, not ledger text: the run either finished or was cancelled, and the file is sealed.
        assert!(
            result.error.is_none()
                || result.error.as_ref().map(|error| error.kind) == Some(ToolErrorKind::Cancelled),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        if path.exists() {
            let body = std::fs::read(&path).unwrap();
            assert!(
                body.iter().all(|byte| *byte == b'x') || body.iter().all(|byte| *byte == b'y'),
                "an in-flight edit must not leave a torn file (len {})",
                body.len()
            );
            assert_eq!(body.len(), 80_000);
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
        let permission =
            engine_with_mode(root, PermissionAction::Ask, BindingPathMode::Unrestricted);
        let ctx = ctx(root);
        let data_root = root.join(".litecode");
        std::fs::create_dir_all(&data_root).unwrap();
        let host = CodeToolHost::new(
            permission.clone(),
            ctx.clone(),
            "default",
            sink,
            data_root.clone(),
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
      // WP-211: create_only conflict resolves { ok: false } instead of rejecting.
      return {
        name,
        statuses: results.map(result => {
          if (result.status !== "fulfilled") return "error";
          if (result.value && result.value.ok === false) return "error";
          return "ok";
        }),
        messages: results.map(result => {
          if (result.status === "rejected") return (result.reason && result.reason.message) || "";
          if (result.value && result.value.ok === false) return result.value.message || "";
          return "";
        }),
        bytes: results.map(result => {
          if (result.status !== "fulfilled" || (result.value && result.value.ok === false)) return 0;
          return result.value.file ? result.value.file.bytes : 0;
        })
      };
    }
    // WP-210 G: domain failures (not_found, permission…) resolve { ok: false }.
    function must(result) {
      if (result && result.ok === false) throw new Error(result.message || result.reason);
      return result;
    }
    async function lineText(result) {
      const line = result && result.lines && result.lines[0];
      return line ? line : "";
    }
    async function markerText(path) {
      try {
        return await lineText(must(await tools.read({ path })));
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
          const result = must(await tools.read({ path }));
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
        after = { status: "ok", text: await lineText(must(await tools.read({ path: "held.txt" }))) };
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
        during = { status: "ok", text: await lineText(must(await tools.read({ path: readPath }))) };
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
        after = { status: "ok", text: await lineText(must(await tools.read({ path: readPath }))) };
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
}

/// Effect locks are taken after approval; cancellation does not write afterwards.
mod approval_locks {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use litecode::config::schema::{AgentProfile, AgentToolBinding};
    use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
    use litecode::context_pipeline::Context;
    use litecode::permission::{
        AskOutcome, AskPrompt, AskReply, PermissionAction, PermissionEngine, PermissionSink,
        ToolPolicy,
    };
    use litecode::tool::Tool;
    use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
    use litecode::tool::write_lock::process_write_lock;
    use litecode::tools::code::{CodeTool, CodeToolHost};
    use litecode::tools::read::ReadTool;
    use litecode::tools::write::WriteTool;
    use litecode::types::FunctionToolCall;
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
        let mut global = GlobalSettings::default();
        let binding = AgentToolBinding {
            enabled: true,
            policy: ToolPolicy::with_default(policy),
            path_mode: Default::default(),
            last_applied_preset: None,
            allowed_tools: None,
        };
        let mut tools = std::collections::HashMap::new();
        for name in ["read", "write"] {
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
    }

    fn hosted(root: &Path, policy: PermissionAction, sink: Arc<dyn PermissionSink>) -> Hosted {
        let permission = engine(root, policy);
        let ctx = ctx(root);
        let data_root = root.join(".litecode");
        std::fs::create_dir_all(&data_root).unwrap();
        let host = CodeToolHost::new(
            permission.clone(),
            ctx.clone(),
            "default",
            sink,
            data_root.clone(),
            process_write_lock(),
            None,
        );
        let code = Arc::new(CodeTool::new(
            vec![
                Arc::new(ReadTool::default()) as Arc<dyn Tool>,
                Arc::new(WriteTool::new()),
            ],
            Arc::new(host),
        ));
        Hosted {
            tools: vec![code],
            permission,
            ctx,
            data_root,
        }
    }

    async fn execute(
        hosted: &Hosted,
        session_id: &str,
        name: &str,
        args: Value,
        sink: &dyn PermissionSink,
        cancel: CancellationToken,
    ) -> litecode::types::ToolCallResult {
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
                code_names: Arc::from([]),
                file_note: None,
                permission_sink: None,
                plan_touch: None,
                deadline: None,
            },
        )
        .await
    }

    struct BlockingAsk {
        entered: Arc<AtomicBool>,
        release: Arc<(Mutex<Option<AskOutcome>>, Condvar)>,
    }

    impl BlockingAsk {
        fn new() -> Self {
            Self {
                entered: Arc::new(AtomicBool::new(false)),
                release: Arc::new((Mutex::new(None), Condvar::new())),
            }
        }

        fn release(&self, outcome: AskOutcome) {
            let (lock, cv) = &*self.release;
            *lock.lock().unwrap() = Some(outcome);
            cv.notify_all();
        }
    }

    impl PermissionSink for BlockingAsk {
        fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, cv) = &*self.release;
            let mut slot = lock.lock().unwrap();
            loop {
                if let Some(outcome) = *slot {
                    return AskReply::from_outcome(outcome);
                }
                if cancel.is_cancelled() {
                    return AskReply::from_outcome(AskOutcome::Aborted);
                }
                let (next, _) = cv
                    .wait_timeout(slot, Duration::from_millis(20))
                    .unwrap_or_else(|error| error.into_inner());
                slot = next;
            }
        }
    }

    struct SlowDeny {
        entered: Arc<AtomicBool>,
        finished: Arc<AtomicBool>,
    }

    impl PermissionSink for SlowDeny {
        fn ask(&self, _prompt: &AskPrompt<'_>, _cancel: &CancellationToken) -> AskReply {
            self.entered.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_secs(5));
            self.finished.store(true, Ordering::SeqCst);
            AskReply::from_outcome(AskOutcome::Deny)
        }
    }

    async fn wait_flag(flag: &AtomicBool) {
        let start = std::time::Instant::now();
        while !flag.load(Ordering::SeqCst) {
            if start.elapsed() > Duration::from_secs(5) {
                panic!("approval was never entered");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn approval_pending_does_not_preclaim_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let ask = Arc::new(BlockingAsk::new());
        let nested = hosted(
            dir.path(),
            PermissionAction::Ask,
            Arc::clone(&ask) as Arc<dyn PermissionSink>,
        );
        let cancel = CancellationToken::new();
        let script = r#"
    try {
      await tools.write({ path: "same.txt", content: "from-nested" });
    } catch (e) {}
    return "done";
    "#;
        let run = tokio::spawn(async move {
            execute(
                &nested,
                "phase4-nested",
                "run_code",
                json!({ "code": script }),
                &litecode::permission::DenyPermissionSink,
                cancel,
            )
            .await
        });
        wait_flag(&ask.entered).await;

        let allow = hosted(
            dir.path(),
            PermissionAction::Allow,
            litecode::permission::deny_permission_sink(),
        );
        let other = r#"
    try {
      const probe = await tools.read({ path: "same.txt" });
      if (probe.ok !== false) {
        throw new Error("read saw a file before the pending write was approved");
      }
    } catch (e) {
      if (!(e instanceof ToolError)) throw e;
    }
    const write = await tools.write({ path: "same.txt", content: "from-direct" });
    if (!write.file) {
      throw new Error("write failed while approval was pending");
    }
    return "wrote";
    "#;
        let other = tokio::time::timeout(
            Duration::from_secs(3),
            execute(
                &allow,
                "phase4-direct",
                "run_code",
                json!({ "code": other }),
                &litecode::permission::DenyPermissionSink,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("a same-path read/write was blocked while approval was still pending");
        assert!(other.error.is_none(), "{}", other.content);
        assert_eq!(run_code_result(&other.content), json!("wrote"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("same.txt")).unwrap(),
            "from-direct"
        );

        ask.release(AskOutcome::Deny);
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("nested write stayed pending after the approval was denied")
            .unwrap();
        assert!(result.error.is_none(), "{}", result.content);
        assert!(
            result.content.contains("tool calls: 0 ok / 1 failed"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("same.txt")).unwrap(),
            "from-direct"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancel_before_approval_does_not_write_later() {
        let dir = tempfile::tempdir().unwrap();
        let ask = Arc::new(BlockingAsk::new());
        let hosted = hosted(
            dir.path(),
            PermissionAction::Ask,
            Arc::clone(&ask) as Arc<dyn PermissionSink>,
        );
        let cancel = CancellationToken::new();
        let cancel_run = cancel.clone();
        let script = r#"
    await tools.write({ path: "gone.txt", content: "later" });
    return "done";
    "#;
        let run = tokio::spawn(async move {
            execute(
                &hosted,
                "phase4-cancel",
                "run_code",
                json!({ "code": script }),
                &litecode::permission::DenyPermissionSink,
                cancel_run,
            )
            .await
        });
        wait_flag(&ask.entered).await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(8), run)
            .await
            .expect("cancelled run did not return")
            .unwrap();
        assert!(
            result.content.contains("cancelled") || result.content.contains("tool calls: 0 ok"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert!(!dir.path().join("gone.txt").exists());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !dir.path().join("gone.txt").exists(),
            "a write landed after cancellation"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn grace_expiry_is_unsettled_and_the_later_denial_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let entered = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(SlowDeny {
            entered: Arc::clone(&entered),
            finished: Arc::clone(&finished),
        });
        let hosted = hosted(
            dir.path(),
            PermissionAction::Ask,
            Arc::clone(&sink) as Arc<dyn PermissionSink>,
        );
        let cancel = CancellationToken::new();
        let cancel_run = cancel.clone();
        let script = r#"
    await tools.write({ path: "late.txt", content: "maybe" });
    return "done";
    "#;
        let run = tokio::spawn(async move {
            execute(
                &hosted,
                "phase4-unsettled",
                "run_code",
                json!({ "code": script }),
                &litecode::permission::DenyPermissionSink,
                cancel_run,
            )
            .await
        });
        wait_flag(&entered).await;
        cancel.cancel();
        let result = tokio::time::timeout(Duration::from_secs(8), run)
            .await
            .expect("grace did not seal the running call")
            .unwrap();
        assert!(
            result.content.contains("calls were still running")
                || result.content.contains("unsettled")
                || result.content.contains("cancelled"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert!(!dir.path().join("late.txt").exists());
        let start = std::time::Instant::now();
        while !finished.load(Ordering::SeqCst) {
            if start.elapsed() > Duration::from_secs(8) {
                panic!("the approval future did not finish");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !dir.path().join("late.txt").exists(),
            "the call wrote after it had already been reported unsettled"
        );
    }
}

/// Calls stay runnable while another approval is pending.
mod approval_concurrency {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use litecode::config::schema::{AgentProfile, AgentToolBinding};
    use litecode::config::{ConfigManager, GlobalSettings, WorkspacePaths, WorkspaceState};
    use litecode::context_pipeline::Context;
    use litecode::permission::{
        AskOutcome, AskPrompt, AskReply, PermissionAction, PermissionEngine, PermissionSink,
        ToolPolicy,
    };
    use litecode::tool::Tool;
    use litecode::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
    use litecode::tool::write_lock::process_write_lock;
    use litecode::tools::code::{CodeTool, CodeToolHost};
    use litecode::tools::edit::EditTool;
    use litecode::tools::glob::GlobTool;
    use litecode::tools::grep::GrepTool;
    use litecode::tools::read::ReadTool;
    use litecode::tools::write::WriteTool;
    use litecode::types::FunctionToolCall;
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
        let mut global = GlobalSettings::default();
        let binding = AgentToolBinding {
            enabled: true,
            policy: ToolPolicy::with_default(policy),
            path_mode: Default::default(),
            last_applied_preset: None,
            allowed_tools: None,
        };
        let mut tools = std::collections::HashMap::new();
        for name in ["read", "glob", "grep", "edit", "write"] {
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

    fn surface() -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(ReadTool::default()),
            Arc::new(GlobTool),
            Arc::new(GrepTool),
            Arc::new(EditTool::new()),
            Arc::new(WriteTool::new()),
        ]
    }

    struct Hosted {
        tools: Vec<Arc<dyn Tool>>,
        permission: PermissionEngine,
        ctx: Context,
        data_root: PathBuf,
    }

    fn hosted(root: &Path, policy: PermissionAction, sink: Arc<dyn PermissionSink>) -> Hosted {
        let permission = engine(root, policy);
        let ctx = ctx(root);
        let data_root = root.join(".litecode");
        std::fs::create_dir_all(&data_root).unwrap();
        let host = CodeToolHost::new(
            permission.clone(),
            ctx.clone(),
            "default",
            sink,
            data_root.clone(),
            process_write_lock(),
            None,
        );
        let code = Arc::new(CodeTool::new(surface(), Arc::new(host)));
        Hosted {
            tools: vec![code],
            permission,
            ctx,
            data_root,
        }
    }

    async fn execute(
        hosted: &Hosted,
        session_id: &str,
        code: &str,
        sink: &dyn PermissionSink,
        cancel: CancellationToken,
    ) -> litecode::types::ToolCallResult {
        let call = FunctionToolCall {
            arguments: json!({ "code": code }).to_string(),
            call_id: format!("call-{session_id}"),
            namespace: None,
            name: "run_code".into(),
            id: None,
            status: None,
        };
        execute_tool_call(
            &call,
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
                code_names: Arc::from([]),
                file_note: None,
                permission_sink: None,
                plan_touch: None,
                deadline: None,
            },
        )
        .await
    }

    struct BlockingAsk {
        entered: Arc<AtomicBool>,
        release: Arc<(Mutex<Option<AskOutcome>>, Condvar)>,
    }

    impl BlockingAsk {
        fn new() -> Self {
            Self {
                entered: Arc::new(AtomicBool::new(false)),
                release: Arc::new((Mutex::new(None), Condvar::new())),
            }
        }

        fn release(&self, outcome: AskOutcome) {
            let (lock, cv) = &*self.release;
            *lock.lock().unwrap() = Some(outcome);
            cv.notify_all();
        }
    }

    impl PermissionSink for BlockingAsk {
        fn ask(&self, _prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
            self.entered.store(true, Ordering::SeqCst);
            let (lock, cv) = &*self.release;
            let mut slot = lock.lock().unwrap();
            loop {
                if let Some(outcome) = *slot {
                    return AskReply::from_outcome(outcome);
                }
                if cancel.is_cancelled() {
                    return AskReply::from_outcome(AskOutcome::Aborted);
                }
                let (next, _) = cv
                    .wait_timeout(slot, Duration::from_millis(20))
                    .unwrap_or_else(|error| error.into_inner());
                slot = next;
            }
        }
    }

    async fn wait_flag(flag: &AtomicBool) {
        let start = std::time::Instant::now();
        while !flag.load(Ordering::SeqCst) {
            if start.elapsed() > Duration::from_secs(5) {
                panic!("approval was never entered");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn approval_pending_does_not_lock_other_calls() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "other.txt", "OTHER_NEEDLE\n");
        let ask = Arc::new(BlockingAsk::new());
        let pending = hosted(
            root,
            PermissionAction::Ask,
            Arc::clone(&ask) as Arc<dyn PermissionSink>,
        );
        let cancel = CancellationToken::new();
        let run = tokio::spawn(async move {
            execute(
                &pending,
                "phase5-pending",
                r#"try { await tools.write({ path: "pending.txt", content: "wait" }); } catch (e) {} return "done";"#,
                &litecode::permission::DenyPermissionSink,
                cancel,
            )
            .await
        });
        wait_flag(&ask.entered).await;

        let allow = hosted(
            root,
            PermissionAction::Allow,
            litecode::permission::deny_permission_sink(),
        );
        let other = tokio::time::timeout(
            Duration::from_secs(3),
            execute(
                &allow,
                "phase5-other",
                r#"
    const page = await tools.read({ path: "other.txt" });
    const found = await tools.grep({ pattern: "OTHER_NEEDLE", path: "other.txt" });
    return { line: page.lines[0], hits: found.hits.length };
    "#,
                &litecode::permission::DenyPermissionSink,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("another call stayed locked while approval was pending");
        assert!(other.error.is_none(), "{}", other.content);
        let value = run_code_result(&other.content);
        assert_eq!(value["line"], "OTHER_NEEDLE");
        assert_eq!(value["hits"], 1);

        ask.release(AskOutcome::Deny);
        let pending_result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("pending write did not finish")
            .unwrap();
        assert!(pending_result.error.is_none(), "{}", pending_result.content);
        assert!(!root.join("pending.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("other.txt")).unwrap(),
            "OTHER_NEEDLE\n"
        );
    }


}
