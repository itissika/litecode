use crate::config::resolved::ResolvedConfig;
use crate::platform_knobs::{ContextMode, effective_api_model_id, effective_context_window};
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, FailReason, InternalEvent,
    TurnEndReason, TurnPhase,
};
use crate::session::live::{LifecycleEvent, TurnProgress};
use crate::session::manager::SessionManager;

use serde_json::{Value, json};

use super::protocol::{
    BufferState, ErrorCode, ModelInfo, OperationKind, SessionBindingProjection, SessionSnapshot,
    StructuredError, TurnSnapshot, TurnTokenStats, WireEvent, WireTurnPhase,
};

/// Project sticky session binding + catalog effective fields for wire.
pub fn binding_projection(
    sessions: &SessionManager,
    session_id: &str,
    resolved: &ResolvedConfig,
    default_primary: &str,
) -> SessionBindingProjection {
    let agent_id = sessions
        .agent_id(session_id)
        .unwrap_or_else(|| default_primary.to_string());
    let model_id = sessions
        .session_model_id(session_id)
        .filter(|s| !s.is_empty());
    let thinking_tier = sessions
        .thinking_tier(session_id)
        .unwrap_or_default()
        .as_str()
        .to_string();
    let context_mode = sessions
        .context_mode(session_id)
        .unwrap_or_default()
        .as_str()
        .to_string();
    let context_mode_enum = ContextMode::parse(&context_mode).unwrap_or_default();
    let (api_model_id, label, context_window) = match model_id.as_deref() {
        Some(id) => match resolved.models().get(id) {
            Some(m) => (
                effective_api_model_id(m),
                m.label.clone(),
                effective_context_window(m, context_mode_enum),
            ),
            None => (String::new(), String::new(), 0),
        },
        None => (String::new(), String::new(), 0),
    };
    SessionBindingProjection {
        agent_id,
        model_id,
        api_model_id,
        label,
        context_window,
        thinking_tier,
        context_mode,
    }
}

pub fn fail_reason_to_error_code(reason: FailReason) -> ErrorCode {
    match reason {
        FailReason::LlmHttp => ErrorCode::LlmHttp,
        FailReason::LlmParse => ErrorCode::LlmParse,
        FailReason::Internal => ErrorCode::Internal,
    }
}

fn compact_fail_code(kind: Option<CompactionFailKind>) -> ErrorCode {
    match kind {
        Some(CompactionFailKind::NothingToCompact) => ErrorCode::NothingToCompact,
        Some(CompactionFailKind::Canceled) => ErrorCode::Cancelled,
        Some(CompactionFailKind::Failed) | None => ErrorCode::CompactionFailed,
    }
}

pub fn wire_phase(phase: &TurnPhase) -> WireTurnPhase {
    match phase {
        TurnPhase::Idle => WireTurnPhase::Idle,
        TurnPhase::Starting => WireTurnPhase::Starting,
        TurnPhase::Compacting => WireTurnPhase::Compacting,
        TurnPhase::CallingLlm => WireTurnPhase::CallingLlm,
        TurnPhase::Streaming => WireTurnPhase::Streaming,
        TurnPhase::ExecutingTools => WireTurnPhase::ExecutingTools,
        TurnPhase::AwaitingPermission {
            tool,
            rule_id,
            summary,
        } => WireTurnPhase::AwaitingPermission {
            tool: tool.clone(),
            rule_id: rule_id.clone(),
            summary: summary.clone(),
        },
        TurnPhase::Cancelling => WireTurnPhase::Cancelling,
        TurnPhase::Finalizing => WireTurnPhase::Finalizing,
        TurnPhase::Failed { reason } => WireTurnPhase::Failed {
            code: fail_reason_to_error_code(*reason),
        },
    }
}

