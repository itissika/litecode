//! Unified main-request envelope and compaction pressure meter.
//!
//! Provider `usage.prompt_tokens` is authoritative for the request prefix it
//! covers. Rows after `covered_seq` are added with the local tokenizer. When no
//! provider bill exists, the local fallback includes the full prompt envelope:
//! instructions/system, tool schemas, fixed request overhead, and input items.
//! The output reserve is a separate side of the same budget: it reduces the
//! prompt limit but is never added to `used_tokens` (which is prompt occupancy).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::context_pipeline::budget::{OccupancyBill, ProviderPromptBaseline, occupancy_bill_from};
use crate::context_pipeline::media_budget::drop_stale_media;
use crate::llm::{CompactionRequestContext, ModelRequest, ToolDef};
use crate::provider_catalog::ResolvedModel;
use crate::session::estimate::{
    ItemTokenBreakdown, apply_prompt_overhead, autocompact_threshold, compute_token_breakdown,
    compute_token_estimate, count_text_tokens,
};
use crate::session::manager::SessionManager;
use crate::session::working::{WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result};

/// Conservative fixed protocol envelope not represented by item/tool payloads.
/// Provider usage, when present, already contains this cost and is never topped up.
pub const REQUEST_PROMPT_OVERHEAD_TOKENS: usize = 64;

/// Upper bound for the explicit pressure headroom.
pub const MAX_SAFETY_HEADROOM_TOKENS: usize = 16_384;

/// Inclusive product floor for user-triggered compaction.
pub const MANUAL_COMPACT_RATIO_PERCENT: usize = 30;

/// Single safety-headroom policy: `min(16_384, context_window / 8)`.
pub fn safety_headroom(context_window: usize) -> usize {
    (context_window / 8).min(MAX_SAFETY_HEADROOM_TOKENS)
}

/// The full fixed side of one main-model request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextEnvelope {
    pub instructions_tokens: usize,
    pub tools_tokens: usize,
    pub prompt_overhead_tokens: usize,
    /// Actual current main request `max_output_tokens`, not a hidden constant.
    pub output_reserve: usize,
}

impl ContextEnvelope {
    pub fn from_request_parts(instructions: &str, tools: &[ToolDef], output_reserve: u32) -> Self {
        Self {
            instructions_tokens: count_text_tokens(instructions),
            tools_tokens: tools.iter().map(tool_schema_tokens).sum(),
            prompt_overhead_tokens: REQUEST_PROMPT_OVERHEAD_TOKENS,
            output_reserve: output_reserve as usize,
        }
    }

    pub fn from_request(request: &ModelRequest) -> Self {
        Self::from_request_parts(
            &request.instructions,
            &request.tools,
            request.max_output_tokens,
        )
    }

    pub fn from_compaction_request(request: &CompactionRequestContext) -> Self {
        Self::from_request_parts(
            &request.instructions,
            &request.tools,
            request.max_output_tokens,
        )
    }

    /// Server-side cold-start fallback. The UI must never rebuild this envelope
    /// from raw transcript/provider numbers.
    pub fn cold(output_reserve: usize) -> Self {
        Self {
            instructions_tokens: 0,
            tools_tokens: 0,
            prompt_overhead_tokens: REQUEST_PROMPT_OVERHEAD_TOKENS,
            output_reserve,
        }
    }

    pub fn fixed_prompt_tokens(&self) -> usize {
        self.instructions_tokens
            .saturating_add(self.tools_tokens)
            .saturating_add(self.prompt_overhead_tokens)
    }

    pub fn with_output_reserve(mut self, output_reserve: usize) -> Self {
        self.output_reserve = output_reserve;
        self
    }

    /// Fill the fixed buckets on an item breakdown without re-tokenizing payloads.
    pub fn apply_to_breakdown(&self, breakdown: &mut ItemTokenBreakdown) {
        breakdown.system = self.instructions_tokens;
        breakdown.tool_schema = self.tools_tokens;
        breakdown.prompt_overhead = self.prompt_overhead_tokens;
    }
}

