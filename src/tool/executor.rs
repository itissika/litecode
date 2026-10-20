use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use tokio_util::sync::CancellationToken;

use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, InputContent, InputFileContent,
    InputImageContent, InputTextContent,
};
use crate::context_pipeline::Context;
use crate::permission::{PermissionEngine, PermissionSink};
use crate::runtime::diagnostics::{AuditEvent, emit_tool_audit};
use crate::session::media::resolve_media_artifact_url;
use crate::tool::authorize::{AuthResult, authorize};
use crate::tool::output;
use crate::tool::schema_validate::{
    check_tool_input, invalid_input_for, parse_tool_arguments, schema_validate,
    unknown_top_level_properties,
};
use crate::tool::resource_gate::{ToolEffects, process_resource_gate};
use crate::tool::trait_::{Tool, ToolExecutionContext, ToolSurface};
use crate::tool::write_lock::{ResourceKey, WorkspaceWriteLock};
use crate::types::{
    FunctionToolCall, Item, MediaArtifact, MediaKind, Result, ToolCallResult, ToolErrorKind,
    ToolOutputPart, ToolSignalLevel,
};

#[derive(Debug)]
pub(crate) struct Batch {
    pub is_concurrency_safe: bool,
    pub blocks: Vec<FunctionToolCall>,
}

enum Schedule {
    Effects(ToolEffects),
    Legacy { keys: Vec<ResourceKey> },
}

pub(crate) fn partition_tool_calls(
    tool_uses: &[FunctionToolCall],
    tools: &[Arc<dyn Tool>],
    workspace_root: &Path,
    path_mode_for: impl Fn(&str) -> crate::workspace::ToolPathMode,
) -> Vec<Batch> {
    let mut batches: Vec<Batch> = Vec::new();

    for tu in tool_uses {
        let tu_input = serde_json::from_str(&tu.arguments).unwrap_or(serde_json::Value::Null);
        let tool = tools.iter().find(|t| t.name() == tu.name);
        let (is_safe, schedule) = match tool {
            Some(tool) => tool_schedule(
                tool.as_ref(),
                &tu_input,
                path_mode_for(&tu.name),
                workspace_root,
            ),
            None => (false, Schedule::Legacy { keys: Vec::new() }),
        };

        if is_safe
            && batches.last().is_some_and(|b| {
                b.is_concurrency_safe
                    && !batch_conflicts_with(
                        b,
                        &schedule,
                        tools,
                        workspace_root,
                        &path_mode_for,
                    )
            })
        {
            batches.last_mut().unwrap().blocks.push(tu.clone());
        } else {
            batches.push(Batch {
                is_concurrency_safe: is_safe,
                blocks: vec![tu.clone()],
            });
        }
    }

    batches
}

fn tool_schedule(
    tool: &dyn Tool,
    input: &serde_json::Value,
    path_mode: crate::workspace::ToolPathMode,
    workspace_root: &Path,
) -> (bool, Schedule) {
    let effects = tool.effects(input, path_mode, workspace_root);
    if effects.declares() {
        let safe = effects.writes.is_empty();
        (safe, Schedule::Effects(effects))
    } else {
        (
            tool.is_concurrency_safe(input),
            Schedule::Legacy {
                keys: tool.resource_keys(input, path_mode, workspace_root),
            },
        )
    }
}

fn batch_conflicts_with(
    batch: &Batch,
    schedule: &Schedule,
    tools: &[Arc<dyn Tool>],
    workspace_root: &Path,
    path_mode_for: &impl Fn(&str) -> crate::workspace::ToolPathMode,
) -> bool {
    for existing in &batch.blocks {
        let input = serde_json::from_str(&existing.arguments).unwrap_or(serde_json::Value::Null);
        let Some(tool) = tools.iter().find(|t| t.name() == existing.name) else {
            continue;
        };
        let (_, existing_schedule) = tool_schedule(
            tool.as_ref(),
            &input,
            path_mode_for(&existing.name),
            workspace_root,
        );
        if schedules_conflict(schedule, &existing_schedule) {
            return true;
        }
    }
    false
}

fn schedules_conflict(left: &Schedule, right: &Schedule) -> bool {
    match (left, right) {
        (Schedule::Effects(left), Schedule::Effects(right)) => effects_conflict(left, right),
        (Schedule::Legacy { keys: left }, Schedule::Legacy { keys: right }) => {
            resource_keys_conflict(left, right)
        }
        (Schedule::Effects(effects), Schedule::Legacy { keys })
        | (Schedule::Legacy { keys }, Schedule::Effects(effects)) => {
            legacy_effects_conflict(keys, effects)
        }
    }
}

fn effects_conflict(left: &ToolEffects, right: &ToolEffects) -> bool {
    if involves_workspace(left) || involves_workspace(right) {
        return left.declares() && right.declares();
    }
    overlaps(&left.writes, &right.writes)
        || overlaps(&left.writes, &right.reads)
        || overlaps(&left.reads, &right.writes)
}

fn legacy_effects_conflict(keys: &[ResourceKey], effects: &ToolEffects) -> bool {
    if keys.iter().any(|key| matches!(key, ResourceKey::Workspace)) || involves_workspace(effects)
    {
        return !keys.is_empty() && effects.declares();
    }
    keys.iter()
        .any(|key| effects.reads.contains(key) || effects.writes.contains(key))
}

fn involves_workspace(effects: &ToolEffects) -> bool {
    effects
        .reads
        .iter()
        .chain(effects.writes.iter())
        .any(|key| matches!(key, ResourceKey::Workspace))
}

fn overlaps(left: &[ResourceKey], right: &[ResourceKey]) -> bool {
    left.iter().any(|key| right.contains(key))
}

fn resource_keys_conflict(a: &[ResourceKey], b: &[ResourceKey]) -> bool {
    for x in a {
        for y in b {
            if x == y {
                return true;
            }
            if matches!(x, ResourceKey::Workspace) || matches!(y, ResourceKey::Workspace) {
                return true;
            }
        }
    }
    false
}

fn timeout_budget_label(budget: Duration) -> String {
    if budget.as_millis() % 1000 == 0 {
        format!("{} seconds", budget.as_secs())
    } else {
        format!("{}ms", budget.as_millis())
    }
}

