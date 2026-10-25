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
use crate::llm::{CompactionRequestContext, ModelRequest, ToolDef};
use crate::provider_catalog::ResolvedModel;
use crate::session::estimate::{
    ItemTokenBreakdown, apply_prompt_overhead, compute_token_breakdown, compute_token_estimate,
    count_text_tokens,
};
use crate::session::manager::SessionManager;
use crate::session::working::WorkingRow;
use crate::types::{Item, LitecodeError, Result};

/// Conservative fixed protocol envelope not represented by item/tool payloads.
/// Provider usage, when present, already contains this cost and is never topped up.
pub const REQUEST_PROMPT_OVERHEAD_TOKENS: usize = 64;

/// Inclusive product floor for user-triggered compaction.
pub use super::limits::MANUAL_COMPACT_RATIO_PERCENT;

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
    /// Provider model maximum output (`max_output`); never added to used tokens.
    pub output_reserve: usize,
    /// Hard prompt ceiling: `W - O`, saturating at zero for tiny windows.
    pub prompt_limit: usize,
    /// Auto pressure line: `min(0.8W, W - O)`.
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
        used_tokens: usize,
        source: ContextMeterSource,
    ) -> Self {
        let prompt_limit = context_window.saturating_sub(output_reserve);
        let compact_at = (context_window.saturating_mul(4) / 5).min(prompt_limit);
        Self {
            context_window,
            output_reserve,
            prompt_limit,
            compact_at,
            used_tokens,
            source,
            compact_eligible: context_window > 1
                && used_tokens.saturating_mul(100)
                    >= context_window.saturating_mul(MANUAL_COMPACT_RATIO_PERCENT),
        }
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
            used_tokens,
            ContextMeterSource::LocalEnvelope,
        )
    }

    pub fn measure_local(
        context_window: usize,
        output_reserve: usize,
        envelope: &ContextEnvelope,
        items: &[Item],
    ) -> Self {
        Self::measure(context_window, output_reserve, envelope, items, &[], None)
    }
}

/// Measure the exact `project_model_view` request shape auto/manual/UI will see.
pub fn measure_request_context(
    sessions: &SessionManager,
    session_id: &str,
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
        model.context_window_max,
        model.max_output as usize,
        envelope,
        &projection.items,
        &projection.coverage_seqs,
        bill,
    )
}

/// Persisted provider bill, with the request-header seq it covers.
pub fn persisted_occupancy_bill(
    sessions: &SessionManager,
    session_id: &str,
) -> Result<Option<OccupancyBill>> {
    let meta = sessions.data().meta_blocking(session_id).ok();
    let compacted_seq = meta.as_ref().and_then(|meta| meta.compacted_seq);
    let surface_rewrite_seq = meta.as_ref().map_or(0, |meta| meta.surface_rewrite_seq);
    let rollup = sessions.provider_usage_rollup(session_id)?;
    let meter = sessions
        .data()
        .meter_blocking(session_id)
        .unwrap_or_default();
    Ok(occupancy_bill_from(
        &ProviderPromptBaseline::default(),
        compacted_seq,
        surface_rewrite_seq,
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
    envelope: &ContextEnvelope,
    model: &ResolvedModel,
    data_root: &Path,
) -> Result<(ContextMeter, ItemTokenBreakdown)> {
    let bill = persisted_occupancy_bill(sessions, session_id)?;
    let rows = sessions.data().working_set_blocking(session_id)?;
    let (_turn_view, projection) =
        crate::context_pipeline::project_working_rows(&rows, model, data_root);
    let meter = ContextMeter::measure(
        model.context_window_max,
        model.max_output as usize,
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
    envelope: &ContextEnvelope,
    model: &ResolvedModel,
    data_root: &Path,
) -> Result<ContextMeter> {
    measure_persisted_context_with_breakdown(sessions, session_id, envelope, model, data_root)
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
    fn provider_window_and_output_table_uses_min_of_eighty_percent_and_prompt_limit() {
        let cases = [
            // W, O, prompt_limit, compact_at
            (1_000_000usize, 128_000usize, 872_000usize, 800_000usize),
            (256_000, 64_000, 192_000, 192_000),
            (1_000_000, 256_000, 744_000, 744_000),
            (256_000, 128_000, 128_000, 128_000),
        ];
        for (window, output, expected_limit, expected_compact_at) in cases {
            let meter = ContextMeter::new(window, output, 0, ContextMeterSource::LocalEnvelope);
            assert_eq!(meter.prompt_limit, expected_limit, "W={window} O={output}");
            assert_eq!(
                meter.compact_at, expected_compact_at,
                "W={window} O={output}"
            );
            assert_eq!(meter.output_reserve, output);
            assert_eq!(meter.compact_at, (window * 4 / 5).min(window - output));
        }
    }

    #[test]
    fn small_windows_only_saturate_non_negative_values() {
        let cases = [
            // W, O, prompt_limit, compact_at, pressure
            (0usize, 0usize, 0usize, 0usize, false),
            (1, 8, 0, 0, false),
            (2, 99, 0, 0, true),
            (8, 4, 4, 4, true),
        ];
        for (window, output, expected_limit, expected_compact_at, expected_pressure) in cases {
            let meter = ContextMeter::new(window, output, 1, ContextMeterSource::LocalEnvelope);
            assert_eq!(meter.output_reserve, output, "W={window} O={output}");
            assert_eq!(meter.prompt_limit, expected_limit, "W={window} O={output}");
            assert_eq!(
                meter.compact_at, expected_compact_at,
                "W={window} O={output}"
            );
            assert_eq!(meter.pressure_enabled(), expected_pressure);
        }
    }

    #[test]
    fn auto_gate_is_strictly_above_min_of_eighty_percent_and_prompt_limit() {
        let below = ContextMeter::new(10_000, 1_024, 7_999, ContextMeterSource::LocalEnvelope);
        let at = ContextMeter::new(10_000, 1_024, 8_000, ContextMeterSource::LocalEnvelope);
        let over = ContextMeter::new(10_000, 1_024, 8_001, ContextMeterSource::LocalEnvelope);
        assert!(!below.should_compact());
        assert!(!at.should_compact(), "auto trigger is strict > compact_at");
        assert!(over.should_compact());
    }

    #[test]
    fn manual_gate_is_inclusive_at_thirty_percent_of_window() {
        let below = ContextMeter::new(10_000, 0, 2_999, ContextMeterSource::LocalEnvelope);
        let at = ContextMeter::new(10_000, 0, 3_000, ContextMeterSource::LocalEnvelope);
        assert!(!below.manual_eligible());
        assert!(at.manual_eligible());
        assert!(at.compact_eligible);
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
        let meter = ContextMeter::measure(10_000, env.output_reserve, &env, &items, &[], None);

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
        let meter =
            ContextMeter::measure(10_000, env.output_reserve, &env, &items, &seqs, Some(bill));

        assert_eq!(meter.source, ContextMeterSource::ProviderPlusSuffix);
        assert_eq!(meter.used_tokens, 7_000 + suffix_tokens);
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
            &env,
            &items,
            &[Some(9)],
            Some(bill),
        );
        assert_eq!(meter.source, ContextMeterSource::Provider);
        assert_eq!(meter.used_tokens, 321);
    }
}
