//! Internal code-mode tool: one model-facing transport that orchestrates a
//! bounded bundle of existing tools through a real JavaScript runtime.
//!
//! Every inner call still enters [`run_tool`], so schema validation,
//! permissions, resource locks, audit, and cancellation stay in the existing
//! tool pipeline. The host is a scheduler and protocol bridge, not a second
//! execution path.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, PermissionSink};
use crate::tool::Tool;
use crate::tool::code_manifest::render_code_tool_description;
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tools::code_process::ProcessTree;
use crate::types::{FunctionToolCall, ToolCallResult, ToolErrorKind, ToolSignalLevel};

/// Model-facing name of the aggregated code transport.
pub const CODE_TOOL_NAME: &str = "run_code";
/// Default total program budget, matching the DSH reference.
pub const DEFAULT_RUN_TIMEOUT_MS: u64 = 120_000;
/// Maximum total program budget accepted from the model.
pub const MAX_RUN_TIMEOUT_MS: u64 = 600_000;
/// Combined serialized logs plus completion-or-diagnostic budget.
pub const MAX_CODE_OUTPUT_BYTES: usize = 1_048_576;
const MAX_PARALLEL_TOOL_CALLS: usize = 10;
const MAX_CONTROL_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_V8_HEAP_MB: u64 = 256;
const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROCESS_EXIT_GRACE: Duration = Duration::from_secs(3);

