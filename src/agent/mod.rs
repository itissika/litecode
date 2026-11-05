pub mod core;
pub mod deps;
pub mod outcome;

pub use core::run;
pub use deps::AgentDeps;
pub use outcome::TurnOutcome;

/// Engine-level step budget for a non-bot session. The old per-agent
/// `max_steps` field is gone; long tasks keep this headroom.
pub const AGENT_MAX_STEPS: u32 = 200_000_000;
