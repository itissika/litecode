//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskPrompt, AskReply, PermissionEngine, PermissionSink};
use crate::ptc::{
    self, BudgetPause, ExecuteRequest, FailureKind, HostBinding, HostFn, Json, Limits, Namespace,
};
use crate::tool::Tool;
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::types::{FunctionToolCall, ToolCallResult, ToolErrorKind, ToolSignalLevel};

/// Model-facing name of the aggregated code transport.
pub const CODE_TOOL_NAME: &str = "run_code";
/// Default total program budget.
pub const DEFAULT_RUN_TIMEOUT_MS: u64 = 120_000;
/// Maximum total program budget accepted from the model.
pub const MAX_RUN_TIMEOUT_MS: u64 = 600_000;
/// Runtime output cap. This is the only hard cap; the outer result is not truncated again.
pub const MAX_CODE_OUTPUT_BYTES: usize = 1_048_576;
/// Nested-call ledger lines kept in the outer result. Further calls are counted, not copied.
const LEDGER_CAP: usize = 64;
/// How long the runtime waits for in-flight nested calls after it cancels them.
const NESTED_CANCEL_GRACE: Duration = Duration::from_secs(3);
const SUMMARY_VALUE_CHARS: usize = 80;

/// Facts needed to execute the caller's code surface from inside [`CodeTool`].
pub struct CodeToolHost {
    permission: PermissionEngine,
    ctx: Context,
    agent_name: String,
    permission_sink: Arc<dyn PermissionSink>,
    data_root: std::path::PathBuf,
    spill_threshold: usize,
    write_lock: Arc<WorkspaceWriteLock>,
    session: Option<crate::session::SessionDataReader>,
    #[allow(clippy::type_complexity)]
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
        data_root: std::path::PathBuf,
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

    #[allow(clippy::type_complexity)]
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

    async fn run_nested(
        &self,
        bundle: &[Arc<dyn Tool>],
        call: &FunctionToolCall,
        session_id: &str,
        cancel: CancellationToken,
        parent_call_id: &str,
        sink: &dyn PermissionSink,
    ) -> ToolCallResult {
        execute_tool_call(
            call,
            bundle,
            &self.permission,
            &self.ctx,
            session_id,
            &self.agent_name,
            sink,
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
                code_names: Arc::from([]),
                file_note: self.file_note.clone(),
                permission_sink: Some(self.sink()),
                plan_touch: self.plan_touch.clone(),
            },
        )
        .await
    }
}

/// Pauses the program budget for the duration of one approval wait.
///
/// The inner sink is the caller's [`PermissionSink`]. Denial stays a normal
/// tool result; this wrapper does not grant anything.
struct PausingSink {
    inner: Arc<dyn PermissionSink>,
    pause: BudgetPause,
}

impl PermissionSink for PausingSink {
    fn ask(&self, prompt: &AskPrompt<'_>, cancel: &CancellationToken) -> AskReply {
        let _guard = self.pause.pause();
        self.inner.ask(prompt, cancel)
    }
}

/// Nested-call ledger. The runtime call log has no arguments and no tool outcome;
/// this record is what the outer `function_call_output` shows.
#[derive(Clone)]
struct LedgerLine {
    tool: String,
    summary: String,
    /// Filled when the nested call returns. Empty means it was still in flight
    /// when the program ended; rendering treats that as cancelled and does not
    /// replace an outcome that was already stored.
    outcome: Option<String>,
}

struct CallLedger {
    next: AtomicU64,
    lines: Mutex<Vec<(u64, LedgerLine)>>,
    omitted: AtomicUsize,
}