/// Internal test switch. The default product path stays unchanged.
pub fn code_mode_enabled() -> bool {
    std::env::var("LITECODE_CODE_MODE").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Runtime facts needed to execute bundled tools from inside [`CodeTool`].
///
/// This is deliberately narrower than `RuntimeContext`: CodeTool does not own
/// the top-level tool list and cannot run an arbitrary global tool name. The
/// caller supplies the bundle, and [`Self::run_inner`] executes only that slice.
pub struct CodeToolHost {
    permission: PermissionEngine,
    ctx: Context,
    agent_name: String,
    permission_sink: Arc<dyn PermissionSink>,
    data_root: PathBuf,
    spill_threshold: usize,
    write_lock: Arc<WorkspaceWriteLock>,
    session: Option<crate::session::SessionDataReader>,
    file_note: Option<Arc<dyn Fn(&[std::path::PathBuf]) + Send + Sync>>,
    plan_touch: Option<crate::tool::executor::PlanTouch>,
}

impl CodeToolHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        permission: PermissionEngine,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        data_root: PathBuf,
        spill_threshold: usize,
        write_lock: Arc<WorkspaceWriteLock>,
        session: Option<crate::session::SessionDataReader>,
    ) -> Self {
        Self {
            permission,
            ctx,
            agent_name: agent_name.into(),
            permission_sink,
            data_root,
            spill_threshold,
            write_lock,
            session,
            file_note: None,
            plan_touch: None,
        }
    }

    pub fn with_file_note(
        mut self,
        file_note: Arc<dyn Fn(&[std::path::PathBuf]) + Send + Sync>,
    ) -> Self {
        self.file_note = Some(file_note);
        self
    }

    pub fn with_plan_touch(mut self, plan_touch: crate::tool::executor::PlanTouch) -> Self {
        self.plan_touch = Some(plan_touch);
        self
    }

    fn sink(&self) -> Arc<dyn PermissionSink> {
        Arc::clone(&self.permission_sink)
    }

    async fn run_inner(
        &self,
        bundle: &[Arc<dyn Tool>],
        call: &FunctionToolCall,
        session_id: &str,
        cancel: CancellationToken,
        parent_call_id: &str,
    ) -> ToolCallResult {
        execute_tool_call(
            call,
            bundle,
            &self.permission,
            &self.ctx,
            session_id,
            &self.agent_name,
            self.permission_sink.as_ref(),
            cancel,
            &self.data_root,
            self.spill_threshold,
            None,
            Arc::clone(&self.write_lock),
            self.session.clone(),
            ToolCallOrigin {
                caller: ToolCaller::Code {
                    parent_call_id: parent_call_id.to_string(),
                },
                gate_open: true,
                code_names: Arc::from([]),
                file_note: self.file_note.clone(),
                permission_sink: Some(self.sink()),
                plan_touch: self.plan_touch.clone(),
            },
        )
        .await
    }

    /// Run one binding call under the tool's declared budget and the optional
    /// script-level `timeout_ms`. A requested timeout is clamped to the tool's
    /// declared maximum; a tool without a declared timeout rejects an explicit
    /// request rather than silently ignoring it.
    async fn run_inner_with_timeout(
        &self,
        bundle: &Arc<Vec<Arc<dyn Tool>>>,
        call: FunctionToolCall,
        session_id: &str,
        outer_cancel: &CancellationToken,
        run_deadline: tokio::time::Instant,
        requested_timeout_ms: Option<u64>,
        parent_call_id: &str,
    ) -> ToolCallResult {
        let Some(tool) = bundle.iter().find(|tool| tool.name() == call.name) else {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::NotFound,
                format!("tool '{}' not found", call.name),
                false,
            );
        };

        let max_timeout_ms = tool.timeout().map(|seconds| seconds.saturating_mul(1_000));
        let effective_timeout_ms = match (requested_timeout_ms, max_timeout_ms) {
            (Some(_), None) => {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    format!(
                        "tool '{}' does not support per-call timeout_ms",
                        tool.name()
                    ),
                    false,
                );
            }
            (Some(requested), Some(maximum)) => Some(requested.min(maximum)),
            (None, maximum) => maximum,
        };

        let now = tokio::time::Instant::now();
        if now >= run_deadline {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Timeout,
                format!(
                    "tool '{}' was not started because the run_code budget was exhausted",
                    tool.name()
                ),
                true,
            );
        }

        let deadline = effective_timeout_ms
            .map(|timeout_ms| now + Duration::from_millis(timeout_ms))
            .unwrap_or(run_deadline)
            .min(run_deadline);
        let call_cancel = outer_cancel.child_token();
        let future = self.run_inner(
            bundle.as_slice(),
            &call,
            session_id,
            call_cancel.clone(),
            parent_call_id,
        );
        tokio::pin!(future);

        tokio::select! {
            result = &mut future => {
                if result
                    .error
                    .as_ref()
                    .is_some_and(|error| error.kind == ToolErrorKind::Timeout)
                {
                    call_cancel.cancel();
                }
                result
            }
            _ = tokio::time::sleep_until(deadline) => {
                call_cancel.cancel();
                let _ = future.await;
                let budget_exhausted = tokio::time::Instant::now() >= run_deadline;
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Timeout,
                    if budget_exhausted {
                        format!(
                            "tool '{}' stopped because the run_code budget was exhausted",
                            tool.name()
                        )
                    } else {
                        format!(
                            "tool '{}' timed out after {}ms",
                            tool.name(),
                            deadline
                                .saturating_duration_since(now)
                                .as_millis()
                        )
                    },
                    true,
                )
            }
            _ = outer_cancel.cancelled() => {
                call_cancel.cancel();
                let _ = future.await;
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Cancelled,
                    format!("tool '{}' cancelled", tool.name()),
                    false,
                )
            }
        }
    }
}

/// A bounded code bundle projected as one model-facing tool.
pub struct CodeTool {
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    sdk: String,
    whitelist: Arc<[String]>,
    host: Arc<CodeToolHost>,
}

impl CodeTool {
    /// Build from an already-admitted surface. `sdk` is the pure rendering of that surface.
    pub fn from_surface(
        bundle: Vec<Arc<dyn Tool>>,
        sdk: String,
        whitelist: Vec<String>,
        host: Arc<CodeToolHost>,
    ) -> Self {
        Self {
            bundle: Arc::new(bundle),
            sdk,
            whitelist: whitelist.into(),
            host,
        }
    }

    pub fn new(bundle: Vec<Arc<dyn Tool>>, host: Arc<CodeToolHost>) -> Self {
        let sdk = render_code_tool_description(&bundle).unwrap_or_default();
        let mut whitelist: Vec<String> = bundle.iter().map(|tool| tool.name().to_string()).collect();
        whitelist.sort();
        Self::from_surface(bundle, sdk, whitelist, host)
    }

