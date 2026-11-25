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
        "id": "s_child",
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
                "settled": [{
                    "id": "s_child",
                    "reason": "cancelled_by_parent",
                    "responsibility": "check",
                    "output": "done",
                }],
                "skipped": [{ "id": "s_other", "reason": "no completed turn" }],
            })),
            all_optional_absent: ToolCallResult::ok("settled").with_data(json!({
                "settled": [{
                    "id": "s_child",
                    "reason": "completed",
                    "output": "done",
                }],
            })),
        },
        DriftCase {
            tool: "subagent.send",
            all_optional_present: ToolCallResult::ok("queued").with_data(json!({
                "status": "queued",
                "position": 1,
            })),
            all_optional_absent: ToolCallResult::ok("started").with_data(json!({
                "status": "started",
            })),
        },
        DriftCase {
            tool: "subagent.stop",
            all_optional_present: ToolCallResult::ok("nothing running").with_data(json!({
                "status": "nothing_running",
                "reason": "completed",
                "output": "ok",
            })),
            all_optional_absent: ToolCallResult::ok("stopped").with_data(json!({
                "status": "stopped",
            })),
        },
        DriftCase {
            tool: "subagent.list",
            all_optional_present: ToolCallResult::ok("sessions: 1").with_data(json!({
                "sessions": [{
                    "id": "s_child",
                    "agent": "research",
                    "responsibility": "check",
                    "state": "idle",
                    "reason": "completed",
                }],
            })),
            all_optional_absent: ToolCallResult::ok("sessions: 1").with_data(json!({
                "sessions": [{
                    "id": "s_child",
                    "agent": "research",
                    "responsibility": "check",
                    "state": "idle",
                }],
            })),
        },
    ]
}

#[cfg(test)]
mod spawn_contract;
