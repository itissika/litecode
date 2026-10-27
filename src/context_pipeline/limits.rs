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

/// Headroom kept below the hard prompt ceiling before a request must stop.
/// The background summary may start at the 0.8 prepare line; this is the
/// later line where continuing without a committed compaction is unsafe.
pub const COMPACT_PREFETCH_MARGIN_PERCENT: usize = 2;
/// A 2% margin is too small on short windows; keep a fixed floor.
pub const COMPACT_PREFETCH_MARGIN_FLOOR_TOKENS: usize = 4_096;

/// Margin below `prompt_limit` used by the send gate. The hard stop is
/// `prompt_limit - margin`, never `compact_at`.
pub fn compact_prefetch_margin(context_window: usize) -> usize {
    (context_window.saturating_mul(COMPACT_PREFETCH_MARGIN_PERCENT) / 100)
        .max(COMPACT_PREFETCH_MARGIN_FLOOR_TOKENS)
}