    async fn execute_code(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        if self.sdk.is_empty() {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Internal,
                "run_code is unavailable because its code surface failed admission",
                false,
            );
        }
        let code = match crate::tool::require_nonempty_string(&input, "code") {
            Ok(code) => code.to_string(),
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false);
            }
        };
        let run_timeout_ms = match parse_run_timeout_ms(&input) {
            Ok(timeout_ms) => timeout_ms,
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false);
            }
        };
        let run_deadline = tokio::time::Instant::now() + Duration::from_millis(run_timeout_ms);

        let code_root = self.host.data_root.join("code");
        if let Err(error) = std::fs::create_dir_all(&code_root) {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Io,
                format!(
                    "create code runtime directory {}: {error}",
                    code_root.display()
                ),
                false,
            );
        }
        let run_dir = code_root.join(format!("run-{}", ulid::Ulid::new()));
        if let Err(error) = std::fs::create_dir_all(&run_dir) {
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Io,
                format!("create code run directory {}: {error}", run_dir.display()),
                false,
            );
        }
        let _cleanup = CodeRunCleanup(run_dir.clone());

        let runner_path = run_dir.join("runner.mjs");
        let libs_path = run_dir.join("libs.mjs");
        let script_path = run_dir.join("user.mjs");
        for (path, source, label) in [
            (&runner_path, include_str!("code_runner.mjs"), "runner"),
            (&libs_path, include_str!("code_libs.mjs"), "libraries"),
            (&script_path, code.as_str(), "user code"),
        ] {
            if let Err(error) = std::fs::write(path, source) {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::Io,
                    format!("write code {label}: {error}"),
                    false,
                );
            }
        }

        let mut names: Vec<&str> = self.bundle.iter().map(|tool| tool.name()).collect();
        names.sort_unstable();
        let tool_names = names.join(",");

        let node_bin = std::env::var("LITECODE_NODE_BIN").unwrap_or_else(|_| "node".to_string());
        let permission_flag = match node_permission_flag(&node_bin).await {
            Ok(flag) => flag,
            Err(error) => {
                return ToolCallResult::error_with_kind(ToolErrorKind::Unavailable, error, false);
            }
        };

        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(listener) => listener,
            Err(error) => {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::Unavailable,
                    format!("failed to bind run_code control channel: {error}"),
                    false,
                );
            }
        };
        let control_addr = match listener.local_addr() {
            Ok(addr) => addr.to_string(),
            Err(error) => {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::Internal,
                    format!("failed to read run_code control address: {error}"),
                    false,
                );
            }
        };
        let control_token = uuid::Uuid::new_v4().to_string();

        let mut command = tokio::process::Command::new(&node_bin);
        command
            .arg(format!("--max-old-space-size={MAX_V8_HEAP_MB}"))
            .arg(permission_flag)
            .arg(format!("--allow-fs-read={}", run_dir.display()))
            .arg(format!("--allow-fs-write={}", run_dir.display()))
            .arg(&runner_path)
            .env_clear()
            .env("LITECODE_CODE_CONTROL_ADDR", &control_addr)
            .env("LITECODE_CODE_CONTROL_TOKEN", &control_token)
            .env("LITECODE_CODE_TOOLS", &tool_names)
            .env("LITECODE_CODE_SCRIPT", &script_path)
            .env(
                "LITECODE_CODE_MAX_FRAME_BYTES",
                MAX_CONTROL_FRAME_BYTES.to_string(),
            )
            .env(
                "LITECODE_CODE_MAX_OUTPUT_BYTES",
                MAX_CODE_OUTPUT_BYTES.to_string(),
            )
            .current_dir(&run_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for key in ["PATH", "PATHEXT", "SYSTEMROOT", "WINDIR"] {
            if let Ok(value) = std::env::var(key) {
                command.env(key, value);
            }
        }
        command.env("TEMP", run_dir.to_string_lossy().to_string());
        command.env("TMP", run_dir.to_string_lossy().to_string());
        #[cfg(unix)]
        command.process_group(0);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::Unavailable,
                    format!(
                        "failed to spawn Node.js ({node_bin}): {error}. Install Node.js 22+ or set LITECODE_NODE_BIN."
                    ),
                    false,
                );
            }
        };
        let mut process_tree = ProcessTree::attach(&child);

        let Some(stdout) = child.stdout.take() else {
            process_tree.terminate();
            let _ = child.kill().await;
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Internal,
                "code runtime missing stdout",
                false,
            );
        };
        let Some(stderr) = child.stderr.take() else {
            process_tree.terminate();
            let _ = child.kill().await;
            return ToolCallResult::error_with_kind(
                ToolErrorKind::Internal,
                "code runtime missing stderr",
                false,
            );
        };

        let logs = Arc::new(Mutex::new(LogLedger::new(MAX_CODE_OUTPUT_BYTES)));
        let mut stdout_task = tokio::spawn(read_process_output(stdout, Arc::clone(&logs)));
        let mut stderr_task = tokio::spawn(read_process_output(stderr, Arc::clone(&logs)));

        let run_cancel = execution.cancel.child_token();
        let outer_cancel = execution.cancel.clone();
        let exchange_run_cancel = run_cancel.clone();
        let execution_call_id = execution.call_id.clone();
        let execution_session_id = execution.session_id.clone();
        let bundle = Arc::clone(&self.bundle);
        let host = Arc::clone(&self.host);
        let expected_token = control_token.clone();

        let exchange = async move {
            let connect_deadline =
                (tokio::time::Instant::now() + CONTROL_CONNECT_TIMEOUT).min(run_deadline);
            let (stream, _) =
                match tokio::time::timeout_at(connect_deadline, listener.accept()).await {
                    Ok(Ok(accepted)) => accepted,
                    Ok(Err(error)) => {
                        return Err(ExchangeError::Failed(format!(
                            "accept code runtime control connection: {error}"
                        )));
                    }
                    Err(_) if connect_deadline >= run_deadline => {
                        return Err(ExchangeError::TimedOut);
                    }
                    Err(_) => {
                        return Err(ExchangeError::Failed(
                            "code runtime control connection timed out".to_string(),
                        ));
                    }
                };
            let (reader, writer) = stream.into_split();
            exchange_control(
                reader,
                Arc::new(Mutex::new(writer)),
                expected_token,
                bundle,
                host,
                execution_session_id,
                execution_call_id,
                exchange_run_cancel,
                run_deadline,
            )
            .await
        };

        let outcome = tokio::select! {
            _ = outer_cancel.cancelled() => Err(ExchangeError::Cancelled),
            result = tokio::time::timeout_at(run_deadline, exchange) => {
                match result {
                    Ok(result) => result,
                    Err(_) => Err(ExchangeError::TimedOut),
                }
            }
        };
        run_cancel.cancel();

        match &outcome {
            Ok(_) => {
                if tokio::time::timeout(PROCESS_EXIT_GRACE, child.wait())
                    .await
                    .is_err()
                {
                    process_tree.terminate();
                    let _ = child.kill().await;
                    let _ = tokio::time::timeout(PROCESS_EXIT_GRACE, child.wait()).await;
                }
                process_tree.close();
            }
            Err(_) => {
                process_tree.terminate();
                let _ = child.kill().await;
                let _ = tokio::time::timeout(PROCESS_EXIT_GRACE, child.wait()).await;
            }
        }

        let _ = tokio::time::timeout(Duration::from_secs(2), &mut stdout_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut stderr_task).await;
        if !stdout_task.is_finished() {
            stdout_task.abort();
        }
        if !stderr_task.is_finished() {
            stderr_task.abort();
        }

        let ledger = logs.lock().await;
        let logs_text = ledger.as_text();
        let overflowed = ledger.overflowed;
        drop(ledger);

        match outcome {
            Ok(value) => {
                let value_bytes = value
                    .as_ref()
                    .and_then(|value| serde_json::to_vec(value).ok())
                    .map_or(0, |bytes| bytes.len());
                let logs_bytes = logs_text.len();
                if overflowed || logs_bytes.saturating_add(value_bytes) > MAX_CODE_OUTPUT_BYTES {
                    return output_limit_result(&logs_text);
                }

                let mut data = Map::new();
                data.insert(
                    "logs".to_string(),
                    Value::Array(
                        logs_text
                            .lines()
                            .map(|line| Value::String(line.to_string()))
                            .collect(),
                    ),
                );
                if let Some(value) = value {
                    data.insert("result".to_string(), value);
                }
                let data = Value::Object(data);
                let payload_bytes = serde_json::to_vec(&data)
                    .map(|bytes| bytes.len())
                    .unwrap_or(usize::MAX);
                if payload_bytes > MAX_CODE_OUTPUT_BYTES {
                    return output_limit_result(&logs_text);
                }
                let content = render_code_output(&logs_text, data.get("result"));
                ToolCallResult::ok(content).with_data(data)
            }
            Err(ExchangeError::Cancelled) => ToolCallResult::error_with_kind(
                ToolErrorKind::Cancelled,
                "code execution cancelled",
                false,
            ),
            Err(ExchangeError::TimedOut) => {
                let detail = output_suffix(&logs_text);
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Timeout,
                    format!("run_code timed out after {run_timeout_ms}ms{detail}"),
                    true,
                )
            }
            Err(ExchangeError::RuntimeFailure { kind, message }) => {
                let detail = output_suffix(&logs_text);
                ToolCallResult::error_with_kind(kind, format!("{message}{detail}"), false)
            }
            Err(ExchangeError::Failed(error)) => {
                let detail = output_suffix(&logs_text);
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Internal,
                    format!("code execution failed: {error}{detail}"),
                    false,
                )
            }
        }
    }
}

