//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::collections::HashMap;
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

/// Same-program reader-writer gate. Writes are exclusive. Reads of one path
/// share. A write does not run beside a read of that path. Distinct paths
/// proceed together. Waiting here consumes the program budget; only an
/// approval card pauses it. The process write lock still runs inside the
/// shared tool pipeline.
struct PathGate {
    slots: Mutex<HashMap<String, PathSlot>>,
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct PathSlot {
    readers: u32,
    writing: bool,
}

struct PathHold {
    gate: Arc<PathGate>,
    key: String,
    exclusive: bool,
}

impl Default for PathGate {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            changed: tokio::sync::Notify::new(),
        }
    }
}

impl PathGate {
    async fn enter(
        self: &Arc<Self>,
        key: String,
        exclusive: bool,
        cancel: &CancellationToken,
    ) -> Result<PathHold, ()> {
        loop {
            if cancel.is_cancelled() {
                return Err(());
            }
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.try_enter(&key, exclusive) {
                return Ok(PathHold {
                    gate: Arc::clone(self),
                    key,
                    exclusive,
                });
            }
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(()),
                _ = notified => {}
            }
        }
    }

    fn try_enter(&self, key: &str, exclusive: bool) -> bool {
        let mut slots = self.slots.lock().unwrap_or_else(|error| error.into_inner());
        let slot = slots.entry(key.to_string()).or_default();
        let free = if exclusive {
            slot.readers == 0 && !slot.writing
        } else {
            !slot.writing
        };
        if !free {
            return false;
        }
        if exclusive {
            slot.writing = true;
        } else {
            slot.readers += 1;
        }
        true
    }
}

