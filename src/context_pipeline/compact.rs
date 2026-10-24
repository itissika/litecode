use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

#[cfg(test)]
use std::cell::Cell;

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputTextContent, MessageItem, OutputStatus,
};
use crate::context_pipeline::keep_recent::find_keep_recent_cut;
use crate::context_pipeline::project_model_view;
use crate::llm::CompactionRequestContext;
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use crate::session::event::Seq;
use crate::session::manager::SessionManager;
#[cfg(test)]
use crate::session::working::WorkingPayload;
use crate::session::working::{CompactedView, CompactionCheckpoint, WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result, Transcript, item_text_preview};

use super::budget::{BudgetPolicy, ProviderPromptBaseline};
use super::context_meter::{ContextEnvelope, ContextMeter, working_set_items};

/// Wall-clock cap for the compact LLM call.
///
/// A provider that accepts the request and then goes silent would otherwise pend
/// forever and wedge the session. Generous on purpose: a legitimate summary is
/// ~1-3k output tokens, while a silent peer never answers at all.
const COMPACT_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Provider usage projection retained alongside a summary.
pub type SummaryUsage = crate::session::store::ProviderUsage;

/// Terminal status of the provider call that produced a summary.
///
/// PR-1 preserves this status so an incomplete/truncated summary can never be
/// committed as a checkpoint. Retry/quality policy stays in PR-5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SummaryFinish {
    Completed,
    Incomplete,
    InProgress,
    Failed,
    Cancelled,
    Queued,
    ToolCall,
    Missing,
}

/// One summarizer output, before the acceptance gates decide whether it may
/// replace the discarded transcript range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryArtifact {
    pub text: String,
    pub finish: SummaryFinish,
    pub usage: Option<SummaryUsage>,
    pub model: String,
}

impl SummaryArtifact {
    fn from_output(
        items: &[Item],
        model: String,
        usage: Option<SummaryUsage>,
        terminal_finish: Option<SummaryFinish>,
    ) -> Self {
        let has_tool_call = items
            .iter()
            .any(|item| matches!(item, Item::FunctionCall(_)));
        let mut item_finish = None;
        for item in items {
            if let Item::Message(MessageItem::Output(message)) = item {
                let status = match message.status {
                    OutputStatus::Completed => SummaryFinish::Completed,
                    OutputStatus::Incomplete => SummaryFinish::Incomplete,
                    OutputStatus::InProgress => SummaryFinish::InProgress,
                };
                item_finish = Some(match item_finish {
                    Some(current) => worse_summary_finish(current, status),
                    None => status,
                });
            }
        }

        let finish = if has_tool_call {
            SummaryFinish::ToolCall
        } else {
            match (terminal_finish, item_finish) {
                (Some(terminal), Some(item)) => worse_summary_finish(terminal, item),
                (Some(terminal), None) => terminal,
                (None, Some(item)) => item,
                (None, None) => SummaryFinish::Missing,
            }
        };
        let text = items
            .iter()
            .filter(|item| matches!(item, Item::Message(MessageItem::Output(_))))
            .map(item_text_preview)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("");
        Self {
            text,
            finish,
            usage,
            model,
        }
    }

    /// PR-1 finish gate. Empty text and any non-complete/tool-call terminal
    /// state fail before a checkpoint can be constructed.
    fn require_complete(&self) -> Result<()> {
        if self.finish != SummaryFinish::Completed || self.text.trim().is_empty() {
            tracing::warn!(
                model = %self.model,
                finish = ?self.finish,
                text_len = self.text.len(),
                "compaction summary rejected by PR-1 finish gate"
            );
            return Err(LitecodeError::CompactionFailed);
        }
        Ok(())
    }
}

/// The seq-aware plan for one compaction. The prefix/cut are item indexes into
/// the persisted working set; `kept_from_seq` and `prior_checkpoint_seq` are
/// identities, never summary text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    pub trigger: CompactionTrigger,
    pub prefix_len: usize,
    pub cut: usize,
    pub kept_from_seq: Seq,
    pub prior_checkpoint_seq: Option<Seq>,
    pub summary_cap: u32,
}

/// First-version mechanical-prune budgets. All values live here; the pruner
/// itself contains no scattered literals.
pub const TOOL_RESULT_PRUNE_THRESHOLD_CHARS: usize = 8_192;
pub const TOOL_RESULT_PRUNE_HEAD_CHARS: usize = 4_096;
pub const TOOL_RESULT_PRUNE_TAIL_CHARS: usize = 1_024;
pub const TOOL_RESULT_PRUNE_MARKER: &str = "[... tool result middle pruned ...]";
/// Floor for the minimum useful prune. The effective line is
/// `max(4_096, context_window / 50)`.
pub const PRUNE_MIN_SAVED_TOKENS_FLOOR: usize = 4_096;

/// Mechanical-prune character budgets. Characters are Unicode code points so a
/// retained boundary never splits a surrogate pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MechanicalPruneConfig {
    pub threshold_chars: usize,
    pub head_chars: usize,
    pub tail_chars: usize,
}

impl Default for MechanicalPruneConfig {
    fn default() -> Self {
        Self {
            threshold_chars: TOOL_RESULT_PRUNE_THRESHOLD_CHARS,
            head_chars: TOOL_RESULT_PRUNE_HEAD_CHARS,
            tail_chars: TOOL_RESULT_PRUNE_TAIL_CHARS,
        }
    }
}

/// The cache-busting edit gates on this amount of estimated token savings.
pub fn prune_min_saved_tokens(context_window: usize) -> usize {
    PRUNE_MIN_SAVED_TOKENS_FLOOR.max(context_window / 50)
}

/// One durable surface replacement. `seq` is the shadowed node; the appended
/// replacement event cites it through `source_seqs`, so the original event
/// remains in the append-only log and replay can recover this exact input.
#[derive(Debug, Clone, PartialEq)]
pub struct PruneReplacement {
    pub seq: Seq,
    pub source_seqs: Vec<Seq>,
    pub item: Item,
    pub chars_before: usize,
    pub chars_after: usize,
    pub estimated_saved_tokens: usize,
}

/// Deterministic, model-free first stage of compaction. Building a plan never
/// mutates the session; only a plan that clears the configured savings floor is
/// committed as one batch (one prefix rewrite / one cache bust).
///
/// `estimated_saved_tokens` is measured against the item the model would
/// actually receive. Historical media is already replaced by the ephemeral
/// show-once projection, so a media-only plan usually saves zero live tokens
/// and never clears the floor by itself; media replacements still ride along in
/// a batch whose text pruning pays for the cache bust.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MechanicalPrunePlan {
    pub replacements: Vec<PruneReplacement>,
    pub estimated_saved_tokens: usize,
}

impl MechanicalPrunePlan {
    /// Build a prune plan from the current durable working set. Pending rows do
    /// not have a seq and cannot be replaced. Only historical (stale) media is
    /// touched; fresh media from the current model/tool batch stays intact.
    pub fn build(rows: &[WorkingRow], config: MechanicalPruneConfig) -> Self {
        // The projection already decides which media is stale. Probe a clone so
        // the durable rows are never rewritten by the ephemeral view path.
        let projected = project_items(rows);
        let mut media_probe = projected.clone();
        let _ = crate::context_pipeline::media_budget::drop_stale_media(&mut media_probe);

        let mut replacements = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let Some(seq) = row.log_seq else {
                continue;
            };
            let Some(item) = row.item() else {
                continue;
            };
            let stale_media = projected.get(index) != media_probe.get(index);
            let Some(replacement_item) = prune_item_for_compaction(item, config, stale_media)
            else {
                continue;
            };
            if replacement_item == *item {
                continue;
            }
            // Savings must be measured against the item the model would
            // actually receive. Historical media is already replaced by the
            // ephemeral show-once projection, so its durable placeholder often
            // saves the live request nothing even though the raw row shrank.
            // Such media rewrites still ride along once another candidate
            // clears the savings floor; they are never the reason to bust cache.
            let effective_before = media_probe.get(index).unwrap_or(item);
            let chars_before = item_text_chars(effective_before);
            let chars_after = item_text_chars(&replacement_item);
            let saved_tokens = crate::session::estimate::compute_token_estimate(
                std::slice::from_ref(effective_before),
            )
            .saturating_sub(crate::session::estimate::compute_token_estimate(
                std::slice::from_ref(&replacement_item),
            ));
            let media_rewrite = stale_media && item_has_media(item);
            if saved_tokens == 0 && !media_rewrite {
                continue;
            }
            replacements.push(PruneReplacement {
                seq,
                source_seqs: vec![seq],
                item: replacement_item,
                chars_before,
                chars_after,
                estimated_saved_tokens: saved_tokens,
            });
        }
        let estimated_saved_tokens = replacements
            .iter()
            .map(|replacement| replacement.estimated_saved_tokens)
            .sum();
        Self {
            replacements,
            estimated_saved_tokens,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }

    /// The plan is only worth a prefix rewrite when it clears
    /// `max(4_096, W / 50)`. Equality counts as clearing the floor.
    pub fn meets_min_savings(&self, context_window: usize) -> bool {
        !self.is_empty() && self.estimated_saved_tokens >= prune_min_saved_tokens(context_window)
    }
}

/// Minimal PR-5 contract placeholder. PR-5 fills retry/quality details; PR-2
/// only carries the PR-1 finish/empty gate result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryFailure {
    Finish(SummaryFinish),
    Empty,
    ToolCall,
}

/// PR-2 acceptance boundary. PR-5 fills the quality score and failure details;
/// PR-2 only records that the PR-1 finish/empty gate accepted the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryReview {
    pub accepted: bool,
    pub score: u8,
    pub failures: Vec<SummaryFailure>,
}

/// One successful compaction at the boundary between summarize and commit.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionOutcome {
    pub checkpoint_seq: Seq,
    pub checkpoint: CompactionCheckpoint,
    pub plan: CompactionPlan,
    pub prune: Option<MechanicalPrunePlan>,
    pub summary: SummaryArtifact,
    pub review: SummaryReview,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub summary_attempts: u8,
}

fn worse_summary_finish(a: SummaryFinish, b: SummaryFinish) -> SummaryFinish {
    fn severity(finish: SummaryFinish) -> u8 {
        match finish {
            SummaryFinish::Missing => 0,
            SummaryFinish::Completed => 1,
            SummaryFinish::Queued => 2,
            SummaryFinish::InProgress => 3,
            SummaryFinish::Incomplete => 4,
            SummaryFinish::Cancelled => 5,
            SummaryFinish::Failed => 6,
            SummaryFinish::ToolCall => 7,
        }
    }
    if severity(b) >= severity(a) { b } else { a }
}

/// Result of one worthwhile, already-committed prune pass.
struct MechanicalPruneRun {
    plan: MechanicalPrunePlan,
    tokens_before: usize,
    meter_after: ContextMeter,
    relieved: bool,
}

fn is_media_part(part: &InputContent) -> bool {
    matches!(
        part,
        InputContent::InputImage(_) | InputContent::InputFile(_)
    )
}

fn media_placeholder(part: &InputContent) -> &'static str {
    match part {
        InputContent::InputImage(_) => "[image already viewed; removed from context]",
        InputContent::InputFile(_) => "[file already viewed; removed from context]",
        InputContent::InputText(_) => "",
    }
}

fn content_text_chars(parts: &[InputContent]) -> usize {
    parts
        .iter()
        .filter_map(|part| match part {
            InputContent::InputText(text) => Some(text.text.chars().count()),
            _ => None,
        })
        .sum()
}

fn item_text_chars(item: &Item) -> usize {
    match item {
        Item::Message(MessageItem::Input(message)) => content_text_chars(&message.content),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Text(text) => text.chars().count(),
            FunctionCallOutput::Content(parts) => content_text_chars(parts),
        },
        _ => 0,
    }
}

fn item_has_media(item: &Item) -> bool {
    match item {
        Item::Message(MessageItem::Input(message)) => message.content.iter().any(is_media_part),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Text(_) => false,
            FunctionCallOutput::Content(parts) => parts.iter().any(is_media_part),
        },
        _ => false,
    }
}

/// Replace each stale media part with a stable text placeholder while keeping
/// every other part in its original relative position.
fn replace_stale_media_parts(parts: &mut [InputContent]) -> bool {
    let mut changed = false;
    for part in parts.iter_mut() {
        if !is_media_part(part) {
            continue;
        }
        let placeholder = media_placeholder(part);
        *part = InputContent::InputText(InputTextContent {
            text: placeholder.to_string(),
        });
        changed = true;
    }
    changed
}

/// Deterministic head/marker/tail pruning across all text blocks in a tool
/// result. Non-text blocks keep their relative order. The emitted replacement
/// is always <= `threshold_chars`, so running the pruner again cannot grow it.
fn prune_text_content(
    parts: Vec<InputContent>,
    config: MechanicalPruneConfig,
) -> Option<Vec<InputContent>> {
    let total_chars = content_text_chars(&parts);
    if total_chars <= config.threshold_chars {
        return None;
    }

    let removed_start = config.head_chars;
    let removed_end = total_chars.saturating_sub(config.tail_chars);
    let mut consumed = 0usize;
    let mut marker_inserted = false;
    let mut out = Vec::with_capacity(parts.len());

    for part in parts {
        let InputContent::InputText(text) = part else {
            out.push(part);
            continue;
        };
        let points: Vec<char> = text.text.chars().collect();
        let block_start = consumed;
        let block_end = block_start + points.len();
        let head_end = points
            .len()
            .min(config.head_chars.saturating_sub(block_start));
        let tail_start = points.len().min(removed_end.saturating_sub(block_start));
        let intersects = block_start < removed_end && block_end > removed_start;
        let marker = if intersects && !marker_inserted {
            marker_inserted = true;
            TOOL_RESULT_PRUNE_MARKER
        } else {
            ""
        };

        let mut next = String::with_capacity(head_end + marker.len() + points.len() - tail_start);
        next.extend(points[..head_end].iter());
        next.push_str(marker);
        next.extend(points[tail_start..].iter());
        if !next.is_empty() {
            out.push(InputContent::InputText(InputTextContent { text: next }));
        }
        consumed = block_end;
    }

    // `total_chars > threshold` and a valid head/tail budget guarantee a
    // removed text span. Keep the invariant explicit instead of emitting an
    // unbounded replacement.
    if !marker_inserted || content_text_chars(&out) > config.threshold_chars {
        return None;
    }
    Some(out)
}

fn prune_text_value(text: &str, config: MechanicalPruneConfig) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= config.threshold_chars {
        return None;
    }
    let head_end = chars.len().min(config.head_chars);
    let tail_start = chars.len().saturating_sub(config.tail_chars);
    if tail_start <= head_end {
        return None;
    }
    let emitted_chars =
        head_end + TOOL_RESULT_PRUNE_MARKER.chars().count() + chars.len() - tail_start;
    if emitted_chars > config.threshold_chars {
        return None;
    }
    let mut out = String::with_capacity(emitted_chars);
    out.extend(chars[..head_end].iter());
    out.push_str(TOOL_RESULT_PRUNE_MARKER);
    out.extend(chars[tail_start..].iter());
    Some(out)
}

fn prune_tool_output(
    output: &mut FunctionCallOutput,
    config: MechanicalPruneConfig,
    strip_stale_media: bool,
) -> bool {
    match output {
        FunctionCallOutput::Text(text) => {
            let Some(next) = prune_text_value(text, config) else {
                return false;
            };
            *text = next;
            true
        }
        FunctionCallOutput::Content(parts) => {
            let mut changed = strip_stale_media && replace_stale_media_parts(parts);
            if let Some(next) = prune_text_content(parts.clone(), config) {
                *parts = next;
                changed = true;
            }
            changed
        }
    }
}

fn prune_item_for_compaction(
    item: &Item,
    config: MechanicalPruneConfig,
    strip_stale_media: bool,
) -> Option<Item> {
    match item {
        Item::Message(MessageItem::Input(message)) if strip_stale_media => {
            let mut message = message.clone();
            replace_stale_media_parts(&mut message.content)
                .then_some(Item::Message(MessageItem::Input(message)))
        }
        Item::FunctionCallOutput(output) => {
            let mut output = output.clone();
            prune_tool_output(&mut output.output, config, strip_stale_media)
                .then_some(Item::FunctionCallOutput(output))
        }
        _ => None,
    }
}

/// Measure the pruned surface with the same request projection as the main
/// request. This deliberately uses the local envelope: a provider bill from the
/// pre-prune prefix no longer describes this request, so replaying it would
/// hide the savings the prune just produced.
fn context_meter_for_rows_local(
    request: &CompactionRequestContext,
    rows: &[WorkingRow],
    data_root: &std::path::Path,
) -> ContextMeter {
    let envelope = ContextEnvelope::from_compaction_request(request);
    let (_turn_view, projection) =
        super::project_working_rows(rows, request.target.as_ref(), data_root);
    ContextMeter::measure_local(
        request.context_window,
        envelope.output_reserve,
        ContextMeter::safety_headroom(request.context_window),
        &envelope,
        &projection.items,
    )
}