/// Reverse of [`wire_phase`]: map a protocol [`WireTurnPhase`] back to the
/// runtime [`TurnPhase`] used for a reconnecting viewer's local view (R7).
///
/// `Failed` uses [`FailReason::Internal`] since the wire shape only carries an
/// `ErrorCode`; `AwaitingPermission` is reconstructed from the relayed tool/risk.
pub fn wire_phase_to_internal(phase: &WireTurnPhase) -> TurnPhase {
    match phase {
        WireTurnPhase::Idle => TurnPhase::Idle,
        WireTurnPhase::Starting => TurnPhase::Starting,
        WireTurnPhase::Compacting => TurnPhase::Compacting,
        WireTurnPhase::CallingLlm => TurnPhase::CallingLlm,
        WireTurnPhase::Streaming => TurnPhase::Streaming,
        WireTurnPhase::ExecutingTools => TurnPhase::ExecutingTools,
        WireTurnPhase::AwaitingPermission {
            tool,
            rule_id,
            summary,
        } => TurnPhase::AwaitingPermission {
            tool: tool.clone(),
            rule_id: rule_id.clone(),
            summary: summary.clone(),
        },
        WireTurnPhase::Cancelling => TurnPhase::Cancelling,
        WireTurnPhase::Finalizing => TurnPhase::Finalizing,
        // FIX-4: preserve ErrorCode granularity where FailReason can carry it
        // (LlmHttp/LlmParse); codes without a dedicated FailReason collapse to
        // Internal — the wire shape keeps the original `code` as the authority.
        WireTurnPhase::Failed { code } => {
            let reason = match code {
                ErrorCode::LlmHttp => FailReason::LlmHttp,
                ErrorCode::LlmParse => FailReason::LlmParse,
                _ => FailReason::Internal,
            };
            TurnPhase::Failed { reason }
        }
    }
}

pub fn notification(method: &str, params: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    })
}

