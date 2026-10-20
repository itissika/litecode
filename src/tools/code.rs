//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::collections::{BTreeMap, HashMap, HashSet};

mod render;
pub use render::RUN_CODE_VIEW_MAX_BYTES;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskPrompt, AskReply, PermissionEngine, PermissionSink};
use crate::ptc::{
    self, BudgetPause, CallInfo, CallState, ExecuteRequest, FailureKind, HostBinding, HostFn, Json,
    Limits, LogLevel, Namespace,
};
use crate::tool::Tool;
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::WorkspaceWriteLock;
use crate::tool::outcome::{self, Diagnostic};
use crate::types::utf8::prefix_utf8;
use crate::types::{FunctionToolCall, ToolCallResult, ToolErrorKind, ToolSignalLevel};

/// Model-facing name of the aggregated code transport.
pub const CODE_TOOL_NAME: &str = "run_code";
/// Default total program budget.
pub const DEFAULT_RUN_TIMEOUT_MS: u64 = 120_000;
/// Maximum total program budget accepted from the model.
pub const MAX_RUN_TIMEOUT_MS: u64 = 600_000;
/// Runtime output cap. This is the only hard cap; the outer result is not truncated again.
pub const MAX_CODE_OUTPUT_BYTES: usize = 1_048_576;
/// Full ledger lines kept in submission order. Later calls are counted, and a
/// bounded priority pool keeps errors, rejections, unsettled calls, and partial edits.
const LEDGER_KEEP: usize = 256;
const PRIORITY_CAP: usize = 64;
const ERROR_MESSAGE_CAP: usize = 300;
/// Nested diagnostics kept for the model view. Only the four outer codes qualify.
const DIAGNOSTIC_CAP: usize = 16;
const DIAGNOSTIC_CODES: &[&str] = &[
    "lsp_error",
    "edit_block_failed",
    "path_outside_workspace",
    "sensitive_path",
];
const LSP_TEXT_CAP: usize = 1024;
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
    /// Display state: `ok`, `ok(a/b)`, `error:{code}`, `cancelled`, `unsettled`.
    outcome: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    diagnostics: Vec<Diagnostic>,
    session_read: bool,
    priority: bool,
}

struct PendingCall {
    tool: String,
    summary: String,
    session_read: bool,
}

struct LedgerSnapshot {
    lines: Vec<LedgerLine>,
    omitted: usize,
    omitted_diagnostics: usize,
    tool_counts: BTreeMap<String, usize>,
    error_counts: BTreeMap<String, usize>,
    session_read_hidden: bool,
}

struct CallLedger {
    lines: Mutex<Vec<LedgerLine>>,
    pending: Mutex<HashMap<u64, PendingCall>>,
    omitted: AtomicUsize,
    omitted_diagnostics: AtomicUsize,
    priority_used: AtomicUsize,
    per_tool: Mutex<BTreeMap<String, usize>>,
    per_error: Mutex<BTreeMap<String, usize>>,
    session_read_hidden: AtomicBool,
}