fn cancelled_tool_result(name: &str) -> ToolCallResult {
    ToolCallResult::error_with_kind(
        ToolErrorKind::Cancelled,
        format!("tool '{name}' cancelled"),
        false,
    )
}

fn wire_result(result: ToolCallResult) -> ToolCallResult {
    result.finalize_signals()
}

/// Direct callers compose wire text. Code callers keep `data` / diagnostics
/// and skip signal composition, emergency cap, spill, and token truncation.
fn present_for_caller(origin: &ToolCallOrigin, result: ToolCallResult) -> ToolCallResult {
    if matches!(origin.caller, ToolCaller::Code { .. }) {
        result
    } else {
        wire_result(result)
    }
}

fn attach_unknown_param_warning(result: ToolCallResult, extra: &[String]) -> ToolCallResult {
    if extra.is_empty() || result.level == ToolSignalLevel::Error {
        return result;
    }
    let msg = format!(
        "ignored unknown parameter(s): {}. Not in this tool's schema; the call ran without them",
        extra.join(", ")
    );
    let existing = result
        .warning_status
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if let Some(existing) = existing {
        result.with_warning(format!("{existing}; {msg}"))
    } else {
        result.with_warning(msg)
    }
}

fn call_id(fc: &FunctionToolCall) -> String {
    fc.call_id.clone()
}

fn filename_for_artifact(artifact: &MediaArtifact) -> String {
    let ext = match artifact.mime_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" => "wav",
        "audio/ogg" => "ogg",
        other if other.contains('/') => other.split('/').nth(1).unwrap_or("bin"),
        _ => "bin",
    };
    match artifact.kind {
        MediaKind::Image => format!("image.{ext}"),
        MediaKind::Video => format!("video.{ext}"),
        MediaKind::Audio => format!("audio.{ext}"),
    }
}

/// Materialize one media artifact into authority [`InputContent`].
///
/// Images → [`InputContent::InputImage`]; video/audio → [`InputContent::InputFile`].
/// Unmaterialized `LocalFile`, missing mime / data_root, or missing blob → [`Err`].
fn input_content_from_artifact(
    artifact: &MediaArtifact,
    data_root: Option<&Path>,
) -> Result<InputContent> {
    let resolved = resolve_media_artifact_url(data_root, artifact)?;
    match artifact.kind {
        MediaKind::Image => Ok(InputContent::InputImage(InputImageContent {
            detail: Default::default(),
            file_id: None,
            image_url: Some(resolved),
        })),
        MediaKind::Video | MediaKind::Audio => {
            let filename = Some(filename_for_artifact(artifact));
            if resolved.starts_with("data:") {
                Ok(InputContent::InputFile(InputFileContent {
                    file_data: Some(resolved),
                    file_id: None,
                    file_url: None,
                    filename,
                    detail: None,
                }))
            } else {
                Ok(InputContent::InputFile(InputFileContent {
                    file_data: None,
                    file_id: None,
                    file_url: Some(resolved),
                    filename,
                    detail: None,
                }))
            }
        }
    }
}

/// Map [`ToolCallResult`] → [`FunctionCallOutput`], materializing media into
/// authority [`InputContent`] (no text-stub dialects for resolvable media).
fn function_call_output_from_result(
    result: &ToolCallResult,
    data_root: Option<&Path>,
) -> Result<FunctionCallOutput> {
    if result.parts.is_empty() {
        return Ok(FunctionCallOutput::Text(result.content.clone()));
    }

    let mut contents: Vec<InputContent> = Vec::new();
    if !result.content.is_empty() {
        contents.push(InputContent::InputText(InputTextContent {
            text: result.content.clone(),
        }));
    }

    for part in &result.parts {
        match part {
            ToolOutputPart::Text { text } => {
                contents.push(InputContent::InputText(InputTextContent {
                    text: text.clone(),
                }));
            }
            ToolOutputPart::Media { artifact } => {
                contents.push(input_content_from_artifact(artifact, data_root)?);
            }
        }
    }

    if contents.is_empty() {
        Ok(FunctionCallOutput::Text(result.content.clone()))
    } else {
        Ok(FunctionCallOutput::Content(contents))
    }
}

fn function_call_output_item(
    call_id: String,
    result: &ToolCallResult,
    data_root: Option<&Path>,
) -> Item {
    match function_call_output_from_result(result, data_root) {
        Ok(output) => Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id,
            output,
            id: None,
            status: None,
        }),
        // Surface encode failure as a tool error (no placeholder media dialects).
        Err(e) => Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id,
            output: FunctionCallOutput::Text(
                wire_result(ToolCallResult::error(e.to_string())).content,
            ),
            id: None,
            status: None,
        }),
    }
}

/// Drop guard that releases all of a session's write locks when the function
/// returns (success or error). Ensures a tool never leaks a held lock.
struct WriteLockGuard {
    lock: Arc<WorkspaceWriteLock>,
    session_id: String,
}

impl Drop for WriteLockGuard {
    fn drop(&mut self) {
        self.lock.release_all(&self.session_id);
    }
}

fn audit_truncate(s: &str) -> String {
    s.chars().take(200).collect()
}

fn emit_audit(
    data_root: &std::path::Path,
    phase: &str,
    session_id: &str,
    tool: &str,
    call_id: &str,
    tweak: impl FnOnce(&mut AuditEvent),
) {
    let mut event = AuditEvent::new(phase, session_id, tool, call_id);
    tweak(&mut event);
    emit_tool_audit(data_root, &event);
}

/// Who is invoking the single-call pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCaller {
    Direct,
    /// Nested call inside `run_code`. `parent_call_id` is the outer call.
    Code {
        parent_call_id: String,
    },
}

/// Per-call side effect shared by direct and nested callers.
pub type PlanTouch = Arc<dyn Fn(&FunctionToolCall) + Send + Sync>;

/// Facts that differ between a direct call and a nested code call.
/// Both callers enter [`execute_tool_call`].
#[derive(Clone)]
pub struct ToolCallOrigin {
    pub caller: ToolCaller,
    /// Admitted code-surface names, used only to route a direct call.
    pub code_names: Arc<[String]>,
    pub file_note: Option<Arc<dyn Fn(&[PathBuf]) + Send + Sync>>,
    pub permission_sink: Option<Arc<dyn PermissionSink>>,
    /// Fired once when a callable tool settles, for both callers.
    pub plan_touch: Option<PlanTouch>,
}