/// Source of the `used_tokens` number exposed by [`ContextMeter`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMeterSource {
    /// Provider bill covers the current request and no uncovered suffix exists.
    Provider,
    /// Provider bill plus a locally estimated suffix written after `covered_seq`.
    ProviderPlusSuffix,
    /// No usable provider bill; the full local request envelope is counted.
    #[default]
    LocalEnvelope,
}

/// One measurement shared by auto trigger, manual eligibility, and UI snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMeter {
    pub context_window: usize,
    /// Effective request output reservation (`max_output_tokens` after small-window clamp).
    pub output_reserve: usize,
    /// Explicit pressure headroom: `min(16_384, context_window / 8)` by policy.
    pub safety_headroom: usize,
    /// Hard prompt ceiling: `context_window - output_reserve`.
    pub prompt_limit: usize,
    /// Auto pressure line: `min(0.8W, prompt_limit - safety_headroom)`, clamped.
    pub compact_at: usize,
    /// Prompt occupancy only. The output reserve is represented by `prompt_limit`.
    pub used_tokens: usize,
    pub source: ContextMeterSource,
    /// Inclusive 30% manual-compaction gate. UI consumes this decision; it must
    /// not recompute a threshold from provider/transcript values.
    pub compact_eligible: bool,
}

impl ContextMeter {
    /// Build the policy side of a meter. Only this constructor contains the
    /// pressure formula; all callers answer their question from this result.
    pub fn new(
        context_window: usize,
        output_reserve: usize,
        safety_headroom: usize,
        used_tokens: usize,
        source: ContextMeterSource,
    ) -> Self {
        if context_window == 0 {
            return Self {
                context_window: 0,
                output_reserve: 0,
                safety_headroom,
                prompt_limit: 0,
                compact_at: 0,
                used_tokens,
                source,
                compact_eligible: false,
            };
        }

        // A one-token window cannot hold a meaningful prompt plus reply. It is
        // explicitly disabled rather than reporting permanent pressure.
        if context_window == 1 {
            return Self {
                context_window,
                output_reserve: 0,
                safety_headroom,
                prompt_limit: 1,
                compact_at: 0,
                used_tokens,
                source,
                compact_eligible: false,
            };
        }

        // Leave at least two prompt tokens on absurdly small windows so the
        // pressure line can stay strictly below the hard prompt limit.
        let output_reserve = output_reserve.min(context_window.saturating_sub(2));
        let prompt_limit = context_window.saturating_sub(output_reserve);
        let soft_limit = autocompact_threshold(context_window);
        let headroom_limited = prompt_limit.saturating_sub(safety_headroom);
        let compact_at = soft_limit
            .min(headroom_limited)
            .clamp(1, prompt_limit.saturating_sub(1));

        Self {
            context_window,
            output_reserve,
            safety_headroom,
            prompt_limit,
            compact_at,
            used_tokens,
            source,
            compact_eligible: used_tokens.saturating_mul(100)
                >= context_window.saturating_mul(MANUAL_COMPACT_RATIO_PERCENT),
        }
    }

    pub fn safety_headroom(context_window: usize) -> usize {
        safety_headroom(context_window)
    }

    /// W=0 means the binding is unknown and W=1 cannot hold a prompt plus
    /// reply; both are non-pressure states.
    pub fn pressure_enabled(&self) -> bool {
        self.context_window > 1
    }

    pub fn should_compact(&self) -> bool {
        self.pressure_enabled() && self.used_tokens > self.compact_at
    }

    pub fn manual_eligible(&self) -> bool {
        self.pressure_enabled() && self.compact_eligible
    }

    pub fn is_over_prompt_limit(&self) -> bool {
        self.pressure_enabled() && self.used_tokens > self.prompt_limit
    }

    /// Prompt occupancy plus the separate output reservation. Useful for
    /// diagnostics; pressure decisions compare `used_tokens` to `compact_at`.
    pub fn occupied_with_output(&self) -> usize {
        self.used_tokens.saturating_add(self.output_reserve)
    }

    pub fn enforce_prompt_limit(&self) -> Result<()> {
        if self.is_over_prompt_limit() {
            Err(LitecodeError::TokenBudgetExceeded)
        } else {
            Ok(())
        }
    }