impl CallLedger {
    fn new() -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            pending: Mutex::new(HashMap::new()),
            omitted: AtomicUsize::new(0),
            omitted_diagnostics: AtomicUsize::new(0),
            priority_used: AtomicUsize::new(0),
            per_tool: Mutex::new(BTreeMap::new()),
            per_error: Mutex::new(BTreeMap::new()),
            session_read_hidden: AtomicBool::new(false),
        }
    }

    fn begin(&self, seq: u64, tool: &str, summary: &str) {
        {
            let mut counts = self
                .per_tool
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            *counts.entry(tool.to_string()).or_default() += 1;
        }
        let session_read = call_reads_session(tool, summary);
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if lines.len() < LEDGER_KEEP {
            lines.push(LedgerLine {
                seq,
                tool: tool.to_string(),
                args_summary: summary.to_string(),
                outcome: None,
                error_code: None,
                error_message: None,
                diagnostics: Vec::new(),
                session_read,
                priority: false,
            });
            return;
        }
        drop(lines);
        self.omitted.fetch_add(1, Ordering::Relaxed);
        self.pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                seq,
                PendingCall {
                    tool: tool.to_string(),
                    summary: summary.to_string(),
                    session_read,
                },
            );
    }

    fn record(&self, seq: u64, result: &ToolCallResult) {
        let outcome = call_outcome_label(result);
        let error_code = call_error_code(result);
        let error_message = call_error_message(result);
        if let Some(code) = &error_code {
            let mut counts = self
                .per_error
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            *counts.entry(code.clone()).or_default() += 1;
        }
        let diagnostics = bounded_diagnostics(&result.diagnostics);
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(line) = lines.iter_mut().find(|line| line.seq == seq) {
            if line.outcome.is_none() {
                line.outcome = Some(outcome);
                line.error_code = error_code;
                line.error_message = error_message;
                line.diagnostics = diagnostics;
            }
            return;
        }
        drop(lines);
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&seq);
        let Some(pending) = pending else {
            self.omitted_diagnostics
                .fetch_add(diagnostics.len(), Ordering::Relaxed);
            return;
        };
        if is_priority_state(&outcome)
            && self.priority_used.load(Ordering::Relaxed) < PRIORITY_CAP
        {
            self.priority_used.fetch_add(1, Ordering::Relaxed);
            self.omitted.fetch_sub(1, Ordering::Relaxed);
            self.lines
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(LedgerLine {
                    seq,
                    tool: pending.tool,
                    args_summary: pending.summary,
                    outcome: Some(outcome),
                    error_code,
                    error_message,
                    diagnostics,
                    session_read: pending.session_read,
                    priority: true,
                });
            return;
        }
        if pending.session_read {
            self.session_read_hidden.store(true, Ordering::Relaxed);
        }
        self.omitted_diagnostics
            .fetch_add(diagnostics.len(), Ordering::Relaxed);
    }

    fn snapshot(&self) -> LedgerSnapshot {
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let pending: Vec<(u64, PendingCall)> = self
            .pending
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drain()
            .collect();
        let mut omitted = self.omitted.load(Ordering::Relaxed);
        let mut priority_used = self.priority_used.load(Ordering::Relaxed);
        let mut session_read_hidden = self.session_read_hidden.load(Ordering::Relaxed);
        for (seq, pending) in pending {
            if priority_used < PRIORITY_CAP {
                priority_used += 1;
                omitted = omitted.saturating_sub(1);
                lines.push(LedgerLine {
                    seq,
                    tool: pending.tool,
                    args_summary: pending.summary,
                    outcome: Some("unsettled".to_string()),
                    error_code: None,
                    error_message: None,
                    diagnostics: Vec::new(),
                    session_read: pending.session_read,
                    priority: true,
                });
            } else if pending.session_read {
                session_read_hidden = true;
            }
        }
        lines.sort_by_key(|line| line.seq);
        LedgerSnapshot {
            lines,
            omitted,
            omitted_diagnostics: self.omitted_diagnostics.load(Ordering::Relaxed),
            tool_counts: self
                .per_tool
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
            error_counts: self
                .per_error
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
            session_read_hidden,
        }
    }
}

fn call_reads_session(tool: &str, summary: &str) -> bool {
    if tool == "session_search" {
        return true;
    }
    matches!(tool, "read" | "grep" | "glob")
        && crate::session::transcript_file::is_session_target(summary)
}

fn is_priority_state(state: &str) -> bool {
    state == "cancelled"
        || state == "unsettled"
        || state.starts_with("error:")
        || state.starts_with("ok(")
}

fn call_outcome_label(result: &ToolCallResult) -> String {
    if result
        .error
        .as_ref()
        .is_some_and(|error| error.kind == ToolErrorKind::Cancelled)
    {
        return "cancelled".to_string();
    }
    if result.error.is_none()
        && result.level != ToolSignalLevel::Error
        && let Some((applied, total)) = edit_counts(result)
    {
        return format!("ok({applied}/{total})");
    }
    if result.error.is_some() || result.level == ToolSignalLevel::Error {
        let code = call_error_code(result).unwrap_or_else(|| "error".to_string());
        return format!("error:{code}");
    }
    "ok".to_string()
}

