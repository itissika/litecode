//! Subagent tool series: launch / send / wait / stop / list.
//!
//! Tools call the same `SessionService` facade humans use (`start_turn` /
//! `cancel_turn`). The hub only routes completion references to a safe parent
//! injection point and consumes the same service.

/// Product lock: subagent tools bind only on primary turns (`depth == 0`).
/// Children cannot nest another subagent series.
pub const SUBAGENT_MAX_DEPTH: u32 = 1;

mod hub;
mod jobs;
mod launch;
mod list;
mod send;
pub mod status;
mod stop;
mod turn;
mod wait;

pub use hub::SubagentHub;
pub use jobs::{CompletionInbox, CompletionRef, NoticeInbox, ParentNotice};
pub use launch::{LaunchSpec, SpawnDeps, SubagentLaunchTool, spawn_child_job};
pub use list::SubagentListTool;
pub use send::SubagentSendTool;
pub use stop::SubagentStopTool;
pub use wait::SubagentWaitTool;

#[cfg(test)]
pub(crate) fn drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
    use crate::tool::code_contract::DriftCase;
    use crate::types::ToolCallResult;
    use serde_json::json;

    let launch_sample = ToolCallResult::ok("started").with_data(json!({
        "child_session_id": "s_child",
        "turn_id": "t_1",
        "status": "running",
        "responsibility": "check tests",
    }));
    vec![
        DriftCase {
            tool: "subagent.launch",
            all_optional_present: launch_sample.clone(),
            all_optional_absent: launch_sample,
        },
        DriftCase {
            tool: "subagent.wait",
            all_optional_present: ToolCallResult::ok("settled").with_data(json!({
                "status": "settled",
                "settled": [{
                    "child_session_id": "s_child",
                    "turn_id": "t_1",
                    "reason": "completed",
                    "agent": "research",
                    "responsibility": "check",
                    "output": "done",
                    "output_truncated": true,
                    "cancelled_by": "parent",
                    "transcript": ".litecode/sessions/s_child.md",
                    "start_line": 1,
                    "end_line": 10,
                }],
                "skipped": [{ "id": "s_other", "reason": "no completed turn" }],
                "summary": "settled",
            })),
            all_optional_absent: ToolCallResult::ok("settled").with_data(json!({
                "status": "settled",
                "settled": [{
                    "child_session_id": "s_child",
                    "turn_id": "t_1",
                    "reason": "completed",
                    "output": "done",
                    "transcript": ".litecode/sessions/s_child.md",
                }],
                "skipped": [{ "id": "s_other", "reason": "no completed turn" }],
                "summary": "settled",
            })),
        },
        DriftCase {
            tool: "subagent.send",
            all_optional_present: ToolCallResult::ok("started").with_data(json!({
                "child_session_id": "s_child",
                "mode": "queue",
                "status": "started",
                "turn_id": "t_2",
                "position": 1,
                "pending_id": "p_1",
                "message": "started",
            })),
            all_optional_absent: ToolCallResult::ok("stopped").with_data(json!({
                "child_session_id": "s_child",
                "mode": "stop_and_send",
                "status": "stopped_and_queued",
                "message": "stopped",
            })),
        },
        DriftCase {
            tool: "subagent.stop",
            all_optional_present: ToolCallResult::ok("nothing running").with_data(json!({
                "child_session_id": "s_child",
                "status": "nothing_running",
                "turn_id": "t_1",
                "reason": "completed",
                "output": "ok",
                "transcript": ".litecode/sessions/s_child.md",
                "message": "nothing running",
            })),
            all_optional_absent: ToolCallResult::ok("stopped").with_data(json!({
                "child_session_id": "s_child",
                "status": "stopped",
                "message": "stopped",
            })),
        },
        DriftCase {
            tool: "subagent.list",
            all_optional_present: ToolCallResult::ok("sessions: 1").with_data(json!({
                "count": 1,
                "sessions": [{
                    "id": "s_child",
                    "agent": "research",
                    "responsibility": "check",
                    "state": "idle",
                    "transcript": ".litecode/sessions/s_child.md",
                    "transcript_lines": 12,
                    "step": 2,
                    "step_max": 20,
                    "reason": "completed",
                }],
            })),
            all_optional_absent: ToolCallResult::ok("sessions: 1").with_data(json!({
                "count": 1,
                "sessions": [{
                    "id": "s_child",
                    "agent": "research",
                    "responsibility": "check",
                    "state": "idle",
                    "transcript": ".litecode/sessions/s_child.md",
                }],
            })),
        },
    ]
}

#[cfg(test)]
mod spawn_contract;
