//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::collections::{BTreeMap, HashMap};

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
use crate::tool::code_contract::CodeSurface;
use crate::tool::executor::{ToolCallOrigin, ToolCaller, execute_tool_call};
use crate::tool::outcome;
use crate::tool::trait_::ToolExecutionContext;
use crate::tool::write_lock::WorkspaceWriteLock;
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
    write_lock: Arc<WorkspaceWriteLock>,
    session: Option<crate::session::SessionDataReader>,
    #[allow(clippy::type_complexity)]
    file_note: Option<Arc<dyn Fn(&[std::path::PathBuf]) + Send + Sync>>,
    plan_touch: Option<crate::tool::executor::PlanTouch>,
}

impl CodeToolHost {
    pub fn new(
        permission: PermissionEngine,
        ctx: Context,
        agent_name: impl Into<String>,
        permission_sink: Arc<dyn PermissionSink>,
        data_root: std::path::PathBuf,
        write_lock: Arc<WorkspaceWriteLock>,
        session: Option<crate::session::SessionDataReader>,
    ) -> Self {
        Self {
            permission,
            ctx,
            agent_name: agent_name.into(),
            permission_sink,
            data_root,
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
        deadline: Option<tokio::time::Instant>,
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
            // Nested output stays in the program. The outer render owns the byte budget.
            0,
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
                deadline,
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
    session_read: bool,
    /// One-line result digest for the failure view (WP-210 A).
    brief: Option<String>,
}

struct PendingCall {
    tool: String,
    summary: String,
    session_read: bool,
}

struct LedgerSnapshot {
    lines: Vec<LedgerLine>,
    omitted: usize,
    /// Settled per-tool ok/fail (unsettled & host-cancelled excluded).
    by_tool: BTreeMap<String, (usize, usize)>,
    error_counts: BTreeMap<String, usize>,
    session_read_hidden: bool,
}

struct CallLedger {
    lines: Mutex<Vec<LedgerLine>>,
    pending: Mutex<HashMap<u64, PendingCall>>,
    omitted: AtomicUsize,
    priority_used: AtomicUsize,
    /// Settled ok/fail per tool name. Host `cancelled` and unsettled are excluded.
    by_tool: Mutex<BTreeMap<String, (usize, usize)>>,
    per_error: Mutex<BTreeMap<String, usize>>,
    session_read_hidden: AtomicBool,
}

impl CallLedger {
    fn new() -> Self {
        Self {
            lines: Mutex::new(Vec::new()),
            pending: Mutex::new(HashMap::new()),
            omitted: AtomicUsize::new(0),
            priority_used: AtomicUsize::new(0),
            by_tool: Mutex::new(BTreeMap::new()),
            per_error: Mutex::new(BTreeMap::new()),
            session_read_hidden: AtomicBool::new(false),
        }
    }

    /// Count a settled call into `by_tool`. Unsettled / host-cancelled stay out.
    fn note_settled(&self, tool: &str, outcome: &str) {
        let ok = outcome == "ok" || outcome.starts_with("ok(");
        let fail = outcome.starts_with("error");
        if !ok && !fail {
            return;
        }
        let mut counts = self
            .by_tool
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let entry = counts.entry(tool.to_string()).or_insert((0, 0));
        if ok {
            entry.0 += 1;
        } else {
            entry.1 += 1;
        }
    }

    fn begin(&self, seq: u64, tool: &str, summary: &str) {
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
                session_read,
                brief: None,
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
        let error_code =
            call_error_code(result).or_else(|| outcome.strip_prefix("error:").map(str::to_string));
        if let Some(code) = &error_code {
            let mut counts = self
                .per_error
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            *counts.entry(code.clone()).or_default() += 1;
        }
        let brief = call_brief(result);
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(line) = lines.iter_mut().find(|line| line.seq == seq) {
            if line.outcome.is_none() {
                self.note_settled(&line.tool, &outcome);
                line.outcome = Some(outcome);
                line.error_code = error_code;
                line.brief = Some(brief);
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
            return;
        };
        if is_priority_state(&outcome) && self.priority_used.load(Ordering::Relaxed) < PRIORITY_CAP
        {
            self.priority_used.fetch_add(1, Ordering::Relaxed);
            self.omitted.fetch_sub(1, Ordering::Relaxed);
            self.note_settled(&pending.tool, &outcome);
            self.lines
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(LedgerLine {
                    seq,
                    tool: pending.tool,
                    args_summary: pending.summary,
                    outcome: Some(outcome),
                    error_code,
                    session_read: pending.session_read,
                    brief: Some(brief),
                });
            return;
        }
        // Settled but not retained in the snap: still count for human by_tool.
        self.note_settled(&pending.tool, &outcome);
        if pending.session_read {
            self.session_read_hidden.store(true, Ordering::Relaxed);
        }
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
                    session_read: pending.session_read,
                    brief: None,
                });
            } else if pending.session_read {
                session_read_hidden = true;
            }
        }
        lines.sort_by_key(|line| line.seq);
        LedgerSnapshot {
            lines,
            omitted,
            by_tool: self
                .by_tool
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

const BRIEF_CAP: usize = 160;

/// Short digest of one settled call: the error message, else the compact
/// result data, else the first content line. UTF-8 safe, at most [`BRIEF_CAP`].
fn call_brief(result: &ToolCallResult) -> String {
    let text = if let Some(error) = result.error.as_ref().filter(|e| !e.message.is_empty()) {
        error.message.clone()
    } else if let Some(data) = result.data.as_ref() {
        if let Some(message) = data
            .get("message")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty() && data.get("ok") == Some(&Value::Bool(false)))
        {
            message.to_string()
        } else {
            let bytes = crate::ptc::json_bytes(data).unwrap_or_default();
            String::from_utf8_lossy(&bytes[..bytes.len().min(BRIEF_CAP * 4)]).into_owned()
        }
    } else {
        result.content.lines().next().unwrap_or("").to_string()
    };
    let one_line = text.replace(['\n', '\r'], " ");
    let cut = prefix_utf8(&one_line, BRIEF_CAP);
    if cut.len() < one_line.len() {
        format!("{cut}…")
    } else {
        cut.to_string()
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
    // WP-210 G: a resolved `{ok:false}` is still a failed tool call in the head.
    if let Some(data) = result.data.as_ref()
        && data.get("ok") == Some(&Value::Bool(false))
    {
        let reason = data
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("not_ok");
        return format!("error:{reason}");
    }
    "ok".to_string()
}

fn edit_counts(result: &ToolCallResult) -> Option<(u64, u64)> {
    if result.error.is_some() || result.level == ToolSignalLevel::Error {
        return None;
    }
    let data = result.data.as_ref()?;
    // apply_edits: { results: EditResult[] }
    if let Some(results) = data.get("results").and_then(Value::as_array) {
        let total = results.len() as u64;
        if total == 0 {
            return None;
        }
        let ok = results
            .iter()
            .filter(|item| item.get("ok").and_then(Value::as_bool) == Some(true))
            .count() as u64;
        if ok < total {
            return Some((ok, total));
        }
        return None;
    }
    None
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
        return Some(display_error_code(code));
    }
    result
        .error
        .as_ref()
        .map(|error| display_error_code(&outcome::error_kind_code(error.kind)))
}

/// The JS-facing code surface uses `invalid_args`; keep the ledger label in
/// the same vocabulary instead of leaking the Rust `invalid_input` code.
fn display_error_code(code: &str) -> String {
    match code {
        "invalid_input" | "invalid_regex" => "invalid_args".to_string(),
        _ => code.to_string(),
    }
}

/// A bounded code bundle projected as one model-facing tool.
pub struct CodeTool {
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    surface: Arc<CodeSurface>,
    host: Arc<CodeToolHost>,
}

impl CodeTool {
    /// Build from an already-admitted JS contract surface.
    pub fn from_surface(
        bundle: Vec<Arc<dyn Tool>>,
        surface: Arc<CodeSurface>,
        host: Arc<CodeToolHost>,
    ) -> Self {
        Self {
            bundle: Arc::new(bundle),
            surface,
            host,
        }
    }