fn edit_counts(result: &ToolCallResult) -> Option<(u64, u64)> {
    if result.error.is_some() || result.level == ToolSignalLevel::Error {
        return None;
    }
    let data = result.data.as_ref()?;
    let applied = data.get("applied")?.as_u64()?;
    let failed = data.get("failed").and_then(Value::as_u64).unwrap_or(0);
    let total = applied.saturating_add(failed);
    if failed == 0 || total == 0 {
        return None;
    }
    Some((applied, total))
}

fn call_error_code(result: &ToolCallResult) -> Option<String> {
    let failed = result.error.is_some() || result.level == ToolSignalLevel::Error;
    if !failed {
        return None;
    }
    if result
        .error
        .as_ref()
        .is_some_and(|error| error.kind == ToolErrorKind::Cancelled)
    {
        return None;
    }
    if let Some(code) = &result.error_code {
        return Some(code.clone());
    }
    result
        .error
        .as_ref()
        .map(|error| outcome::error_kind_code(error.kind).to_string())
}

fn call_error_message(result: &ToolCallResult) -> Option<String> {
    let message = result.error.as_ref().map(|error| error.message.as_str())?;
    let message = prefix_utf8(message, ERROR_MESSAGE_CAP);
    if message.is_empty() {
        None
    } else {
        Some(message.to_string())
    }
}

fn bounded_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|diagnostic| DIAGNOSTIC_CODES.contains(&diagnostic.code.as_str()))
        .cloned()
        .map(truncate_lsp_text)
        .collect()
}

