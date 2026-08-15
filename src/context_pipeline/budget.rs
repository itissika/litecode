use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::context_pipeline::estimate::{autocompact_threshold, compute_token_estimate};
use crate::context_pipeline::keep_recent::default_keep_recent_tokens;
use crate::types::{Item, LitecodeError, Result};

/// Provider-reported prompt usage tied to the exact Item prefix sent in that request.
///
/// Items appended by the model/tool loop have not reached the provider yet, so
/// budget checks add a local estimate for that suffix to the authoritative count.
#[derive(Debug, Default)]
pub struct ProviderPromptBaseline {
    prompt_tokens: AtomicU64,
    item_count: AtomicUsize,
}

impl ProviderPromptBaseline {
    pub fn record(&self, prompt_tokens: u64, item_count: usize) {
        self.item_count.store(item_count, Ordering::Relaxed);
        self.prompt_tokens.store(prompt_tokens, Ordering::Release);
    }

    pub fn clear(&self) {
        self.prompt_tokens.store(0, Ordering::Release);
        self.item_count.store(0, Ordering::Relaxed);
    }

    pub fn prompt_tokens(&self) -> u64 {
        self.prompt_tokens.load(Ordering::Acquire)
    }

    fn snapshot(&self) -> Option<(usize, usize)> {
        let prompt_tokens = self.prompt_tokens.load(Ordering::Acquire);
        if prompt_tokens == 0 {
            return None;
        }
        Some((
            prompt_tokens as usize,
            self.item_count.load(Ordering::Relaxed),
        ))
    }
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

    /// Provider truth for the last request plus a local estimate for Items appended
    /// since that request. A shorter working set invalidates the recorded prefix.
    pub fn token_count_with_baseline(
        &self,
        items: &[Item],
        baseline: &ProviderPromptBaseline,
    ) -> usize {
        let Some((provider_tokens, item_count)) = baseline.snapshot() else {
            return self.local_token_count(items);
        };
        if item_count > items.len() {
            return self.local_token_count(items);
        }

        provider_tokens.saturating_add(compute_token_estimate(&items[item_count..]))
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

    pub fn enforce_hard_limit_with_baseline(
        &self,
        items: &[Item],
        baseline: &ProviderPromptBaseline,
    ) -> Result<()> {
        let count = self.token_count_with_baseline(items, baseline);
        if count > self.budget_limit() {
            return Err(LitecodeError::TokenBudgetExceeded);
        }
        Ok(())
    }

    pub fn log_iteration(&self, step: u64, token_count: usize) {
        let budget = self.budget_limit();
        tracing::info!(step, token_count, budget, "loop iteration");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::user_text;

    #[test]
    fn token_count_prefers_provider_when_set() {
        let budget = BudgetPolicy::new(10_000);
        let tiny = vec![user_text("hi")];
        assert_eq!(budget.token_count(&tiny, 8_500), 8_500);
    }

    #[test]
    fn token_count_falls_back_to_estimate_without_provider() {
        let budget = BudgetPolicy::new(10_000);
        let tiny = vec![user_text("hi")];
        assert_eq!(budget.token_count(&tiny, 0), compute_token_estimate(&tiny));
    }

    #[test]
    fn baseline_adds_items_appended_after_provider_request() {
        let budget = BudgetPolicy::new(10_000);
        let baseline = ProviderPromptBaseline::default();
        baseline.record(7_000, 1);
        let items = vec![user_text("sent"), user_text("x".repeat(4_000))];
        assert_eq!(
            budget.token_count_with_baseline(&items, &baseline),
            7_000 + compute_token_estimate(&items[1..])
        );
    }

    #[test]
    fn token_count_does_not_apply_a_separate_media_slice() {
        // Trigger, success check, and stored estimate share compute_token_estimate
        // / token_count_with_baseline — no window/5 media trim on the counter.
        let budget = BudgetPolicy::new(10_000);
        let items = vec![user_text("hi")];
        assert_eq!(
            budget.token_count(&items, 0),
            compute_token_estimate(&items)
        );
        assert_eq!(
            budget.token_count_with_baseline(&items, &ProviderPromptBaseline::default()),
            compute_token_estimate(&items)
        );
    }

    #[test]
    fn baseline_falls_back_when_recorded_prefix_no_longer_exists() {
        let budget = BudgetPolicy::new(10_000);
        let baseline = ProviderPromptBaseline::default();
        baseline.record(7_000, 3);
        let items = vec![user_text("rewritten")];
        assert_eq!(
            budget.token_count_with_baseline(&items, &baseline),
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