pub fn project(ev: &InternalEvent, snapshot: &SessionSnapshot) -> Option<serde_json::Value> {
    match ev {
        InternalEvent::TurnStarted {
            turn_id,
            input,
            step_max,
        } => Some(notification(
            "agent/turn_started",
            serde_json::json!({
                "session_id": snapshot.session_id,
                "turn_id": turn_id,
                "input": input,
                "step_max": step_max,
            }),
        )),
        InternalEvent::PhaseChanged { phase, step } => turn_event(
            snapshot,
            WireEvent::PhaseChanged {
                phase: wire_phase(phase),
                step: *step,
            },
        ),
        InternalEvent::StepStarted { step, step_max } => turn_event(
            snapshot,
            WireEvent::StepStarted {
                step: *step,
                step_max: *step_max,
            },
        ),
        InternalEvent::StreamEvent(ev) => {
            turn_event(snapshot, WireEvent::StreamEvent { event: ev.clone() })
        }
        InternalEvent::TodoProgress {
            pending,
            in_progress,
            completed,
            items,
        } => turn_event(
            snapshot,
            WireEvent::TodoProgress {
                pending: *pending,
                in_progress: *in_progress,
                completed: *completed,
                items: items.clone(),
            },
        ),
        InternalEvent::LlmRequestBuilt {
            model,
            endpoint,
            token_estimate,
            tools_count,
            context_window,
        } => turn_event(
            snapshot,
            WireEvent::LlmRequestBuilt {
                model: model.clone(),
                endpoint: endpoint.clone(),
                token_estimate: *token_estimate,
                tools_count: *tools_count,
                context_window: *context_window,
            },
        ),
        InternalEvent::LlmCompleted {
            prompt_tokens,
            completion_tokens,
            cache_hit_tokens,
            cache_miss_tokens,
            stop_reason,
        } => turn_event(
            snapshot,
            WireEvent::LlmCompleted {
                prompt_tokens: *prompt_tokens,
                completion_tokens: *completion_tokens,
                cache_hit_tokens: *cache_hit_tokens,
                cache_miss_tokens: *cache_miss_tokens,
                stop_reason: stop_reason.clone(),
            },
        ),
        InternalEvent::Compaction { kind, detail } => turn_event(
            snapshot,
            WireEvent::Compaction {
                kind: *kind,
                detail: detail.clone(),
            },
        ),
        InternalEvent::CompactionLifecycle {
            trigger,
            stage,
            operation_id,
            fail_kind,
            error,
        } => {
            let mut started_snapshot = snapshot.clone();
            started_snapshot.compacting =
                *trigger == CompactionTrigger::Manual && *stage == CompactionStage::Started;
            let error = if *stage == CompactionStage::Failed {
                Some(StructuredError {
                    code: compact_fail_code(*fail_kind),
                    message: error.clone().unwrap_or_else(|| "compaction failed".into()),
                })
            } else {
                None
            };
            Some(notification(
                super::protocol::methods::SESSION_COMPACT_LIFECYCLE,
                serde_json::json!({
                    "session_id": snapshot.session_id,
                    "trigger": trigger,
                    "stage": stage,
                    "operation_id": operation_id,
                    "error": error,
                    "snapshot": started_snapshot,
                }),
            ))
        }
        InternalEvent::HookFired { phase, action } => turn_event(
            snapshot,
            WireEvent::HookFired {
                phase: phase.clone(),
                action: action.clone(),
            },
        ),
        InternalEvent::PermissionResolved {
            tool,
            approved,
            always,
        } => turn_event(
            snapshot,
            WireEvent::PermissionResolved {
                tool: tool.clone(),
                approved: *approved,
                always: *always,
            },
        ),
        InternalEvent::PermissionAwaiting { .. } => None,
        InternalEvent::SnapshotNotice { level, message } => turn_event(
            snapshot,
            WireEvent::SnapshotNotice {
                level: level.clone(),
                message: message.clone(),
            },
        ),
        InternalEvent::FileRevertUpdated { .. } => Some(session_snapshot(snapshot.clone())),
        InternalEvent::Error(error) => turn_event(
            snapshot,
            WireEvent::Error {
                code: fail_reason_to_error_code(error.reason),
                message: error.message.clone(),
            },
        ),
        InternalEvent::PermissionAsk {
            session_id,
            turn_id,
            request_id,
            tool,
            rule_id,
            summary,
        } => Some(notification(
            "agent/permission_request",
            serde_json::json!({
                "session_id": session_id,
                "turn_id": turn_id,
                "request_id": request_id,
                "tool": tool,
                "rule_id": rule_id,
                "summary": summary,
            }),
        )),
        InternalEvent::StepCommitted => None,
        InternalEvent::ProjectionLagged { .. } => None,
        InternalEvent::SessionPreviewUpdated { .. } => None,
        InternalEvent::BufferItem {
            buffer_index,
            item,
            kind,
            child_session_id,
        } => {
            let mut params = serde_json::json!({
                "session_id": snapshot.session_id,
                "buffer_index": buffer_index,
                "item": item,
            });
            if let Some(kind) = kind {
                params["kind"] = serde_json::Value::String(kind.clone());
            }
            if let Some(child_id) = child_session_id {
                params["child_session_id"] = serde_json::Value::String(child_id.clone());
            }
            Some(notification(super::protocol::methods::BUFFER_ITEM, params))
        }
        InternalEvent::SubagentBound {
            call_id,
            child_session_id,
        } => Some(notification(
            super::protocol::methods::AGENT_SUBAGENT_BOUND,
            serde_json::json!({
                "session_id": snapshot.session_id,
                "call_id": call_id,
                "child_session_id": child_session_id,
            }),
        )),
        InternalEvent::TurnCompleted {
            turn_id,
            final_text,
            reason,
            turn_token_stats,
            committed_start: _,
        } => {
            let error = turn_error(reason, final_text);
            let mut params = serde_json::json!({
                "session_id": snapshot.session_id,
                "turn_id": turn_id,
                "reason": reason,
                "snapshot": snapshot,
            });
            if let Some(ft) = final_text {
                params["final_text"] = serde_json::Value::String(ft.clone());
            }
            if let Some(err) = error {
                params["error"] = serde_json::to_value(err).unwrap_or_default();
            }
            params["turn_token_stats"] = serde_json::to_value(turn_token_stats).unwrap_or_default();
            Some(notification("agent/turn_finished", params))
        }
        InternalEvent::BufferChanged { .. } => {
            // Degraded: no longer auto-emit session/snapshot
            None
        }
        InternalEvent::WorkspaceChanged { paths, kind } => Some(notification(
            "workspace/changed",
            serde_json::json!({
                "paths": paths,
                "kind": kind,
            }),
        )),
        InternalEvent::BashJobs { snapshot: bash } => Some(notification(
            super::protocol::methods::BASH_JOBS,
            serde_json::json!({
                "session_id": snapshot.session_id,
                "jobs": bash.jobs,
                "waits": bash.waits,
            }),
        )),
    }
}