/// Build, gate, and commit one mechanical prune pass. Returns `None` when the
/// plan is empty or below the savings floor; in that case no event is written.
fn apply_mechanical_prune_if_worthwhile(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    prompt_baseline: &ProviderPromptBaseline,
    rows: &mut Vec<WorkingRow>,
    data_root: &std::path::Path,
) -> Result<Option<MechanicalPruneRun>> {
    let plan = MechanicalPrunePlan::build(rows, MechanicalPruneConfig::default());
    if plan.is_empty() {
        return Ok(None);
    }
    let min_saved = prune_min_saved_tokens(request.context_window);
    if plan.estimated_saved_tokens < min_saved {
        tracing::debug!(
            estimated_saved_tokens = plan.estimated_saved_tokens,
            min_saved,
            context_window = request.context_window,
            replacements = plan.replacements.len(),
            "mechanical prune skipped: below savings floor"
        );
        return Ok(None);
    }

    let tokens_before = context_meter_for_rows_local(request, rows, data_root).used_tokens;
    let pending_tail: Vec<WorkingRow> = rows
        .iter()
        .filter(|row| row.log_seq.is_none())
        .cloned()
        .collect();
    let expected_revision = sessions.data().revision_blocking(session_id)?;
    let replacements = plan
        .replacements
        .iter()
        .map(
            |replacement| crate::session::data::command::SurfaceReplacement {
                start: replacement.seq,
                end: replacement.seq,
                item: replacement.item.clone(),
                source_seqs: replacement.source_seqs.clone(),
            },
        )
        .collect();
    let receipt = sessions.mutate_blocking(
        crate::session::data::command::SessionMutation::ReplaceSurface {
            session_id: session_id.to_string(),
            expected_revision,
            operation_id: crate::session::data::command::MutationId::new(),
            replacements,
        },
    )?;
    match receipt.outcome {
        crate::session::data::command::CommitKind::Replaced { .. } => {}
        other => {
            return Err(LitecodeError::ToolExecution(format!(
                "mechanical prune mutation returned unexpected outcome {other:?}"
            )));
        }
    }

    let mut model = sessions.data().working_set_blocking(session_id)?;
    model.extend(pending_tail);
    *rows = model;
    // The old bill describes a prefix that no longer exists. Keep the local
    // envelope authoritative until the next provider request records a fresh
    // bill over the rewritten surface.
    prompt_baseline.invalidate();

    let meter_after = context_meter_for_rows_local(request, rows, data_root);
    let relieved = meter_after.pressure_enabled() && !meter_after.should_compact();
    tracing::info!(
        replacements = plan.replacements.len(),
        estimated_saved_tokens = plan.estimated_saved_tokens,
        tokens_before,
        tokens_after = meter_after.used_tokens,
        min_saved,
        relieved,
        "mechanical prune committed as one surface replacement batch"
    );
    Ok(Some(MechanicalPruneRun {
        plan,
        tokens_before,
        meter_after,
        relieved,
    }))
}

/// Compaction policy and execution.
pub struct CompactPolicy;

impl CompactPolicy {
    pub fn can_compact(budget: &BudgetPolicy, transcript: &Transcript) -> bool {
        find_keep_recent_cut(transcript, budget.keep_recent_tokens).is_some()
    }

    /// Manual admission fallback when keep-recent has no cut but a mechanical
    /// prune would clear its cache-bust savings floor. This is deliberately only
    /// a discovery predicate: the summary path still runs prune first and may
    /// skip the LLM entirely when the pruned surface is already below pressure.
    pub fn can_prune(rows: &[WorkingRow], context_window: usize) -> bool {
        MechanicalPrunePlan::build(rows, MechanicalPruneConfig::default())
            .meets_min_savings(context_window)
    }

    /// One-shot user-triggered compaction.
    ///
    /// Product eligibility is enforced by the caller's `ContextMeter`. This
    /// deliberately skips the automatic pressure trigger and post-compact loop
    /// reminders.
    pub async fn compact_now(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        cancel: &CancellationToken,
        operation_id: Option<&str>,
        data_root: &std::path::Path,
    ) -> Result<bool> {
        let mut rows = sessions.data().working_set_blocking(session_id)?;
        let prompt_baseline = ProviderPromptBaseline::default();
        let _ = sessions.settle_stale_plan(session_id);
        let before_prune = rows.clone();
        let outcome = Self::compact_transcript(
            budget,
            sessions,
            session_id,
            request,
            max_tokens,
            &prompt_baseline,
            &mut rows,
            cancel,
            CompactionTrigger::Manual,
            operation_id,
            data_root,
        )
        .await?;
        let Some(artifact) = outcome else {
            if rows != before_prune {
                let meter = context_meter_for_rows_local(request, &rows, data_root);
                if !meter.pressure_enabled() || !meter.should_compact() {
                    tracing::info!(
                        session_id,
                        tokens_after = meter.used_tokens,
                        "mechanical prune relieved context pressure; manual summary skipped"
                    );
                    return Ok(true);
                }
                // The prune was worthwhile and durable, but the prefix still
                // has no keep-recent cut to summarize. Report that no summary
                // checkpoint was possible; the committed trim is not rolled back.
                tracing::warn!(
                    session_id,
                    tokens_after = meter.used_tokens,
                    "mechanical prune committed but pressure remains and no summary cut exists"
                );
            }
            emit_compact_lifecycle(
                sessions,
                session_id,
                CompactionTrigger::Manual,
                CompactionStage::Failed,
                operation_id,
                Some(CompactionFailKind::NothingToCompact),
                Some(LitecodeError::NothingToCompact.to_string()),
            );
            return Err(LitecodeError::NothingToCompact);
        };
        tracing::info!(
            finish = ?artifact.finish,
            model = %artifact.model,
            usage = ?artifact.usage,
            "manual compaction committed"
        );
        Ok(true)
    }

    pub async fn compact_if_needed(
        &self,
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        step: u64,
        cancel: &CancellationToken,
        data_root: &std::path::Path,
    ) -> Result<bool> {
        // Returns `true` when this compaction stage changed the durable surface:
        // a checkpoint was written, or a worthwhile mechanical prune relieved
        // pressure. This is the single source of truth the caller uses to
        // restage its working set.
        if cancel.is_cancelled() {
            return Ok(false);
        }

        let meter = context_meter_for_rows(
            sessions,
            session_id,
            request,
            prompt_baseline,
            rows,
            data_root,
        );
        meter.log_iteration(step);

        if meter.should_compact() {
            if cancel.is_cancelled() {
                return Ok(false);
            }

            tracing::info!(
                used_tokens = meter.used_tokens,
                compact_at = meter.compact_at,
                prompt_limit = meter.prompt_limit,
                source = ?meter.source,
                "context pressure reached; triggering compaction"
            );
            // Prune is the first stage of this trigger. `compact_transcript`
            // commits it before selecting a keep-recent cut, so a prune that
            // relieves pressure can skip summarization even when the whole
            // persisted prefix would otherwise fit in the keep window.
            let before_prune = rows.clone();
            let outcome = Self::compact_transcript(
                budget,
                sessions,
                session_id,
                request,
                max_tokens,
                prompt_baseline,
                rows,
                cancel,
                CompactionTrigger::Auto,
                None,
                data_root,
            )
            .await?;
            let did_compact = outcome.is_some();
            let prune_applied = rows != &before_prune;
            if let Some(artifact) = outcome {
                tracing::info!(
                    finish = ?artifact.finish,
                    model = %artifact.model,
                    usage = ?artifact.usage,
                    "automatic compaction committed"
                );
            } else if prune_applied {
                tracing::info!(
                    "mechanical prune changed the surface; no summary checkpoint was written"
                );
            }

            if cancel.is_cancelled() {
                return Ok(false);
            }
            // Defensive: if a summary was skipped (cut race or prune-only), the
            // current surface must still honor the hard prompt limit. The old
            // provider bill cannot price the rewritten surface, so use the same
            // local ContextMeter projection the prune stage used.
            if !did_compact {
                let meter = if prune_applied {
                    context_meter_for_rows_local(request, rows, data_root)
                } else {
                    context_meter_for_rows(
                        sessions,
                        session_id,
                        request,
                        prompt_baseline,
                        rows,
                        data_root,
                    )
                };
                meter.enforce_prompt_limit()?;
            }
            return Ok(did_compact || prune_applied);
        }

        meter.enforce_prompt_limit()?;
        Ok(false)
    }