impl ToolCallOrigin {
    pub fn direct() -> Self {
        Self {
            caller: ToolCaller::Direct,
            code_names: Arc::from([]),
            file_note: None,
            permission_sink: None,
            plan_touch: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_tool(
    tool_use: &FunctionToolCall,
    tools: &[Arc<dyn Tool>],
    permission: &PermissionEngine,
    ctx: &Context,
    session_id: &str,
    agent_name: &str,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
    data_root: &std::path::Path,
    spill_threshold: usize,
    turn_anchor_seq: Option<i64>,
    write_lock: Arc<WorkspaceWriteLock>,
    session: Option<crate::session::SessionDataReader>,
) -> ToolCallResult {
    execute_tool_call(
        tool_use,
        tools,
        permission,
        ctx,
        session_id,
        agent_name,
        sink,
        cancel,
        data_root,
        spill_threshold,
        turn_anchor_seq,
        write_lock,
        session,
        ToolCallOrigin::direct(),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_call(
    tool_use: &FunctionToolCall,
    tools: &[Arc<dyn Tool>],
    permission: &PermissionEngine,
    ctx: &Context,
    session_id: &str,
    agent_name: &str,
    sink: &dyn PermissionSink,
    cancel: CancellationToken,
    data_root: &std::path::Path,
    spill_threshold: usize,
    _turn_anchor_seq: Option<i64>,
    write_lock: Arc<WorkspaceWriteLock>,
    session: Option<crate::session::SessionDataReader>,
    origin: ToolCallOrigin,
) -> ToolCallResult {
    let tu_name = tool_use.name.clone();
    let tu_id = call_id(tool_use);

    if cancel.is_cancelled() {
        if let Some(tool) = callable_tool(tools, &tu_name, &origin) {
            note_settled_call(&origin, tool.as_ref(), tool_use, permission, &ctx.cwd);
        }
        return present_for_caller(&origin, cancelled_tool_result(&tu_name));
    }

    let tool = match callable_tool(tools, &tu_name, &origin) {
        Some(tool) => tool,
        None => return surface_or_missing(&tu_name, tools, &origin),
    };
    let _note = ArmedNote {
        arm: true,
        tool: Some(Arc::clone(&tool)),
        origin: &origin,
        call: tool_use,
        permission,
        cwd: &ctx.cwd,
    };

    tool.set_active_session(session_id.to_string());
    if let Some(permission_sink) = &origin.permission_sink {
        tool.set_permission_sink(Arc::clone(permission_sink));
    }
    if let ToolCaller::Code { parent_call_id } = &origin.caller {
        tracing::debug!(%parent_call_id, tool = %tu_name, "nested code tool call");
    }

    let mut tool_use_input = match parse_tool_arguments(&tool_use.arguments) {
        Ok(value) => value,
        Err(detail) => {
            return present_for_caller(
                &origin,
                ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    invalid_input_for(&tu_name, detail),
                    false,
                ),
            );
        }
    };
    tool.prepare_input(&mut tool_use_input);
    let schema_input = tool.schema_arguments(&tool_use_input);

    if let Err(msg) = check_tool_input(tool.as_ref(), &schema_input) {
            return present_for_caller(
                &origin,
                ToolCallResult::error_with_kind(
                    ToolErrorKind::InvalidInput,
                    invalid_input_for(&tu_name, msg),
                    false,
                )
                .with_error_code("invalid_input"),
            );
    }

    // WP-188 observe-only: start → auth_ask → authorize → auth_done (no AskOutcome change).
    emit_audit(data_root, "start", session_id, &tu_name, &tu_id, |_| {});
    emit_audit(data_root, "auth_ask", session_id, &tu_name, &tu_id, |_| {});

    let auth = authorize(
        tool_use,
        tool.as_ref(),
        permission,
        ctx,
        agent_name,
        sink,
        &cancel,
    )
    .await;

    let auth_outcome = match &auth {
        AuthResult::Proceed { .. } => "proceed",
        AuthResult::Denied(_) => "deny",
        AuthResult::Aborted => "abort",
    };
    emit_audit(data_root, "auth_done", session_id, &tu_name, &tu_id, |e| {
        e.auth_outcome = Some(auth_outcome.to_string());
    });

    if matches!(auth, AuthResult::Aborted) {
        cancel.cancel();
        emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
            e.ok = Some(false);
            e.error_kind = Some("canceled".into());
            e.error = Some("aborted".into());
        });
        return present_for_caller(&origin, cancelled_tool_result(&tu_name));
    }

    if cancel.is_cancelled() {
        emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
            e.ok = Some(false);
            e.error_kind = Some("canceled".into());
        });
        return present_for_caller(&origin, cancelled_tool_result(&tu_name));
    }