fn turn_error(reason: &TurnEndReason, final_text: &Option<String>) -> Option<StructuredError> {
    match reason {
        TurnEndReason::Error => Some(StructuredError {
            code: ErrorCode::Internal,
            message: final_text.clone().unwrap_or_else(|| "turn failed".into()),
        }),
        TurnEndReason::Cancelled => Some(StructuredError {
            code: ErrorCode::Cancelled,
            message: "cancelled".into(),
        }),
        TurnEndReason::MaxSteps => Some(StructuredError {
            code: ErrorCode::MaxSteps,
            message: "max steps reached".into(),
        }),
        TurnEndReason::HookBlocked => Some(StructuredError {
            code: ErrorCode::HookBlocked,
            message: "blocked by hook".into(),
        }),
        TurnEndReason::Completed => None,
    }
}

pub fn session_list(sessions: Vec<super::protocol::SessionInfo>) -> serde_json::Value {
    notification("session/list", serde_json::json!({ "sessions": sessions }))
}

fn turn_event(snapshot: &SessionSnapshot, event: WireEvent) -> Option<serde_json::Value> {
    let turn_id = snapshot.turn.as_ref()?.turn_id.clone();
    Some(notification(
        "agent/turn_event",
        serde_json::json!({
            "session_id": snapshot.session_id,
            "turn_id": turn_id,
            "event": event,
        }),
    ))
}

pub fn operation_result(
    op: OperationKind,
    ok: bool,
    error: Option<StructuredError>,
    snapshot: SessionSnapshot,
) -> serde_json::Value {
    let mut params = serde_json::json!({
        "op": op,
        "ok": ok,
        "snapshot": snapshot,
    });
    if let Some(ref err) = error {
        params["error"] = serde_json::to_value(err).unwrap_or_default();
    }
    notification("agent/operation_result", params)
}

pub fn server_hello(
    version: String,
    version_channel: String,
    session_id: String,
    project: String,
    workspace_id: String,
    settings_revision: u64,
    active_primary: String,
    primary_agents: Vec<super::protocol::PrimaryAgentInfo>,
    llm_ecosystem: String,
    models: Vec<ModelInfo>,
) -> serde_json::Value {
    notification(
        "server/hello",
        serde_json::json!({
            "version": version,
            "version_channel": version_channel,
            "session_id": session_id,
            "project": project,
            "workspace_id": workspace_id,
            "settings_revision": settings_revision,
            "active_primary": active_primary,
            "primary_agents": primary_agents,
            "llm_ecosystem": llm_ecosystem,
            "models": models,
        }),
    )
}

pub fn settings_changed(
    revision: u64,
    summary: crate::config::SettingsSummary,
) -> serde_json::Value {
    notification(
        "settings/changed",
        serde_json::json!({
            "revision": revision,
            "summary": summary,
        }),
    )
}

pub fn server_stats(sample: crate::telemetry::MemorySample) -> serde_json::Value {
    notification(
        "server/stats",
        serde_json::json!({
            "rss_kb": sample.total_kb(),
            "core_rss_kb": sample.core_kb,
            "embed_rss_kb": sample.embed_kb,
            "lsp_rss_kb": sample.lsp_kb,
            "ts_ms": chrono::Utc::now().timestamp_millis(),
        }),
    )
}