impl CallLedger {
    fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
            lines: Mutex::new(Vec::new()),
            omitted: AtomicUsize::new(0),
        }
    }

    fn begin(&self, tool: &str, summary: &str) -> u64 {
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if lines.len() >= LEDGER_CAP {
            self.omitted.fetch_add(1, Ordering::Relaxed);
            return index;
        }
        lines.push((
            index,
            LedgerLine {
                tool: tool.to_string(),
                summary: summary.to_string(),
                outcome: None,
            },
        ));
        index
    }

    fn record(&self, index: u64, outcome: &str) {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some((_, line)) = lines.iter_mut().find(|(slot, _)| *slot == index)
            && line.outcome.is_none()
        {
            line.outcome = Some(outcome.to_string());
        }
    }

    fn render(&self) -> String {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        lines.sort_by_key(|(index, _)| *index);
        let mut text = String::new();
        for (index, line) in lines {
            if !text.is_empty() {
                text.push('\n');
            }
            let outcome = line.outcome.as_deref().unwrap_or("cancelled");
            if line.summary.is_empty() {
                text.push_str(&format!("{index}. {} {outcome}", line.tool));
            } else {
                text.push_str(&format!(
                    "{index}. {} {} {outcome}",
                    line.tool, line.summary
                ));
            }
        }
        let omitted = self.omitted.load(Ordering::Relaxed);
        if omitted > 0 {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("… {omitted} more calls omitted"));
        }
        text
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
        let sdk =
            crate::tool::code_manifest::render_code_tool_description(&bundle).unwrap_or_default();
        let mut whitelist: Vec<String> =
            bundle.iter().map(|tool| tool.name().to_string()).collect();
        whitelist.sort();
        Self::from_surface(bundle, sdk, whitelist, host)
    }

    fn binding(
        &self,
        ledger: Arc<CallLedger>,
        execution: &ToolExecutionContext,
    ) -> Vec<HostBinding> {
        self.bundle
            .iter()
            .map(|tool| {
                let name = tool.name().to_string();
                let bundle = Arc::clone(&self.bundle);
                let host = Arc::clone(&self.host);
                let ledger = Arc::clone(&ledger);
                let session_id = execution.session_id.clone();
                let parent_call_id = execution.call_id.clone();
                HostBinding {
                    name: name.clone(),
                    function: HostFn::new(move |args, cancel, pause| {
                        let name = name.clone();
                        let bundle = Arc::clone(&bundle);
                        let host = Arc::clone(&host);
                        let ledger = Arc::clone(&ledger);
                        let session_id = session_id.clone();
                        let parent_call_id = parent_call_id.clone();
                        async move {
                            invoke_binding(
                                name,
                                args,
                                cancel,
                                pause,
                                bundle,
                                host,
                                ledger,
                                session_id,
                                parent_call_id,
                            )
                            .await
                        }
                    }),
                }
            })
            .collect()
    }

    async fn execute_code(&self, input: Value, execution: ToolExecutionContext) -> ToolCallResult {
        if self.sdk.is_empty() || self.bundle.is_empty() {
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

        let ledger = Arc::new(CallLedger::new());
        let request = ExecuteRequest {
            source: code,
            namespaces: vec![Namespace {
                name: "tools".to_string(),
                functions: self.binding(Arc::clone(&ledger), &execution),
            }],
            limits: Limits {
                output_bytes: MAX_CODE_OUTPUT_BYTES,
                ..Limits::default()
            },
            budget: Duration::from_millis(run_timeout_ms),
            grace: NESTED_CANCEL_GRACE,
            cancel: execution.cancel,
        };
        let outcome = ptc::execute(request).await;
        let ledger_text = ledger.render();
        render_run_output(&ledger_text, &outcome, run_timeout_ms)
    }
}

