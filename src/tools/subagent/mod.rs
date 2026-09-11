//! Subagent tool series: launch / wait / stop / list, consuming a process-scoped hub.

mod hub;
mod launch;
mod list;
pub mod status;
mod stop;
mod wait;

pub use hub::{
    ExitNotice, MAX_SUBAGENTS_PER_PARENT, RunningJob, SpawnDeps, SubagentHub, SubagentJobWire,
    SubagentJobsSnapshot, SubagentWaitWire, WaitOutcome,
};
pub use launch::SubagentLaunchTool;
pub use list::SubagentListTool;
pub use stop::SubagentStopTool;
pub use wait::SubagentWaitTool;

#[cfg(test)]
mod jobs_contract;