impl Tool for CodeTool {
    fn name(&self) -> &str {
        CODE_TOOL_NAME
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Async JavaScript function body. Use `tools` for host tools and `libs` for pure-JavaScript analysis helpers. Return a JSON-serializable value."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Total elapsed budget for this program, including nested tool calls and approval waits. Defaults to 120000; values above 600000 are clamped by the host."
                }
            },
            "required": ["code"],
            "additionalProperties": false
        })
    }

    fn code_bindings(&self) -> Option<Arc<[String]>> {
        Some(Arc::clone(&self.whitelist))
    }

    fn is_code_transport(&self) -> bool {
        true
    }

    fn description(&self, _ctx: &Context) -> String {
        if self.sdk.is_empty() {
            "run_code is unavailable because its code surface failed admission".to_string()
        } else {
            self.sdk.clone()
        }
    }

    fn validate_input(&self, input: &Value) -> std::result::Result<(), String> {
        crate::tool::require_nonempty_string(input, "code")?;
        if let Some(value) = input.get("timeout_ms").filter(|value| !value.is_null())
            && (value.as_u64().is_none_or(|timeout| timeout == 0))
        {
            return Err("parameter 'timeout_ms' must be a positive integer".to_string());
        }
        Ok(())
    }

    fn execute(
        &self,
        input: Value,
        execution: ToolExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>> {
        Box::pin(self.execute_code(input, execution))
    }

    fn call_inner(&self, _input: Value) -> ToolCallResult {
        ToolCallResult::error_with_kind(
            ToolErrorKind::Internal,
            "code tool requires the production execution context",
            false,
        )
    }

    fn is_cancellable(&self) -> bool {
        true
    }

    fn max_result_size(&self) -> usize {
        // The runtime owns the exact output ledger. The outer pipeline may
        // still spill the already-bounded presentation text without silently
        // truncating it here.
        usize::MAX
    }
}