    /// Returns the PR-1 artifact when a checkpoint was written. `Ok(None)`
    /// means no checkpoint: either keep-recent found nothing to discard, or a
    /// worthwhile mechanical prune relieved pressure and the summary was
    /// skipped (the prune itself is already durable). The provider
    /// finish/usage/model stays on the artifact across the compaction boundary
    /// instead of being collapsed to a boolean or plain summary text.
    pub async fn compact_transcript(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        cancel: &CancellationToken,
        trigger: CompactionTrigger,
        operation_id: Option<&str>,
        data_root: &std::path::Path,
    ) -> Result<Option<SummaryArtifact>> {
        if cancel.is_cancelled() {
            return Ok(None);
        }

        // Stage one of every compaction trigger: deterministic mechanical
        // pruning. It runs here, never in `build_view`, so ordinary turns keep
        // their cached prefix. A below-floor plan writes no surface event.
        let prune_run = apply_mechanical_prune_if_worthwhile(
            sessions,
            session_id,
            request,
            prompt_baseline,
            rows,
            data_root,
        )?;
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let prune_plan = prune_run.as_ref().map(|run| run.plan.clone());
        if prune_run.as_ref().is_some_and(|run| run.relieved) {
            let tokens_after = prune_run
                .as_ref()
                .map(|run| run.meter_after.used_tokens)
                .unwrap_or_else(|| {
                    context_meter_for_rows_local(request, rows, data_root).used_tokens
                });
            tracing::info!(
                tokens_after,
                replacements = prune_plan
                    .as_ref()
                    .map_or(0, |plan| plan.replacements.len()),
                "mechanical prune alone relieved context pressure; skipping summary"
            );
            emit_compact_lifecycle(
                sessions,
                session_id,
                trigger,
                CompactionStage::Started,
                operation_id,
                None,
                None,
            );
            emit_compact_lifecycle(
                sessions,
                session_id,
                trigger,
                CompactionStage::Succeeded,
                operation_id,
                None,
                None,
            );
            return Ok(None);
        }

        let snapshot = rows.clone();
        // Capture revision with the snapshot so the optimistic lock covers
        // snapshot→persist (ISSUE-035). Fail closed — never unwrap_or(0).
        let expected_revision = sessions.data().revision_blocking(session_id)?;
        let prefix_len = persisted_prefix_len(rows);
        let tail = rows.split_off(prefix_len);
        let transcript = project_items(rows);

        let Some(raw_cut) = find_keep_recent_cut(&transcript, budget.keep_recent_tokens) else {
            tracing::debug!(
                keep_recent_tokens = budget.keep_recent_tokens,
                prefix_len,
                "keep-recent: entire persisted prefix within keep window, skipping compact"
            );
            *rows = snapshot;
            return Ok(None);
        };

        // Checkpoint identity, not summary text, decides the leading boundary.
        // A leading checkpoint is item zero; when keep-recent can compact, the
        // discarded range must include it, so cut is at least one.
        let prior_checkpoint_seq = rows
            .first()
            .and_then(|row| row.checkpoint().and_then(|_| row.log_seq));
        let cut = if prior_checkpoint_seq.is_some() {
            raw_cut.max(1)
        } else {
            raw_cut
        };
        debug_assert!(prior_checkpoint_seq.is_none() || cut > 0);

        let kept_from_seq = match rows.get(cut).and_then(|row| row.log_seq) {
            Some(seq) => seq,
            None => {
                *rows = snapshot;
                let err = LitecodeError::ToolExecution(format!(
                    "compact cut {cut} has no log seq (persisted prefix len={prefix_len})"
                ));
                emit_compact_lifecycle(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Failed,
                    operation_id,
                    Some(CompactionFailKind::Failed),
                    Some(err.to_string()),
                );
                return Err(err);
            }
        };

        let summary_max_tokens = budget
            .compact_output_tokens(max_tokens)
            .min(request.max_output_tokens);
        let plan = CompactionPlan {
            trigger,
            prefix_len,
            cut,
            kept_from_seq,
            prior_checkpoint_seq,
            summary_cap: summary_max_tokens,
        };
        let tokens_before = prune_run
            .as_ref()
            .map(|run| run.tokens_before)
            .unwrap_or_else(|| {
                context_meter_for_rows(
                    sessions,
                    session_id,
                    request,
                    prompt_baseline,
                    &snapshot,
                    data_root,
                )
                .used_tokens
            });

        emit_compact_lifecycle(
            sessions,
            session_id,
            trigger,
            CompactionStage::Started,
            operation_id,
            None,
            None,
        );

        let artifact = match Self::first_pass_compaction(
            request,
            summary_max_tokens,
            cut,
            &snapshot,
            sessions,
            cancel,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                *rows = snapshot;
                emit_compact_failed(sessions, session_id, trigger, operation_id, &e);
                return Err(e);
            }
        };

        if cancel.is_cancelled() {
            *rows = snapshot;
            let err = LitecodeError::Canceled;
            emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
            return Err(err);
        }

        // PR-2 keeps the PR-1 finish/empty gate as the only review criterion.
        // PR-5 fills score and quality failures; this boundary is already the
        // one the commit below must pass.
        let mut review_failures = Vec::new();
        if artifact.finish != SummaryFinish::Completed {
            review_failures.push(SummaryFailure::Finish(artifact.finish));
        }
        if artifact.text.trim().is_empty() {
            review_failures.push(SummaryFailure::Empty);
        }
        let review = SummaryReview {
            accepted: review_failures.is_empty(),
            score: 0,
            failures: review_failures,
        };
        if !review.accepted {
            *rows = snapshot;
            let err = LitecodeError::CompactionFailed;
            emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
            return Err(err);
        }

        let kept = transcript[cut..].to_vec();
        let checkpoint = CompactionCheckpoint {
            summary: artifact.text.clone(),
            from: snapshot
                .first()
                .and_then(|row| row.log_seq)
                .unwrap_or(kept_from_seq),
            to: kept_from_seq,
        };
        let envelope = ContextEnvelope::from_compaction_request(request);
        let final_meter = {
            let mut view = vec![checkpoint.model_item()];
            view.extend(kept.iter().cloned());
            view.extend(tail.iter().map(|row| row.payload.model_item()));
            let view_seqs = vec![None; view.len()];
            let projection = super::project_model_view(
                &view,
                &view_seqs,
                None,
                None,
                &request.target,
                data_root,
            );
            // The rewritten view is not a suffix of the billed request; count
            // the complete local envelope over the same request projection.
            ContextMeter::measure_local(
                request.context_window,
                envelope.output_reserve,
                ContextMeter::safety_headroom(request.context_window),
                &envelope,
                &projection.items,
            )
        };
        let final_count = final_meter.used_tokens;
        if final_meter.is_over_prompt_limit() {
            *rows = snapshot;
            tracing::error!(
                final_count,
                limit = final_meter.prompt_limit,
                "token budget still exceeded after autocompact"
            );
            let err = LitecodeError::TokenBudgetExceeded;
            emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
            return Err(err);
        }

        // Persist only the raw, role-free summary. The framed user checkpoint is
        // a model projection and must never round-trip into durable storage.
        let receipt = match sessions.mutate_blocking(
            crate::session::data::command::SessionMutation::Compact {
                session_id: session_id.to_string(),
                expected_revision,
                operation_id: crate::session::data::command::MutationId::new(),
                summary: artifact.text.clone(),
                token_estimate: final_count as i64,
                kept_from: Some(kept_from_seq),
                expected_prefix: Some(prefix_len),
            },
        ) {
            Ok(receipt) => receipt,
            Err(e) => {
                *rows = snapshot;
                emit_compact_lifecycle(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Failed,
                    operation_id,
                    Some(CompactionFailKind::Failed),
                    Some(e.to_string()),
                );
                return Err(e.into());
            }
        };
        let checkpoint_seq = match receipt.outcome {
            crate::session::data::command::CommitKind::Compacted { seq } => seq,
            other => {
                *rows = snapshot;
                let err = LitecodeError::ToolExecution(format!(
                    "compact mutation returned unexpected outcome {other:?}"
                ));
                emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
                return Err(err);
            }
        };
        // Prefer the exact body just persisted. The computed body is a fallback
        // for an events read failure; the seq is already authoritative.
        let checkpoint = sessions
            .data()
            .events_blocking(session_id)
            .ok()
            .and_then(|events| events.into_iter().find(|event| event.seq == checkpoint_seq))
            .and_then(|event| {
                serde_json::from_value::<crate::session::model::CompactedBody>(event.data).ok()
            })
            .map(CompactionCheckpoint::from)
            .unwrap_or(checkpoint);
        let outcome = CompactionOutcome {
            checkpoint_seq,
            checkpoint: checkpoint.clone(),
            plan,
            prune: prune_plan,
            summary: artifact,
            review,
            tokens_before,
            tokens_after: final_count,
            summary_attempts: 1,
        };

