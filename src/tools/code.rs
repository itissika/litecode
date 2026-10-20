//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::collections::HashSet;

mod render;
pub use render::RUN_CODE_VIEW_MAX_BYTES;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskPrompt, AskReply, PermissionEngine, PermissionSink};
use crate::ptc::{
    self, BudgetPause, CallState, ExecuteRequest, FailureKind, HostBinding, HostFn, Json, Limits,
    LogLevel, Namespace,
};
use crate::tool::Tool;
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tool::outcome::{self, Diagnostic};
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
/// Nested diagnostics kept for the model view. Further diagnostics are counted.
const DIAGNOSTIC_CAP: usize = 32;
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

/// Host side of one nested call. Merged with the runtime [`crate::ptc::CallRecord`].
#[derive(Clone)]
struct LedgerLine {
    seq: u64,
    tool: String,
    args_summary: String,
    /// `ok` / `partial` / `error` / `cancelled` / `unsettled` once the host call returns.
    outcome: Option<String>,
    error_code: Option<String>,
    diagnostics: Vec<Diagnostic>,
}

struct CallLedger {
    next: AtomicU64,
    lines: Mutex<Vec<LedgerLine>>,
    omitted: AtomicUsize,
    omitted_diagnostics: AtomicUsize,
}

impl CallLedger {
    fn new() -> Self {
        Self {
            next: AtomicU64::new(1),
            lines: Mutex::new(Vec::new()),
            omitted: AtomicUsize::new(0),
            omitted_diagnostics: AtomicUsize::new(0),
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
        lines.push(LedgerLine {
            seq: index,
            tool: tool.to_string(),
            args_summary: summary.to_string(),
            outcome: None,
            error_code: None,
            diagnostics: Vec::new(),
        });
        index
    }

    fn record(&self, index: u64, result: &ToolCallResult) {
        let outcome = call_outcome_label(result).to_string();
        let error_code = call_error_code(result);
        let diagnostics = result.diagnostics.clone();
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(line) = lines.iter_mut().find(|line| line.seq == index) {
            if line.outcome.is_none() {
                line.outcome = Some(outcome);
                line.error_code = error_code;
                line.diagnostics = diagnostics;
            }
        } else {
            self.omitted_diagnostics
                .fetch_add(diagnostics.len(), Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> (Vec<LedgerLine>, usize, usize) {
        let lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        (
            lines,
            self.omitted.load(Ordering::Relaxed),
            self.omitted_diagnostics.load(Ordering::Relaxed),
        )
    }
}

fn call_outcome_label(result: &ToolCallResult) -> &'static str {
    if result
        .error
        .as_ref()
        .is_some_and(|error| error.kind == ToolErrorKind::Cancelled)
    {
        return "cancelled";
    }
    if result.outcome_status == Some(outcome::OutcomeStatus::Partial)
        && result.error.is_none()
        && result.level != ToolSignalLevel::Error
    {
        return "partial";
    }
    if result.error.is_some()
        || result.level == ToolSignalLevel::Error
        || result.outcome_status == Some(outcome::OutcomeStatus::Error)
    {
        return "error";
    }
    "ok"
}

fn call_error_code(result: &ToolCallResult) -> Option<String> {
    if call_outcome_label(result) != "error" {
        return None;
    }
    if let Some(code) = &result.error_code {
        return Some(code.clone());
    }
    result
        .error
        .as_ref()
        .map(|error| outcome::error_kind_code(error.kind))
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
        render_run_output(&ledger, &outcome, run_timeout_ms)
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
    let (arguments, summary) = match decoded {
        Ok(decoded) => (decoded.arguments, decoded.summary),
        Err(DecodeFail::ExtraArgument { summary }) => {
            let index = ledger.begin(&name, &summary);
            let result = ToolCallResult::error_with_kind(
                ToolErrorKind::InvalidInput,
                "tool bindings accept one argument",
                false,
            );
            ledger.record(index, &result);
            return Ok(tool_result_value(&result));
        }
        Err(DecodeFail::Illegal(illegal)) => {
            let index = ledger.begin(&name, &illegal.summary);
            let result = ToolCallResult::error_with_kind(
                ToolErrorKind::InvalidOutput,
                illegal.message,
                false,
            );
            ledger.record(index, &result);
            return Ok(tool_result_value(&result));
        }
    };
    let index = ledger.begin(&name, &summary);

    if !bundle.iter().any(|tool| tool.name() == name) {
        let result = ToolCallResult::error_with_kind(
            ToolErrorKind::NotFound,
            format!("tool '{name}' not found"),
            false,
        );
        ledger.record(index, &result);
        return Ok(tool_result_value(&result));
    }

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
    let result = host
        .run_nested(
            bundle.as_slice(),
            &call,
            &session_id,
            cancel.child_token(),
            &parent_call_id,
            &sink,
        )
        .await;
    ledger.record(index, &result);
    Ok(tool_result_value(&result))
}

#[derive(Debug)]
struct DecodedInvocation {
    arguments: String,
    summary: String,
}

#[derive(Debug)]
struct IllegalInvocation {
    summary: String,
    message: String,
}

#[derive(Debug)]
enum DecodeFail {
    ExtraArgument { summary: String },
    Illegal(IllegalInvocation),
}

/// Borrow the runtime JSON. A deep argument is rejected here so the tool
/// pipeline never builds a `serde_json::Value` it would later drop recursively.
/// Two or more JavaScript arguments arrive as one JSON array. Bindings accept
/// exactly one argument.
fn decode_invocation(payload: &Json) -> Result<DecodedInvocation, DecodeFail> {
    let value = payload.as_value();
    if let Value::Array(items) = value
        && items.len() >= 2
    {
        let summary = items
            .first()
            .map(key_arg_summary)
            .filter(|summary| !summary.is_empty())
            .unwrap_or_else(|| "extra argument".to_string());
        return Err(DecodeFail::ExtraArgument { summary });
    }
    let summary = key_arg_summary(value);
    if exceeds_owned_depth(value) {
        return Err(DecodeFail::Illegal(IllegalInvocation {
            summary,
            message: "tool arguments are nested too deeply to enter the host pipeline".to_string(),
        }));
    }
    let arguments = match encode_json(value) {
        Ok(arguments) => arguments,
        Err(message) => {
            return Err(DecodeFail::Illegal(IllegalInvocation { summary, message }));
        }
    };
    Ok(DecodedInvocation { arguments, summary })
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

/// Tool failures resolve. They are not host rejections, so the program can continue.
fn tool_result_value(result: &ToolCallResult) -> Value {
    outcome::program_value(result)
}

fn render_run_output(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    run_timeout_ms: u64,
) -> ToolCallResult {
    let failure_text = outcome.failure.as_ref().map(|failure| match failure.kind {
        FailureKind::Timeout => format!("run_code timed out after {run_timeout_ms}ms"),
        FailureKind::Cancelled => "code execution cancelled".to_string(),
        FailureKind::OutputLimit => failure.message.clone(),
        FailureKind::IllegalOutput => {
            format!("program result is not JSON: {}", failure.message)
        }
        FailureKind::Syntax => failure.message.clone(),
        _ => failure.message.clone(),
    });
    // A failed program does not present its value. `Json` drops that tree on the heap.
    let presented = if outcome.failure.is_none() {
        match outcome.value.as_ref().map(present_program_value) {
            Some(Ok(presented)) => Some(presented),
            Some(Err(message)) => {
                let content = render::render_model_view(&run_report(
                    ledger,
                    outcome,
                    None,
                    Some(message),
                    Some(FailureKind::IllegalOutput),
                    None,
                    run_timeout_ms,
                ));
                return ToolCallResult::error_with_kind(ToolErrorKind::InvalidOutput, content, false);
            }
            None => None,
        }
    } else {
        None
    };
    let failure_kind = outcome.failure.as_ref().map(|failure| failure.kind);
    let resource = outcome.failure.as_ref().and_then(|failure| failure.resource);
    let content = render::render_model_view(&run_report(
        ledger,
        outcome,
        presented,
        failure_text,
        failure_kind,
        resource,
        run_timeout_ms,
    ));

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

    ToolCallResult::ok(content)
}

fn run_report(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    value: Option<Value>,
    failure_text: Option<String>,
    failure_kind: Option<FailureKind>,
    resource: Option<crate::ptc::ResourceKind>,
    timeout_ms: u64,
) -> render::RunReport {
    let (lines, calls_omitted, omitted_diagnostics) = ledger.snapshot();
    let mut calls = Vec::with_capacity(lines.len());
    for line in &lines {
        let record = outcome.calls.iter().find(|call| call.seq == line.seq);
        calls.push(render::LedgerSnap {
            seq: line.seq,
            tool: line.tool.clone(),
            summary: line.args_summary.clone(),
            state: merge_call_outcome(line.outcome.as_deref(), record.map(|call| call.state)),
            error_code: line.error_code.clone(),
        });
    }
    let (diagnostics, diagnostics_omitted) = aggregate_diagnostics(&lines, omitted_diagnostics);
    render::RunReport {
        logs: outcome
            .logs
            .iter()
            .map(|entry| (log_level_name(entry.level).to_string(), entry.message.clone()))
            .collect(),
        value,
        failure_text,
        failure_kind,
        resource,
        timeout_ms,
        output_limited: failure_kind == Some(FailureKind::OutputLimit),
        calls,
        calls_omitted,
        diagnostics,
        diagnostics_omitted,
    }
}

fn aggregate_diagnostics(lines: &[LedgerLine], already_omitted: usize) -> (Vec<Diagnostic>, usize) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut omitted = already_omitted;
    for line in lines {
        for diagnostic in &line.diagnostics {
            let key = format!(
                "{}|{}|{}",
                diagnostic.code,
                diagnostic.message,
                diagnostic
                    .details
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default()
            );
            if !seen.insert(key) {
                continue;
            }
            if out.len() >= DIAGNOSTIC_CAP {
                omitted += 1;
            } else {
                out.push(diagnostic.clone());
            }
        }
    }
    (out, omitted)
}

/// A call that is still running when the grace period ends is `unsettled`.
/// A call that returned keeps the host outcome, including a write that
/// committed after cancellation was signaled.
fn merge_call_outcome(host: Option<&str>, state: Option<CallState>) -> String {
    if matches!(state, Some(CallState::CancelledUnfinished)) {
        return "unsettled".to_string();
    }
    match host {
        Some(outcome @ ("ok" | "partial" | "error" | "cancelled" | "unsettled")) => {
            outcome.to_string()
        }
        _ => "unsettled".to_string(),
    }
}

fn log_level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Log => "log",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
        LogLevel::Debug => "debug",
    }
}

fn present_program_value(value: &Json) -> Result<Value, String> {
    let raw = value.as_value();
    if let Value::String(text) = raw {
        return Ok(Value::String(text.clone()));
    }
    if exceeds_owned_depth(raw) {
        return Ok(Value::String(encode_json(raw)?));
    }
    // The heap walk above proved this tree is shallow enough for derived Drop.
    serde_json::to_string(raw).map_err(|error| error.to_string())?;
    Ok(raw.clone())
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
                    "description": "Async JavaScript function body. Use `tools` for host tools and `libs` for pure-JavaScript analysis helpers. Return a JSON-serializable value: `undefined` is not JSON, so use `null` or omit the property."
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
        // render 保证预算，外层不 spill
        usize::MAX
    }
}