pub fn log_line(line: super::protocol::LogLine) -> serde_json::Value {
    notification("log/line", serde_json::to_value(line).unwrap_or_default())
}

pub fn terminal_data(id: &str, data: &str) -> serde_json::Value {
    notification(
        "terminal/data",
        serde_json::json!({
            "id": id,
            "data": data,
        }),
    )
}

pub fn terminal_exit(id: &str, code: Option<u32>) -> serde_json::Value {
    notification(
        "terminal/exit",
        serde_json::json!({
            "id": id,
            "code": code,
        }),
    )
}

pub fn session_snapshot(snapshot: SessionSnapshot) -> serde_json::Value {
    notification(
        "session/snapshot",
        serde_json::to_value(snapshot).unwrap_or_default(),
    )
}

/// G0: lifecycle event — session deleted (sent before the entry is removed).
pub fn session_lifecycle_deleted(session_id: &str) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "deleted",
            "turn": null,
        }),
    )
}

/// G0: lifecycle event — a turn started.
pub fn session_lifecycle_turn_started(session_id: &str, turn: &TurnSnapshot) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "turn_started",
            "turn": turn,
        }),
    )
}

/// G0: lifecycle event — a turn finished.
pub fn session_lifecycle_turn_finished(session_id: &str, turn: &TurnSnapshot) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "turn_finished",
            "turn": turn,
        }),
    )
}

pub fn session_lifecycle_preview_updated(
    session_id: &str,
    preview: &str,
    updated_at: i64,
) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "preview_updated",
            "preview": preview,
            "updated_at": updated_at,
            "turn": null,
        }),
    )
}

pub fn session_lifecycle_turn_step(
    session_id: &str,
    kind: &crate::session::live::TurnStepKind,
    turn: &TurnSnapshot,
) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "turn_step",
            "step_kind": kind,
            "turn": turn,
        }),
    )
}

/// G0: lifecycle event — a running turn's coarse snapshot changed
/// (phase / step transition). Carries the same [`TurnSnapshot`] as
/// `turn_started` / `turn_finished` so the session list can reflect live
/// step progress without subscribing to the per-session token stream.
pub fn session_lifecycle_turn_updated(session_id: &str, turn: &TurnSnapshot) -> Value {
    notification(
        "session/lifecycle",
        json!({
            "session_id": session_id,
            "event": "turn_updated",
            "turn": turn,
        }),
    )
}

/// Notify a freshly-(re)subscribed connection of a running turn's cached state.
///
/// Sent immediately after `attach` when the `SessionManager` reports a live
/// [`TurnSnapshot`] (R7 reconnect): lets the late-joining viewer render the
/// in-progress turn instead of appearing idle.
pub fn turn_progress_to_snapshot(p: &TurnProgress) -> TurnSnapshot {
    TurnSnapshot {
        turn_id: p.turn_id.clone(),
        phase: wire_phase(&p.phase),
        step: p.step,
        step_max: p.step_max,
        started_at_ms: p.started_at_ms,
        awaiting_permission: p.awaiting_permission,
    }
}

pub fn lifecycle_event_to_wire(ev: &LifecycleEvent) -> Value {
    match ev {
        LifecycleEvent::SessionRemoved { session_id } => session_lifecycle_deleted(session_id),
        LifecycleEvent::TurnStarted {
            session_id,
            progress,
        } => session_lifecycle_turn_started(session_id, &turn_progress_to_snapshot(progress)),
        LifecycleEvent::TurnProgress {
            session_id,
            progress,
        } => session_lifecycle_turn_updated(session_id, &turn_progress_to_snapshot(progress)),
        LifecycleEvent::TurnFinished {
            session_id,
            progress,
        } => session_lifecycle_turn_finished(session_id, &turn_progress_to_snapshot(progress)),
        LifecycleEvent::SessionPreviewUpdated {
            session_id,
            preview,
            updated_at,
        } => session_lifecycle_preview_updated(session_id, preview, *updated_at),
        LifecycleEvent::TurnStep {
            session_id,
            kind,
            progress,
        } => session_lifecycle_turn_step(session_id, kind, &turn_progress_to_snapshot(progress)),
    }
}