    let effective_input = match auth {
        AuthResult::Denied(msg) => {
            emit_audit(data_root, "exec_done", session_id, &tu_name, &tu_id, |e| {
                e.ok = Some(false);
                e.error_kind = Some("denied".into());
                e.error = Some(audit_truncate(&msg));
            });
            return present_for_caller(
                &origin,
                ToolCallResult::error_with_kind(ToolErrorKind::PermissionDenied, msg, false),
            );
        }
        AuthResult::Aborted => {
            emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
                e.ok = Some(false);
                e.error_kind = Some("canceled".into());
            });
            return present_for_caller(&origin, cancelled_tool_result(&tu_name));
        }
        AuthResult::Proceed { effective_input } => effective_input,
    };

    let path_mode = permission.path_mode(&tu_name).to_tool_path_mode();
    let effects = tool.effects(&effective_input, path_mode, &ctx.cwd);
    let _effect_guard = match process_resource_gate().acquire(&effects, &cancel).await {
        Ok(guard) => guard,
        Err(()) => {
            emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
                e.ok = Some(false);
                e.error_kind = Some("canceled".into());
            });
            return present_for_caller(&origin, cancelled_tool_result(&tu_name));
        }
    };
    if cancel.is_cancelled() {
        emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
            e.ok = Some(false);
            e.error_kind = Some("canceled".into());
        });
        return present_for_caller(&origin, cancelled_tool_result(&tu_name));
    }
    let resource_keys = effects.writes.clone();
    let _lock_guard = if !resource_keys.is_empty() {
        match write_lock.try_acquire(&resource_keys, session_id) {
            Ok(()) => Some(WriteLockGuard {
                lock: Arc::clone(&write_lock),
                session_id: session_id.to_string(),
            }),
            Err(holder) => {
                let code_caller = matches!(origin.caller, ToolCaller::Code { .. });
                let msg = if code_caller {
                    format!("resource busy: held by session {holder}")
                } else {
                    format!(
                        "resource busy: held by session {holder}. Retry after the other session finishes writing"
                    )
                };
                emit_audit(data_root, "exec_done", session_id, &tu_name, &tu_id, |e| {
                    e.ok = Some(false);
                    e.error_kind = Some("other".into());
                    e.error = Some(audit_truncate(&msg));
                });
                let mut result =
                    ToolCallResult::error_with_kind(ToolErrorKind::Conflict, msg, true);
                if code_caller {
                    result = result.with_error_code("conflict").with_error_details(
                        serde_json::json!({ "holder": holder }),
                    );
                }
                return present_for_caller(&origin, result);
            }
        }
    } else {
        None
    };

    if cancel.is_cancelled() {
        emit_audit(data_root, "cancel", session_id, &tu_name, &tu_id, |e| {
            e.ok = Some(false);
            e.error_kind = Some("canceled".into());
        });
        return present_for_caller(&origin, cancelled_tool_result(&tu_name));
    }

    let tool_clone = tool.clone();
    let input = effective_input.clone();
    let name = tu_name.clone();
    // A child token lets the tool timeout stop its own work without cancelling
    // the surrounding turn. The parent still propagates into this child.
    let execution_cancel = cancel.child_token();
    let execution = ToolExecutionContext {
        path_mode,
        workspace_root: ctx.cwd.clone(),
        call_id: tu_id.clone(),
        cancel: execution_cancel.clone(),
        output_limit: if matches!(origin.caller, ToolCaller::Code { .. }) {
            usize::MAX
        } else {
            output::direct_output_limit(tool.as_ref())
        },
        session_id: session_id.to_string(),
        session,
    };
    // Build the tool execution future
    let tool_fut = AssertUnwindSafe(tool_clone.execute(input, execution)).catch_unwind();
    tokio::pin!(tool_fut);

    // The tool's own budget is a safety net. On expiry, ask the tool to stop
    // and wait for it to settle before returning the timeout result.
    let raw_result = if let Some(budget) = tool.timeout() {
        let timeout = tokio::time::sleep(budget);
        tokio::pin!(timeout);
        tokio::select! {
            result = &mut tool_fut => result,
            _ = &mut timeout => {
                execution_cancel.cancel();
                let late = tool_fut.await;
                // A result that already committed beats the tool timeout. A
                // cancelled or panicked finish did not commit, so the timeout
                // stands.
                match late {
                    Ok(result)
                        if result.error.as_ref().is_none_or(|error| {
                            error.kind != ToolErrorKind::Cancelled
                        }) =>
                    {
                        tracing::warn!(tool = %name, "completed after timeout");
                        Ok(result)
                    }
                    _ => {
                        let budget_label = timeout_budget_label(budget);
                        let message = if matches!(origin.caller, ToolCaller::Code { .. }) {
                            format!("tool '{name}' timed out after {budget_label}")
                        } else {
                            format!(
                                "tool '{name}' timed out after {budget_label}. Retry with a narrower scope, or use bash run_in_background for long commands"
                            )
                        };
                        Ok(ToolCallResult::error_with_kind(
                            ToolErrorKind::Timeout,
                            message,
                            true,
                        ))
                    }
                }
            }
        }
    } else {
        tool_fut.await
    };

    // Once a tool future returns, its result is the source of truth. The
    // cancellation token may have fired while a mutating tool was finishing
    // post-write sync/feedback; replacing that result would hide committed
    // side effects and invite an unsafe retry.
    let mut panicked = false;
    let output = match raw_result {
        Ok(r) => r,
        Err(panic_payload) => {
            let msg = panic_payload_to_string(panic_payload);
            tracing::error!(tool = %name, "tool panicked: {}", msg);
            panicked = true;
            emit_audit(data_root, "panic", session_id, &name, &tu_id, |e| {
                e.ok = Some(false);
                e.error_kind = Some("panic".into());
                e.error = Some(audit_truncate(&msg));
            });
            ToolCallResult::error_with_kind(
                ToolErrorKind::Internal,
                format!("tool '{name}' panicked: {msg}"),
                false,
            )
        }
    };
    let unknown = unknown_top_level_properties(&tool.schema(), &effective_input);
    let output = if matches!(origin.caller, ToolCaller::Code { .. }) {
        enforce_code_output(tool.as_ref(), output)
    } else if tool.is_code_transport() {
        let output = wire_result(attach_unknown_param_warning(output, &unknown));
        if output.content.len() > crate::tools::code::RUN_CODE_VIEW_MAX_BYTES {
            tracing::warn!(
                bytes = output.content.len(),
                limit = crate::tools::code::RUN_CODE_VIEW_MAX_BYTES,
                "run_code model view exceeded its budget"
            );
        }
        output
    } else {
        let mut output = attach_unknown_param_warning(output, &unknown);
        output = wire_result(output);
        output.content = output::cap_direct_content(tool.as_ref(), &output.content);
        match output::finalize_tool_call_result(output, data_root, spill_threshold) {
            Ok(processed) => processed,
            Err(e) => {
                tracing::error!(tool = %tu_name, error = %e, "tool output processing failed");
                wire_result(ToolCallResult::error_with_kind(
                    ToolErrorKind::Io,
                    format!("output processing failed: {e}"),
                    false,
                ))
            }
        }
    };
    tracing::info!(
        tool = %tu_name,
        id = %tu_id,
        output_len = output.content.len(),
        "tool executed"
    );

    // WP-188 observe-only: exec_done (skip if panic already recorded).
    if !panicked {
        let ok = output.level != crate::types::ToolSignalLevel::Error;
        let err_kind = if ok {
            None
        } else if output.content.contains("timed out") {
            Some("timeout".into())
        } else {
            Some("output".into())
        };
        emit_audit(data_root, "exec_done", session_id, &tu_name, &tu_id, |e| {
            e.ok = Some(ok);
            e.error_kind = err_kind;
            if !ok {
                e.error = Some(audit_truncate(&output.content));
            }
        });
    }

    output
}