fn truncate_lsp_text(mut diagnostic: Diagnostic) -> Diagnostic {
    if diagnostic.code != "lsp_error" {
        return diagnostic;
    }
    let Some(details) = diagnostic.details.as_mut() else {
        return diagnostic;
    };
    let Some(text) = details.get("text").and_then(Value::as_str) else {
        return diagnostic;
    };
    let cut = prefix_utf8(text, LSP_TEXT_CAP);
    if cut.len() != text.len() {
        details["text"] = Value::String(cut.to_string());
    }
    diagnostic
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
                    function: HostFn::new(move |args, cancel, pause, info: CallInfo| {
                        let name = name.clone();
                        let bundle = Arc::clone(&bundle);
                        let host = Arc::clone(&host);
                        let ledger = Arc::clone(&ledger);
                        let session_id = session_id.clone();
                        let parent_call_id = parent_call_id.clone();
                        let seq = info.seq;
                        async move {
                            invoke_binding(
                                name,
                                seq,
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
            source: code.clone(),
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
        render_run_output(&ledger, &outcome, run_timeout_ms, &code)
    }
}

#[allow(clippy::too_many_arguments)]
async fn invoke_binding(
    name: String,
    seq: u64,
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
            ledger.begin(seq, &name, &summary);
            let index = seq;
            ledger.record(index, &protocol_failure("tool bindings accept one argument"));
            return Err(crate::ptc::HostError::type_error(
                "tool bindings accept one argument",
            ));
        }
        Err(DecodeFail::Illegal(illegal)) => {
            ledger.begin(seq, &name, &illegal.summary);
            let index = seq;
            ledger.record(index, &protocol_failure(&illegal.message));
            return Err(crate::ptc::HostError::type_error(illegal.message));
        }
    };
    ledger.begin(seq, &name, &summary);
    let index = seq;

    if !bundle.iter().any(|tool| tool.name() == name) {
        let message = format!("tool '{name}' not found");
        ledger.record(index, &protocol_failure(&message));
        return Err(crate::ptc::HostError::type_error(message));
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
    outcome::program_result(&result)
}

fn protocol_failure(message: &str) -> ToolCallResult {
    let mut result = ToolCallResult::error_with_kind(
        ToolErrorKind::InvalidInput,
        message.to_string(),
        false,
    );
    result.error_code = Some("invalid_input".to_string());
    result
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

fn render_run_output(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    run_timeout_ms: u64,
    source: &str,
) -> ToolCallResult {
    let failure_text = outcome.failure.as_ref().map(|failure| match failure.kind {
        FailureKind::Timeout => format!("run_code timed out after {run_timeout_ms}ms"),
        FailureKind::Cancelled if outcome.value.is_some() => {
            "cancelled after the program returned".to_string()
        }
        FailureKind::Cancelled => "code execution cancelled".to_string(),
        FailureKind::OutputLimit => failure.message.clone(),
        FailureKind::IllegalOutput => {
            format!("program result is not JSON: {}", failure.message)
        }
        FailureKind::Syntax => failure.message.clone(),
        _ => failure.message.clone(),
    });
    // A failed program does not present its value. `Json` drops that tree on the heap.
    let keep_value = outcome.value.is_some()
        && outcome.failure.as_ref().is_none_or(|failure| {
            failure.kind == FailureKind::Cancelled
        });
    let presented = if keep_value {
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
                    source,
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
        source,
    ));

    if let Some(failure) = &outcome.failure {
        let (kind, retryable, code) = if let Some(info) = &failure.tool_error {
            (
                outcome::kind_for_code(&info.code),
                false,
                Some(info.code.clone()),
            )
        } else {
            match failure.kind {
                FailureKind::Syntax => (ToolErrorKind::InvalidInput, false, None),
                FailureKind::Exception => {
                    (ToolErrorKind::Internal, false, Some("exception".to_string()))
                }
                FailureKind::Timeout => (ToolErrorKind::Timeout, true, None),
                FailureKind::Cancelled => (ToolErrorKind::Cancelled, false, None),
                FailureKind::ResourceLimit => (ToolErrorKind::Internal, false, None),
                FailureKind::OutputLimit => (ToolErrorKind::OutputLimit, true, None),
                FailureKind::IllegalOutput => (ToolErrorKind::InvalidOutput, false, None),
                FailureKind::Internal => (ToolErrorKind::Internal, false, None),
            }
        };
        let mut result = ToolCallResult::error_with_kind(kind, content, retryable);
        result.error_code = code;
        return result;
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
    source: &str,
) -> render::RunReport {
    let snapshot = ledger.snapshot();
    let lines = &snapshot.lines;
    let calls_omitted = snapshot.omitted;
    let omitted_diagnostics = snapshot.omitted_diagnostics;
    let mut calls = Vec::with_capacity(lines.len());
    for line in lines {
        let record = outcome.calls.iter().find(|call| call.seq == line.seq);
        calls.push(render::LedgerSnap {
            seq: line.seq,
            tool: line.tool.clone(),
            summary: line.args_summary.clone(),
            state: merge_call_outcome(line.outcome.as_deref(), record.map(|call| call.state)),
            error_code: line.error_code.clone(),
            error_message: line.error_message.clone(),
            session_read: line.session_read,
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
        tool_counts: snapshot.tool_counts.clone(),
        error_counts: snapshot.error_counts.clone(),
        call_total: Some(outcome.calls.len()),
        unsettled_total: Some(
            outcome
                .calls
                .iter()
                .filter(|call| call.state == CallState::CancelledUnfinished)
                .count(),
        ),
        session_read_hidden: snapshot.session_read_hidden,
        failure_location: outcome
            .failure
            .as_ref()
            .and_then(|failure| failure.location),
        failure_at_end: outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.at_end_of_input),
        failure_stack: outcome
            .failure
            .as_ref()
            .and_then(|failure| failure.stack.clone()),
        source: source.to_string(),
        tool_error: outcome.failure.as_ref().and_then(|failure| {
            let info = failure.tool_error.as_ref()?;
            let summary = lines
                .iter()
                .find(|line| line.seq == info.seq)
                .map(|line| line.args_summary.clone())
                .unwrap_or_default();
            Some(render::UncaughtToolError {
                seq: info.seq,
                tool: if info.tool.is_empty() {
                    lines
                        .iter()
                        .find(|line| line.seq == info.seq)
                        .map(|line| line.tool.clone())
                        .unwrap_or_default()
                } else {
                    info.tool.clone()
                },
                code: info.code.clone(),
                message: failure.message.clone(),
                summary,
                details: info.details.clone(),
            })
        }),
        uncaught_type_error: outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.uncaught_type_error),
    }
}

fn aggregate_diagnostics(lines: &[LedgerLine], already_omitted: usize) -> (Vec<Diagnostic>, usize) {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut omitted = already_omitted;
    for line in lines {
        for diagnostic in &line.diagnostics {
            if !DIAGNOSTIC_CODES.contains(&diagnostic.code.as_str()) {
                continue;
            }
            let path = diagnostic
                .details
                .as_ref()
                .and_then(|details| details.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let key = format!("{}|{path}|{}", diagnostic.code, diagnostic.message);
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
        Some(outcome) if !outcome.is_empty() => outcome.to_string(),
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
                    "description": "Async JavaScript function body. `await tools.name(args)` resolves to that tool's data. A domain failure rejects ToolError; a protocol failure throws TypeError. Use `libs` for pure-JavaScript analysis helpers. Return a JSON-serializable value: `undefined` is not JSON, so use `null` or omit the property."
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
        ledger.begin(1, "write", "path=a.txt");
        let index = 1u64;
        ledger.record(index, result);
        render_run_output(&ledger, outcome, 120_000, "").content
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
        assert_eq!(ledger, "1. write path=a.txt error:permission_denied", "{text}");
        assert!(
            text.contains("#1 write path=a.txt: permission_denied: denied"),
            "{text}"
        );
        assert!(!text.contains("@@run_code_data"), "{text}");
    }

    #[test]
    fn unknown_host_outcome_is_unsettled() {
        assert_eq!(merge_call_outcome(None, Some(CallState::Completed)), "unsettled");
        assert_eq!(merge_call_outcome(Some(""), None), "unsettled");
        assert_eq!(
            merge_call_outcome(Some("error:timeout"), None),
            "error:timeout"
        );
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

    #[test]
    fn ledger_keeps_a_priority_row_past_the_head_and_marks_a_hidden_session_read() {
        let ledger = CallLedger::new();
        for seq in 1..=256 {
            ledger.begin(seq, "read", "path=probe.txt");
            ledger.record(seq, &ToolCallResult::ok("ok"));
        }
        ledger.begin(300, "read", "path=.litecode/sessions/abc.md");
        ledger.record(300, &ToolCallResult::ok("session"));
        ledger.begin(301, "write", "path=a.txt");
        ledger.record(
            301,
            &ToolCallResult::error_with_kind(ToolErrorKind::NotFound, "missing", false),
        );
        let snapshot = ledger.snapshot();
        assert!(snapshot.lines.iter().any(|line| line.seq == 301));
        assert!(!snapshot.lines.iter().any(|line| line.seq == 300));
        assert!(snapshot.session_read_hidden);
        assert_eq!(snapshot.tool_counts.get("read").copied(), Some(257));
        assert_eq!(snapshot.error_counts.get("not_found").copied(), Some(1));

        let mut result = ToolCallResult::ok("edited");
        result.data = Some(json!({"applied": 9, "failed": 1}));
        let edit = CallLedger::new();
        edit.begin(1, "edit", "path=note.txt");
        edit.record(1, &result);
        let edit_line = &edit.snapshot().lines[0];
        assert_eq!(edit_line.outcome.as_deref(), Some("ok(9/10)"));
    }

    #[test]
    fn omitted_session_read_marks_the_ledger_and_states_the_count() {
        let ledger = CallLedger::new();
        let mut calls = Vec::new();
        for seq in 1..=466 {
            let summary = if seq == 300 {
                "path=.litecode/sessions/abc.md"
            } else {
                "path=probe.txt"
            };
            ledger.begin(seq, "read", summary);
            ledger.record(seq, &ToolCallResult::ok("ok"));
            calls.push(CallRecord {
                seq,
                namespace: "tools".into(),
                name: "read".into(),
                started: Duration::from_millis(1),
                ended: Some(Duration::from_millis(2)),
                state: CallState::Completed,
            });
        }
        let outcome = Outcome {
            logs: Vec::new(),
            value: Some(Json::new(json!("done"))),
            failure: None,
            calls,
        };
        let text = render_run_output(&ledger, &outcome, 120_000, "").content;
        let ledger_text = text
            .split_once("calls (")
            .expect("ledger")
            .1;
        let ledger_section = ledger_text.split_once("\n\n").map(|(head, _)| head).unwrap_or(ledger_text);
        assert!(ledger_section.len() <= 8 * 1024, "{}", ledger_section.len());
        assert!(text.contains("of 466"), "{text}");
        assert!(text.contains("read×466"), "{text}");
        assert!(text.contains(crate::session::transcript_file::RUN_CODE_SESSION_NOTE), "{text}");
        assert!(crate::session::transcript_file::ledger_reads_sessions(&text));
    }
}
