use std::sync::atomic::{AtomicU64, Ordering};

use crate::session::estimate::autocompact_threshold;
use crate::session::store::{ProviderUsageRollup, SessionContextMeter, occupancy_usage};

/// Provider-reported prompt usage tied to the request header seq that opened it.
///
/// Rows after that seq, and rows that have not been persisted yet, have not
/// reached the provider. A [`ContextMeter`](super::ContextMeter) adds a local
/// estimate for only that suffix.
#[derive(Debug, Default)]
pub struct ProviderPromptBaseline {
    prompt_tokens: AtomicU64,
    covered_seq: AtomicU64,
}

impl ProviderPromptBaseline {
    /// `covered_seq` is the request-header seq. `u64::MAX` covers every persisted row.
    pub fn record(&self, prompt_tokens: u64, covered_seq: u64) {
        self.covered_seq.store(covered_seq, Ordering::Relaxed);
        self.prompt_tokens.store(prompt_tokens, Ordering::Release);
    }

    pub fn clear(&self) {
        self.prompt_tokens.store(0, Ordering::Release);
        self.covered_seq.store(0, Ordering::Relaxed);
    }

    pub fn prompt_tokens(&self) -> u64 {
        self.prompt_tokens.load(Ordering::Acquire)
    }

    /// `(prompt_tokens, covered_seq)` when this process has a live bill.
    pub fn covered(&self) -> Option<(u64, u64)> {
        let prompt_tokens = self.prompt_tokens.load(Ordering::Acquire);
        if prompt_tokens == 0 {
            return None;
        }
        Some((prompt_tokens, self.covered_seq.load(Ordering::Relaxed)))
    }
}

/// One provider prompt that still describes the current transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccupancyBill {
    pub prompt_tokens: u64,
    /// Request-header seq this prompt covers. `None` covers every persisted row
    /// (meter fallback, which has no header seq).
    pub covered_seq: Option<u64>,
}

/// In-memory bill when it is still newer than the last compact; otherwise the
/// same persisted occupancy the ring shows.
pub fn occupancy_bill_from(
    baseline: &ProviderPromptBaseline,
    compacted_seq: Option<u64>,
    rollup: &ProviderUsageRollup,
    meter: &SessionContextMeter,
) -> Option<OccupancyBill> {
    let retired = |seq: u64| compacted_seq.is_some_and(|compacted| compacted > seq);
    if let Some((prompt, seq)) = baseline.covered()
        && !retired(seq)
    {
        return Some(OccupancyBill {
            prompt_tokens: prompt,
            covered_seq: Some(seq),
        });
    }
    let usage = occupancy_usage(rollup, compacted_seq, meter)?;
    if usage.prompt_tokens == 0 {
        return None;
    }
    Some(OccupancyBill {
        prompt_tokens: usage.prompt_tokens,
        covered_seq: rollup.latest_seq.filter(|seq| !retired(*seq)),
    })
}

/// Fallback when context_window is unset (rough tokens for keep-recent only).
pub const FALLBACK_BUDGET_LIMIT: usize = 128_000;

/// Retention policy for context preparation.
///
/// Pressure/eligibility lives exclusively in
/// [`ContextMeter`](super::ContextMeter); this type retains the existing
/// LiteCode keep-recent and summary-cap policy.
#[derive(Debug, Clone)]
pub struct BudgetPolicy {
    pub context_window: usize,
    /// Verbatim recent window retained across compaction (`min(20k, window/4)` by default).
    pub keep_recent_tokens: usize,
}

impl BudgetPolicy {
    pub fn new(context_window: usize) -> Self {
        let limit = if context_window > 0 {
            context_window
        } else {
            FALLBACK_BUDGET_LIMIT
        };
        Self {
            context_window,
            keep_recent_tokens: default_keep_recent_tokens(limit),
        }
    }

    /// Override keep-recent window (tests / tuning).
    pub fn with_keep_recent_tokens(mut self, tokens: usize) -> Self {
        self.keep_recent_tokens = tokens.max(1);
        self
    }

    /// Compact LLM `max_output_tokens` for this window and keep-recent split.
    pub fn compact_output_tokens(&self, configured: u32) -> u32 {
        crate::context_pipeline::keep_recent::compact_output_tokens(
            self.budget_limit(),
            self.keep_recent_tokens,
            configured,
        )
    }

    pub fn budget_limit(&self) -> usize {
        if self.context_window > 0 {
            self.context_window
        } else {
            FALLBACK_BUDGET_LIMIT
        }
    }

    /// The 0.8 soft-window component retained for keep-recent sizing and fixture
    /// calibration. Pressure decisions must use [`ContextMeter::compact_at`].
    pub fn autocompact_threshold(&self) -> usize {
        autocompact_threshold(self.budget_limit())
    }
}

use crate::context_pipeline::keep_recent::default_keep_recent_tokens;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_pipeline::context_meter::ContextMeter;
    use crate::context_pipeline::context_meter::ContextMeterSource;
    use crate::types::user_text;

    #[test]
    fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix() {
        let env = crate::context_pipeline::ContextEnvelope::default();
        let items = vec![user_text("sent"), user_text("x".repeat(4_000))];
        let seqs = vec![Some(4), Some(9)];
        let bill = OccupancyBill {
            prompt_tokens: 7_000,
            covered_seq: Some(4),
        };
        let suffix = crate::context_pipeline::estimate::compute_token_estimate(&items[1..]);
        let meter = ContextMeter::measure(
            10_000,
            env.output_reserve,
            ContextMeter::safety_headroom(10_000),
            &env,
            &items,
            &seqs,
            Some(bill),
        );
        assert_eq!(meter.used_tokens, 7_000 + suffix);
        assert_eq!(meter.source, ContextMeterSource::ProviderPlusSuffix);
    }

    #[test]
    fn compact_retires_the_in_memory_bill_and_the_stored_one() {
        use crate::session::store::ProviderUsage;

        let baseline = ProviderPromptBaseline::default();
        baseline.record(9_000, 4);
        let rollup = ProviderUsageRollup::from_rows([(
            4,
            ProviderUsage {
                prompt_tokens: 9_000,
                completion_tokens: 1,
                cache_hit_tokens: 0,
                cache_miss_tokens: 9_000,
            },
        )]);
        let meter = SessionContextMeter {
            prompt_tokens: 9_000,
            ..SessionContextMeter::default()
        };
        assert_eq!(
            occupancy_bill_from(&baseline, None, &rollup, &meter)
                .unwrap()
                .prompt_tokens,
            9_000
        );
        assert!(occupancy_bill_from(&baseline, Some(8), &rollup, &meter).is_none());
        let empty = ProviderPromptBaseline::default();
        assert_eq!(
            occupancy_bill_from(&empty, None, &rollup, &meter)
                .unwrap()
                .covered_seq,
            Some(4)
        );
        assert!(occupancy_bill_from(&empty, Some(8), &rollup, &meter).is_none());
    }
}