fn callable_tool(
    tools: &[Arc<dyn Tool>],
    name: &str,
    origin: &ToolCallOrigin,
) -> Option<Arc<dyn Tool>> {
    let tool = tools.iter().find(|tool| tool.name() == name)?.clone();
    if surface_mismatch(tool.as_ref(), origin) {
        None
    } else {
        Some(tool)
    }
}

fn surface_mismatch(tool: &dyn Tool, origin: &ToolCallOrigin) -> bool {
    match &origin.caller {
        ToolCaller::Direct => tool.surface() == ToolSurface::Code,
        ToolCaller::Code { .. } => tool.surface() != ToolSurface::Code,
    }
}

fn surface_or_missing(
    name: &str,
    tools: &[Arc<dyn Tool>],
    origin: &ToolCallOrigin,
) -> ToolCallResult {
    let declared = tools.iter().find(|tool| tool.name() == name);
    let routed = match &origin.caller {
        ToolCaller::Direct if origin.code_names.iter().any(|n| n == name) => {
            Some(code_route_message(name))
        }
        ToolCaller::Direct if declared.is_some_and(|tool| tool.surface() == ToolSurface::Code) => {
            Some(code_route_message(name))
        }
        ToolCaller::Code { .. }
            if declared.is_some_and(|tool| tool.surface() != ToolSurface::Code) =>
        {
            Some(format!(
                "`{name}` is a direct tool and is not available inside run_code"
            ))
        }
        _ => None,
    };
    let message = routed.unwrap_or_else(|| format!("tool '{name}' not found"));
    present_for_caller(
        origin,
        ToolCallResult::error_with_kind(ToolErrorKind::NotFound, message, false),
    )
}

fn code_route_message(name: &str) -> String {
    if is_js_identifier(name) {
        format!("`{name}` is only available as tools.{name} inside run_code")
    } else {
        let key = serde_json::to_string(name).unwrap_or_else(|_| "\"\"".to_string());
        format!("`{name}` is only available as tools[{key}] inside run_code")
    }
}

fn is_js_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

fn enforce_code_output(tool: &dyn Tool, result: ToolCallResult) -> ToolCallResult {
    let success = result.error.is_none()
        && matches!(result.level, ToolSignalLevel::Ok | ToolSignalLevel::Warning);
    if !success {
        return result;
    }
    let name = tool.name();
    let Some(data) = result.data.as_ref() else {
        let reason = format!("tool '{name}' returned success without data");
        tracing::error!(tool = %name, "{reason}");
        return ToolCallResult::error_with_kind(ToolErrorKind::Internal, reason, false);
    };
    let Some(schema) = tool.data_schema() else {
        let reason = format!("tool '{name}' is on the code surface without a data schema");
        tracing::error!(tool = %name, "{reason}");
        return ToolCallResult::error_with_kind(ToolErrorKind::Internal, reason, false);
    };
    if let Err(error) = schema_validate(&schema, data) {
        let reason = format!("tool '{name}' returned data that does not match its schema: {error}");
        tracing::error!(tool = %name, "{reason}");
        return ToolCallResult::error_with_kind(ToolErrorKind::Internal, reason, false);
    }
    result
}

fn note_settled_call(
    origin: &ToolCallOrigin,
    tool: &dyn Tool,
    call: &FunctionToolCall,
    permission: &PermissionEngine,
    cwd: &Path,
) {
    note_tool_paths(origin, tool, call, permission, cwd);
    if let Some(hook) = &origin.plan_touch {
        hook(call);
    }
}

fn note_tool_paths(
    origin: &ToolCallOrigin,
    tool: &dyn Tool,
    call: &FunctionToolCall,
    permission: &PermissionEngine,
    cwd: &Path,
) {
    let Some(hook) = &origin.file_note else {
        return;
    };
    let input = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
    let mode = permission.path_mode(call.name.as_str()).to_tool_path_mode();
    let paths = tool
        .resource_keys(&input, mode, cwd)
        .into_iter()
        .filter_map(|key| match key {
            ResourceKey::File(path) => Some(PathBuf::from(path)),
            ResourceKey::Workspace => None,
        })
        .collect::<Vec<_>>();
    if !paths.is_empty() {
        hook(&paths);
    }
}

struct ArmedNote<'a> {
    arm: bool,
    tool: Option<Arc<dyn Tool>>,
    origin: &'a ToolCallOrigin,
    call: &'a FunctionToolCall,
    permission: &'a PermissionEngine,
    cwd: &'a Path,
}

impl Drop for ArmedNote<'_> {
    fn drop(&mut self) {
        if !self.arm {
            return;
        }
        let Some(tool) = self.tool.as_deref() else {
            return;
        };
        note_settled_call(self.origin, tool, self.call, self.permission, self.cwd);
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Build FunctionCallOutput Items only (FunctionCall already in transcript from model).
///
/// `data_root` is required to resolve `BlobRef` media into data URLs. Encode failures
/// for a single call become tool-error text outputs (fail closed — no stub dialects).
pub fn outputs_from_tool_results(
    tool_uses: &[FunctionToolCall],
    results_by_id: HashMap<String, ToolCallResult>,
    data_root: &Path,
) -> Vec<Item> {
    let mut items = Vec::new();
    for fc in tool_uses {
        let cid = call_id(fc);
        let result = results_by_id.get(&cid);
        let item = match result {
            Some(r) => function_call_output_item(cid, r, Some(data_root)),
            // Reached on cancelled turns (user stopped the loop) and as a
            // fail-closed fallback: the model must never see a dangling tool call.
            None => Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: cid.clone(),
                output: FunctionCallOutput::Text(
                    wire_result(ToolCallResult::error(format!(
                        "tool '{}' was interrupted: the user cancelled the turn before a result arrived",
                        fc.name
                    )))
                    .content,
                ),
                id: None,
                status: None,
            }),
        };
        items.push(item);
    }
    items
}