impl Drop for PathHold {
    fn drop(&mut self) {
        {
            let mut slots = self
                .gate
                .slots
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(slot) = slots.get_mut(&self.key) {
                if self.exclusive {
                    slot.writing = false;
                } else if slot.readers > 0 {
                    slot.readers -= 1;
                }
                if slot.readers == 0 && !slot.writing {
                    slots.remove(&self.key);
                }
            }
        }
        self.gate.changed.notify_waiters();
    }
}

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
        gate: Arc<PathGate>,
    ) -> Vec<HostBinding> {
        self.bundle
            .iter()
            .map(|tool| {
                let name = tool.name().to_string();
                let bundle = Arc::clone(&self.bundle);
                let host = Arc::clone(&self.host);
                let ledger = Arc::clone(&ledger);
                let gate = Arc::clone(&gate);
                let session_id = execution.session_id.clone();
                let parent_call_id = execution.call_id.clone();
                HostBinding {
                    name: name.clone(),
                    function: HostFn::new(move |args, cancel, pause| {
                        let name = name.clone();
                        let bundle = Arc::clone(&bundle);
                        let host = Arc::clone(&host);
                        let ledger = Arc::clone(&ledger);
                        let gate = Arc::clone(&gate);
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
                                gate,
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
        let gate = Arc::new(PathGate::default());
        let request = ExecuteRequest {
            source: code,
            namespaces: vec![Namespace {
                name: "tools".to_string(),
                functions: self.binding(Arc::clone(&ledger), &execution, gate),
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
    gate: Arc<PathGate>,
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

    let claim = path_gate_claim(&host, &name, &arguments);
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
    let _hold = if let Some((key, exclusive)) = claim {
        match gate.enter(key, exclusive, &call_cancel).await {
            Ok(hold) => Some(hold),
            Err(()) => {
                let result = ToolCallResult::error_with_kind(
                    ToolErrorKind::Cancelled,
                    format!("tool '{name}' cancelled"),
                    false,
                );
                ledger.record(index, outcome_label(&result));
                return Ok(tool_result_value(&result));
            }
        }
    } else {
        None
    };
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

fn path_gate_claim(host: &CodeToolHost, name: &str, arguments: &str) -> Option<(String, bool)> {
    let exclusive = match name {
        "edit" | "write" => true,
        "read" => false,
        _ => return None,
    };
    let value: Value = serde_json::from_str(arguments).ok()?;
    let raw = value
        .get("path")
        .or_else(|| value.get("file_path"))
        .and_then(Value::as_str)?;
    if raw.is_empty() {
        return None;
    }
    let mode = host.permission.path_mode(name).to_tool_path_mode();
    let key = crate::workspace::resolve_agent(&host.ctx.cwd, raw, mode)
        .map(|path| path_gate_key(&path))
        .unwrap_or_else(|_| raw.to_string());
    Some((key, exclusive))
}

/// Identity of one file for [`PathGate`].
///
/// `resolve_agent` keeps the caller's spelling when the file does not exist
/// yet. Rebuild from `components()`: skip `.`, and let `..` pop one preceding
/// normal component without walking past a root or prefix. Separators agree
/// because the key is rebuilt. On Windows, each normal component loses
/// trailing dots and spaces and is case-folded with the file-system uppercase
/// table. Reads and writes of that file share this key.
fn path_gate_key(path: &std::path::Path) -> String {
    use std::path::{Component, PathBuf};
    let mut rebuilt = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => pop_preceding_normal(&mut rebuilt),
            Component::Normal(name) => push_normal(&mut rebuilt, name),
            other => rebuilt.push(other.as_os_str()),
        }
    }
    rebuilt.to_string_lossy().into_owned()
}

/// `..` pops one normal component. A prefix or root stays put.
fn pop_preceding_normal(path: &mut std::path::PathBuf) {
    if matches!(
        path.components().next_back(),
        Some(std::path::Component::Normal(_))
    ) {
        path.pop();
    }
}

fn push_normal(path: &mut std::path::PathBuf, name: &std::ffi::OsStr) {
    let name = normalize_normal(name);
    if name.is_empty() || name == std::ffi::OsStr::new(".") {
        return;
    }
    if name == std::ffi::OsStr::new("..") {
        pop_preceding_normal(path);
        return;
    }
    path.push(name);
}

#[cfg(not(windows))]
fn normalize_normal(name: &std::ffi::OsStr) -> std::ffi::OsString {
    name.to_os_string()
}

/// Strip trailing dots and spaces, then apply the Windows file-system
/// uppercase table. `LCMapStringEx` without `LCMAP_LINGUISTIC_CASING` is that
/// table; it does not open the file.
#[cfg(windows)]
fn normalize_normal(name: &std::ffi::OsStr) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = name.encode_wide().collect();
    while wide
        .last()
        .is_some_and(|unit| *unit == u16::from(b' ') || *unit == u16::from(b'.'))
    {
        wide.pop();
    }
    windows_fs_upcase(&wide)
}

#[cfg(windows)]
fn windows_fs_upcase(wide: &[u16]) -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Globalization::{
        LCMAP_UPPERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT,
    };
    if wide.is_empty() {
        return std::ffi::OsString::new();
    }
    let Ok(len) = i32::try_from(wide.len()) else {
        return std::ffi::OsString::from_wide(wide);
    };
    let mut mapped = vec![0u16; wide.len()];
    // File-system uppercase: no LCMAP_LINGUISTIC_CASING, so the mapping does not
    // follow the user locale and does not open the path.
    let wrote = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_UPPERCASE,
            wide.as_ptr(),
            len,
            mapped.as_mut_ptr(),
            len,
            std::ptr::null::<windows_sys::Win32::Globalization::NLSVERSIONINFO>(),
            std::ptr::null::<core::ffi::c_void>(),
            0,
        )
    };
    if wrote == len {
        std::ffi::OsString::from_wide(&mapped)
    } else {
        std::ffi::OsString::from_wide(wide)
    }
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

#[cfg(test)]
mod path_gate_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::PathGate;
    use tokio_util::sync::CancellationToken;

    #[tokio::test(flavor = "current_thread")]
    async fn nested_writes_are_exclusive_and_a_write_excludes_a_read_of_the_same_path() {
        let gate = Arc::new(PathGate::default());
        let first = gate
            .enter("a".into(), true, &CancellationToken::new())
            .await
            .unwrap();
        let gate2 = Arc::clone(&gate);
        let entered = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&entered);
        let second = tokio::spawn(async move {
            gate2
                .enter("a".into(), true, &CancellationToken::new())
                .await
                .unwrap();
            flag.store(true, Ordering::SeqCst);
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(!entered.load(Ordering::SeqCst), "two writes overlapped");
        drop(first);
        second.await.unwrap();
        assert!(entered.load(Ordering::SeqCst));

        let reader = gate
            .enter("b".into(), false, &CancellationToken::new())
            .await
            .unwrap();
        let other = gate
            .enter("b".into(), false, &CancellationToken::new())
            .await
            .unwrap();
        drop((reader, other));

        let reader = gate
            .enter("c".into(), false, &CancellationToken::new())
            .await
            .unwrap();
        let gate3 = Arc::clone(&gate);
        let wrote = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&wrote);
        let writer = tokio::spawn(async move {
            gate3
                .enter("c".into(), true, &CancellationToken::new())
                .await
                .unwrap();
            flag.store(true, Ordering::SeqCst);
        });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        assert!(
            !wrote.load(Ordering::SeqCst),
            "a write ran beside a read of the same path"
        );
        drop(reader);
        writer.await.unwrap();
        assert!(wrote.load(Ordering::SeqCst));

        let left = gate
            .enter("d".into(), true, &CancellationToken::new())
            .await
            .unwrap();
        let right = gate
            .enter("e".into(), true, &CancellationToken::new())
            .await
            .unwrap();
        drop((left, right));
    }

    #[test]
    fn path_gate_key_folds_dot_case_and_separators() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let key = |raw: &str, mode| {
            let resolved = crate::workspace::resolve_agent(root, raw, mode).unwrap();
            super::path_gate_key(&resolved)
        };
        let all = crate::workspace::ToolPathMode::All;
        let dot = crate::workspace::resolve_agent(root, "./dot.txt", all).unwrap();
        let plain = crate::workspace::resolve_agent(root, "dot.txt", all).unwrap();
        assert_ne!(
            dot.to_string_lossy(),
            plain.to_string_lossy(),
            "All mode keeps the lexical dot; the gate key has to drop it"
        );
        assert_eq!(key("./dot.txt", all), key("dot.txt", all));
        let lexical = root.join("gone").join("..").join("parented.txt");
        assert!(
            lexical
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir)),
            "the lexical path must still contain '..'"
        );
        assert_eq!(
            super::path_gate_key(&lexical),
            super::path_gate_key(&root.join("parented.txt"))
        );
        #[cfg(windows)]
        {
            let safe = crate::workspace::ToolPathMode::Safe;
            for mode in [all, safe] {
                let lower = crate::workspace::resolve_agent(root, "case.txt", mode).unwrap();
                let upper = crate::workspace::resolve_agent(root, "Case.txt", mode).unwrap();
                assert_ne!(
                    lower.to_string_lossy().into_owned(),
                    upper.to_string_lossy().into_owned(),
                    "resolve keeps case; the gate key has to fold it"
                );
                assert_eq!(key("case.txt", mode), key("Case.txt", mode));
                assert_eq!(key("sub/nested.txt", mode), key(r"sub\nested.txt", mode));
                assert_eq!(key("slash.txt", mode), key(r".\slash.txt", mode));
                assert_eq!(key("trail.txt", mode), key("trail.txt.", mode));
                assert_eq!(key("trail.txt", mode), key("trail.txt ", mode));
                assert_eq!(key("ä.txt", mode), key("Ä.txt", mode));
                let folded = key("Case.txt", mode);
                assert!(
                    folded.chars().all(|ch| !ch.is_ascii_lowercase()),
                    "file-system case fold uppercases ASCII letters: {folded}"
                );
            }
            let abs = format!(r"{}\gone\..\parented.txt", root.display());
            let parented = crate::workspace::resolve_agent(root, &abs, all).unwrap();
            let plain = crate::workspace::resolve_agent(root, "parented.txt", all).unwrap();
            assert_ne!(
                parented.to_string_lossy().into_owned(),
                plain.to_string_lossy().into_owned(),
                "resolve keeps '..'; the gate key has to pop it"
            );
            assert_eq!(
                super::path_gate_key(&parented),
                super::path_gate_key(&plain)
            );
            assert_eq!(
                super::path_gate_key(std::path::Path::new(r"C:\..\..\parented.txt")),
                super::path_gate_key(std::path::Path::new(r"C:\parented.txt"))
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_waiting_write_does_not_start_after_cancel() {
        let gate = Arc::new(PathGate::default());
        let hold = gate
            .enter("a".into(), true, &CancellationToken::new())
            .await
            .unwrap();
        let cancel = CancellationToken::new();
        let gate2 = Arc::clone(&gate);
        let cancel2 = cancel.clone();
        let waiting = tokio::spawn(async move { gate2.enter("a".into(), true, &cancel2).await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        cancel.cancel();
        assert!(waiting.await.unwrap().is_err(), "cancelled waiter entered");
        drop(hold);
        let again = gate
            .enter("a".into(), true, &CancellationToken::new())
            .await
            .unwrap();
        drop(again);
    }
}