    pub fn new(bundle: Vec<Arc<dyn Tool>>, host: Arc<CodeToolHost>) -> Self {
        let surface = Arc::new(
            crate::tool::code_contract::build_surface(&bundle)
                .expect("test code bundle must match the JS SDK contract"),
        );
        Self::from_surface(bundle, surface, host)
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
                let surface = Arc::clone(&self.surface);
                let ledger = Arc::clone(&ledger);
                let session_id = execution.session_id.clone();
                let parent_call_id = execution.call_id.clone();
                HostBinding {
                    name: name.clone(),
                    function: HostFn::new(move |args, cancel, pause, info: CallInfo| {
                        let name = name.clone();
                        let bundle = Arc::clone(&bundle);
                        let host = Arc::clone(&host);
                        let surface = Arc::clone(&surface);
                        let ledger = Arc::clone(&ledger);
                        let session_id = session_id.clone();
                        let parent_call_id = parent_call_id.clone();
                        let seq = info.seq;
                        let deadline = tokio::time::Instant::now() + info.remaining;
                        async move {
                            invoke_binding(
                                name,
                                seq,
                                deadline,
                                args,
                                cancel,
                                pause,
                                bundle,
                                host,
                                surface,
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
        if self.surface.sdk.is_empty() || self.bundle.is_empty() {
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
            fs_root: Some(execution.workspace_root.clone()),
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
    deadline: tokio::time::Instant,
    args: Json,
    cancel: CancellationToken,
    pause: BudgetPause,
    bundle: Arc<Vec<Arc<dyn Tool>>>,
    host: Arc<CodeToolHost>,
    surface: Arc<CodeSurface>,
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
            ledger.record(
                index,
                &protocol_failure("tool bindings accept one argument"),
            );
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
    let input_value: Value = match serde_json::from_str(&arguments) {
        Ok(value) => value,
        Err(error) => {
            let message = format!("tool arguments for '{name}' are not valid JSON: {error}");
            ledger.record(index, &protocol_failure(&message));
            return Err(crate::ptc::HostError::type_error(message));
        }
    };
    if let Err(error) = surface.validate_input(&name, &input_value) {
        let failure = protocol_failure(&format!("invalid input for '{name}': {error}"));
        ledger.record(index, &failure);
        return Ok(
            outcome::program_result(&failure).expect("invalid input is a resolved domain failure")
        );
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
    let mut result = host
        .run_nested(
            bundle.as_slice(),
            &call,
            &session_id,
            cancel.child_token(),
            &parent_call_id,
            &sink,
            Some(deadline),
        )
        .await;
    let success = result.error.is_none()
        && matches!(
            result.level,
            crate::types::ToolSignalLevel::Ok | crate::types::ToolSignalLevel::Warning
        );
    if success
        && let Ok(program_value) = outcome::program_result(&result)
        && let Err(error) = surface.validate_output(&name, &program_value)
    {
        let mut invalid = ToolCallResult::error_with_kind(
            ToolErrorKind::Internal,
            format!("tool '{name}' returned data that does not match its JS contract: {error}"),
            false,
        );
        invalid.error_code = Some("invalid_output".into());
        result = invalid;
    }
    ledger.record(index, &result);
    outcome::program_result(&result)
}

fn protocol_failure(message: &str) -> ToolCallResult {
    let mut result =
        ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, message.to_string(), false);
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
    const PREFERRED: &[&str] = &["path", "pattern", "glob", "query", "command", "url"];
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
    // Keep any returned value even on failure (pi-style: don't blank wipe partial).
    let presented = if outcome.value.is_some() {
        match outcome.value.as_ref().map(present_returned_value) {
            Some(Ok(presented)) => Some(presented),
            Some(Err(message)) => {
                let report = run_report(
                    ledger,
                    outcome,
                    None,
                    Some(message),
                    Some(FailureKind::IllegalOutput),
                    None,
                    run_timeout_ms,
                    source,
                );
                return finish_run_result(
                    &report,
                    Some((ToolErrorKind::InvalidOutput, false, None)),
                );
            }
            None => None,
        }
    } else {
        None
    };
    let failure_kind = outcome.failure.as_ref().map(|failure| failure.kind);
    let resource = outcome
        .failure
        .as_ref()
        .and_then(|failure| failure.resource);
    let report = run_report(
        ledger,
        outcome,
        presented,
        failure_text,
        failure_kind,
        resource,
        run_timeout_ms,
        source,
    );

    let error = outcome.failure.as_ref().map(|failure| {
        if let Some(info) = &failure.tool_error {
            (
                outcome::kind_for_code(&info.code),
                false,
                Some(info.code.clone()),
            )
        } else {
            match failure.kind {
                FailureKind::Syntax => (ToolErrorKind::InvalidInput, false, None),
                FailureKind::Exception => (
                    ToolErrorKind::Internal,
                    false,
                    Some("exception".to_string()),
                ),
                FailureKind::Timeout => (ToolErrorKind::Timeout, true, None),
                FailureKind::Cancelled => (ToolErrorKind::Cancelled, false, None),
                FailureKind::ResourceLimit => (ToolErrorKind::Internal, false, None),
                FailureKind::OutputLimit => (ToolErrorKind::OutputLimit, true, None),
                FailureKind::IllegalOutput => (ToolErrorKind::InvalidOutput, false, None),
                FailureKind::Internal => (ToolErrorKind::Internal, false, None),
            }
        }
    });
    finish_run_result(&report, error)
}

/// Compose model `content` + human/FE `data` aggregate (WP-201).
fn finish_run_result(
    report: &render::RunReport,
    error: Option<(ToolErrorKind, bool, Option<String>)>,
) -> ToolCallResult {
    let content = render::render_model_view(report);
    let data = render::human_data(report);
    let mut result = if let Some((kind, retryable, code)) = error {
        let mut result = ToolCallResult::error_with_kind(kind, content, retryable);
        result.error_code = code;
        result
    } else {
        ToolCallResult::ok(content)
    };
    if let Some(data) = data {
        result.data = Some(data);
    }
    result
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
    let mut calls = Vec::with_capacity(lines.len());
    for line in lines {
        let record = outcome.calls.iter().find(|call| call.seq == line.seq);
        calls.push(render::LedgerSnap {
            seq: line.seq,
            tool: line.tool.clone(),
            summary: line.args_summary.clone(),
            state: merge_call_outcome(line.outcome.as_deref(), record.map(|call| call.state)),
            error_code: line.error_code.clone(),
            session_read: line.session_read,
            brief: line.brief.clone(),
        });
    }
    render::RunReport {
        dropped_undefined: outcome.dropped_undefined.head_line(),
        coerced_non_finite: outcome.dropped_undefined.non_finite_head_line(),
        logs: outcome
            .logs
            .iter()
            .map(|entry| {
                (
                    log_level_name(entry.level).to_string(),
                    entry.message.clone(),
                )
            })
            .collect(),
        value,
        failure_text,
        failure_kind,
        resource,
        timeout_ms,
        duration_ms: outcome.duration_ms,
        output_limited: failure_kind == Some(FailureKind::OutputLimit),
        calls,
        calls_omitted,
        by_tool: snapshot.by_tool.clone(),
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
            })
        }),
        uncaught_type_error: outcome
            .failure
            .as_ref()
            .is_some_and(|failure| failure.uncaught_type_error),
    }
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

fn present_returned_value(value: &Json) -> Result<Value, String> {
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

    fn direct_spec(&self, _ctx: &Context) -> Option<crate::tool::DirectToolSpec> {
        let description = {
            if self.surface.sdk.is_empty() {
                "run_code is unavailable because its code surface failed admission".to_string()
            } else {
                self.surface.sdk.clone()
            }
        };
        let input_schema = {
            json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Async JavaScript function body for the SDK below."
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
        };
        Some(crate::tool::DirectToolSpec {
            description,
            input_schema,
        })
    }

    fn code_bindings(&self) -> Option<Arc<[String]>> {
        Some(
            self.surface
                .tools
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .into(),
        )
    }

    fn is_code_transport(&self) -> bool {
        true
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

    use super::{
        CallLedger, DecodeFail, call_outcome_label, decode_invocation, merge_call_outcome,
        render_run_output,
    };

    fn runtime(state: CallState) -> Outcome {
        Outcome {
            logs: Vec::new(),
            dropped_undefined: Default::default(),
            value: None,
            failure: Some(Failure::cancelled()),
            duration_ms: 0,
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
        // Unsettled is a head aside, not a ledger row; it is not counted as failed.
        assert!(text.contains("calls were still running"), "{text}");
        assert!(text.contains("tool calls: 0 ok / 0 failed"), "{text}");
        assert!(!text.contains("calls ("), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
    }

    #[test]
    fn ledger_uses_the_js_invalid_args_vocabulary() {
        let mut result =
            ToolCallResult::error_with_kind(ToolErrorKind::InvalidInput, "bad input", false);
        result.error_code = Some("invalid_input".into());
        assert_eq!(call_outcome_label(&result), "error:invalid_args");
    }

    #[test]
    fn permission_denial_is_counted_as_a_failed_settled_call() {
        let completed = Outcome {
            logs: Vec::new(),
            dropped_undefined: Default::default(),
            value: None,
            failure: None,
            duration_ms: 0,
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
        assert!(text.contains("tool calls: 0 ok / 1 failed"), "{text}");
        assert!(!text.contains("calls ("), "{text}");
        assert!(!text.contains("#1 "), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
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
    fn render_run_output_attaches_by_tool_data_without_leaking_into_content() {
        let ledger = CallLedger::new();
        ledger.begin(1, "read", "path=a.txt");
        ledger.record(1, &ToolCallResult::ok("ok"));
        ledger.begin(2, "write", "path=b.txt");
        ledger.record(
            2,
            &ToolCallResult::error_with_kind(ToolErrorKind::PermissionDenied, "denied", false),
        );
        let outcome = Outcome {
            logs: Vec::new(),
            dropped_undefined: Default::default(),
            value: Some(Json::new(json!("done"))),
            failure: None,
            duration_ms: 42,
            calls: vec![
                CallRecord {
                    seq: 1,
                    namespace: "tools".into(),
                    name: "read".into(),
                    started: Duration::from_millis(1),
                    ended: Some(Duration::from_millis(2)),
                    state: CallState::Completed,
                },
                CallRecord {
                    seq: 2,
                    namespace: "tools".into(),
                    name: "write".into(),
                    started: Duration::from_millis(3),
                    ended: Some(Duration::from_millis(4)),
                    state: CallState::Completed,
                },
            ],
        };
        let result = render_run_output(&ledger, &outcome, 120_000, "");
        assert!(result.error.is_none(), "{}", result.content);
        assert!(
            result
                .content
                .starts_with("run_code ok · program: ok · tool calls: 1 ok / 1 failed · 42ms"),
            "{}",
            result.content
        );
        assert!(!result.content.contains("by_tool"), "{}", result.content);
        assert!(!result.content.contains("calls ("), "{}", result.content);
        assert!(
            !result.content.contains("@@run_code_data"),
            "{}",
            result.content
        );
        let data = result.data.expect("data");
        assert_eq!(data["succeeded"], 1);
        assert_eq!(data["failed"], 1);
        assert_eq!(data["duration_ms"], 42);
        assert_eq!(data["ok"], true);
        assert_eq!(data["by_tool"]["read"]["ok"], 1);
        assert_eq!(data["by_tool"]["read"]["fail"], 0);
        assert_eq!(data["by_tool"]["write"]["ok"], 0);
        assert_eq!(data["by_tool"]["write"]["fail"], 1);
    }
}