        // One reload. The pending tail was split off before the summary call.
        // ISSUE-035 / WP-039: after Compact has committed, DB is sole truth.
        // Reload failure must not roll memory back to the pre-compact snapshot
        // or emit Failed (that would pretend compaction never happened).
        // Keep `working_set_blocking` in this fn body (death_list g2 item-level gate).
        let reload_result = {
            #[cfg(test)]
            {
                if INJECT_RELOAD_ERR_AFTER_COMPACT.with(|flag| flag.replace(false)) {
                    Err(LitecodeError::ToolExecution(
                        "injected working_set reload failure after Compact persist".into(),
                    ))
                } else {
                    sessions.data().working_set_blocking(session_id)
                }
            }
            #[cfg(not(test))]
            {
                sessions.data().working_set_blocking(session_id)
            }
        };
        let mut model = match reload_result {
            Ok(loaded) => loaded,
            Err(e) => {
                tracing::warn!(
                    session_id,
                    error = %e,
                    "compact persisted but working_set reload failed; installing emergency cache"
                );
                let mut emergency = vec![WorkingRow::persisted_checkpoint(
                    checkpoint_seq,
                    outcome.checkpoint.clone(),
                )];
                emergency.extend(snapshot[cut..].iter().cloned());
                *rows = emergency;
                prompt_baseline.clear();
                emit_compact_lifecycle(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Succeeded,
                    operation_id,
                    None,
                    None,
                );
                return Ok(Some(outcome.summary));
            }
        };
        model.extend(tail);
        *rows = model;

        tracing::info!(
            checkpoint_seq = outcome.checkpoint_seq,
            checkpoint_from = outcome.checkpoint.from,
            checkpoint_to = outcome.checkpoint.to,
            prior_checkpoint_seq = ?outcome.plan.prior_checkpoint_seq,
            tokens_before = outcome.tokens_before,
            tokens_after = outcome.tokens_after,
            summary_attempts = outcome.summary_attempts,
            review_score = outcome.review.score,
            "compaction checkpoint committed"
        );
        let summary = outcome.summary;
        prompt_baseline.clear();
        emit_compact_lifecycle(
            sessions,
            session_id,
            trigger,
            CompactionStage::Succeeded,
            operation_id,
            None,
            None,
        );
        Ok(Some(summary))
    }
    async fn first_pass_compaction(
        request: &CompactionRequestContext,
        max_tokens: u32,
        cut: usize,
        rows: &[WorkingRow],
        sessions: &SessionManager,
        cancel: &CancellationToken,
    ) -> Result<SummaryArtifact> {
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        // The request input is the discarded prefix in original Item order,
        // projected exactly as the main view would project it. `kept` stays in
        // the working set and never enters this request.
        let prefix = build_compaction_input(request, sessions, rows, cut)?;
        let artifact = Self::call_llm_compact(request, prefix, max_tokens, cancel).await?;

        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }
        tracing::info!(
            summary_len = artifact.text.len(),
            finish = ?artifact.finish,
            model = %artifact.model,
            usage = ?artifact.usage,
            "keep-recent compaction succeeded"
        );
        Ok(artifact)
    }

    pub(crate) async fn call_llm_compact(
        request: &CompactionRequestContext,
        input: Vec<Item>,
        max_tokens: u32,
        cancel: &CancellationToken,
    ) -> Result<SummaryArtifact> {
        Self::call_llm_compact_with_timeout(
            request,
            input,
            max_tokens,
            cancel,
            COMPACT_REQUEST_TIMEOUT,
        )
        .await
    }

    /// Cancellable, wall-clock-capped compact call via the unified stream path.
    ///
    /// Cancellation is observed while awaiting the provider (and passed into it),
    /// and a silent peer fails the request instead of pending forever.
    async fn call_llm_compact_with_timeout(
        request: &CompactionRequestContext,
        input: Vec<Item>,
        max_tokens: u32,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<SummaryArtifact> {
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        let session_id = request.session_id.as_str();
        let model = request.model.as_str();
        let provider_request = request.build_model_request(input, max_tokens);
        let usage_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<SummaryUsage>));
        let terminal_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<SummaryFinish>));
        let usage_cb = std::sync::Arc::clone(&usage_slot);
        let terminal_cb = std::sync::Arc::clone(&terminal_slot);
        let on_event: Option<Box<dyn FnMut(crate::types::StreamEvents) + Send + '_>> =
            Some(Box::new(move |event| {
                let response = match &event {
                    crate::types::StreamEvents::ResponseCompleted(completed) => {
                        Some((SummaryFinish::Completed, &completed.response))
                    }
                    crate::types::StreamEvents::ResponseIncomplete(incomplete) => {
                        Some((SummaryFinish::Incomplete, &incomplete.response))
                    }
                    crate::types::StreamEvents::ResponseFailed(failed) => {
                        Some((SummaryFinish::Failed, &failed.response))
                    }
                    _ => None,
                };
                let Some((event_finish, response)) = response else {
                    return;
                };
                if let Ok(mut slot) = terminal_cb.lock() {
                    *slot = Some(worse_summary_finish(
                        event_finish,
                        finish_from_response_status(&response.status),
                    ));
                }
                if let Some(usage) = provider_usage_from_response(response) {
                    if let Ok(mut slot) = usage_cb.lock() {
                        *slot = Some(usage);
                    }
                }
            }));

        let started = Instant::now();
        let items = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!(
                    session_id,
                    model,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "compaction request canceled while waiting for the provider"
                );
                return Err(LitecodeError::Canceled);
            }
            result = tokio::time::timeout(
                timeout,
                crate::llm::reconnect::scope_compaction(
                    request
                        .provider
                        .as_ref()
                        .complete_with_stream_events(
                            &provider_request,
                            &request.api_key,
                            on_event,
                            cancel,
                        ),
                ),
            ) => {
                match result {
                    Ok(Ok(items)) => items,
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {
                        tracing::error!(
                            session_id,
                            model,
                            timeout_secs = timeout.as_secs(),
                            "compaction request timed out with no provider response"
                        );
                        return Err(LitecodeError::Llm(format!(
                            "compaction request timed out after {}s with no response (provider silent)",
                            timeout.as_secs()
                        )));
                    }
                }
            }
        };
        let usage = usage_slot.lock().ok().and_then(|mut slot| slot.take());
        let terminal_finish = terminal_slot.lock().ok().and_then(|mut slot| slot.take());
        let artifact =
            SummaryArtifact::from_output(&items, request.model.clone(), usage, terminal_finish);
        artifact.require_complete()?;
        Ok(artifact)
    }
}

/// Build the compaction input from the same projected view as the main
/// request. The selected items are the prefix whose source row index is in the
/// discarded range; synthetic pads inherit the unanswered call's index.
fn build_compaction_input(
    request: &CompactionRequestContext,
    sessions: &SessionManager,
    rows: &[WorkingRow],
    cut: usize,
) -> Result<Vec<Item>> {
    let (turn_view, source_seqs, inherited_before_checkpoint, replacement_media) =
        if let Some(view) = CompactedView::from_working_rows(rows) {
            (
                view.items,
                view.item_seqs,
                Some(view.inherited_before_checkpoint),
                view.replacement_media,
            )
        } else {
            (
                project_items(rows),
                rows.iter().map(|row| row.log_seq).collect(),
                None,
                rows.iter().map(|row| row.replacement_media).collect(),
            )
        };
    let projection = project_model_view(
        &turn_view,
        &source_seqs,
        inherited_before_checkpoint.as_deref(),
        Some(&replacement_media),
        request.target.as_ref(),
        &sessions.data_root_path(),
    );

    let mut input = Vec::new();
    let mut input_seqs = Vec::new();
    let mut input_rebased = Vec::new();
    for (index, item) in projection.items.into_iter().enumerate() {
        if projection.source_indices[index].is_some_and(|source| source < cut) {
            input.push(item);
            input_seqs.push(projection.item_seqs[index]);
            input_rebased.push(projection.rebased[index]);
        }
    }

    let producers = crate::runtime::llm_resolve::producers_from_origins(
        sessions
            .request_origins(&request.session_id)
            .unwrap_or_default(),
        &input_seqs,
    );
    let report = crate::llm::port::port(
        &mut input,
        &producers,
        &input_rebased,
        request.target.as_ref(),
        request.thinking,
        !request.tools.is_empty(),
    );
    if let Some(block) = report.blocked {
        return Err(LitecodeError::Config(format!(
            "model '{}' cannot serve compaction: {}",
            request.target.reference,
            block.message()
        )));
    }
    Ok(input)
}

fn finish_from_response_status(status: &crate::authority::responses::Status) -> SummaryFinish {
    use crate::authority::responses::Status;
    match status {
        Status::Completed => SummaryFinish::Completed,
        Status::Incomplete => SummaryFinish::Incomplete,
        Status::InProgress => SummaryFinish::InProgress,
        Status::Failed => SummaryFinish::Failed,
        Status::Cancelled => SummaryFinish::Cancelled,
        Status::Queued => SummaryFinish::Queued,
    }
}

