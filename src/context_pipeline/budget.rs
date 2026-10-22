use std::sync::atomic::{AtomicU64, Ordering};

use crate::context_pipeline::estimate::{autocompact_threshold, compute_token_estimate};
use crate::context_pipeline::keep_recent::default_keep_recent_tokens;
use crate::session::store::{ProviderUsageRollup, SessionContextMeter, occupancy_usage};
use crate::types::{Item, LitecodeError, Result};

/// Provider-reported prompt usage tied to the request header seq that opened it.
///
/// Rows after that seq, and rows that have not been persisted yet, have not
/// reached the provider. Budget checks add a local estimate for that suffix.
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

/// Occupancy the ring shows: provider prompt when present, otherwise the local estimate.
pub fn ring_occupancy(provider_prompt_tokens: Option<u64>, local_estimate: usize) -> usize {
    provider_prompt_tokens
        .filter(|tokens| *tokens > 0)
        .map(|tokens| tokens as usize)
        .unwrap_or(local_estimate)
}

/// Fallback when context_window is unset (rough tokens).
pub const FALLBACK_BUDGET_LIMIT: usize = 128_000;

/// Inclusive 30% product gate for user-triggered compaction.
pub fn manual_compact_eligible(context_window: usize, token_count: usize) -> bool {
    context_window > 0 && token_count.saturating_mul(100) >= context_window.saturating_mul(30)
}

/// Token budget policy for context preparation.
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

    pub fn autocompact_threshold(&self) -> usize {
        autocompact_threshold(self.budget_limit())
    }

    /// Provider `usage.prompt_tokens` (last request) is authoritative when present;
    /// otherwise whole-transcript local estimate — never a sum across LLM calls.
    pub fn token_count(&self, items: &[Item], provider_tokens: u64) -> usize {
        if provider_tokens > 0 {
            provider_tokens as usize
        } else {
            self.local_token_count(items)
        }
    }

    /// Provider prompt plus a local estimate of rows the bill does not cover.
    ///
    /// A row is uncovered when it has no seq, or its seq is after `covered_seq`.
    /// `None` uses the local estimate of the whole set — a rewritten transcript
    /// after compact is not a suffix of the billed request.
    pub fn occupancy_token_count(
        &self,
        items: &[Item],
        seqs: &[Option<u64>],
        bill: Option<OccupancyBill>,
    ) -> usize {
        let Some(bill) = bill.filter(|bill| bill.prompt_tokens > 0) else {
            return self.local_token_count(items);
        };
        if seqs.len() != items.len() {
            return self.local_token_count(items);
        }
        let mut suffix = Vec::new();
        for (item, seq) in items.iter().zip(seqs.iter()) {
            let after = match (bill.covered_seq, seq) {
                (Some(covered), Some(row_seq)) => *row_seq > covered,
                (Some(_), None) => true,
                (None, None) => true,
                (None, Some(_)) => false,
            };
            if after {
                suffix.push(item.clone());
            }
        }
        (bill.prompt_tokens as usize).saturating_add(compute_token_estimate(&suffix))
    }

    pub fn enforce_token_count(&self, token_count: usize) -> Result<()> {
        if token_count > self.budget_limit() {
            Err(LitecodeError::TokenBudgetExceeded)
        } else {
            Ok(())
        }
    }

    fn local_token_count(&self, items: &[Item]) -> usize {
        compute_token_estimate(items)
    }

    pub fn should_compact(&self, token_count: usize) -> bool {
        token_count > self.autocompact_threshold()
    }

    pub fn enforce_hard_limit(&self, items: &[Item], provider_tokens: u64) -> Result<()> {
        let count = self.token_count(items, provider_tokens);
        if count > self.budget_limit() {
            return Err(LitecodeError::TokenBudgetExceeded);
        }
        Ok(())
    }

    pub fn log_iteration(&self, step: u64, token_count: usize) {
        let budget = self.budget_limit();
        tracing::debug!(step, token_count, budget, "loop iteration");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::user_text;

    #[test]
    fn token_count_prefers_provider_or_falls_back_to_estimate() {
        let budget = BudgetPolicy::new(10_000);
        let tiny = vec![user_text("hi")];
        // (provider_tokens, expected)
        let cases = [(8_500_u64, 8_500_usize), (0, compute_token_estimate(&tiny))];
        for (provider_tokens, expected) in cases {
            assert_eq!(
                budget.token_count(&tiny, provider_tokens),
                expected,
                "provider_tokens={provider_tokens}"
            );
        }
    }

    #[test]
    fn provider_bill_covers_its_prefix_and_estimates_only_the_suffix() {
        let budget = BudgetPolicy::new(10_000);
        let items = vec![user_text("sent"), user_text("x".repeat(4_000))];
        let seqs = vec![Some(4), Some(9)];
        let bill = OccupancyBill {
            prompt_tokens: 7_000,
            covered_seq: Some(4),
        };
        assert_eq!(
            budget.occupancy_token_count(&items, &seqs, Some(bill)),
            7_000 + compute_token_estimate(&items[1..])
        );
    }

    #[test]
    fn low_provider_bill_stays_under_threshold_when_local_estimate_is_over() {
        let budget = BudgetPolicy::new(10_000);
        let items = vec![user_text("word ".repeat(12_000))];
        let seqs = vec![Some(3_u64)];
        let local = budget.occupancy_token_count(&items, &seqs, None);
        assert!(
            local > budget.autocompact_threshold(),
            "local estimate {local} must clear 80%"
        );
        let under = OccupancyBill {
            prompt_tokens: 7_000,
            covered_seq: Some(3),
        };
        let count = budget.occupancy_token_count(&items, &seqs, Some(under));
        assert_eq!(count, 7_000);
        assert!(!budget.should_compact(count));

        let over = OccupancyBill {
            prompt_tokens: 8_500,
            covered_seq: Some(3),
        };
        assert!(budget.should_compact(budget.occupancy_token_count(&items, &seqs, Some(over))));
    }

    #[test]
    fn rewritten_view_does_not_reuse_a_provider_bill() {
        let budget = BudgetPolicy::new(10_000);
        let items = vec![user_text("summary")];
        let local = compute_token_estimate(&items);
        assert_eq!(budget.occupancy_token_count(&items, &[], None), local);
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

    #[test]
    fn ring_occupancy_prefers_provider_prompt() {
        assert_eq!(ring_occupancy(Some(3_000), 100), 3_000);
        assert_eq!(ring_occupancy(Some(0), 100), 100);
        assert_eq!(ring_occupancy(None, 100), 100);
        assert!(manual_compact_eligible(
            10_000,
            ring_occupancy(Some(3_000), 100)
        ));
        assert!(!manual_compact_eligible(
            10_000,
            ring_occupancy(Some(2_000), 9_000)
        ));
    }

    #[test]
    fn token_count_does_not_apply_a_separate_media_slice() {
        // Trigger and stored estimate share compute_token_estimate. Stale-media
        // dropping happens on the items passed in, not inside the count.
        let budget = BudgetPolicy::new(10_000);
        let items = vec![user_text("hi")];
        assert_eq!(
            budget.token_count(&items, 0),
            compute_token_estimate(&items)
        );
        assert_eq!(
            budget.occupancy_token_count(&items, &[Some(1)], None),
            compute_token_estimate(&items)
        );
    }

    #[test]
    fn manual_compact_gate_is_inclusive_at_thirty_percent() {
        assert!(!manual_compact_eligible(0, 10_000));
        assert!(!manual_compact_eligible(10_000, 2_999));
        assert!(manual_compact_eligible(10_000, 3_000));
    }
}