    pub fn log_iteration(&self, step: u64) {
        tracing::debug!(
            step,
            context_window = self.context_window,
            output_reserve = self.output_reserve,
            safety_headroom = self.safety_headroom,
            prompt_limit = self.prompt_limit,
            compact_at = self.compact_at,
            used_tokens = self.used_tokens,
            source = ?self.source,
            "context pressure"
        );
    }

    /// Measure a projected request against an optional provider bill.
    ///
    /// A valid bill wins and is never mixed with local fixed overhead. The local
    /// suffix contains only projected items whose coverage seq is after
    /// `covered_seq` (or truly pending/synthetic items with no provenance). A
    /// length mismatch falls back to the full local envelope because there is no
    /// safe suffix boundary.
    ///
    /// `coverage_seqs` is not the same as compaction `item_seqs`: synthetic pads
    /// inherit their source call's seq for billing coverage while retaining
    /// `None` in `item_seqs` for prefix/checkpoint identity.
    #[allow(clippy::too_many_arguments)]
    pub fn measure(
        context_window: usize,
        output_reserve: usize,
        safety_headroom: usize,
        envelope: &ContextEnvelope,
        items: &[Item],
        coverage_seqs: &[Option<u64>],
        bill: Option<OccupancyBill>,
    ) -> Self {
        if let Some(bill) = bill.filter(|bill| bill.prompt_tokens > 0)
            && coverage_seqs.len() == items.len()
        {
            let mut suffix = Vec::new();
            for (item, seq) in items.iter().zip(coverage_seqs.iter()) {
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
            let source = if suffix.is_empty() {
                ContextMeterSource::Provider
            } else {
                ContextMeterSource::ProviderPlusSuffix
            };
            return Self::new(
                context_window,
                output_reserve,
                safety_headroom,
                (bill.prompt_tokens as usize).saturating_add(compute_token_estimate(&suffix)),
                source,
            );
        }

        let used_tokens = envelope
            .fixed_prompt_tokens()
            .saturating_add(compute_token_estimate(items));
        Self::new(
            context_window,
            output_reserve,
            safety_headroom,
            used_tokens,
            ContextMeterSource::LocalEnvelope,
        )
    }

    pub fn measure_local(
        context_window: usize,
        output_reserve: usize,
        safety_headroom: usize,
        envelope: &ContextEnvelope,
        items: &[Item],
    ) -> Self {
        Self::measure(
            context_window,
            output_reserve,
            safety_headroom,
            envelope,
            items,
            &[],
            None,
        )
    }
}

/// Measure the exact `project_model_view` request shape auto/manual/UI will see.
pub fn measure_request_context(
    sessions: &SessionManager,
    session_id: &str,
    context_window: usize,
    envelope: &ContextEnvelope,
    baseline: &ProviderPromptBaseline,
    rows: &[WorkingRow],
    model: &ResolvedModel,
    data_root: &Path,
) -> ContextMeter {
    let bill = super::resolve_occupancy_bill(sessions, session_id, baseline);
    let (_turn_view, projection) =
        crate::context_pipeline::project_working_rows(rows, model, data_root);
    ContextMeter::measure(
        context_window,
        envelope.output_reserve,
        ContextMeter::safety_headroom(context_window),
        envelope,
        &projection.items,
        &projection.coverage_seqs,
        bill,
    )
}

/// Project persisted working rows the same way budget checks do: model Items
/// with stale media removed, retaining their durable seq for suffix accounting.
pub fn working_set_items(rows: &[WorkingRow]) -> (Vec<Item>, Vec<Option<u64>>) {
    let item_seqs = rows.iter().map(|row| row.log_seq).collect();
    let mut items = project_items(rows);
    drop_stale_media(&mut items);
    (items, item_seqs)
}

/// Persisted provider bill, with the request-header seq it covers.
pub fn persisted_occupancy_bill(
    sessions: &SessionManager,
    session_id: &str,
) -> Result<Option<OccupancyBill>> {
    let compacted_seq = sessions
        .data()
        .meta_blocking(session_id)
        .ok()
        .and_then(|meta| meta.compacted_seq);
    let rollup = sessions.provider_usage_rollup(session_id)?;
    let meter = sessions
        .data()
        .meter_blocking(session_id)
        .unwrap_or_default();
    Ok(occupancy_bill_from(
        &ProviderPromptBaseline::default(),
        compacted_seq,
        &rollup,
        &meter,
    ))
}

/// Last request-shaped envelope persisted on a request-header row. This keeps
/// a cold UI's local fallback anchored to server-owned instructions/tools/main
/// output caps instead of asking the browser to reconstruct them.
pub fn latest_persisted_envelope(
    sessions: &SessionManager,
    session_id: &str,
) -> Option<ContextEnvelope> {
    sessions
        .request_origins(session_id)
        .ok()?
        .into_iter()
        .rev()
        .find_map(|(_, body)| {
            serde_json::from_value::<ContextEnvelope>(body.get("context_envelope")?.clone()).ok()
        })
}

/// Single persisted-session measurement used by manual eligibility and the UI
/// snapshot. Auto passes its in-memory baseline to [`measure_request_context`].
pub fn measure_persisted_context_with_breakdown(
    sessions: &SessionManager,
    session_id: &str,
    context_window: usize,
    envelope: &ContextEnvelope,
    model: &ResolvedModel,
    data_root: &Path,
) -> Result<(ContextMeter, ItemTokenBreakdown)> {
    let bill = persisted_occupancy_bill(sessions, session_id)?;
    let rows = sessions.data().working_set_blocking(session_id)?;
    let (_turn_view, projection) =
        crate::context_pipeline::project_working_rows(&rows, model, data_root);
    let meter = ContextMeter::measure(
        context_window,
        envelope.output_reserve,
        ContextMeter::safety_headroom(context_window),
        envelope,
        &projection.items,
        &projection.coverage_seqs,
        bill,
    );
    // Same projection object, same rows: the bucket mix cannot drift from the
    // occupancy/source decision published in the meter.
    let breakdown = request_token_breakdown_for_items(&projection.items, envelope);
    Ok((meter, breakdown))
}

pub fn measure_persisted_context(
    sessions: &SessionManager,
    session_id: &str,
    context_window: usize,
    envelope: &ContextEnvelope,
    model: &ResolvedModel,
    data_root: &Path,
) -> Result<ContextMeter> {
    measure_persisted_context_with_breakdown(
        sessions,
        session_id,
        context_window,
        envelope,
        model,
        data_root,
    )
    .map(|(meter, _)| meter)
}

/// Token-bucket breakdown for the same projected request shape used by the
/// occupancy meter. Provider sources scale this mix to `used_tokens`.
pub fn request_token_breakdown_for_rows(
    rows: &[WorkingRow],
    envelope: &ContextEnvelope,
    model: &ResolvedModel,
    data_root: &Path,
) -> ItemTokenBreakdown {
    let (_turn_view, projection) =
        crate::context_pipeline::project_working_rows(rows, model, data_root);
    self::request_token_breakdown_for_items(&projection.items, envelope)
}

fn request_token_breakdown_for_items(
    items: &[Item],
    envelope: &ContextEnvelope,
) -> ItemTokenBreakdown {
    let mut breakdown = compute_token_breakdown(items);
    envelope.apply_to_breakdown(&mut breakdown);
    breakdown
}

pub fn tool_schema_tokens(tool: &ToolDef) -> usize {
    let payload = serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    });
    count_text_tokens(&payload.to_string())
}

