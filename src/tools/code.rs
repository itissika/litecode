//! `run_code` transport. The program runs on the embedded runtime in `crate::ptc`.
//!
//! Nested tool calls re-enter [`execute_tool_call`]. This module does not own a
//! second permission, lock, or cancellation path. The runtime stays tool-agnostic:
//! the host binding records the nested-call ledger and pauses the program budget
//! while an approval card is waiting.

use std::collections::{BTreeMap, HashMap};

mod render;
pub use render::RUN_CODE_VIEW_MAX_BYTES;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::permission::{AskPrompt, AskReply, PermissionEngine, PermissionSink};
use crate::ptc::{
    self, BudgetPause, CallInfo, CallState, ExecuteRequest, FailureKind, HostBinding, HostFn, Json,
    Limits, Namespace,
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
/// Hard maximum active-JS run cap. Paused while nested host tool calls wait.
pub const MAX_RUN_TIMEOUT_MS: u64 = 600_000;
/// Default active-JS run cap.
pub const DEFAULT_RUN_TIMEOUT_MS: u64 = MAX_RUN_TIMEOUT_MS;
/// Hard wall-clock cap per `run_code` call (30 minutes), including nested tool waits.
pub const MAX_RUN_WALL_TIMEOUT_MS: u64 = 1_800_000;
/// Runtime runaway output circuit breaker (8 MiB, aligned with terminal log max).
pub const MAX_CODE_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
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
        let _guard = self.pause.pause_human();
        self.inner.ask(prompt, cancel)
    }
}

#[derive(Default)]
struct MediaRegistry {
    next_id: u64,
    items: HashMap<String, crate::types::MediaArtifact>,
}

impl MediaRegistry {
    fn register(&mut self, artifact: crate::types::MediaArtifact) -> Value {
        self.next_id += 1;
        let id = format!("media_{}", self.next_id);
        let kind = match artifact.kind {
            crate::types::MediaKind::Image => "image",
            crate::types::MediaKind::Video => "video",
            crate::types::MediaKind::Audio => "audio",
        };
        let mut obj = serde_json::Map::new();
        obj.insert("__litecode_media".into(), Value::String(id.clone()));
        obj.insert("kind".into(), Value::String(kind.to_string()));
        obj.insert("mime_type".into(), Value::String(artifact.mime_type.clone()));
        if let Some(w) = artifact.width {
            obj.insert("width".into(), Value::from(w));
        }
        if let Some(h) = artifact.height {
            obj.insert("height".into(), Value::from(h));
        }
        self.items.insert(id, artifact);
        Value::Object(obj)
    }

    fn collect_inspected_parts(
        &self,
        entries: &[crate::ptc::Entry],
    ) -> (Vec<crate::types::ToolOutputPart>, Vec<String>) {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let mut invalid = Vec::new();
        for entry in entries {
            for json in &entry.values {
                collect_media_handles_iterative(
                    json.as_value(),
                    &self.items,
                    &mut seen,
                    &mut out,
                    &mut invalid,
                );
            }
        }
        (out, invalid)
    }
}

fn collect_media_handles_iterative(
    root: &Value,
    items: &HashMap<String, crate::types::MediaArtifact>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<crate::types::ToolOutputPart>,
    invalid: &mut Vec<String>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Array(arr) => {
                for item in arr.iter().rev() {
                    stack.push(item);
                }
            }
            Value::Object(map) => {
                if let Some(raw_handle) = map.get("__litecode_media") {
                    match raw_handle.as_str() {
                        Some(id) if items.contains_key(id) => {
                            if seen.insert(id.to_string()) {
                                out.push(crate::types::ToolOutputPart::Media {
                                    artifact: items[id].clone(),
                                });
                            }
                        }
                        Some(id) => {
                            let label = id.to_string();
                            if !invalid.contains(&label) {
                                invalid.push(label);
                            }
                        }
                        None => {
                            let label = raw_handle.to_string();
                            if !invalid.contains(&label) {
                                invalid.push(label);
                            }
                        }
                    }
                    continue;
                }
                let mut values: Vec<&Value> = map.values().collect();
                values.reverse();
                stack.extend(values);
            }
            _ => {}
        }
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
    /// One-line result digest for the failure view (WP-210 A).
    brief: Option<String>,
}