#[derive(Debug)]
enum ExchangeError {
    Cancelled,
    TimedOut,
    RuntimeFailure {
        kind: ToolErrorKind,
        message: String,
    },
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
async fn exchange_control(
    mut reader: OwnedReadHalf,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    expected_token: String,
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    host: Arc<CodeToolHost>,
    session_id: String,
    execution_call_id: String,
    run_cancel: CancellationToken,
    run_deadline: tokio::time::Instant,
) -> Result<Option<Value>, ExchangeError> {
    let hello = read_control_frame(&mut reader).await?;
    if hello["type"].as_str() != Some("hello")
        || hello["token"].as_str() != Some(expected_token.as_str())
    {
        return Err(ExchangeError::Failed(
            "code runtime control handshake rejected".to_string(),
        ));
    }
    write_control_message(&writer, &json!({ "type": "ready" })).await?;

    let mut active = JoinSet::new();
    let permits = Arc::new(Semaphore::new(MAX_PARALLEL_TOOL_CALLS));
    let mut dispatch_id = 0_u64;

    loop {
        let message = read_control_frame(&mut reader).await?;
        match message["type"].as_str() {
            Some("tool_call") => {
                let id = message["id"]
                    .as_str()
                    .ok_or_else(|| ExchangeError::Failed("code tool call missing id".to_string()))?
                    .to_string();
                let name = message["name"]
                    .as_str()
                    .ok_or_else(|| {
                        ExchangeError::Failed("code tool call missing name".to_string())
                    })?
                    .to_string();
                let args = message.get("args").cloned().unwrap_or_else(|| json!({}));
                let options = message.get("options").cloned().unwrap_or(Value::Null);
                let requested_timeout_ms = match parse_call_timeout_ms(&options) {
                    Ok(timeout_ms) => timeout_ms,
                    Err(error) => {
                        let result = ToolCallResult::error_with_kind(
                            ToolErrorKind::InvalidInput,
                            error,
                            false,
                        );
                        send_tool_result(&writer, &id, &result).await?;
                        continue;
                    }
                };

                let Some(tool) = bundle.iter().find(|tool| tool.name() == name) else {
                    let result = ToolCallResult::error_with_kind(
                        ToolErrorKind::NotFound,
                        format!("tool '{name}' not found"),
                        false,
                    );
                    send_tool_result(&writer, &id, &result).await?;
                    continue;
                };

                dispatch_id = dispatch_id.saturating_add(1);
                let child_call = FunctionToolCall {
                    arguments: args.to_string(),
                    call_id: format!("{execution_call_id}:code:{dispatch_id}"),
                    namespace: None,
                    name,
                    id: None,
                    status: None,
                };
                let tool_safe = tool.is_concurrency_safe(&args);

                if tool_safe {
                    let permit = Arc::clone(&permits).acquire_owned().await.map_err(|_| {
                        ExchangeError::Failed("code concurrency semaphore closed".to_string())
                    })?;
                    let host = Arc::clone(&host);
                    let bundle = Arc::clone(&bundle);
                    let writer = Arc::clone(&writer);
                    let cancel = run_cancel.clone();
                    let session_id = session_id.clone();
                    let parent_call_id = execution_call_id.clone();
                    active.spawn(async move {
                        dispatch_tool_call(
                            host,
                            bundle,
                            child_call,
                            session_id,
                            cancel,
                            run_deadline,
                            requested_timeout_ms,
                            parent_call_id,
                            writer,
                            id,
                            Some(permit),
                        )
                        .await;
                    });
                } else {
                    // Exclusive calls form a barrier: settle every overlapping
                    // call before running alone.
                    while let Some(joined) = active.join_next().await {
                        log_join_result(joined);
                    }
                    dispatch_tool_call(
                        Arc::clone(&host),
                        Arc::clone(&bundle),
                        child_call,
                        session_id.clone(),
                        run_cancel.clone(),
                        run_deadline,
                        requested_timeout_ms,
                        execution_call_id.clone(),
                        Arc::clone(&writer),
                        id,
                        None,
                    )
                    .await;
                }
            }
            Some("final") => {
                run_cancel.cancel();
                while let Some(joined) = active.join_next().await {
                    log_join_result(joined);
                }
                if message["ok"].as_bool() == Some(true) {
                    let has_value = message
                        .get("has_value")
                        .and_then(Value::as_bool)
                        .unwrap_or_else(|| message.get("value").is_some());
                    return Ok(if has_value {
                        Some(message.get("value").cloned().unwrap_or(Value::Null))
                    } else {
                        None
                    });
                }
                let kind = match message["error_kind"].as_str() {
                    Some("output_limit") => ToolErrorKind::OutputLimit,
                    Some("invalid_output") => ToolErrorKind::InvalidOutput,
                    Some("exception") | None => ToolErrorKind::Internal,
                    _ => ToolErrorKind::Internal,
                };
                let message = message["error"]
                    .as_str()
                    .unwrap_or("code execution failed without an error")
                    .to_string();
                return Err(ExchangeError::RuntimeFailure { kind, message });
            }
            other => {
                return Err(ExchangeError::Failed(format!(
                    "unexpected code runtime message type: {other:?}"
                )));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_tool_call(
    host: Arc<CodeToolHost>,
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    call: FunctionToolCall,
    session_id: String,
    run_cancel: CancellationToken,
    run_deadline: tokio::time::Instant,
    requested_timeout_ms: Option<u64>,
    parent_call_id: String,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    id: String,
    _permit: Option<OwnedSemaphorePermit>,
) {
    let result = host
        .run_inner_with_timeout(
            &bundle,
            call,
            &session_id,
            &run_cancel,
            run_deadline,
            requested_timeout_ms,
            &parent_call_id,
        )
        .await;
    if let Err(error) = send_tool_result(&writer, &id, &result).await {
        tracing::debug!("write code tool result: {error:?}");
    }
}

async fn send_tool_result(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    id: &str,
    result: &ToolCallResult,
) -> Result<(), ExchangeError> {
    let response = json!({
        "type": "tool_result",
        "id": id,
        "ok": true,
        "result": tool_result_value(result),
    });
    match write_control_message(writer, &response).await {
        Ok(()) => Ok(()),
        Err(ExchangeError::Failed(_))
            if serde_json::to_vec(&response)
                .is_ok_and(|bytes| bytes.len() > MAX_CONTROL_FRAME_BYTES) =>
        {
            let fallback = ToolCallResult::error_with_kind(
                ToolErrorKind::OutputLimit,
                format!("tool result exceeds {MAX_CONTROL_FRAME_BYTES} bytes"),
                false,
            );
            let response = json!({
                "type": "tool_result",
                "id": id,
                "ok": true,
                "result": tool_result_value(&fallback),
            });
            write_control_message(writer, &response).await
        }
        Err(error) => Err(error),
    }
}

fn log_join_result(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::debug!("code tool task ended abnormally: {error}");
    }
}

fn parse_run_timeout_ms(input: &Value) -> Result<u64, String> {
    match input.get("timeout_ms").filter(|value| !value.is_null()) {
        None => Ok(DEFAULT_RUN_TIMEOUT_MS),
        Some(value) => {
            let timeout_ms = value
                .as_u64()
                .ok_or_else(|| "parameter 'timeout_ms' expected a positive integer".to_string())?;
            if timeout_ms == 0 {
                return Err("parameter 'timeout_ms' must be a positive integer".to_string());
            }
            Ok(timeout_ms.min(MAX_RUN_TIMEOUT_MS))
        }
    }
}

fn parse_call_timeout_ms(options: &Value) -> Result<Option<u64>, String> {
    let Value::Object(options) = options else {
        if options.is_null() {
            return Ok(None);
        }
        return Err("tool call options must be an object".to_string());
    };
    match options.get("timeout_ms").filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => {
            let timeout_ms = value.as_u64().ok_or_else(|| {
                "tool call option 'timeout_ms' expected a positive integer".to_string()
            })?;
            if timeout_ms == 0 {
                return Err("tool call option 'timeout_ms' must be a positive integer".to_string());
            }
            Ok(Some(timeout_ms))
        }
    }
}

fn tool_result_value(result: &ToolCallResult) -> Value {
    let status = match result.level {
        ToolSignalLevel::Ok => "ok",
        ToolSignalLevel::Warning => "warning",
        ToolSignalLevel::Error => "error",
    };
    json!({
        "status": status,
        "data": result.data,
        "error": result.error,
    })
}

async fn read_control_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Value, ExchangeError> {
    let mut header = [0_u8; 4];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|error| ExchangeError::Failed(format!("read control header: {error}")))?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_CONTROL_FRAME_BYTES {
        return Err(ExchangeError::Failed(format!(
            "invalid control frame length: {length}"
        )));
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|error| ExchangeError::Failed(format!("read control body: {error}")))?;
    serde_json::from_slice(&body)
        .map_err(|error| ExchangeError::Failed(format!("decode control JSON: {error}")))
}

async fn write_control_message(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    message: &Value,
) -> Result<(), ExchangeError> {
    let body = serde_json::to_vec(message)
        .map_err(|error| ExchangeError::Failed(format!("encode control message: {error}")))?;
    if body.is_empty() || body.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(ExchangeError::Failed(format!(
            "control frame exceeds {MAX_CONTROL_FRAME_BYTES} bytes"
        )));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    let mut writer = writer.lock().await;
    writer
        .write_all(&frame)
        .await
        .map_err(|error| ExchangeError::Failed(format!("write control frame: {error}")))?;
    writer
        .flush()
        .await
        .map_err(|error| ExchangeError::Failed(format!("flush control frame: {error}")))
}

async fn node_permission_flag(node_bin: &str) -> Result<&'static str, String> {
    let output = tokio::process::Command::new(node_bin)
        .arg("--version")
        .output()
        .await
        .map_err(|error| {
            format!(
                "failed to run Node.js ({node_bin}): {error}. Install Node.js 22+ or set LITECODE_NODE_BIN."
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "Node.js version probe failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let version = String::from_utf8_lossy(&output.stdout);
    let major = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|part| part.parse::<u32>().ok());
    match major {
        Some(major) if major >= 24 => Ok("--permission"),
        Some(major) if major >= 22 => Ok("--experimental-permission"),
        _ => Err(format!(
            "run_code requires Node.js 22+ for its permission model; found {}",
            version.trim()
        )),
    }
}

async fn read_process_output<R: AsyncRead + Unpin>(mut reader: R, logs: Arc<Mutex<LogLedger>>) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => return,
            Ok(bytes) => logs.lock().await.push(&buffer[..bytes]),
            Err(error) => {
                tracing::debug!("read code runtime output: {error}");
                return;
            }
        }
    }
}