/// Alias kept for call sites that still say "items from results" — output-only.
pub fn items_from_tool_results(
    tool_uses: &[FunctionToolCall],
    results_by_id: HashMap<String, ToolCallResult>,
    data_root: &Path,
) -> Vec<Item> {
    outputs_from_tool_results(tool_uses, results_by_id, data_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::media::write_media_blob;
    use crate::types::{MediaArtifact, MediaSource, ToolOutputPart};

    #[test]
    fn cross_session_lock_follows_effect_writes() {
        let root = Path::new("/proj");
        let mode = crate::workspace::ToolPathMode::All;
        let write = crate::tools::write::WriteTool::new().effects(
            &serde_json::json!({"path": "a.txt", "content": "x"}),
            mode,
            root,
        );
        let edit = crate::tools::edit::EditTool::new().effects(
            &serde_json::json!({"path": "a.txt", "edits": []}),
            mode,
            root,
        );
        let read = crate::tools::read::ReadTool::default().effects(
            &serde_json::json!({"path": "a.txt"}),
            mode,
            root,
        );
        assert!(!write.writes.is_empty());
        assert_eq!(write.writes, edit.writes);
        assert!(read.writes.is_empty());
    }

    #[test]
    fn blob_ref_image_becomes_input_image_data_url() {
        let dir = tempfile::tempdir().unwrap();
        let blob_id = write_media_blob(dir.path(), b"\x89PNG").unwrap();
        let result = ToolCallResult {
            content: "shot".into(),
            parts: vec![ToolOutputPart::Media {
                artifact: MediaArtifact::image(MediaSource::BlobRef { blob_id }, "image/png"),
            }],
            ..ToolCallResult::ok("")
        };
        let out = function_call_output_from_result(&result, Some(dir.path())).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                let img = parts.iter().find_map(|p| match p {
                    InputContent::InputImage(i) => Some(i),
                    _ => None,
                });
                let img = img.expect("InputImage");
                let url = img.image_url.as_deref().expect("image_url");
                assert!(url.starts_with("data:image/png;base64,"));
                assert!(!parts.iter().any(|p| matches!(
                    p,
                    InputContent::InputText(t) if t.text.contains("blob:")
                )));
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    #[test]
    fn local_file_after_finalize_encodes_as_input_image() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("shot.png");
        // Minimal PNG header so finalize can optionally read dims; bytes still materialize.
        std::fs::write(&file_path, b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR").unwrap();
        let raw = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::image_file(
                file_path.to_string_lossy(),
                "image/png",
            )],
            ..ToolCallResult::ok("")
        };
        let finalized = output::finalize_tool_call_result(raw, dir.path(), 1024).unwrap();
        assert!(matches!(
            finalized.parts[0],
            ToolOutputPart::Media {
                artifact: MediaArtifact {
                    source: MediaSource::BlobRef { .. },
                    ..
                }
            }
        ));
        let out = function_call_output_from_result(&finalized, Some(dir.path())).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                assert!(
                    parts
                        .iter()
                        .any(|p| matches!(p, InputContent::InputImage(_)))
                );
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    #[test]
    fn missing_blob_returns_err() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::Media {
                artifact: MediaArtifact::image(
                    MediaSource::BlobRef {
                        blob_id: "missing-blob".into(),
                    },
                    "image/png",
                ),
            }],
            ..ToolCallResult::ok("")
        };
        let dir = tempfile::tempdir().unwrap();
        let err = function_call_output_from_result(&result, Some(dir.path())).unwrap_err();
        assert!(
            matches!(err, crate::types::LitecodeError::MediaBlobMissing(_))
                || err.to_string().contains("missing")
                || err.to_string().contains("unavailable")
        );
    }

    #[test]
    fn unmaterialized_local_file_returns_err() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::image_file("/tmp/nope.png", "image/png")],
            ..ToolCallResult::ok("")
        };
        let err = function_call_output_from_result(&result, None).unwrap_err();
        assert!(err.to_string().contains("unmaterialized"));
    }

    #[test]
    fn video_becomes_input_file() {
        let result = ToolCallResult {
            content: String::new(),
            parts: vec![ToolOutputPart::video(
                MediaSource::Url {
                    url: "https://example.com/v.mp4".into(),
                },
                "video/mp4",
            )],
            ..ToolCallResult::ok("")
        };
        let out = function_call_output_from_result(&result, None).unwrap();
        match out {
            FunctionCallOutput::Content(parts) => {
                let file = parts.iter().find_map(|p| match p {
                    InputContent::InputFile(f) => Some(f),
                    _ => None,
                });
                let file = file.expect("InputFile");
                assert_eq!(file.file_url.as_deref(), Some("https://example.com/v.mp4"));
            }
            FunctionCallOutput::Text(t) => panic!("expected Content, got Text: {t}"),
        }
    }

    /// 2.6: a blocking tool whose work runs in `spawn_blocking` (the fixed
    /// custom/webfetch pattern) must be interruptible by the pipeline timeout —
    /// it must not stall the async executor so the timeout never fires.
    #[tokio::test]
    async fn blocking_tool_is_interrupted_by_pipeline_timeout() {
        struct SlowTool;
        impl Tool for SlowTool {
            fn name(&self) -> &str {
                "slow"
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }
            fn description(&self, _ctx: &Context) -> String {
                "blocks for a while".into()
            }
            fn timeout(&self) -> Option<Duration> {
                Some(Duration::from_secs(1))
            }
            fn execute(
                &self,
                _input: serde_json::Value,
                _execution: ToolExecutionContext,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
            {
                let cancel = _execution.cancel.clone();
                Box::pin(async move {
                    // Mirrors the 2.6 fix: blocking work runs off the executor so
                    // the pipeline timeout can interrupt the call.
                    let _ = tokio::task::spawn_blocking(|| {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    })
                    .await;
                    if cancel.is_cancelled() {
                        return ToolCallResult::error_with_kind(
                            ToolErrorKind::Cancelled,
                            "slow tool cancelled",
                            false,
                        );
                    }
                    ToolCallResult::ok("done")
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut global = crate::config::schema::GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    "slow".to_string(),
                    crate::config::schema::AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve_without_catalog(
            global,
            crate::config::resolved::WorkspaceState::new("/tmp/test"),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(SlowTool)];
        let tu = FunctionToolCall {
            arguments: "{}".into(),
            call_id: "call_1".into(),
            name: "slow".into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let start = std::time::Instant::now();
        let result = run_tool(
            &tu,
            &tools,
            &permission,
            &ctx,
            "sess",
            "default",
            &sink,
            CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        assert!(
            result.content.contains("timed out after 1 seconds"),
            "expected timeout error, got: {:?}",
            result.content
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(3),
            "timeout did not interrupt promptly: {:?}",
            start.elapsed()
        );
    }

    /// E-7: a tool that ignores cancellation and returns a committed result
    /// after the budget wins over the tool timeout.
    #[tokio::test]
    async fn committed_result_wins_over_tool_timeout() {
        struct CommitTool;
        impl Tool for CommitTool {
            fn name(&self) -> &str {
                "commit"
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object", "properties": {}})
            }
            fn description(&self, _ctx: &Context) -> String {
                "returns after the budget".into()
            }
            fn timeout(&self) -> Option<Duration> {
                Some(Duration::from_secs(1))
            }
            fn execute(
                &self,
                _input: serde_json::Value,
                _execution: ToolExecutionContext,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
            {
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    ToolCallResult::ok("committed").with_data(serde_json::json!({"ok": true}))
                })
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut global = crate::config::schema::GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    "commit".to_string(),
                    crate::config::schema::AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve_without_catalog(
            global,
            crate::config::resolved::WorkspaceState::new("/tmp/test"),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(CommitTool)];
        let tu = FunctionToolCall {
            arguments: "{}".into(),
            call_id: "call_commit".into(),
            name: "commit".into(),
            namespace: None,
            id: Some("fc_commit".into()),
            status: None,
        };
        let result = run_tool(
            &tu,
            &tools,
            &permission,
            &ctx,
            "sess",
            "default",
            &sink,
            CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        assert!(result.error.is_none(), "{}", result.content);
        assert!(result.content.contains("committed"), "{}", result.content);
        assert!(!result.content.contains("timed out"), "{}", result.content);
    }

    fn fc(call_id: &str, name: &str, args: serde_json::Value) -> FunctionToolCall {
        FunctionToolCall {
            arguments: args.to_string(),
            call_id: call_id.into(),
            name: name.into(),
            namespace: None,
            id: Some(call_id.into()),
            status: None,
        }
    }

    #[test]
    fn partition_splits_same_path_read_and_write() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::read::ReadTool::default()),
            Arc::new(crate::tools::write::WriteTool::default()),
        ];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"path": "a.txt"})),
            fc(
                "w1",
                "write",
                serde_json::json!({"path": "a.txt", "content": "x"}),
            ),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert!(
            batches.len() >= 2,
            "same-path read/write must not share a concurrent batch, got {batches:?}"
        );
        assert!(
            !batches
                .iter()
                .any(|b| b.is_concurrency_safe && b.blocks.len() > 1),
            "overlapping path tools must not run in one concurrent batch"
        );
    }

    #[test]
    fn partition_splits_same_path_read_and_readonly_bash() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::read::ReadTool::default()),
            Arc::new(crate::tools::bash::BashTool::new(Arc::new(
                crate::terminal::TerminalHub::new(),
            ))),
        ];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"path": "a.txt"})),
            fc("b1", "bash", serde_json::json!({"command": "cat a.txt"})),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert!(
            batches.len() >= 2,
            "same-path read and bash must not share a concurrent batch, got {} batches",
            batches.len()
        );
    }

    #[test]
    fn partition_keeps_different_path_reads_concurrent() {
        let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(crate::tools::read::ReadTool::default())];
        let root = Path::new("/proj");
        let uses = vec![
            fc("r1", "read", serde_json::json!({"path": "a.txt"})),
            fc("r2", "read", serde_json::json!({"path": "b.txt"})),
        ];
        let batches =
            partition_tool_calls(&uses, &tools, root, |_| crate::workspace::ToolPathMode::All);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].is_concurrency_safe);
        assert_eq!(batches[0].blocks.len(), 2);
    }

    #[test]
    fn bash_is_cancellable_other_tools_are_not_by_default() {
        let bash = crate::tools::bash::BashTool::new(Arc::new(crate::terminal::TerminalHub::new()));
        assert!(bash.is_cancellable());
        struct Dummy;
        impl Tool for Dummy {
            fn name(&self) -> &str {
                "dummy"
            }
            fn schema(&self) -> serde_json::Value {
                serde_json::json!({})
            }
            fn description(&self, _ctx: &crate::context_pipeline::Context) -> String {
                String::new()
            }
        }
        assert!(!Dummy.is_cancellable());
    }

    async fn run_schema_tool(tool: Arc<dyn Tool>, arguments: &str) -> ToolCallResult {
        let name = tool.name().to_string();
        let dir = tempfile::tempdir().unwrap();
        let mut global = crate::config::schema::GlobalSettings::default();
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: HashMap::from([(
                    name.clone(),
                    crate::config::schema::AgentToolBinding {
                        enabled: true,
                        policy: crate::permission::ToolPolicy::allow_all(),
                        path_mode: crate::permission::BindingPathMode::default(),
                        last_applied_preset: None,
                        allowed_tools: None,
                    },
                )]),
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve_without_catalog(
            global,
            crate::config::resolved::WorkspaceState::new("/tmp/test"),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: dir.path().to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(dir.path()),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tu = FunctionToolCall {
            arguments: arguments.into(),
            call_id: "call_1".into(),
            name,
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let result = run_tool(
            &tu,
            &[tool],
            &permission,
            &ctx,
            "sess",
            "default",
            &sink,
            CancellationToken::new(),
            dir.path(),
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        result
    }

    #[tokio::test]
    async fn glob_wrong_pattern_type_is_unified_error() {
        let result =
            run_schema_tool(Arc::new(crate::tools::glob::GlobTool), r#"{"pattern": 1}"#).await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'glob': parameter 'pattern' expected string, got integer 1"
        );
    }

    #[tokio::test]
    async fn invalid_json_arguments_are_not_swallowed() {
        let result = run_schema_tool(Arc::new(crate::tools::glob::GlobTool), "{not json").await;
        assert!(
            result
                .content
                .starts_with("Error: invalid input for 'glob': arguments were not valid JSON"),
            "{}",
            result.content
        );
    }

    struct CustomShapeTool;

    impl Tool for CustomShapeTool {
        fn name(&self) -> &str {
            "my_custom"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {"key": {"type": "string"}},
                "required": ["key"]
            })
        }
        fn description(&self, _ctx: &Context) -> String {
            "custom-shaped".into()
        }
        fn call_inner(&self, _input: serde_json::Value) -> ToolCallResult {
            ToolCallResult::ok("ran")
        }
    }

    struct SelfCancellingTool;

    impl Tool for SelfCancellingTool {
        fn name(&self) -> &str {
            "self_cancelling"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        fn description(&self, _ctx: &Context) -> String {
            "returns a truthful result after cancellation".into()
        }
        fn execute(
            &self,
            _input: serde_json::Value,
            execution: crate::tool::trait_::ToolExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolCallResult> + Send + '_>>
        {
            Box::pin(async move {
                execution.cancel.cancel();
                ToolCallResult::ok("file committed")
            })
        }
    }

    #[tokio::test]
    async fn completed_tool_result_wins_over_late_cancellation() {
        let result = run_schema_tool(Arc::new(SelfCancellingTool), "{}").await;
        assert_eq!(result.content, "file committed");
    }

    #[tokio::test]
    async fn custom_shaped_schema_rejects_wrong_type() {
        let result = run_schema_tool(Arc::new(CustomShapeTool), r#"{"key": false}"#).await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'my_custom': parameter 'key' expected string, got boolean false"
        );
    }

    #[tokio::test]
    async fn unknown_parameters_warn_and_still_run() {
        let result =
            run_schema_tool(Arc::new(CustomShapeTool), r#"{"key": "ok", "timeout": 3}"#).await;
        assert!(result.content.contains("ran"), "{}", result.content);
        assert!(
            result
                .content
                .contains("ignored unknown parameter(s): timeout"),
            "{}",
            result.content
        );
        assert_eq!(result.level, crate::types::ToolSignalLevel::Warning);
    }

    #[tokio::test]
    async fn custom_shaped_schema_rejects_missing_required() {
        let result = run_schema_tool(Arc::new(CustomShapeTool), "{}").await;
        assert_eq!(
            result.content,
            "Error: invalid input for 'my_custom': missing required parameter 'key'"
        );
    }

    struct EchoTool;

    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo_tool"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        fn description(&self, _ctx: &Context) -> String {
            "echo".into()
        }
        fn call_inner(&self, _input: serde_json::Value) -> ToolCallResult {
            ToolCallResult::ok("echo-ok")
        }
    }

    async fn run_named(
        tools: Vec<Arc<dyn Tool>>,
        name: &str,
        arguments: &str,
        session_id: &str,
        cwd: &std::path::Path,
    ) -> ToolCallResult {
        let mut global = crate::config::schema::GlobalSettings::default();
        let mut bindings = HashMap::new();
        for t in &tools {
            bindings.insert(
                t.name().to_string(),
                crate::config::schema::AgentToolBinding {
                    enabled: true,
                    policy: crate::permission::ToolPolicy::allow_all(),
                    path_mode: crate::permission::BindingPathMode::default(),
                    last_applied_preset: None,
                    allowed_tools: None,
                },
            );
        }
        global.agents.insert(
            "default".into(),
            crate::config::schema::AgentProfile {
                role: crate::config::schema::AgentRole::Primary,
                model_ref: "default".into(),
                tools: bindings,
                ..Default::default()
            },
        );
        let resolved = crate::config::resolved::resolve_without_catalog(
            global,
            crate::config::resolved::WorkspaceState::new(cwd),
        );
        let permission = crate::permission::PermissionEngine::resolver(resolved, "default", 0);
        let ctx = Context {
            cwd: cwd.to_path_buf(),
            workspace_paths: crate::config::WorkspacePaths::for_legacy_root(cwd),
            agents_md: None,
            claude_md: None,
        };
        let sink = crate::permission::sinks::RecordingPermissionSink::from_reply(true, true);
        let tu = FunctionToolCall {
            arguments: arguments.into(),
            call_id: "call_1".into(),
            name: name.into(),
            namespace: None,
            id: Some("fc_1".into()),
            status: None,
        };
        let result = run_tool(
            &tu,
            &tools,
            &permission,
            &ctx,
            session_id,
            "default",
            &sink,
            CancellationToken::new(),
            cwd,
            4096,
            None,
            Arc::new(WorkspaceWriteLock::new()),
            None,
        )
        .await;
        result
    }

    fn wait_job_exit(hub: &crate::terminal::TerminalHub, id: &str) {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(8) {
            if hub.jobs.get(id).is_some_and(|(alive, _, _, _)| !alive) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("job {id} did not exit");
    }

    #[tokio::test]
    async fn mailbox_stays_pending_for_the_next_seam() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let spawned = hub
            .spawn_command("echo hi", None, dir.path(), "sess", "")
            .unwrap();
        wait_job_exit(&hub, &spawned.id);
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(EchoTool),
            Arc::new(crate::tools::bash::BashTool::new(Arc::clone(&hub))),
        ];
        let result = run_named(tools, "echo_tool", "{}", "sess", dir.path()).await;
        assert_eq!(result.content, "echo-ok");
        assert!(!result.content.contains("<system-reminder>"));
        assert!(hub.jobs.mailbox_pending("sess"));
    }

    #[tokio::test]
    async fn wait_shell_exit_does_not_wrap_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(crate::terminal::TerminalHub::new());
        let spawned = hub
            .spawn_command("echo hi", None, dir.path(), "sess", "")
            .unwrap();
        wait_job_exit(&hub, &spawned.id);
        let notice = hub.jobs.notice_snapshot(&spawned.id).expect("notice");
        let expected = crate::tools::bash_status::format_exited_status(
            &notice,
            dir.path(),
            &hub.jobs.running("sess"),
        );
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(crate::tools::wait_shell::WaitShellTool::new(Arc::clone(
                &hub,
            ))),
            Arc::new(crate::tools::bash::BashTool::new(Arc::clone(&hub))),
        ];
        let args = serde_json::json!({ "bash_id": spawned.id }).to_string();
        let result = run_named(tools, "wait_shell", &args, "sess", dir.path()).await;
        assert_eq!(result.content, expected);
        assert!(!result.content.contains("<system-reminder>"));
        assert!(hub.jobs.take_mailbox("sess").is_empty());
    }
}