fn provider_usage_from_response(
    response: &crate::authority::responses::Response,
) -> Option<SummaryUsage> {
    let usage = response.usage.as_ref()?;
    Some(SummaryUsage {
        prompt_tokens: usage.input_tokens as u64,
        completion_tokens: usage.output_tokens as u64,
        cache_hit_tokens: usage.input_tokens_details.cached_tokens as u64,
        cache_miss_tokens: usage
            .input_tokens
            .saturating_sub(usage.input_tokens_details.cached_tokens)
            as u64,
    })
}
// Test-only: next post-persist `working_set_blocking` in compact returns Err once.
#[cfg(test)]
thread_local! {
    static INJECT_RELOAD_ERR_AFTER_COMPACT: Cell<bool> = const { Cell::new(false) };
}

/// Arm a one-shot reload failure after a successful Compact persist (WP-039 / ISSUE-035).
#[cfg(test)]
pub fn inject_reload_err_after_compact_once() {
    INJECT_RELOAD_ERR_AFTER_COMPACT.with(|flag| flag.set(true));
}

fn compact_fail_kind(err: &LitecodeError) -> CompactionFailKind {
    match err {
        LitecodeError::NothingToCompact => CompactionFailKind::NothingToCompact,
        LitecodeError::Canceled => CompactionFailKind::Canceled,
        _ => CompactionFailKind::Failed,
    }
}

fn emit_compact_lifecycle(
    sessions: &SessionManager,
    session_id: &str,
    trigger: CompactionTrigger,
    stage: CompactionStage,
    operation_id: Option<&str>,
    fail_kind: Option<CompactionFailKind>,
    error: Option<String>,
) {
    sessions.publish_internal(
        session_id,
        InternalEvent::CompactionLifecycle {
            trigger,
            stage,
            operation_id: operation_id.map(str::to_string),
            fail_kind,
            error,
        },
    );
}

fn emit_compact_failed(
    sessions: &SessionManager,
    session_id: &str,
    trigger: CompactionTrigger,
    operation_id: Option<&str>,
    err: &LitecodeError,
) {
    let fail_kind = compact_fail_kind(err);
    tracing::warn!(
        session_id,
        trigger = ?trigger,
        fail_kind = ?fail_kind,
        operation_id = operation_id.unwrap_or(""),
        error = %err,
        "compaction failed"
    );
    emit_compact_lifecycle(
        sessions,
        session_id,
        trigger,
        CompactionStage::Failed,
        operation_id,
        Some(fail_kind),
        Some(err.to_string()),
    );
}

/// Items as the model view will carry them, so the budget never counts media
/// that `drop_stale_media` removes from the request.
fn budget_items(rows: &[WorkingRow]) -> Vec<Item> {
    working_set_items(rows).0
}

/// The one auto/manual/UI measurement path for current working rows. Auto
/// supplies its in-memory baseline; manual/UI use the persisted equivalent.
fn context_meter_for_rows(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    baseline: &ProviderPromptBaseline,
    rows: &[WorkingRow],
    data_root: &std::path::Path,
) -> ContextMeter {
    let envelope = ContextEnvelope::from_compaction_request(request);
    crate::context_pipeline::measure_request_context(
        sessions,
        session_id,
        request.context_window,
        &envelope,
        baseline,
        rows,
        &request.target,
        data_root,
    )
}