struct LogLedger {
    bytes: Vec<u8>,
    max: usize,
    overflowed: bool,
}

impl LogLedger {
    fn new(max: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max,
            overflowed: false,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        if self.overflowed {
            return;
        }
        let remaining = self.max.saturating_sub(self.bytes.len());
        if chunk.len() > remaining {
            self.bytes.extend_from_slice(&chunk[..remaining]);
            self.overflowed = true;
        } else {
            self.bytes.extend_from_slice(chunk);
        }
    }

    fn as_text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

fn render_final_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other)
            .unwrap_or_else(|_| serde_json::to_string(other).unwrap_or_default()),
    }
}

fn render_code_output(logs: &str, result: Option<&Value>) -> String {
    let mut parts = Vec::new();
    if !logs.trim().is_empty() {
        parts.push(logs.trim_end_matches('\n').to_string());
    }
    if let Some(result) = result {
        parts.push(render_final_value(result));
    }
    if parts.is_empty() {
        "(run_code completed with no output)".to_string()
    } else {
        parts.join("\n")
    }
}

fn output_suffix(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let tail: String = trimmed
        .chars()
        .rev()
        .take(4_000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("\nCaptured output:\n{tail}")
}

fn output_limit_result(logs: &str) -> ToolCallResult {
    let suffix = output_suffix(logs);
    ToolCallResult::error_with_kind(
        ToolErrorKind::OutputLimit,
        format!("run_code output exceeded {MAX_CODE_OUTPUT_BYTES} bytes{suffix}"),
        true,
    )
}

struct CodeRunCleanup(PathBuf);

impl Drop for CodeRunCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.as_path());
    }
}