pub fn session_attached(session_id: &str, turn: &TurnSnapshot) -> serde_json::Value {
    notification(
        "session/attached",
        serde_json::json!({
            "session_id": session_id,
            "turn": turn,
        }),
    )
}

/// CLI-facing view of an incoming notification.
#[derive(Debug)]
pub enum IncomingWire {
    TurnEvent(WireEvent),
    PermissionRequest {
        session_id: String,
        turn_id: String,
        request_id: String,
        tool: String,
        rule_id: String,
        summary: String,
    },
    TurnFinished {
        session_id: String,
        turn_id: String,
        reason: TurnEndReason,
        final_text: Option<String>,
        error: Option<StructuredError>,
    },
    OperationFailed(String),
    Ignored,
}

pub fn classify_incoming(msg: &serde_json::Value) -> IncomingWire {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params");

    match method {
        "agent/turn_event" => {
            if let Some(params) = params
                && let Some(event) = params.get("event")
                && let Ok(ev) = serde_json::from_value::<WireEvent>(event.clone())
            {
                return IncomingWire::TurnEvent(ev);
            }
            IncomingWire::Ignored
        }
        "agent/permission_request" => {
            if let Some(params) = params {
                let session_id = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let turn_id = params
                    .get("turn_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let request_id = params
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let rule_id = params
                    .get("rule_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let summary = params
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return IncomingWire::PermissionRequest {
                    session_id,
                    turn_id,
                    request_id,
                    tool,
                    rule_id,
                    summary,
                };
            }
            IncomingWire::Ignored
        }
        "agent/turn_finished" => {
            if let Some(params) = params {
                let session_id = params
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let turn_id = params
                    .get("turn_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let reason: TurnEndReason = params
                    .get("reason")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(TurnEndReason::Completed);
                let final_text = params
                    .get("final_text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let error = params
                    .get("error")
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                return IncomingWire::TurnFinished {
                    session_id,
                    turn_id,
                    reason,
                    final_text,
                    error,
                };
            }
            IncomingWire::Ignored
        }
        "agent/operation_result" => {
            if let Some(params) = params {
                let ok = params.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
                if !ok {
                    let msg = params
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("operation failed")
                        .to_string();
                    return IncomingWire::OperationFailed(msg);
                }
            }
            IncomingWire::Ignored
        }
        _ => IncomingWire::Ignored,
    }
}

pub fn as_turn_finished(msg: &serde_json::Value) -> Option<serde_json::Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    if method == "agent/turn_finished" {
        Some(msg.clone())
    } else {
        None
    }
}

pub fn buffer_snapshot(
    session_id: &str,
    project: &str,
    binding: &SessionBindingProjection,
    len: usize,
    revision: u64,
    committed_end: usize,
    turn: Option<super::protocol::TurnSnapshot>,
    last_turn_token_stats: Option<TurnTokenStats>,
    cumulative_token_stats: Option<TurnTokenStats>,
    context_tokens_estimate: usize,
    compacting: bool,
) -> SessionSnapshot {
    SessionSnapshot {
        session_id: session_id.to_string(),
        project: project.to_string(),
        agent_id: binding.agent_id.clone(),
        model_id: binding.model_id.clone(),
        api_model_id: binding.api_model_id.clone(),
        label: binding.label.clone(),
        buffer: BufferState {
            len,
            revision,
            committed_end,
        },
        turn,
        context_window: binding.context_window,
        context_tokens_estimate,
        compact_eligible: crate::context_pipeline::manual_compact_eligible(
            binding.context_window,
            context_tokens_estimate,
        ),
        compacting,
        last_turn_token_stats,
        cumulative_token_stats,
        thinking_tier: binding.thinking_tier.clone(),
        context_mode: binding.context_mode.clone(),
        max_file_revert_k: None,
        bash: None,
        todos: Vec::new(),
    }
}
