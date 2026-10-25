use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
    /// Set when the transcript surface was rewritten (mechanical prune) and
    /// neither the live bill nor the persisted rollup describes the new request
    /// any more. A real provider bill clears it via [`Self::record`].
    invalidated: AtomicBool,
}

impl ProviderPromptBaseline {
    /// `covered_seq` is the request-header seq. `u64::MAX` covers every persisted row.
    pub fn record(&self, prompt_tokens: u64, covered_seq: u64) {
        self.covered_seq.store(covered_seq, Ordering::Relaxed);
        self.prompt_tokens.store(prompt_tokens, Ordering::Release);
        self.invalidated.store(false, Ordering::Release);
    }

    pub fn clear(&self) {
        self.prompt_tokens.store(0, Ordering::Release);
        self.covered_seq.store(0, Ordering::Relaxed);
        self.invalidated.store(false, Ordering::Release);
    }

    /// Drop every stored bill and force the local request envelope until the
    /// next real provider response records a fresh bill.
    pub fn invalidate(&self) {
        self.invalidated.store(true, Ordering::Release);
        self.prompt_tokens.store(0, Ordering::Release);
        self.covered_seq.store(0, Ordering::Relaxed);
    }

    pub fn is_invalidated(&self) -> bool {
        self.invalidated.load(Ordering::Acquire)
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
    surface_rewrite_seq: u64,
    rollup: &ProviderUsageRollup,
    meter: &SessionContextMeter,
) -> Option<OccupancyBill> {
    // A mechanical prune changed the prefix after the last bill. The persisted
    // rollup would double-count the removed bytes, so fall back to the exact
    // local envelope until the next provider bill arrives.
    if baseline.is_invalidated() {
        return None;
    }
    let retired = |seq: u64| {
        compacted_seq.is_some_and(|compacted| compacted > seq)
            || (surface_rewrite_seq > 0 && seq <= surface_rewrite_seq)
    };
    if let Some((prompt, seq)) = baseline.covered()
        && !retired(seq)
    {
        return Some(OccupancyBill {
            prompt_tokens: prompt,
            covered_seq: Some(seq),
        });
    }
    let usage = occupancy_usage(rollup, compacted_seq, surface_rewrite_seq, meter)?;
    if usage.prompt_tokens == 0 {
        return None;
    }
    Some(OccupancyBill {
        prompt_tokens: usage.prompt_tokens,
        covered_seq: rollup.latest_seq.filter(|seq| !retired(*seq)),
    })
}

/// Fallback when context_window is unset (rough tokens for keep-recent only).
pub const FALLBACK_BUDGET_LIMIT: usize = super::limits::FALLBACK_CONTEXT_WINDOW;

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
        let meter =
            ContextMeter::measure(10_000, env.output_reserve, &env, &items, &seqs, Some(bill));
        assert_eq!(meter.used_tokens, 7_000 + suffix);
        assert_eq!(meter.source, ContextMeterSource::ProviderPlusSuffix);
    }

    #[test]
    fn surface_rewrite_invalidates_old_bill_until_the_next_provider_record() {
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
        assert!(occupancy_bill_from(&baseline, None, 0, &rollup, &meter).is_some());

        baseline.invalidate();
        assert!(baseline.is_invalidated());
        assert!(
            occupancy_bill_from(&baseline, None, 0, &rollup, &meter).is_none(),
            "a pruned prefix must not be priced with the old bill or rollup"
        );

        baseline.record(3_000, 12);
        assert!(!baseline.is_invalidated());
        let fresh = occupancy_bill_from(&baseline, None, 0, &rollup, &meter).unwrap();
        assert_eq!(fresh.prompt_tokens, 3_000);
        assert_eq!(fresh.covered_seq, Some(12));
    }

    #[test]
    fn surface_rewrite_seq_retires_live_and_persisted_bills() {
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
        // No rewrite: the bill is valid.
        assert!(occupancy_bill_from(&baseline, None, 0, &rollup, &meter).is_some());
        // Bill header seq 4 <= rewrite seq 10: both the live baseline and the
        // persisted rollup are retired, forcing a local-envelope measurement.
        assert!(
            occupancy_bill_from(&baseline, None, 10, &rollup, &meter).is_none(),
            "latest_seq <= surface_rewrite_seq must invalidate the bill"
        );
        // A bill recorded after the rewrite is valid again.
        baseline.record(3_000, 12);
        let fresh = occupancy_bill_from(&baseline, None, 10, &rollup, &meter).unwrap();
        assert_eq!(fresh.prompt_tokens, 3_000);
        assert_eq!(fresh.covered_seq, Some(12));

        // Without a seq-bearing rollup, a meter-only fallback cannot prove it
        // postdates the rewrite, so it must not resurrect old prompt counts.
        let empty_rollup = ProviderUsageRollup::default();
        assert!(
            occupancy_bill_from(
                &ProviderPromptBaseline::default(),
                None,
                10,
                &empty_rollup,
                &meter,
            )
            .is_none()
        );
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
            occupancy_bill_from(&baseline, None, 0, &rollup, &meter)
                .unwrap()
                .prompt_tokens,
            9_000
        );
        assert!(occupancy_bill_from(&baseline, Some(8), 0, &rollup, &meter).is_none());
        let empty = ProviderPromptBaseline::default();
        assert_eq!(
            occupancy_bill_from(&empty, None, 0, &rollup, &meter)
                .unwrap()
                .covered_seq,
            Some(4)
        );
        assert!(occupancy_bill_from(&empty, Some(8), 0, &rollup, &meter).is_none());
    }
}