#[allow(clippy::too_many_arguments)]
async fn invoke_binding(
    name: String,
    args: Json,
    cancel: CancellationToken,
    pause: BudgetPause,
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    host: Arc<CodeToolHost>,
    ledger: Arc<CallLedger>,
    session_id: String,
    parent_call_id: String,
) -> Result<Value, crate::ptc::HostError> {
    let decoded = decode_invocation(&args);
    // Drop the runtime JSON before any await. A deep tree stays inside `Json`.
    drop(args);
    let (arguments, summary, requested_timeout) = match decoded {
        Ok(decoded) => (decoded.arguments, decoded.summary, decoded.timeout),
        Err(illegal) => {
            let index = ledger.begin(&name, &illegal.summary);
            let result = ToolCallResult::error_with_kind(
                ToolErrorKind::InvalidOutput,
                illegal.message,
                false,
            );
            ledger.record(index, outcome_label(&result));
            return Ok(tool_result_value(&result));
        }
    };
    let index = ledger.begin(&name, &summary);

    let Some(tool) = bundle.iter().find(|tool| tool.name() == name) else {
        let result = ToolCallResult::error_with_kind(
            ToolErrorKind::NotFound,
            format!("tool '{name}' not found"),
            false,
        );
        ledger.record(index, outcome_label(&result));
        return Ok(tool_result_value(&result));
    };

    let requested_timeout = match requested_timeout {
        Ok(timeout) => timeout,
        Err(error) => {
            let result = ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, error, false);
            ledger.record(index, outcome_label(&result));
            return Ok(tool_result_value(&result));
        }
    };
    let declared_timeout_ms = tool.timeout().map(|seconds| seconds.saturating_mul(1_000));
    let effective_timeout_ms = match (requested_timeout, declared_timeout_ms) {
        (Some(_), None) => {
            let result = ToolCallResult::error_with_kind(
                ToolErrorKind::InvalidInput,
                format!("tool '{name}' does not support per-call timeout_ms"),
                false,
            );
            ledger.record(index, outcome_label(&result));
            return Ok(tool_result_value(&result));
        }
        (Some(requested), Some(maximum)) => Some(requested.min(maximum)),
        (None, maximum) => maximum,
    };

    let call = FunctionToolCall {
        arguments,
        call_id: format!("{parent_call_id}:code:{index}"),
        namespace: None,
        name: name.clone(),
        id: None,
        status: None,
    };
    let sink = PausingSink {
        inner: host.sink(),
        pause,
    };
    let call_cancel = cancel.child_token();
    let run = host.run_nested(
        bundle.as_slice(),
        &call,
        &session_id,
        call_cancel.clone(),
        &parent_call_id,
        &sink,
    );
    let result = match effective_timeout_ms {
        Some(timeout_ms) => await_with_timeout(run, call_cancel, timeout_ms, &name).await,
        None => run.await,
    };
    ledger.record(index, outcome_label(&result));
    Ok(tool_result_value(&result))
}

/// Per-call budget. A result the tool already produced is kept; cancellation is
/// signaled and then joined, never dropped.
async fn await_with_timeout(
    run: impl std::future::Future<Output = ToolCallResult>,
    call_cancel: CancellationToken,
    timeout_ms: u64,
    name: &str,
) -> ToolCallResult {
    let mut future = std::pin::pin!(run);
    tokio::select! {
        biased;
        result = &mut future => result,
        _ = tokio::time::sleep(Duration::from_millis(timeout_ms)) => {
            call_cancel.cancel();
            let result = future.await;
            if result
                .error
                .as_ref()
                .is_some_and(|error| error.kind == ToolErrorKind::Cancelled)
            {
                ToolCallResult::error_with_kind(
                    ToolErrorKind::Timeout,
                    format!("tool '{name}' timed out after {timeout_ms}ms"),
                    true,
                )
            } else {
                result
            }
        }
    }
}

struct DecodedInvocation {
    arguments: String,
    summary: String,
    timeout: Result<Option<u64>, String>,
}

struct IllegalInvocation {
    summary: String,
    message: String,
}

/// Borrow the runtime JSON. A deep argument is rejected here so the tool
/// pipeline never builds a `serde_json::Value` it would later drop recursively.
fn decode_invocation(payload: &Json) -> Result<DecodedInvocation, IllegalInvocation> {
    let (args, options) = split_invocation(payload.as_value());
    let summary = key_arg_summary(args);
    if exceeds_owned_depth(args) {
        return Err(IllegalInvocation {
            summary,
            message: "tool arguments are nested too deeply to enter the host pipeline".to_string(),
        });
    }
    let arguments = match encode_json(args) {
        Ok(arguments) => arguments,
        Err(message) => {
            return Err(IllegalInvocation { summary, message });
        }
    };
    let timeout = parse_call_timeout_ms(options);
    Ok(DecodedInvocation {
        arguments,
        summary,
        timeout,
    })
}