/// Leading rows that already have a `log_seq`. Pending rows sit after them.
fn persisted_prefix_len(rows: &[WorkingRow]) -> usize {
    rows.iter()
        .position(|row| row.log_seq.is_none())
        .unwrap_or(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::TurnGuard;
    use crate::llm::{LlmProvider, ModelRequest, RequestPurpose, ToolDef};
    use crate::platform_knobs::ThinkingSpec;
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::{StreamEvents, assistant_text, user_text};
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    /// Silent provider whose stream call never resolves — the failure mode that
    /// used to pend the turn forever.
    struct PendingProvider;

    impl LlmProvider for PendingProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(PendingProvider)
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            Box::pin(std::future::pending::<Result<Vec<Item>>>())
        }
    }

    struct TextProvider;

    impl LlmProvider for TextProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(TextProvider)
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            Box::pin(
                async move { Ok::<_, LitecodeError>(vec![assistant_text("## summary\nkept")]) },
            )
        }
    }

    /// Counts unified-stream entry calls — proves compact LLM uses
    /// `complete_with_stream_events` (ISSUE-013), not a bypass path.
    struct CountingProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl CountingProvider {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl LlmProvider for CountingProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(CountingProvider::new())
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(
                async move { Ok::<_, LitecodeError>(vec![assistant_text("## summary\nkept")]) },
            )
        }
    }

    fn compact_context(provider: Arc<dyn LlmProvider>) -> CompactionRequestContext {
        let catalog = ProviderCatalog::parse(
            r#"version = 1

[[providers]]
id = "p"
name = "Provider"
endpoint = "https://compact.invalid/v1"
endpoint_type = "responses"

[[models]]
id = "compact-model"
provider_id = "p"
context_window = 128000
context_window_max = 128000
max_output = 8192
"#,
            Path::new("<compact-unit-test>"),
        )
        .expect("compact test catalog");
        let target = catalog
            .model("p/compact-model")
            .expect("compact test model")
            .clone();
        CompactionRequestContext {
            provider,
            api_key: "sk-test".into(),
            model: target.id.clone(),
            context_window: target.context_window,
            target,
            thinking: ThinkingSpec::Tier(Default::default()),
            max_output_tokens: 8192,
            instructions: "system".into(),
            tools: vec![],
            session_id: "s1".into(),
        }
    }

    #[tokio::test]
    async fn compact_request_times_out_on_silent_provider() {
        let cancel = CancellationToken::new();
        let request = compact_context(Arc::new(PendingProvider));
        let err = CompactPolicy::call_llm_compact_with_timeout(
            &request,
            vec![user_text("prefix")],
            128,
            &cancel,
            Duration::from_millis(50),
        )
        .await
        .expect_err("a silent provider must fail, not pend forever");
        assert!(
            matches!(err, LitecodeError::Llm(_)),
            "expected timeout error, got {err}"
        );
        assert!(err.to_string().contains("timed out"), "got {err}");
    }

    #[tokio::test]
    async fn compact_request_observes_cancel_while_waiting() {
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            trigger.cancel();
        });
        let request = compact_context(Arc::new(PendingProvider));
        let err = CompactPolicy::call_llm_compact_with_timeout(
            &request,
            vec![user_text("prefix")],
            128,
            &cancel,
            Duration::from_secs(30),
        )
        .await
        .expect_err("cancel must abort the pending request");
        assert!(matches!(err, LitecodeError::Canceled), "got {err}");
    }

    #[test]
    fn summary_artifact_preserves_incomplete_provider_finish() {
        let mut truncated = assistant_text("partial summary");
        let Item::Message(MessageItem::Output(message)) = &mut truncated else {
            panic!("assistant_text must be an output message");
        };
        message.status = OutputStatus::Incomplete;
        let artifact = SummaryArtifact::from_output(&[truncated], "m".into(), None, None);
        assert_eq!(artifact.finish, SummaryFinish::Incomplete);
        assert!(matches!(
            artifact.require_complete(),
            Err(LitecodeError::CompactionFailed)
        ));

        let terminal = SummaryArtifact::from_output(
            &[assistant_text("looks complete")],
            "m".into(),
            None,
            Some(SummaryFinish::Incomplete),
        );
        assert_eq!(terminal.finish, SummaryFinish::Incomplete);
        assert!(matches!(
            terminal.require_complete(),
            Err(LitecodeError::CompactionFailed)
        ));
    }

    #[test]
    fn compaction_summary_rejects_any_tool_call() {
        use crate::authority::responses::FunctionToolCall;

        let items = vec![
            assistant_text("looks like a summary"),
            Item::FunctionCall(FunctionToolCall {
                call_id: "call-1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                id: Some("fc-1".into()),
                status: None,
                namespace: None,
            }),
        ];
        let artifact = SummaryArtifact::from_output(&items, "m".into(), None, None);
        assert_eq!(artifact.finish, SummaryFinish::ToolCall);
        assert!(matches!(
            artifact.require_complete(),
            Err(LitecodeError::CompactionFailed)
        ));
    }

    #[test]
    fn compaction_is_the_main_request_with_an_output_side_cap_and_instruction() {
        let tools = vec![ToolDef {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let mut request = compact_context(Arc::new(TextProvider));
        request.tools = tools.clone();
        request.instructions = "main system / instructions".into();
        let prefix = vec![
            user_text("kept in original order"),
            assistant_text("answer"),
        ];

        let req = request.build_model_request(prefix.clone(), 20_480);
        assert_eq!(req.model, request.model);
        assert_eq!(req.instructions, request.instructions);
        assert_eq!(req.tools.len(), tools.len());
        assert_eq!(req.tools[0].name, tools[0].name);
        assert_eq!(req.tools[0].description, tools[0].description);
        assert_eq!(req.tools[0].input_schema, tools[0].input_schema);
        assert_eq!(&req.input[..prefix.len()], prefix.as_slice());
        assert_eq!(req.input.len(), prefix.len() + 1);
        assert_eq!(
            item_text_preview(req.input.last().expect("instruction")),
            crate::llm::COMPACTION_INSTRUCTION
        );
        assert_eq!(req.max_output_tokens, 20_480);
        assert_eq!(req.thinking, request.thinking);
        assert_eq!(req.purpose, RequestPurpose::Compaction);
        assert_eq!(req.session_id.as_deref(), Some("s1"));
    }

    /// WP-039 / ISSUE-035: Compact persist succeeds, reload fails → Ok(true),
    /// Succeeded (not Failed), DB has summary, memory ≠ pre-compact snapshot.
    #[tokio::test]
    async fn compact_persist_ok_reload_err_keeps_db_truth() {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session_sync("/p", "default", Some("test/test-primary-model"))
            .expect("open session");
        let seed: Vec<Item> = (0..12)
            .map(|i| user_text(format!("reload-err-seed-{i}")))
            .collect();
        sessions.insert_detail_rows(&sid, &seed).expect("seed");

        let mut rx = sessions.subscribe(&sid).expect("subscribe lifecycle");

        let mut rows = sessions
            .data()
            .working_set_blocking(&sid)
            .expect("pre-compact working set");
        let before_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.payload.model_item()))
            .collect();
        rows.push(WorkingRow::pending(user_text("pending-tail-after-prefix")));

        inject_reload_err_after_compact_once();

        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let baseline = ProviderPromptBaseline::default();
        let request = compact_context(Arc::new(TextProvider));
        let outcome = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            1024,
            &baseline,
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Manual,
            Some("wp039-reload-err"),
            &sessions.data_root_path(),
        )
        .await
        .expect("persist ok + reload err must return Ok");
        assert!(outcome.is_some(), "compaction committed");

        let mem_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.payload.model_item()))
            .collect();
        assert!(
            mem_previews.first().is_some_and(|t| {
                t.contains("## summary") && !t.starts_with("[Conversation summary]")
            }),
            "emergency cache must lead with summary, got {mem_previews:?}"
        );
        assert!(
            mem_previews
                .iter()
                .any(|t| t.contains("pending-tail-after-prefix")),
            "emergency cache must keep pending tail, got {mem_previews:?}"
        );
        assert_ne!(
            &mem_previews[..before_previews.len().min(mem_previews.len())],
            before_previews.as_slice(),
            "memory must not equal pre-compact snapshot content"
        );
        let checkpoint_row = rows.first().expect("emergency checkpoint");
        assert!(checkpoint_row.checkpoint().is_some());
        assert!(
            checkpoint_row.log_seq.is_some(),
            "emergency checkpoint must retain the committed seq"
        );
        assert!(
            rows.last().is_some_and(|row| row.log_seq.is_none()),
            "the unpersisted tail remains pending"
        );

        let db_rows = sessions
            .data()
            .working_set_blocking(&sid)
            .expect("DB working set");
        let db0 = item_text_preview(&db_rows[0].payload.model_item());
        assert!(
            db0.contains("## summary") && !db0.starts_with("[Conversation summary]"),
            "DB must show compacted summary, got {db0}"
        );
        assert_eq!(
            checkpoint_row.log_seq, db_rows[0].log_seq,
            "emergency checkpoint seq must match DB truth"
        );
        assert!(matches!(&db_rows[0].payload, WorkingPayload::Checkpoint(_)));
        assert!(
            db_rows.iter().all(|row| row.log_seq.is_some()),
            "DB fold rows must be persisted"
        );

        let mut saw_succeeded = false;
        let mut saw_failed = false;
        while let Ok(env) = rx.try_recv() {
            if let InternalEvent::CompactionLifecycle {
                trigger: CompactionTrigger::Manual,
                stage,
                operation_id,
                ..
            } = env.event
            {
                assert_eq!(operation_id.as_deref(), Some("wp039-reload-err"));
                match stage {
                    CompactionStage::Succeeded => saw_succeeded = true,
                    CompactionStage::Failed => saw_failed = true,
                    _ => {}
                }
            }
        }
        assert!(saw_succeeded, "must emit Succeeded after Compact persist");
        assert!(!saw_failed, "must not emit Failed after Compact persist");
    }

    fn long_seed() -> Vec<Item> {
        (0..30)
            .map(|i| user_text(format!("seed-{i}-{}", "word ".repeat(400))))
            .collect()
    }

    fn open_seeded() -> (Arc<SessionManager>, String, u64) {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session_sync("/p", "default", Some("test/test-primary-model"))
            .expect("open session");
        sessions
            .insert_detail_rows(&sid, &long_seed())
            .expect("seed");
        let header = sessions
            .append_request_origin(
                &sid,
                &serde_json::json!({"schema": 1, "turn": "t", "step": 1}),
            )
            .expect("header");
        (sessions, sid, header)
    }

    fn bill(prompt: u64) -> crate::session::store::ProviderUsage {
        crate::session::store::ProviderUsage {
            prompt_tokens: prompt,
            completion_tokens: 1,
            cache_hit_tokens: 0,
            cache_miss_tokens: prompt,
        }
    }

    #[tokio::test]
    async fn low_provider_bill_does_not_compact_a_hot_local_estimate() {
        let (sessions, sid, header) = open_seeded();
        sessions
            .record_provider_usage(&sid, header, &bill(1_000))
            .unwrap();
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let budget = BudgetPolicy::new(10_000);
        let baseline = ProviderPromptBaseline::default();
        let local = crate::context_pipeline::estimate::compute_token_estimate(&budget_items(&rows));
        assert!(
            local > budget.autocompact_threshold(),
            "local estimate {local} must clear the soft window line"
        );
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        request.context_window = budget.budget_limit();
        request.max_output_tokens = 1_024;
        let meter = context_meter_for_rows(
            &sessions,
            &sid,
            &request,
            &baseline,
            &rows,
            &sessions.data_root_path(),
        );
        assert_eq!(meter.used_tokens, 1_000);
        assert_eq!(
            meter.source,
            crate::context_pipeline::ContextMeterSource::Provider
        );
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                &request,
                1024,
                &baseline,
                &mut rows,
                1,
                &CancellationToken::new(),
                &sessions.data_root_path(),
            )
            .await
            .unwrap();
        assert!(!did);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn provider_bill_over_threshold_compacts_then_the_bill_stays_retired() {
        let (sessions, sid, header) = open_seeded();
        sessions
            .record_provider_usage(&sid, header, &bill(9_000))
            .unwrap();
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let budget = BudgetPolicy::new(10_000);
        let baseline = ProviderPromptBaseline::default();
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        request.context_window = budget.budget_limit();
        request.max_output_tokens = 1_024;
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                &request,
                1024,
                &baseline,
                &mut rows,
                1,
                &CancellationToken::new(),
                &sessions.data_root_path(),
            )
            .await
            .unwrap();
        assert!(did, "a provider prompt over 80% must compact");
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let stale = ProviderPromptBaseline::default();
        stale.record(9_000, header);
        let rows = sessions.data().working_set_blocking(&sid).unwrap();
        let meter = context_meter_for_rows(
            &sessions,
            &sid,
            &request,
            &stale,
            &rows,
            &sessions.data_root_path(),
        );
        let envelope = ContextEnvelope::from_compaction_request(&request);
        let local = envelope.fixed_prompt_tokens().saturating_add(
            crate::context_pipeline::estimate::compute_token_estimate(&budget_items(&rows)),
        );
        assert_eq!(
            meter.used_tokens, local,
            "a compact must retire the old provider prompt and count the full local envelope"
        );
        assert_eq!(
            meter.source,
            crate::context_pipeline::ContextMeterSource::LocalEnvelope
        );
        assert!(meter.used_tokens <= meter.prompt_limit, "count {local}");
        assert!(!meter.should_compact(), "count {local}");
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        request.context_window = budget.budget_limit();
        request.max_output_tokens = 1_024;
        let mut rows = rows;
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                &request,
                1024,
                &stale,
                &mut rows,
                2,
                &CancellationToken::new(),
                &sessions.data_root_path(),
            )
            .await
            .unwrap();
        assert!(!did);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    use crate::authority::responses::{
        FunctionCallOutputItemParam, FunctionToolCall, InputImageContent,
    };

    fn prune_call(call_id: &str) -> Item {
        Item::FunctionCall(FunctionToolCall {
            call_id: call_id.into(),
            name: "read".into(),
            arguments: "{}".into(),
            id: Some(format!("call-item-{call_id}")),
            status: None,
            namespace: None,
        })
    }

    fn prune_tool_text(call_id: &str, text: String) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Text(text),
            id: Some(format!("output-{call_id}")),
            status: Some(OutputStatus::Completed),
        })
    }

    fn prune_tool_image(call_id: &str, url: &str) -> Item {
        Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: call_id.into(),
            output: FunctionCallOutput::Content(vec![
                InputContent::InputText(InputTextContent {
                    text: "caption".into(),
                }),
                InputContent::InputImage(InputImageContent {
                    detail: Default::default(),
                    file_id: None,
                    image_url: Some(url.into()),
                }),
            ]),
            id: Some(format!("output-{call_id}")),
            status: Some(OutputStatus::Completed),
        })
    }

    fn prune_media_count(item: &Item) -> usize {
        match item {
            Item::FunctionCallOutput(output) => match &output.output {
                FunctionCallOutput::Content(parts) => {
                    parts.iter().filter(|part| is_media_part(part)).count()
                }
                FunctionCallOutput::Text(_) => 0,
            },
            Item::Message(MessageItem::Input(message)) => message
                .content
                .iter()
                .filter(|part| is_media_part(part))
                .count(),
            _ => 0,
        }
    }

    fn replace_output_in_rows(
        rows: &[WorkingRow],
        replacement: &PruneReplacement,
    ) -> Vec<WorkingRow> {
        let mut rows = rows.to_vec();
        let index = rows
            .iter()
            .position(|row| row.log_seq == Some(replacement.seq))
            .expect("replacement source row");
        rows[index] = WorkingRow::persisted(replacement.seq + 10_000, replacement.item.clone());
        rows
    }

    #[test]
    fn tool_result_prune_uses_exact_head_marker_tail_boundaries() {
        let config = MechanicalPruneConfig::default();
        let at_limit = prune_tool_text(
            "call-at-limit",
            "x".repeat(TOOL_RESULT_PRUNE_THRESHOLD_CHARS),
        );
        let mut unchanged = at_limit.clone();
        let Item::FunctionCallOutput(output) = &mut unchanged else {
            panic!("expected tool result");
        };
        assert!(
            !prune_tool_output(&mut output.output, config, false),
            "exactly threshold chars must not be rewritten"
        );
        assert_eq!(unchanged, at_limit);

        let over = prune_tool_text(
            "call-over",
            "x".repeat(TOOL_RESULT_PRUNE_THRESHOLD_CHARS + 1),
        );
        let replacement = prune_item_for_compaction(&over, config, false).expect("over threshold");
        let Item::FunctionCallOutput(output) = &replacement else {
            panic!("expected tool result");
        };
        let FunctionCallOutput::Text(text) = &output.output else {
            panic!("text tool result must stay a text result");
        };
        assert_eq!(
            text.chars().count(),
            TOOL_RESULT_PRUNE_HEAD_CHARS
                + TOOL_RESULT_PRUNE_MARKER.chars().count()
                + TOOL_RESULT_PRUNE_TAIL_CHARS
        );
        assert!(text.starts_with(&"x".repeat(TOOL_RESULT_PRUNE_HEAD_CHARS)));
        assert!(text.ends_with(&"x".repeat(TOOL_RESULT_PRUNE_TAIL_CHARS)));
        assert_eq!(text.matches(TOOL_RESULT_PRUNE_MARKER).count(), 1);
    }

    #[test]
    fn tool_result_prune_counts_unicode_code_points_without_splitting() {
        let config = MechanicalPruneConfig::default();
        let over = prune_tool_text(
            "call-unicode",
            "é".repeat(TOOL_RESULT_PRUNE_THRESHOLD_CHARS + 1),
        );
        let replacement = prune_item_for_compaction(&over, config, false).expect("over threshold");
        let Item::FunctionCallOutput(output) = &replacement else {
            panic!("expected tool result");
        };
        let FunctionCallOutput::Text(text) = &output.output else {
            panic!("text tool result must stay text");
        };
        assert_eq!(
            text.chars().count(),
            TOOL_RESULT_PRUNE_HEAD_CHARS
                + TOOL_RESULT_PRUNE_MARKER.chars().count()
                + TOOL_RESULT_PRUNE_TAIL_CHARS
        );
        assert!(!text.contains('�'));
        assert_eq!(text.matches(TOOL_RESULT_PRUNE_MARKER).count(), 1);
    }

    #[test]
    fn tool_result_prune_preserves_pair_and_provider_metadata_and_is_idempotent() {
        let rows = vec![
            WorkingRow::persisted(1, user_text("go")),
            WorkingRow::persisted(2, prune_call("call-1")),
            WorkingRow::persisted(3, prune_tool_text("call-1", "word ".repeat(2_400))),
            WorkingRow::persisted(4, assistant_text("done")),
        ];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default());
        assert_eq!(
            MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default()),
            plan,
            "the same input must produce the same plan"
        );
        assert_eq!(plan.replacements.len(), 1, "only the result is pruned");
        let replacement = &plan.replacements[0];
        assert_eq!(replacement.seq, 3);
        assert_eq!(replacement.source_seqs, vec![3]);
        let Item::FunctionCallOutput(output) = &replacement.item else {
            panic!("expected replacement tool result");
        };
        assert_eq!(output.call_id, "call-1", "pair identity must survive");
        assert_eq!(output.id.as_deref(), Some("output-call-1"));
        assert_eq!(output.status, Some(OutputStatus::Completed));

        let applied = replace_output_in_rows(&rows, replacement);
        let second = MechanicalPrunePlan::build(&applied, MechanicalPruneConfig::default());
        assert!(
            second.is_empty(),
            "replaying the pruned surface must not create another replacement"
        );
        assert_eq!(
            item_text_chars(&applied[2].item().unwrap()),
            replacement.chars_after,
            "second pass must not grow the replacement"
        );
    }

    #[test]
    fn stale_media_gets_a_placeholder_while_fresh_media_is_kept() {
        let old_user = crate::types::user_message("look", &["litecode-media:old.png".into()]);
        let rows = vec![
            WorkingRow::persisted(1, old_user),
            WorkingRow::persisted(2, prune_call("call-old")),
            WorkingRow::persisted(
                3,
                prune_tool_image("call-old", "https://example.com/old.png"),
            ),
            WorkingRow::persisted(4, assistant_text("done")),
            WorkingRow::persisted(5, user_text("next")),
            WorkingRow::persisted(6, prune_call("call-new")),
            WorkingRow::persisted(
                7,
                prune_tool_image("call-new", "https://example.com/new.png"),
            ),
        ];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default());
        assert_eq!(
            plan.replacements.len(),
            2,
            "one stale user media, one stale tool media"
        );
        for replacement in &plan.replacements {
            assert_eq!(
                prune_media_count(&replacement.item),
                0,
                "durable replacement must carry only text"
            );
            assert!(
                item_text_chars(&replacement.item) > 0,
                "media replacement must keep a stable text placeholder"
            );
        }
        let Item::Message(MessageItem::Input(message)) = &plan.replacements[0].item else {
            panic!("stale user media replacement must stay a user message");
        };
        let text = message
            .content
            .iter()
            .filter_map(|part| match part {
                InputContent::InputText(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text, "look\n[image already viewed; removed from context]");
        assert_eq!(
            prune_media_count(&rows[6].item().unwrap()),
            1,
            "fresh tool media stays in the row until it becomes historical"
        );

        assert_eq!(prune_media_count(&plan.replacements[0].item), 0);

        let mut applied = rows.clone();
        for replacement in &plan.replacements {
            let index = applied
                .iter()
                .position(|row| row.log_seq == Some(replacement.seq))
                .expect("media source row");
            applied[index] =
                WorkingRow::persisted(replacement.seq + 10_000, replacement.item.clone());
        }
        assert!(
            MechanicalPrunePlan::build(&applied, MechanicalPruneConfig::default()).is_empty(),
            "durable media placeholders must be stable on a second pass"
        );
    }

    #[test]
    fn mechanical_prune_gate_uses_max_of_floor_and_window_ratio() {
        assert_eq!(prune_min_saved_tokens(128_000), 4_096);
        assert_eq!(prune_min_saved_tokens(400_000), 8_000);
        let rows = vec![
            WorkingRow::persisted(1, prune_call("call-small")),
            WorkingRow::persisted(2, prune_tool_text("call-small", "x".repeat(8_300))),
        ];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default());
        assert_eq!(plan.replacements.len(), 1);
        assert!(
            plan.estimated_saved_tokens < 4_096,
            "a single small text result is below the cache-bust floor: {plan:?}"
        );
        assert!(!plan.meets_min_savings(128_000));
    }
}
