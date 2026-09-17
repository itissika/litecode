//! Subagent tool series: launch / send / wait / stop / list.
//!
//! Tools call the same session primitives humans use (`reserve_turn` →
//! `spawn_turn` → `start_turn` / `cancel_turn_sync`). The hub only routes
//! completion references to a safe parent injection point.

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
pub use jobs::{CompletionInbox, CompletionRef};
pub use launch::{LaunchSpec, SpawnDeps, SubagentLaunchTool, spawn_child_job};
pub use list::SubagentListTool;
pub use send::SubagentSendTool;
pub use stop::SubagentStopTool;
pub use wait::SubagentWaitTool;

#[cfg(test)]
mod spawn_contract;