/// One argument stays that value. Two or more arrive as a JSON array from the runtime.
fn split_invocation(payload: &Value) -> (&Value, &Value) {
    match payload {
        Value::Array(items) if items.len() >= 2 => {
            (&items[0], items.get(1).unwrap_or(&Value::Null))
        }
        other => (other, &Value::Null),
    }
}

/// Derived `Value` Drop, Clone, and Debug recurse on the thread stack.
/// This is only the point at which the host refuses to own that tree.
const OWNED_VALUE_DEPTH: usize = 128;

fn exceeds_owned_depth(value: &Value) -> bool {
    let mut stack = vec![(value, 0usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > OWNED_VALUE_DEPTH {
            return true;
        }
        match value {
            Value::Array(items) => {
                for item in items {
                    stack.push((item, depth + 1));
                }
            }
            Value::Object(map) => {
                for child in map.values() {
                    stack.push((child, depth + 1));
                }
            }
            _ => {}
        }
    }
    false
}

fn encode_json(value: &Value) -> Result<String, String> {
    let bytes = crate::ptc::json_bytes(value)?;
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

fn key_arg_summary(args: &Value) -> String {
    let Some(object) = args.as_object() else {
        return scalar_text(args)
            .map(|text| truncate_chars(&text, SUMMARY_VALUE_CHARS))
            .unwrap_or_default();
    };
    const PREFERRED: &[&str] = &[
        "path",
        "file_path",
        "pattern",
        "glob",
        "query",
        "command",
        "url",
    ];
    let mut parts = Vec::new();
    for key in PREFERRED {
        if let Some(text) = object.get(*key).and_then(scalar_text) {
            parts.push(format!(
                "{key}={}",
                truncate_chars(&text, SUMMARY_VALUE_CHARS)
            ));
        }
    }
    if parts.is_empty() {
        for (key, value) in object.iter().take(3) {
            if let Some(text) = scalar_text(value) {
                parts.push(format!(
                    "{key}={}",
                    truncate_chars(&text, SUMMARY_VALUE_CHARS)
                ));
            }
        }
    }
    parts.join(" ")
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn outcome_label(result: &ToolCallResult) -> &'static str {
    if let Some(error) = &result.error {
        return match error.kind {
            ToolErrorKind::InvalidInput => "invalid_input",
            ToolErrorKind::NotFound => "not_found",
            ToolErrorKind::PermissionDenied => "permission_denied",
            ToolErrorKind::Conflict => "conflict",
            ToolErrorKind::Unavailable => "unavailable",
            ToolErrorKind::Timeout => "timeout",
            ToolErrorKind::Cancelled => "cancelled",
            ToolErrorKind::Io => "io",
            ToolErrorKind::OutputLimit => "output_limit",
            ToolErrorKind::InvalidOutput => "invalid_output",
            ToolErrorKind::Internal => "internal",
        };
    }
    match result.level {
        ToolSignalLevel::Ok => "ok",
        ToolSignalLevel::Warning => "warning",
        ToolSignalLevel::Error => "error",
    }
}

/// Tool failures resolve. They are not host rejections, so the program can continue.
fn tool_result_value(result: &ToolCallResult) -> Value {
    if result.error.is_some() || result.level == ToolSignalLevel::Error {
        json!({
            "status": "error",
            "data": null,
            "error": result.error,
        })
    } else {
        let status = match result.level {
            ToolSignalLevel::Ok => "ok",
            ToolSignalLevel::Warning => "warning",
            ToolSignalLevel::Error => "error",
        };
        json!({
            "status": status,
            "data": result.data,
            "error": null,
        })
    }
}

fn render_run_output(
    ledger: &str,
    outcome: &crate::ptc::Outcome,
    run_timeout_ms: u64,
) -> ToolCallResult {
    let logs: Vec<String> = outcome
        .logs
        .iter()
        .map(|entry| entry.message.clone())
        .collect();
    let failure_text = outcome.failure.as_ref().map(|failure| match failure.kind {
        FailureKind::Timeout => format!("run_code timed out after {run_timeout_ms}ms"),
        FailureKind::Cancelled => "code execution cancelled".to_string(),
        FailureKind::OutputLimit => failure.message.clone(),
        FailureKind::IllegalOutput => failure.message.clone(),
        FailureKind::Syntax => failure.message.clone(),
        _ => failure.message.clone(),
    });
    // A failed program does not present its value. `Json` drops that tree on the heap.
    let presented = if outcome.failure.is_none() {
        match outcome.value.as_ref().map(present_program_value) {
            Some(Ok(presented)) => Some(presented),
            Some(Err(message)) => {
                return ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidOutput,
                    message,
                    false,
                );
            }
            None => None,
        }
    } else {
        None
    };
    let content = compose_output(
        ledger,
        &logs,
        presented.as_ref().map(|presented| presented.text.as_str()),
        failure_text.as_deref(),
    );

    if let Some(failure) = &outcome.failure {
        let (kind, retryable) = match failure.kind {
            FailureKind::Syntax => (ToolErrorKind::InvalidInput, false),
            FailureKind::Exception => (ToolErrorKind::Internal, false),
            FailureKind::Timeout => (ToolErrorKind::Timeout, true),
            FailureKind::Cancelled => (ToolErrorKind::Cancelled, false),
            FailureKind::ResourceLimit => (ToolErrorKind::Internal, false),
            FailureKind::OutputLimit => (ToolErrorKind::OutputLimit, true),
            FailureKind::IllegalOutput => (ToolErrorKind::InvalidOutput, false),
            FailureKind::Internal => (ToolErrorKind::Internal, false),
        };
        return ToolCallResult::error_with_kind(kind, content, retryable);
    }

    let mut data = Map::new();
    data.insert(
        "logs".to_string(),
        Value::Array(logs.into_iter().map(Value::String).collect()),
    );
    if let Some(presented) = presented {
        data.insert("result".to_string(), presented.data);
    }
    ToolCallResult::ok(content).with_data(Value::Object(data))
}

struct PresentedResult {
    text: String,
    /// Shallow JSON. A deep program result is the encoded string, not the tree.
    data: Value,
}

fn present_program_value(value: &Json) -> Result<PresentedResult, String> {
    let raw = value.as_value();
    if let Value::String(text) = raw {
        return Ok(PresentedResult {
            text: text.clone(),
            data: Value::String(text.clone()),
        });
    }
    if exceeds_owned_depth(raw) {
        let text = encode_json(raw)?;
        return Ok(PresentedResult {
            text: text.clone(),
            data: Value::String(text),
        });
    }
    // The heap walk above proved this tree is shallow enough for derived Drop.
    let text = serde_json::to_string_pretty(raw)
        .or_else(|_| serde_json::to_string(raw))
        .map_err(|error| error.to_string())?;
    Ok(PresentedResult {
        text,
        data: raw.clone(),
    })
}

fn compose_output(
    ledger: &str,
    logs: &[String],
    result: Option<&str>,
    failure: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    if !ledger.is_empty() {
        parts.push(ledger.to_string());
    }
    if !logs.is_empty() {
        parts.push(logs.join("\n"));
    }
    if let Some(failure) = failure {
        parts.push(failure.to_string());
    } else if let Some(result) = result {
        parts.push(result.to_string());
    }
    if parts.is_empty() {
        "(run_code completed with no output)".to_string()
    } else {
        parts.join("\n\n")
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
                    "description": "Total program budget. Time spent waiting for approval is not counted. Defaults to 120000; values above 600000 are clamped."
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
        // The runtime output cap is the only hard limit. The outer pipeline may
        // spill this presentation; it does not truncate it again.
        usize::MAX
    }
}
