//! Shared compaction/context limits.
//!
//! These values are policy inputs, not UI copies. Keep one definition here so
//! keep-recent retention, summary output caps, and the ContextMeter cannot
//! drift independently.

/// Context-window fallback when a session/model has no catalog window.
pub const FALLBACK_CONTEXT_WINDOW: usize = 128_000;

/// Largest prompt-side reserve used by the keep-recent split.
pub const MAX_PROMPT_RESERVE_TOKENS: usize = 16_384;

/// Verbatim recent-window cap retained across compaction.
pub const KEEP_RECENT_CAP: usize = 20_000;

/// Hard ceiling for the main-model compaction summary call.
pub const COMPACT_MAX_OUTPUT_TOKENS: u32 = 20_480;

/// Inclusive product floor for user-triggered compaction.
pub const MANUAL_COMPACT_RATIO_PERCENT: usize = 30;