/// Shared request breakdown used by `LlmRequestBuilt`; fixed request buckets are
/// filled by the same [`ContextEnvelope`] the pressure meter measures.
pub fn request_token_breakdown(request: &ModelRequest) -> ItemTokenBreakdown {
    let mut breakdown = compute_token_breakdown(&request.input);
    let schemas: Vec<(String, usize)> = request
        .tools
        .iter()
        .map(|tool| (tool.name.clone(), tool_schema_tokens(tool)))
        .collect();
    apply_prompt_overhead(&mut breakdown, &request.instructions, &schemas);
    ContextEnvelope::from_request(request).apply_to_breakdown(&mut breakdown);
    breakdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolDef;
    use crate::types::{assistant_text, user_text};

    fn envelope(output_reserve: usize) -> ContextEnvelope {
        ContextEnvelope {
            instructions_tokens: 11,
            tools_tokens: 13,
            prompt_overhead_tokens: REQUEST_PROMPT_OVERHEAD_TOKENS,
            output_reserve,
        }
    }

    #[test]
    fn threshold_table_covers_window_output_and_headroom() {
        let cases = [
            // W, O, H, prompt_limit, compact_at
            (
                1_000_000usize,
                100_000usize,
                16_384usize,
                900_000usize,
                800_000usize,
            ),
            (1_000_000, 256_000, 16_384, 744_000, 727_616),
            (128_000, 8_192, 16_000, 119_808, 102_400),
            (10_000, 1_024, 1_250, 8_976, 7_726),
            (1_000, 100, 125, 900, 775),
        ];
        for (window, output, headroom, expected_limit, expected_compact_at) in cases {
            let meter = ContextMeter::new(
                window,
                output,
                headroom,
                0,
                ContextMeterSource::LocalEnvelope,
            );
            assert_eq!(meter.prompt_limit, expected_limit, "W={window} O={output}");
            assert_eq!(
                meter.compact_at, expected_compact_at,
                "W={window} O={output} H={headroom}"
            );
            assert!(meter.compact_at > 0);
            assert!(meter.compact_at < meter.prompt_limit);
        }
    }

    #[test]
    fn safety_headroom_is_one_policy() {
        assert_eq!(ContextMeter::safety_headroom(0), 0);
        assert_eq!(ContextMeter::safety_headroom(1_000), 125);
        assert_eq!(ContextMeter::safety_headroom(128_000), 16_000);
        assert_eq!(ContextMeter::safety_headroom(1_000_000), 16_384);
    }

    #[test]
    fn small_windows_clamp_output_and_keep_pressure_below_hard_limit() {
        for (window, output) in [(2usize, 99usize), (8, 100), (100, 90), (128, 65_536)] {
            let meter = ContextMeter::new(
                window,
                output,
                ContextMeter::safety_headroom(window),
                0,
                ContextMeterSource::LocalEnvelope,
            );
            assert!(meter.prompt_limit > 0, "W={window} O={output}");
            assert!(meter.compact_at > 0, "W={window} O={output}");
            if meter.prompt_limit > 1 {
                assert!(meter.compact_at < meter.prompt_limit);
            }
            assert!(meter.output_reserve < window);
        }
    }

    #[test]
    fn degenerate_window_table_disables_pressure() {
        // W, O, H, effective O, prompt_limit, compact_at, used
        let cases = [
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 100usize),
            (1, 0, 0, 0, 1, 0, 100),
            (1, 8, 0, 0, 1, 0, 100),
            (2, 0, 0, 0, 2, 1, 1),
            (2, 99, 0, 0, 2, 1, 2),
            (8, 4, 1, 4, 4, 3, 4),
            (8, 100, 1, 6, 2, 1, 2),
        ];
        for (window, output, headroom, effective_output, prompt_limit, compact_at, used) in cases {
            let meter = ContextMeter::new(
                window,
                output,
                headroom,
                used,
                ContextMeterSource::LocalEnvelope,
            );
            assert_eq!(
                meter.output_reserve, effective_output,
                "W={window} O={output}"
            );
            assert_eq!(meter.prompt_limit, prompt_limit, "W={window} O={output}");
            assert_eq!(
                meter.compact_at, compact_at,
                "W={window} O={output} H={headroom}"
            );
            if window <= 1 {
                assert!(!meter.pressure_enabled());
                assert!(
                    !meter.should_compact(),
                    "W={window} must not report pressure"
                );
                assert!(!meter.manual_eligible());
                assert!(!meter.is_over_prompt_limit());
                meter
                    .enforce_prompt_limit()
                    .expect("degenerate windows are non-pressure");
            } else {
                assert!(meter.pressure_enabled());
                assert!(meter.compact_at > 0);
                assert!(meter.compact_at < meter.prompt_limit);
            }
        }
    }

    #[test]
    fn local_fallback_counts_system_tools_overhead_and_input() {
        let tools = vec![ToolDef {
            name: "read_file".into(),
            description: "read a file".into(),
            input_schema: serde_json::json!({"type":"object","properties":{}}),
        }];
        let env = ContextEnvelope::from_request_parts("system instructions", &tools, 512);
        let items = vec![user_text("hello world"), assistant_text("done")];
        let input_tokens = compute_token_estimate(&items);
        let meter = ContextMeter::measure(
            10_000,
            env.output_reserve,
            ContextMeter::safety_headroom(10_000),
            &env,
            &items,
            &[],
            None,
        );

        assert_eq!(meter.source, ContextMeterSource::LocalEnvelope);
        assert_eq!(
            meter.used_tokens,
            env.fixed_prompt_tokens() + input_tokens,
            "local fallback must count instructions + tools + fixed overhead + input"
        );
        assert_eq!(meter.output_reserve, 512);
        assert_eq!(meter.prompt_limit, 10_000 - 512);
        assert_eq!(
            meter.occupied_with_output(),
            meter.used_tokens + meter.output_reserve
        );
    }

    #[test]
    fn provider_bill_plus_suffix_never_double_counts_prefix_or_fixed_overhead() {
        let env = envelope(777);
        let items = vec![
            user_text("covered prefix ".repeat(500)),
            user_text("suffix"),
            assistant_text("tail"),
        ];
        let seqs = vec![Some(4), Some(9), Some(10)];
        let bill = OccupancyBill {
            prompt_tokens: 7_000,
            covered_seq: Some(4),
        };
        let suffix_tokens = compute_token_estimate(&items[1..]);
        let meter = ContextMeter::measure(
            10_000,
            env.output_reserve,
            ContextMeter::safety_headroom(10_000),
            &env,
            &items,
            &seqs,
            Some(bill),
        );

        assert_eq!(meter.source, ContextMeterSource::ProviderPlusSuffix);
        assert_eq!(meter.used_tokens, 7_000 + suffix_tokens);
        // Exactly one bill + suffix: the covered prefix and the local fixed
        // overhead are not present in the local suffix term.
    }

    #[test]
    fn provider_bill_without_uncovered_rows_is_provider_only() {
        let env = envelope(100);
        let items = vec![user_text("covered")];
        let bill = OccupancyBill {
            prompt_tokens: 321,
            covered_seq: Some(9),
        };
        let meter = ContextMeter::measure(
            1_000,
            env.output_reserve,
            ContextMeter::safety_headroom(1_000),
            &env,
            &items,
            &[Some(9)],
            Some(bill),
        );
        assert_eq!(meter.source, ContextMeterSource::Provider);
        assert_eq!(meter.used_tokens, 321);
    }

    #[test]
    fn auto_gate_is_strictly_above_the_pressure_line() {
        let below = ContextMeter::new(
            10_000,
            1_024,
            1_250,
            7_725,
            ContextMeterSource::LocalEnvelope,
        );
        let at = ContextMeter::new(
            10_000,
            1_024,
            1_250,
            7_726,
            ContextMeterSource::LocalEnvelope,
        );
        let over = ContextMeter::new(
            10_000,
            1_024,
            1_250,
            7_727,
            ContextMeterSource::LocalEnvelope,
        );
        assert!(!below.should_compact());
        assert!(!at.should_compact(), "auto trigger is strict > compact_at");
        assert!(over.should_compact());
    }

    #[test]
    fn manual_gate_is_inclusive_at_thirty_percent_of_window() {
        let below = ContextMeter::new(10_000, 0, 0, 2_999, ContextMeterSource::LocalEnvelope);
        let at = ContextMeter::new(10_000, 0, 0, 3_000, ContextMeterSource::LocalEnvelope);
        assert!(!below.manual_eligible());
        assert!(at.manual_eligible());
        assert!(at.compact_eligible);
    }
}