#[cfg(test)]
mod outcome_tests {
    use std::time::Duration;

    use serde_json::json;

    use crate::ptc::{CallRecord, CallState, Failure, Json, Outcome};
    use crate::types::{ToolCallResult, ToolErrorKind};

    use super::{decode_invocation, merge_call_outcome, render_run_output, CallLedger, DecodeFail};

    fn runtime(state: CallState) -> Outcome {
        Outcome {
            logs: Vec::new(),
            value: None,
            failure: Some(Failure::cancelled()),
            calls: vec![CallRecord {
                seq: 1,
                namespace: "tools".into(),
                name: "write".into(),
                started: Duration::from_millis(1),
                ended: if state == CallState::CancelledUnfinished {
                    None
                } else {
                    Some(Duration::from_millis(4))
                },
                state,
            }],
        }
    }

    fn rendered(result: &ToolCallResult, outcome: &Outcome) -> String {
        let ledger = CallLedger::new();
        let index = ledger.begin("write", "path=a.txt");
        ledger.record(index, result);
        render_run_output(&ledger, outcome, 120_000).content
    }

    #[test]
    fn unfinished_call_stays_unsettled_even_when_the_host_later_records_ok() {
        assert_eq!(
            merge_call_outcome(Some("ok"), Some(CallState::CancelledUnfinished)),
            "unsettled"
        );
        let text = rendered(
            &ToolCallResult::ok("wrote"),
            &runtime(CallState::CancelledUnfinished),
        );
        assert!(text.contains("1. write path=a.txt unsettled"), "{text}");
        assert!(!text.contains("1. write path=a.txt cancelled"), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
    }

    #[test]
    fn a_finished_call_keeps_the_host_outcome() {
        let text = rendered(
            &ToolCallResult::ok("wrote"),
            &runtime(CallState::CancelledFinished),
        );
        assert!(text.contains("1. write path=a.txt ok"), "{text}");

        let text = rendered(
            &ToolCallResult::error_with_kind(
                ToolErrorKind::Cancelled,
                "write cancelled before write; file was not modified",
                false,
            ),
            &runtime(CallState::CancelledFinished),
        );
        assert!(text.contains("1. write path=a.txt cancelled"), "{text}");
    }

    #[test]
    fn permission_denial_is_an_error_outcome_and_the_ledger_does_not_invent_the_code() {
        let completed = Outcome {
            logs: Vec::new(),
            value: None,
            failure: None,
            calls: vec![CallRecord {
                seq: 1,
                namespace: "tools".into(),
                name: "write".into(),
                started: Duration::from_millis(1),
                ended: Some(Duration::from_millis(2)),
                state: CallState::Completed,
            }],
        };
        let text = rendered(
            &ToolCallResult::error_with_kind(ToolErrorKind::PermissionDenied, "denied", false),
            &completed,
        );
        let ledger = text
            .lines()
            .find(|line| line.starts_with("1. write"))
            .unwrap_or("");
        assert_eq!(ledger, "1. write path=a.txt error", "{text}");
        assert!(
            !ledger.contains("permission_denied"),
            "the text ledger must not invent an error code: {ledger}"
        );
        assert!(text.contains("permission_denied"), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
    }

    #[test]
    fn unknown_host_outcome_is_unsettled() {
        assert_eq!(merge_call_outcome(None, Some(CallState::Completed)), "unsettled");
        assert_eq!(merge_call_outcome(Some("timeout"), None), "unsettled");
    }

    #[test]
    fn a_second_javascript_argument_is_rejected_before_the_tool_runs() {
        let payload = Json::new(json!([{"path": "a.txt"}, {"timeout_ms": 1}]));
        match decode_invocation(&payload) {
            Err(DecodeFail::ExtraArgument { summary }) => assert_eq!(summary, "path=a.txt"),
            other => panic!("expected the extra argument to be rejected: {other:?}"),
        }
        let one = Json::new(json!({"path": "a.txt"}));
        assert!(decode_invocation(&one).is_ok());
    }
}