struct PendingCall {
    tool: String,
    summary: String,
}

struct LedgerSnapshot {
    lines: Vec<LedgerLine>,
    omitted: usize,
    /// Settled per-tool ok/fail (unsettled & host-cancelled excluded).
    by_tool: BTreeMap<String, (usize, usize)>,
    error_counts: BTreeMap<String, usize>,
}

struct CallLedger {
    lines: Mutex<Vec<LedgerLine>>,
    pending: Mutex<HashMap<u64, PendingCall>>,
    omitted: AtomicUsize,
    priority_used: AtomicUsize,
    /// Settled ok/fail per tool name. Host `cancelled` and unsettled are excluded.
    by_tool: Mutex<BTreeMap<String, (usize, usize)>>,
    per_error: Mutex<BTreeMap<String, usize>>,
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
                    brief: Some(brief),
                });
            return;
        }
        // Settled but not retained in the snap: still count for human by_tool.
        self.note_settled(&pending.tool, &outcome);
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
                    brief: None,
                });
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
        }
    }
}

const BRIEF_CAP: usize = 160;

/// Short digest of one settled call: the error message, else the compact
/// result data, else compact JSON content, else the first content line. UTF-8
/// safe, at most [`BRIEF_CAP`].
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
        render::compact_json_text(&result.content)
            .unwrap_or_else(|| result.content.lines().next().unwrap_or("").to_string())
    };
    let one_line = text.replace(['\n', '\r'], " ");
    let cut = prefix_utf8(&one_line, BRIEF_CAP);
    if cut.len() < one_line.len() {
        format!("{cut}…")
    } else {
        cut.to_string()
    }
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
    run_timeout_ms: u64,
    wall_timeout_ms: u64,
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
            run_timeout_ms: DEFAULT_RUN_TIMEOUT_MS,
            wall_timeout_ms: MAX_RUN_WALL_TIMEOUT_MS,
        }
    }

    /// Test-only override for the active-JS run cap. This is not model-facing:
    /// `run_code` has no timeout parameter.
    #[doc(hidden)]
    pub fn with_run_timeout(mut self, timeout: Duration) -> Self {
        self.run_timeout_ms = timeout.as_millis().clamp(1, MAX_RUN_TIMEOUT_MS as u128) as u64;
        self
    }

    /// Test-only override for the hard wall-clock cap per `run_code` call.
    #[doc(hidden)]
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout_ms = timeout
            .as_millis()
            .clamp(1, MAX_RUN_WALL_TIMEOUT_MS as u128) as u64;
        self
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
        reminders: Arc<std::sync::Mutex<Vec<render::CallReminder>>>,
        media: Arc<Mutex<MediaRegistry>>,
        execution: &ToolExecutionContext,
    ) -> Vec<HostBinding> {
        self.surface
            .bindings
            .iter()
            .filter_map(|(sdk_path, runtime_name)| {
                if !self
                    .bundle
                    .iter()
                    .any(|tool| tool.name() == runtime_name.as_str())
                {
                    return None;
                }
                let sdk_path = sdk_path.clone();
                let runtime_name = runtime_name.clone();
                let reminders = Arc::clone(&reminders);
                let media = Arc::clone(&media);
                // Result fields come from this tool's output schema, and the
                // failure fields from the envelope the program is handed.
                let contract = self.surface.tools.get(runtime_name.as_str()).and_then(|spec| {
                    let output_schema = spec.output_schema.as_ref()?;
                    let properties = output_schema
                        .get("properties")
                        .and_then(serde_json::Value::as_object)?;
                    if properties.is_empty()
                        && output_schema
                            .get("additionalProperties")
                            .and_then(serde_json::Value::as_bool)
                            != Some(false)
                    {
                        return None;
                    }
                    let mut fields: Vec<String> = properties.keys().cloned().collect();
                    fields.sort();
                    Some(std::sync::Arc::new(crate::ptc::BindingContract {
                        fields: fields.into(),
                    }))
                });
                let bundle = Arc::clone(&self.bundle);
                let host = Arc::clone(&self.host);
                let surface = Arc::clone(&self.surface);
                let ledger = Arc::clone(&ledger);
                let session_id = execution.session_id.clone();
                let parent_call_id = execution.call_id.clone();
                Some(HostBinding {
                    contract,
                    name: sdk_path.clone(),
                    function: HostFn::new(move |args, cancel, pause, info: CallInfo| {
                        let sdk_path = sdk_path.clone();
                        let runtime_name = runtime_name.clone();
                        let bundle = Arc::clone(&bundle);
                        let host = Arc::clone(&host);
                        let surface = Arc::clone(&surface);
                        let ledger = Arc::clone(&ledger);
                        let reminders = Arc::clone(&reminders);
                        let media = Arc::clone(&media);
                        let session_id = session_id.clone();
                        let parent_call_id = parent_call_id.clone();
                        let seq = info.seq;
                        async move {
                            invoke_binding(
                                runtime_name,
                                sdk_path,
                                reminders,
                                media,
                                seq,
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
                })
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
        let run_timeout_ms = self.run_timeout_ms;
        let wall_timeout_ms = self.wall_timeout_ms;
        // Report-only: typo-class errors are cheap to see now and expensive at
        // runtime, where they cost the whole run. Findings join the run's
        // warnings; the program still runs either way.
        let precheck = crate::ptc::precheck::check(&code, &self.surface);

        let ledger = Arc::new(CallLedger::new());
        let reminders: Arc<std::sync::Mutex<Vec<render::CallReminder>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let media = Arc::new(Mutex::new(MediaRegistry::default()));
        let fs_allow_outside_workspace = self
            .host
            .permission
            .binding("read")
            .is_none_or(|binding| binding.enabled)
            && self.host.permission.path_mode("read")
                == crate::permission::BindingPathMode::Unrestricted;
        let request = ExecuteRequest {
            source: code.clone(),
            namespaces: vec![Namespace {
                name: "tools".to_string(),
                functions: self.binding(
                    Arc::clone(&ledger),
                    Arc::clone(&reminders),
                    Arc::clone(&media),
                    &execution,
                ),
            }],
            limits: Limits {
                output_bytes: MAX_CODE_OUTPUT_BYTES,
                ..Limits::default()
            },
            budget: Duration::from_millis(run_timeout_ms),
            wall_budget: Some(Duration::from_millis(wall_timeout_ms)),
            grace: NESTED_CANCEL_GRACE,
            fs_root: Some(execution.workspace_root.clone()),
            fs_allow_outside_workspace,
            cancel: execution.cancel,
        };
        let mut outcome = ptc::execute(request).await;
        if !precheck.is_empty() {
            // The same typo can be reported twice — once from the source, once
            // when the read actually happens. One line is enough.
            let mut merged = precheck;
            for warning in outcome.warnings.drain(..) {
                if !merged.contains(&warning) {
                    merged.push(warning);
                }
            }
            outcome.warnings = merged;
        }
        let mut collected = reminders.lock().expect("reminder buffer").split_off(0);
        collected.sort_by_key(|reminder| reminder.seq);
        let (returned_parts, invalid_media) = media
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .collect_inspected_parts(&outcome.entries);
        if outcome.failure.is_none() && !invalid_media.is_empty() {
            outcome.failure = Some(crate::ptc::Failure::illegal(format!(
                "unknown __litecode_media handle(s) ({}) not produced by any bound tool in this run",
                invalid_media.join(", ")
            )));
        }
        render_run_output_in(
            &ledger,
            &outcome,
            run_timeout_ms,
            wall_timeout_ms,
            &code,
            collected,
            returned_parts,
            Some(&execution.workspace_root),
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn invoke_binding(
    tool_name: String,
    sdk_name: String,
    reminders: Arc<std::sync::Mutex<Vec<render::CallReminder>>>,
    media: Arc<Mutex<MediaRegistry>>,
    seq: u64,
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
        Err(DecodeFail::ExtraArgument { summary, count }) => {
            ledger.begin(seq, &sdk_name, &summary);
            let message = format!("'{sdk_name}' accepts exactly one argument, got {count}");
            return Err(reject_as_error(&ledger, seq, message));
        }
        Err(DecodeFail::Illegal(illegal)) => {
            ledger.begin(seq, &sdk_name, &illegal.summary);
            // No source-level check can see this one (it is the value shape at
            // run time), so the block is its only voice.
            remind(
                &reminders,
                seq,
                &sdk_name,
                render::ReminderKind::Failure,
                None,
                render::rejected_arguments(&illegal.message, &[]),
            );
            return Err(reject_as_error(&ledger, seq, illegal.message));
        }
    };
    ledger.begin(seq, &sdk_name, &summary);
    let index = seq;

    if !bundle.iter().any(|tool| tool.name() == tool_name.as_str()) {
        return Err(contract_reject(
            &ledger,
            index,
            "not_found",
            &sdk_name,
            format!("tool '{sdk_name}' not found"),
        ));
    }
    let input_value: Value = match serde_json::from_str(&arguments) {
        Ok(value) => value,
        Err(error) => {
            // Internal error, not a call the model got wrong: the arguments
            // never reached the contract as JSON.
            let message = format!("tool arguments for '{sdk_name}' are not valid JSON: {error}");
            ledger.record(index, &protocol_failure(&message));
            return Err(crate::ptc::HostError::type_error(message));
        }
    };
    if let Err(error) = surface.validate_input(&tool_name, &input_value) {
        let message = format!("invalid input for '{sdk_name}': {error}");
        // The host refused the call before running it. A literal key is also
        // in the precheck, but a computed one is invisible there, and the
        // program may read the envelope and carry on.
        remind(
            &reminders,
            index,
            &sdk_name,
            render::ReminderKind::Failure,
            Some(error.clone()),
            render::rejected_arguments(&error, &legal_parameters(&surface, &tool_name)),
        );
        return Err(reject_as_error(&ledger, index, message));
    }

    let call = FunctionToolCall {
        arguments,
        call_id: format!("{parent_call_id}:code:{index}"),
        namespace: None,
        name: tool_name.clone(),
        id: None,
        status: None,
    };
    let sink = PausingSink {
        inner: host.sink(),
        pause: pause.clone(),
    };
    // Two-layer timeout: freeze the outer JS active budget while the nested
    // tool runs on its own timeout / yield budget (still bounded by the outer
    // wall-clock cap via `cancel`).
    let _nested_pause = pause.pause();
    let mut result = host
        .run_nested(
            bundle.as_slice(),
            &call,
            &session_id,
            cancel.child_token(),
            &parent_call_id,
            &sink,
            None,
        )
        .await;
    drop(_nested_pause);
    if let Some(error) = surface.output_violation(&tool_name, &result) {
        // The ledger row is a failure too: the call did not produce a usable
        // result, and the manifest must not say "ok" for it.
        return Err(contract_reject(
            &ledger,
            index,
            "invalid_output",
            &sdk_name,
            format!("tool '{sdk_name}' returned data that does not match its JS contract: {error}"),
        ));
    }
    let media_parts = if result.error.is_none() && result.level != ToolSignalLevel::Error {
        std::mem::take(&mut result.parts)
    } else {
        Vec::new()
    };
    let media_handles: Vec<Value> = if media_parts.is_empty() {
        Vec::new()
    } else {
        match crate::tool::output::materialize_media_parts(media_parts, &host.data_root) {
            Ok(materialized) => {
                let mut reg = media.lock().unwrap_or_else(|poison| poison.into_inner());
                materialized
                    .into_iter()
                    .filter_map(|part| match part {
                        crate::types::ToolOutputPart::Media { artifact } => {
                            Some(reg.register(artifact))
                        }
                        crate::types::ToolOutputPart::Text { .. } => None,
                    })
                    .collect()
            }
            Err(error) => {
                let failed = ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidOutput,
                    format!("failed to materialize media output: {error}"),
                    false,
                );
                ledger.record(index, &failed);
                return Err(crate::ptc::HostError::tool(
                    "invalid_output",
                    format!("failed to materialize media output: {error}"),
                    None,
                    false,
                ));
            }
        }
    };
    ledger.record(index, &result);
    let mut value = match outcome::program_result(&result) {
        Ok(value) => value,
        Err(error) => {
            let code = call_error_code(&result).unwrap_or_else(|| "error".to_string());
            remind(
                &reminders,
                seq,
                &sdk_name,
                render::ReminderKind::Failure,
                None,
                format!("failed({code}); the program may have caught the error"),
            );
            return Err(error);
        }
    };
    if !media_handles.is_empty() {
        let handles = Value::Array(media_handles);
        match &mut value {
            Value::Object(map) => {
                map.insert("media".to_string(), handles);
            }
            Value::Null => {
                value = json!({ "media": handles });
            }
            other => {
                let inner = std::mem::take(other);
                *other = json!({ "value": inner, "media": handles });
            }
        }
    }
    // Facts are anchored to the call, not to a line of output: the program may
    // return only a slice of this value, or none of it. A failure envelope is
    // one of those facts — the program may read it and carry on, and the view
    // would otherwise say nothing at all about that call.
    let texts: Vec<(render::ReminderKind, String)> =
        render::reminders_for_call_with_metadata(&sdk_name, &value, result.metadata.as_ref())
            .into_iter()
            .map(|text| (render::ReminderKind::Boundary, text))
            .collect();
    for (kind, text) in texts {
        remind(&reminders, seq, &sdk_name, kind, None, text);
    }
    Ok(value)
}

/// Push one upfloat line for this call.
fn remind(
    reminders: &std::sync::Mutex<Vec<render::CallReminder>>,
    seq: u64,
    tool: &str,
    kind: render::ReminderKind,
    dedupe: Option<String>,
    text: String,
) {
    reminders.lock().expect("reminder buffer").push(render::CallReminder {
        seq,
        tool: tool.to_string(),
        kind,
        dedupe,
        text,
    });
}

/// Parameter names a call may use, sorted: the actionable half of a rejection
/// line, read from the same input schema the validator just refused.
fn legal_parameters(surface: &crate::tool::code_contract::CodeSurface, tool: &str) -> Vec<String> {
    let mut names: Vec<String> = surface
        .tools
        .get(tool)
        .and_then(|spec| spec.input_schema.get("properties"))
        .and_then(serde_json::Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// Fail the call, not the run: the program gets a failure envelope and can
/// correct its arguments on the next call.
///
/// The ledger row is a failure for the same reason, so the manifest never says
/// "ok" for a call the contract refused.
fn reject_as_error(ledger: &CallLedger, index: u64, message: String) -> crate::ptc::HostError {
    ledger.record(index, &contract_record("invalid_args", &message));
    crate::ptc::HostError::tool("invalid_args", message, None, false)
}

/// Record one contract failure and build the error that ends the run.
///
/// The ledger row and the thrown error carry the same code, so the manifest
/// line, the header and `ToolErrorInfo.code` cannot disagree.
fn contract_reject(
    ledger: &CallLedger,
    index: u64,
    code: &str,
    tool: &str,
    message: String,
) -> crate::ptc::HostError {
    ledger.record(index, &contract_record(code, &message));
    crate::ptc::HostError::contract(code, message, tool)
}

/// The ledger row for a contract failure: a failure row, never a success one.
fn contract_record(code: &str, message: &str) -> ToolCallResult {
    ToolCallResult::error_with_kind(
        crate::tool::outcome::kind_for_code(code),
        message.to_string(),
        false,
    )
    .with_error_code(code)
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
    ExtraArgument { summary: String, count: usize },
    Illegal(IllegalInvocation),
}

/// Borrow the runtime JSON. A deep argument is rejected here so the tool
/// pipeline never builds a `serde_json::Value` it would later drop recursively.
/// Two or more JavaScript arguments arrive as one JSON array. Bindings accept
/// exactly one argument.
fn decode_invocation(payload: &Json) -> Result<DecodedInvocation, DecodeFail> {
    let value = payload.as_value();
    if value.is_null() || matches!(value, Value::Array(items) if items.is_empty()) {
        return Ok(DecodedInvocation {
            arguments: "{}".to_string(),
            summary: String::new(),
        });
    }
    if let Value::Array(items) = value
        && items.len() >= 2
    {
        let summary = items
            .first()
            .map(key_arg_summary)
            .filter(|summary| !summary.is_empty())
            .unwrap_or_else(|| "extra argument".to_string());
        return Err(DecodeFail::ExtraArgument {
            summary,
            count: items.len(),
        });
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

#[cfg(test)]
fn render_run_output(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    run_timeout_ms: u64,
    source: &str,
    reminders: Vec<render::CallReminder>,
) -> ToolCallResult {
    render_run_output_in(
        ledger,
        outcome,
        run_timeout_ms,
        MAX_RUN_WALL_TIMEOUT_MS,
        source,
        reminders,
        Vec::new(),
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_run_output_in(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    run_timeout_ms: u64,
    wall_timeout_ms: u64,
    source: &str,
    reminders: Vec<render::CallReminder>,
    media_parts: Vec<crate::types::ToolOutputPart>,
    workspace_root: Option<&std::path::Path>,
) -> ToolCallResult {
    let effective_timeout_ms = outcome
        .failure
        .as_ref()
        .filter(|failure| {
            failure.kind == FailureKind::Timeout && failure.message == "wall budget exceeded"
        })
        .map_or(run_timeout_ms, |_| wall_timeout_ms);
    let failure_text = outcome.failure.as_ref().map(|failure| match failure.kind {
        FailureKind::Timeout => format!("run_code timed out after {effective_timeout_ms}ms"),
        FailureKind::Cancelled => "code execution cancelled".to_string(),
        FailureKind::OutputLimit => failure.message.clone(),
        FailureKind::IllegalOutput => {
            if failure.message.starts_with("unknown __litecode_media") {
                failure.message.clone()
            } else {
                format!("inspected output is not presentable: {}", failure.message)
            }
        }
        FailureKind::Syntax => failure.message.clone(),
        _ => failure.message.clone(),
    });
    // Keep whatever the program inspected even on failure: the stream is the
    // record.
    let failure_kind = outcome.failure.as_ref().map(|failure| failure.kind);
    let resource = outcome
        .failure
        .as_ref()
        .and_then(|failure| failure.resource);
    let (entries, illegal_output) = match entry_views(&outcome.entries) {
        Ok(entries) => (entries, None),
        Err(message) => (Vec::new(), Some(message)),
    };
    let (failure_text, failure_kind) = match &illegal_output {
        Some(message) => (Some(message.clone()), Some(FailureKind::IllegalOutput)),
        None => (failure_text, failure_kind),
    };
    let report = run_report(
        ledger,
        outcome,
        entries,
        reminders,
        failure_text,
        failure_kind,
        resource,
        effective_timeout_ms,
        source,
    );
    if illegal_output.is_some() {
        return finish_run_result(
            &report,
            Some((ToolErrorKind::InvalidOutput, false, None)),
            Vec::new(),
            workspace_root,
        );
    }

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
    let attach_parts = if error.is_none() {
        media_parts
    } else {
        Vec::new()
    };
    finish_run_result(&report, error, attach_parts, workspace_root)
}

/// Compose model `content` + human/FE `data` aggregate (WP-201).
fn finish_run_result(
    report: &render::RunReport,
    error: Option<(ToolErrorKind, bool, Option<String>)>,
    parts: Vec<crate::types::ToolOutputPart>,
    workspace_root: Option<&std::path::Path>,
) -> ToolCallResult {
    let content = render::render_model_view_in(report, workspace_root);
    let data = render::human_data(report);
    let mut result = if let Some((kind, retryable, code)) = error {
        let mut result = ToolCallResult::error_with_kind(kind, content, retryable);
        result.error_code = code;
        result
    } else {
        ToolCallResult::ok(content)
    };
    result.parts = parts;
    if let Some(data) = data {
        result.data = Some(data);
    }
    result
}

fn run_report(
    ledger: &CallLedger,
    outcome: &crate::ptc::Outcome,
    entries: Vec<render::EntryView>,
    reminders: Vec<render::CallReminder>,
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
            brief: line.brief.clone(),
        });
    }
    render::RunReport {
        reminders,
        warnings: outcome.warnings.clone(),
        dropped_undefined: outcome.dropped_undefined.head_line(),
        entries,
        failure_text,
        failure_kind,
        resource,
        timeout_ms,
        duration_ms: outcome.duration_ms,
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

/// The report's view of the stream: each `inspect(...)` call's values, depth-guarded.
fn entry_views(entries: &[crate::ptc::Entry]) -> Result<Vec<render::EntryView>, String> {
    let mut views = Vec::with_capacity(entries.len());
    for entry in entries {
        let mut values = Vec::with_capacity(entry.values.len());
        for value in &entry.values {
            values.push(present_inspected_value(value)?);
        }
        views.push(render::EntryView { values });
    }
    Ok(views)
}

fn present_inspected_value(value: &Json) -> Result<Value, String> {
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

impl Tool for CodeTool {
    fn name(&self) -> &str {
        CODE_TOOL_NAME
    }

    fn transport_spec(&self, _ctx: &Context) -> Option<crate::tool::TransportToolSpec> {
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
                        "description": "Async TypeScript/JavaScript body for the bound SDK below. Call inspect(value) to emit outputs."
                    }
                },
                "required": ["code"],
                "additionalProperties": false
            })
        };
        Some(crate::tool::TransportToolSpec {
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
mod bridge_tests;

#[cfg(test)]
mod outcome_tests {
    use std::time::Duration;

    use serde_json::json;

    use crate::ptc::{CallRecord, CallState, Failure, Json, Outcome};
    use crate::types::{ToolCallResult, ToolErrorKind};

    use super::{
        CallLedger, DecodeFail, MediaRegistry, call_brief, call_outcome_label, decode_invocation,
        merge_call_outcome, render_run_output,
    };

    fn runtime(state: CallState) -> Outcome {
        Outcome {
            entries: Vec::new(),
            warnings: Vec::new(),
            dropped_undefined: Default::default(),
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
        render_run_output(&ledger, outcome, 120_000, "", Vec::new()).content
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
        assert!(text.starts_with("run_code failed: "), "{text}");
        assert!(!text.contains("tool calls:"), "{text}");
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
            entries: Vec::new(),
            warnings: Vec::new(),
            dropped_undefined: Default::default(),
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
        // Nothing failed at the program level and no reminder was built, so the
        // view is empty: a clean run has no status line.
        assert_eq!(text, "", "{text}");
        assert!(!text.contains("#1 "), "{text}");
        assert!(!text.contains("@@run_code_data"), "{text}");
    }

    #[test]
    fn a_second_javascript_argument_is_rejected_before_the_tool_runs() {
        let payload = Json::new(json!([{"path": "a.txt"}, {"unexpected": 1}]));
        match decode_invocation(&payload) {
            Err(DecodeFail::ExtraArgument { summary, .. }) => assert_eq!(summary, "path=a.txt"),
            other => panic!("expected the extra argument to be rejected: {other:?}"),
        }
        let one = Json::new(json!({"path": "a.txt"}));
        assert!(decode_invocation(&one).is_ok());
    }

    #[test]
    fn zero_javascript_arguments_are_normalized_to_an_empty_object() {
        for payload in [Json::new(json!([])), Json::new(json!(null))] {
            let decoded = decode_invocation(&payload).expect("zero args");
            assert_eq!(decoded.arguments, "{}");
        }
    }

    #[test]
    fn call_brief_compacts_pretty_json_content() {
        let result = ToolCallResult::ok("{\n  \"a\": 1\n}");
        assert_eq!(call_brief(&result), r#"{"a":1}"#);
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
            warnings: Vec::new(),
            entries: vec![crate::ptc::Entry {
                order: 1,
                values: vec![Json::new(json!("done"))],
            }],
            dropped_undefined: Default::default(),
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
        let result = render_run_output(&ledger, &outcome, 120_000, "", Vec::new());
        assert!(result.error.is_none(), "{}", result.content);
        assert!(!result.content.contains("run_code"), "{}", result.content);
        assert!(result.content.ends_with("done"), "{}", result.content);
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

    #[test]
    fn runtime_warnings_render_in_upfloat_above_stream() {
        let ledger = CallLedger::new();
        let outcome = Outcome {
            warnings: vec!["unhandled promise rejection (1): boom".into()],
            entries: vec![crate::ptc::Entry {
                order: 1,
                values: vec![Json::new(json!("done"))],
            }],
            dropped_undefined: Default::default(),
            failure: None,
            duration_ms: 5,
            calls: Vec::new(),
        };
        let result = render_run_output(&ledger, &outcome, 120_000, "", Vec::new());
        assert!(result.error.is_none(), "{}", result.content);
        assert!(
            result
                .content
                .contains("↳ unhandled promise rejection (1): boom"),
            "{}",
            result.content
        );
        assert!(result.content.ends_with("done"), "{}", result.content);
    }

    #[test]
    fn media_registry_attaches_only_handles_present_in_inspected_values() {
        let mut reg = MediaRegistry::default();
        let _uninspected = reg.register(crate::types::MediaArtifact::image(
            crate::types::MediaSource::Url {
                url: "https://example.com/a.png".into(),
            },
            "image/png",
        ));
        let h2 = reg.register(crate::types::MediaArtifact::image(
            crate::types::MediaSource::Url {
                url: "https://example.com/b.png".into(),
            },
            "image/png",
        ));
        // Only h2 is inspected (via labeled call + nested object); h1 is NOT inspected -> 不 inspect 不给
        let entries = vec![
            crate::ptc::Entry {
                order: 1,
                values: vec![Json::new(json!("shot")), Json::new(h2.clone())],
            },
            crate::ptc::Entry {
                order: 2,
                values: vec![Json::new(json!({ "dup": h2 }))],
            },
        ];
        let (parts, invalid) = reg.collect_inspected_parts(&entries);
        assert!(invalid.is_empty(), "{invalid:?}");
        assert_eq!(parts.len(), 1);
        match &parts[0] {
            crate::types::ToolOutputPart::Media { artifact } => {
                assert_eq!(
                    artifact.source,
                    crate::types::MediaSource::Url {
                        url: "https://example.com/b.png".into()
                    }
                );
            }
            other => panic!("expected media part, got {other:?}"),
        }
    }

    #[test]
    fn media_registry_reports_unknown_forged_handles() {
        let reg = MediaRegistry::default();
        let entries = vec![crate::ptc::Entry {
            order: 1,
            values: vec![Json::new(json!({
                "media": [{ "__litecode_media": "small.png", "kind": "image", "mime_type": "image/png" }]
            }))],
        }];
        let (parts, invalid) = reg.collect_inspected_parts(&entries);
        assert!(parts.is_empty());
        assert_eq!(invalid, vec!["small.png"]);
    }
}
