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
    CompactionFailKind, CompactionStage, CompactionTelemetry, CompactionTrigger, InternalEvent,
};
use crate::session::event::Seq;
use crate::session::manager::SessionManager;
#[cfg(test)]
use crate::session::working::WorkingPayload;
use crate::session::working::{CompactedView, CompactionCheckpoint, WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result, Transcript, item_text_preview, user_text};

use super::budget::{BudgetPolicy, ProviderPromptBaseline};
use super::context_meter::{ContextEnvelope, ContextMeter};

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
/// committed as a checkpoint. PR-7 keeps the acceptance gate minimal.
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
    pub has_image: bool,
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
        let has_image = items.iter().any(item_has_image);
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
            match terminal_finish {
                // The terminal provider event is the authority. Without it,
                // an item's default completed status must not be promoted to
                // a successful finish.
                Some(terminal) => match item_finish {
                    Some(item) => worse_summary_finish(terminal, item),
                    None => terminal,
                },
                None => SummaryFinish::Missing,
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
            has_image,
            usage,
            model,
        }
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
    /// Tail seq the loop had when this cut froze. A reader draws the checkpoint
    /// here so everything the turn writes afterwards stays below it. Display
    /// only: the shadow range is still `kept_from_seq`.
    pub at: Seq,
}

/// Read-only plan material for one compaction. The sync path commits this
/// snapshot directly; a background summary reuses the identities and lets the
/// commit re-anchor them on the current surface.
struct CompactionPlanBuild {
    plan: CompactionPlan,
    snapshot: Vec<WorkingRow>,
    tail: Vec<WorkingRow>,
    kept: Vec<Item>,
    expected_revision: u64,
}

/// A summary produced away from the main loop, with its cut frozen.
/// The branch owns both this value and the one compaction commit it feeds.
#[derive(Debug, Clone)]
pub struct PreparedCompaction {
    plan: CompactionPlan,
    artifact: SummaryArtifact,
    review: SummaryReview,
    summary_attempts: u8,
    /// The branch already emitted `Started` before the summary call.
    started: bool,
}

impl PreparedCompaction {
    /// The first row the frozen plan keeps; the commit must land the same cut.
    pub fn kept_from_seq(&self) -> u64 {
        self.plan.kept_from_seq
    }
}

/// One resolved commit candidate. Both the synchronous and prepared paths
/// converge here; only this value feeds the single Compact mutation.
struct CompactionCommit {
    snapshot: Vec<WorkingRow>,
    prefix_len: usize,
    tail: Vec<WorkingRow>,
    kept: Vec<Item>,
    expected_revision: u64,
    plan: CompactionPlan,
    artifact: SummaryArtifact,
    review: SummaryReview,
    summary_attempts: u8,
    tokens_before: usize,
    tokens_pruned: usize,
    tokens_after_prune: usize,
    prune_plan: Option<MechanicalPrunePlan>,
    started: bool,
}

/// Build the seq-aware cut without touching the durable surface.
#[allow(clippy::too_many_arguments)]
fn build_compaction_plan(
    budget: &BudgetPolicy,
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    max_tokens: u32,
    rows: &[WorkingRow],
    trigger: CompactionTrigger,
) -> Result<Option<CompactionPlanBuild>> {
    let snapshot = rows.to_vec();
    // Capture revision with the snapshot so the optimistic lock covers
    // snapshot→persist (ISSUE-035). Fail closed — never unwrap_or(0).
    let expected_revision = sessions.data().revision_blocking(session_id)?;
    let prefix_len = persisted_prefix_len(rows);
    let mut persisted = rows.to_vec();
    let tail = persisted.split_off(prefix_len);
    let transcript = project_items(&persisted);

    let Some(raw_cut) = find_keep_recent_cut(&transcript, budget.keep_recent_tokens) else {
        tracing::debug!(
            keep_recent_tokens = budget.keep_recent_tokens,
            prefix_len,
            "keep-recent: entire persisted prefix within keep window, skipping compact"
        );
        return Ok(None);
    };

    // Checkpoint identity, not summary text, decides the leading boundary.
    // A leading checkpoint is item zero; when keep-recent can compact, the
    // discarded range must include it, so cut is at least one.
    let prior_checkpoint_seq = persisted
        .first()
        .and_then(|row| row.checkpoint().and(row.log_seq));
    let cut = if prior_checkpoint_seq.is_some() {
        raw_cut.max(1)
    } else {
        raw_cut
    };
    debug_assert!(prior_checkpoint_seq.is_none() || cut > 0);

    let kept_from_seq = match persisted.get(cut).and_then(|row| row.log_seq) {
        Some(seq) => seq,
        None => {
            return Err(LitecodeError::ToolExecution(format!(
                "compact cut {cut} has no log seq (persisted prefix len={prefix_len})"
            )));
        }
    };

    let summary_max_tokens = budget
        .compact_output_tokens(max_tokens)
        .min(request.max_output_tokens);
    // The display anchor: the newest persisted row at freeze time. Rows written
    // after it are the ones that must render below the checkpoint.
    let at = persisted
        .iter()
        .rev()
        .find_map(|row| row.log_seq)
        .unwrap_or(kept_from_seq);
    let plan = CompactionPlan {
        trigger,
        prefix_len,
        cut,
        kept_from_seq,
        prior_checkpoint_seq,
        summary_cap: summary_max_tokens,
        at,
    };
    Ok(Some(CompactionPlanBuild {
        plan,
        snapshot,
        tail,
        kept: transcript[cut..].to_vec(),
        expected_revision,
    }))
}

/// First-version mechanical-prune budgets. All values live here; the pruner
/// itself contains no scattered literals.
pub const TOOL_RESULT_PRUNE_THRESHOLD_CHARS: usize = 8_192;
pub const TOOL_RESULT_PRUNE_HEAD_CHARS: usize = 4_096;
pub const TOOL_RESULT_PRUNE_TAIL_CHARS: usize = 1_024;
pub const TOOL_RESULT_PRUNE_MARKER: &str = "[... tool result middle pruned ...]";
/// The first share of the effective context window whose request prefix is
/// never rewritten by mechanical pruning. A replacement may start only at or
/// after this token position, so the provider's cached prefix survives.
pub const PRUNE_PROTECTED_WINDOW_PERCENT: usize = 60;
/// Absolute floor for one useful prune. The relative gate is separate: a batch
/// must remove at least [`PRUNE_MIN_SAVED_SUFFIX_PERCENT`] of the suffix it
/// actually rewrites, because that suffix pays one full-price cache miss.
pub const PRUNE_MIN_SAVED_TOKENS_FLOOR: usize = 12_288;
/// Minimum share of the rewritten suffix (from the first replacement to the
/// end of the persisted prefix), in percent.
pub const PRUNE_MIN_SAVED_SUFFIX_PERCENT: usize = 60;

/// Token position inside the persisted conversation that mechanical pruning
/// must not touch. The first [`PRUNE_PROTECTED_WINDOW_PERCENT`] of the
/// model-visible request stays cache-stable; the fixed request envelope is
/// subtracted because it is never pruned and always precedes the conversation.
pub fn protected_conversation_tokens(context_window: usize, envelope_fixed_tokens: usize) -> usize {
    context_window
        .saturating_mul(PRUNE_PROTECTED_WINDOW_PERCENT)
        .saturating_div(100)
        .saturating_sub(envelope_fixed_tokens)
}

/// The same protected position for a compaction request: the effective window
/// and fixed envelope are exactly the ones the pressure meter uses.
pub fn protected_conversation_tokens_for_request(request: &CompactionRequestContext) -> usize {
    protected_conversation_tokens(
        request.context_window,
        ContextEnvelope::from_compaction_request(request).fixed_prompt_tokens(),
    )
}

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

/// Absolute cache-bust floor. The rewritten-suffix share gate is separate and
/// usually dominates at normal session sizes: see
/// [`MechanicalPrunePlan::is_worthwhile`].
pub fn prune_min_saved_tokens() -> usize {
    PRUNE_MIN_SAVED_TOKENS_FLOOR
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

/// Cumulative token starts for the persisted rows: `starts[i]` is the token
/// offset at which row `i` begins, and `starts[persisted_len]` is the total.
/// One pass over the projected items, so the positional filter stays cheap.
fn row_start_tokens(rows: &[WorkingRow], persisted_len: usize) -> Vec<usize> {
    let items = project_items(&rows[..persisted_len]);
    debug_assert_eq!(items.len(), persisted_len, "projection is one item per row");
    let mut starts = Vec::with_capacity(items.len() + 1);
    let mut total: usize = 0;
    starts.push(0);
    for item in &items {
        total = total.saturating_add(crate::session::estimate::item_token_estimate(item));
        starts.push(total);
    }
    starts
}

/// Deterministic, model-free first stage of compaction. Building a plan never
/// mutates the session; only a plan that clears the protected line, the savings
/// floor, the rewritten-suffix share, and the relief preflight is committed as
/// one batch (one suffix rewrite / one cache bust).
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
    /// Conversation-token position below which no replacement may land.
    pub protected_tokens: usize,
    /// Tokens from the first replacement's row start to the end of the
    /// persisted prefix: the exact suffix whose KV cache this plan rewrites.
    pub rewritten_suffix_tokens: usize,
}

impl MechanicalPrunePlan {
    /// Build a prune plan from the current durable working set. Pending rows do
    /// not have a seq and cannot be replaced. No replacement may start before
    /// `protected_tokens`; a row that straddles that line stays protected.
    /// Only historical (stale) media is touched; fresh media from the current
    /// model/tool batch stays intact.
    pub fn build(
        rows: &[WorkingRow],
        config: MechanicalPruneConfig,
        protected_tokens: usize,
    ) -> Self {
        let persisted_len = persisted_prefix_len(rows);
        let starts = row_start_tokens(rows, persisted_len);
        let total_tokens = starts.last().copied().unwrap_or(0);
        let Some(eligible_start) = starts
            .iter()
            .position(|&start| start >= protected_tokens)
            .filter(|&index| index < persisted_len)
        else {
            return Self {
                replacements: Vec::new(),
                estimated_saved_tokens: 0,
                protected_tokens,
                rewritten_suffix_tokens: 0,
            };
        };
        // The projection already decides which media is stale. Probe a clone so
        // the durable rows are never rewritten by the ephemeral view path.
        let projected = project_items(rows);
        let mut media_probe = projected.clone();
        let _ = crate::context_pipeline::media_budget::drop_stale_media(&mut media_probe);

        let mut replacements = Vec::new();
        let mut first_replacement_index = None;
        for (index, row) in rows
            .iter()
            .enumerate()
            .take(persisted_len)
            .skip(eligible_start)
        {
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
            if first_replacement_index.is_none() {
                first_replacement_index = Some(index);
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
        let rewritten_suffix_tokens =
            first_replacement_index.map_or(0, |index| total_tokens.saturating_sub(starts[index]));
        Self {
            replacements,
            estimated_saved_tokens,
            protected_tokens,
            rewritten_suffix_tokens,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.replacements.is_empty()
    }

    /// The absolute floor. Equality counts as clearing it.
    pub fn meets_min_savings(&self) -> bool {
        !self.is_empty() && self.estimated_saved_tokens >= prune_min_saved_tokens()
    }

    /// The plan must also remove at least
    /// [`PRUNE_MIN_SAVED_SUFFIX_PERCENT`] of the suffix it actually rewrites
    /// (from the first replacement's row start to the end of the persisted
    /// prefix). Equality counts as clearing the share.
    pub fn meets_rewritten_share(&self) -> bool {
        !self.is_empty()
            && self.rewritten_suffix_tokens > 0
            && self.estimated_saved_tokens.saturating_mul(100)
                >= self
                    .rewritten_suffix_tokens
                    .saturating_mul(PRUNE_MIN_SAVED_SUFFIX_PERCENT)
    }

    /// Both dimensions: the absolute savings floor and the rewritten-suffix
    /// share. Clearing only one still pays a cache miss for too little.
    pub fn is_worthwhile(&self) -> bool {
        self.meets_min_savings() && self.meets_rewritten_share()
    }
}

/// Minimal summary acceptance failures. No section, anchor, placeholder,
/// quality-score, prior-summary, or latest-user heuristics participate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryFailure {
    Finish(SummaryFinish),
    Empty,
    ToolCall,
    Image,
    NotSmaller {
        summary_tokens: usize,
        source_tokens: usize,
    },
}

/// One total-attempt budget: first try plus one identical retry.
pub const MAX_SUMMARY_ATTEMPTS: u8 = 2;
/// Provider-confirmed context overflow may recover at most once.
pub const MAX_CONTEXT_OVERFLOW_RETRIES: u8 = 1;

/// Minimal accepted-summary review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryReview {
    pub accepted: bool,
    pub failures: Vec<SummaryFailure>,
}

/// Model-free summary gate: completed, non-empty, no tool/image output, and
/// strictly smaller than the discarded source prefix.
pub struct SummaryAcceptance;

impl SummaryAcceptance {
    pub fn review(artifact: &SummaryArtifact, source_items: &[Item]) -> SummaryReview {
        let text = artifact.text.trim();
        let source_tokens = crate::session::estimate::compute_token_estimate(source_items).max(1);
        let summary_tokens = if text.is_empty() {
            0
        } else {
            crate::session::estimate::compute_token_estimate(&[user_text(text.to_string())])
        };
        let mut failures = Vec::new();

        match artifact.finish {
            SummaryFinish::Completed => {}
            SummaryFinish::ToolCall => failures.push(SummaryFailure::ToolCall),
            other => failures.push(SummaryFailure::Finish(other)),
        }
        if text.is_empty() {
            failures.push(SummaryFailure::Empty);
        }
        if artifact.has_image {
            failures.push(SummaryFailure::Image);
        }
        if summary_tokens >= source_tokens {
            failures.push(SummaryFailure::NotSmaller {
                summary_tokens,
                source_tokens,
            });
        }

        SummaryReview {
            accepted: failures.is_empty(),
            failures,
        }
    }
}

fn as_transport_compaction_error(error: LitecodeError) -> LitecodeError {
    match error {
        LitecodeError::Canceled | LitecodeError::CompactionFailure { .. } => error,
        other => LitecodeError::CompactionFailure {
            kind: crate::types::CompactionFailureKind::Transport,
            detail: other.to_string(),
        },
    }
}

struct SummaryPassFailure {
    error: LitecodeError,
    attempts: u8,
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

fn item_has_image(item: &Item) -> bool {
    match item {
        Item::Message(MessageItem::Input(message)) => message
            .content
            .iter()
            .any(|part| matches!(part, InputContent::InputImage(_))),
        Item::FunctionCallOutput(output) => match &output.output {
            FunctionCallOutput::Content(parts) => parts
                .iter()
                .any(|part| matches!(part, InputContent::InputImage(_))),
            FunctionCallOutput::Text(_) => false,
        },
        _ => false,
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
        request.target.context_window_max,
        request.max_output_tokens as usize,
        &envelope,
        &projection.items,
    )
}

/// Whether the deterministic prune can pay for its own suffix rewrite:
/// position protection, absolute savings floor, and the rewritten-suffix share.
pub fn prune_is_worthwhile(rows: &[WorkingRow], protected_tokens: usize) -> bool {
    MechanicalPrunePlan::build(rows, MechanicalPruneConfig::default(), protected_tokens)
        .is_worthwhile()
}

/// Build, gate, and commit one mechanical prune pass. Returns `None` when the
/// plan is empty, misses the protected line or either savings gate, or would
/// not relieve pressure; in those cases no event is written.
fn apply_mechanical_prune_if_worthwhile(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    prompt_baseline: &ProviderPromptBaseline,
    rows: &mut Vec<WorkingRow>,
    data_root: &std::path::Path,
) -> Result<Option<MechanicalPruneRun>> {
    let protected_tokens = protected_conversation_tokens_for_request(request);
    let plan = MechanicalPrunePlan::build(rows, MechanicalPruneConfig::default(), protected_tokens);
    if !plan.is_worthwhile() {
        tracing::debug!(
            estimated_saved_tokens = plan.estimated_saved_tokens,
            protected_tokens,
            rewritten_suffix_tokens = plan.rewritten_suffix_tokens,
            min_saved = prune_min_saved_tokens(),
            min_suffix_percent = PRUNE_MIN_SAVED_SUFFIX_PERCENT,
            replacements = plan.replacements.len(),
            "mechanical prune skipped: no eligible tail, below savings floor, or below suffix share"
        );
        return Ok(None);
    }

    // Preflight the relief gate before writing: a prune that still leaves the
    // surface over the pressure line only buys a cache miss that the summary
    // rewrite would pay again.
    let mut candidate = rows.clone();
    for replacement in &plan.replacements {
        if let Some(row) = candidate
            .iter_mut()
            .find(|row| row.log_seq == Some(replacement.seq))
        {
            row.replace_item(replacement.item.clone());
        }
    }
    let meter_after = context_meter_for_rows_local(request, &candidate, data_root);
    if meter_after.should_compact() {
        tracing::debug!(
            estimated_saved_tokens = plan.estimated_saved_tokens,
            protected_tokens,
            rewritten_suffix_tokens = plan.rewritten_suffix_tokens,
            used_after = meter_after.used_tokens,
            compact_at = meter_after.compact_at,
            "mechanical prune skipped: would not relieve pressure"
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
        protected_tokens,
        rewritten_suffix_tokens = plan.rewritten_suffix_tokens,
        min_saved = prune_min_saved_tokens(),
        min_suffix_percent = PRUNE_MIN_SAVED_SUFFIX_PERCENT,
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
    /// prune would clear the fixed protected line and both savings gates. This
    /// is deliberately only a discovery predicate: the summary path still runs
    /// prune first and may skip the LLM entirely when the pruned surface is
    /// already below pressure.
    pub fn can_prune(rows: &[WorkingRow], protected_tokens: usize) -> bool {
        prune_is_worthwhile(rows, protected_tokens)
    }

    /// One mechanical-prune decision for one compaction episode. A prune is
    /// all-or-nothing: it writes only when it alone relieves pressure. If it
    /// does not, the summary path sees the untouched surface; the decision is
    /// never reopened by rows that arrive later.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prune_once(
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        data_root: &std::path::Path,
    ) -> Result<bool> {
        let Some(run) = apply_mechanical_prune_if_worthwhile(
            sessions,
            session_id,
            request,
            prompt_baseline,
            rows,
            data_root,
        )?
        else {
            return Ok(false);
        };
        let tokens_after = run.meter_after.used_tokens;
        let telemetry = compaction_telemetry(
            CompactionTrigger::Auto,
            run.tokens_before,
            run.tokens_before.saturating_sub(tokens_after),
            tokens_after,
            Some(0),
        );
        tracing::info!(
            tokens_before = run.tokens_before,
            tokens_after,
            replacements = run.plan.replacements.len(),
            relieved = run.relieved,
            "mechanical prune committed at the compaction trigger"
        );
        emit_compact_lifecycle_with_telemetry(
            sessions,
            session_id,
            CompactionTrigger::Auto,
            CompactionStage::Started,
            None,
            None,
            None,
            telemetry.clone(),
            None,
        );
        emit_compact_lifecycle_with_telemetry(
            sessions,
            session_id,
            CompactionTrigger::Auto,
            CompactionStage::Succeeded,
            None,
            None,
            None,
            telemetry,
            None,
        );
        Ok(true)
    }

    /// The meter used by the auto trigger and the hard send gate. Keeping this
    /// on the policy means the runtime and the pipeline never re-derive it.
    pub fn measure_meter(
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &[WorkingRow],
        data_root: &std::path::Path,
    ) -> ContextMeter {
        context_meter_for_rows(
            sessions,
            session_id,
            request,
            prompt_baseline,
            rows,
            data_root,
        )
    }

    /// Shadow mode: choose the same cut the synchronous path would use, but
    /// never call the provider and never write. The caller logs the decision.
    pub fn plan_preview(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        rows: &[WorkingRow],
    ) -> Result<Option<CompactionPlan>> {
        Ok(build_compaction_plan(
            budget,
            sessions,
            session_id,
            request,
            max_tokens,
            rows,
            CompactionTrigger::Auto,
        )?
        .map(|build| build.plan))
    }

    /// Prepare half of compaction: choose the cut and ask the model for a
    /// summary without writing the session. The branch owns the follow-up
    /// commit; a synchronous caller may use the value as its frozen plan.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_compaction(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        rows: &[WorkingRow],
        cancel: &CancellationToken,
        _data_root: &std::path::Path,
    ) -> Result<Option<PreparedCompaction>> {
        Self::prepare_compaction_with_attempts(
            budget, sessions, session_id, request, max_tokens, rows, cancel, _data_root, 1, false,
        )
        .await
    }

    /// Prepare a summary from one frozen plan. The whole attempt budget uses
    /// the same request/prefix/cut; no retry may re-plan.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_compaction_with_attempts(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        max_tokens: u32,
        rows: &[WorkingRow],
        cancel: &CancellationToken,
        data_root: &std::path::Path,
        max_attempts: u8,
        emit_started: bool,
    ) -> Result<Option<PreparedCompaction>> {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let Some(build) = build_compaction_plan(
            budget,
            sessions,
            session_id,
            request,
            max_tokens,
            rows,
            CompactionTrigger::Auto,
        )?
        else {
            return Ok(None);
        };
        let tokens_before = context_meter_for_rows_local(request, rows, data_root).used_tokens;
        if emit_started {
            emit_compact_lifecycle_with_telemetry(
                sessions,
                session_id,
                CompactionTrigger::Auto,
                CompactionStage::Started,
                None,
                None,
                None,
                compaction_telemetry(
                    CompactionTrigger::Auto,
                    tokens_before,
                    0,
                    tokens_before,
                    None,
                ),
                Some(build.plan.at),
            );
        }
        let (artifact, review, summary_attempts) = Self::first_pass_compaction(
            request,
            build.plan.summary_cap,
            build.plan.cut,
            &build.snapshot,
            sessions,
            cancel,
            max_attempts,
        )
        .await
        .map_err(|failure| {
            if emit_started {
                if matches!(&failure.error, LitecodeError::Canceled) {
                    emit_compaction_abandoned(sessions, session_id);
                } else {
                    emit_compact_failed_with_telemetry(
                        sessions,
                        session_id,
                        CompactionTrigger::Auto,
                        None,
                        &failure.error,
                        compaction_telemetry(
                            CompactionTrigger::Auto,
                            tokens_before,
                            0,
                            tokens_before,
                            Some(failure.attempts),
                        ),
                    );
                }
            }
            failure.error
        })?;
        debug_assert!(review.accepted);
        Ok(Some(PreparedCompaction {
            plan: build.plan,
            artifact,
            review,
            summary_attempts,
            started: emit_started,
        }))
    }

    /// One-shot user-triggered compaction.
    ///
    /// Product eligibility is enforced by the caller's `ContextMeter`. This
    /// deliberately skips the automatic pressure trigger and post-compact loop
    /// reminders.
    #[allow(clippy::too_many_arguments)]
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
            None,
            MAX_SUMMARY_ATTEMPTS,
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
            let before_tokens =
                context_meter_for_rows_local(request, &before_prune, data_root).used_tokens;
            let after_meter = context_meter_for_rows_local(request, &rows, data_root);
            let telemetry = compaction_telemetry(
                CompactionTrigger::Manual,
                before_tokens,
                before_tokens.saturating_sub(after_meter.used_tokens),
                after_meter.used_tokens,
                Some(0),
            );
            let err = LitecodeError::NothingToCompact;
            emit_compact_failed_with_telemetry(
                sessions,
                session_id,
                CompactionTrigger::Manual,
                operation_id,
                &err,
                telemetry,
            );
            return Err(err);
        };
        tracing::info!(
            finish = ?artifact.finish,
            model = %artifact.model,
            usage = ?artifact.usage,
            "manual compaction committed"
        );
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
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
        prepared: Option<PreparedCompaction>,
        max_attempts: u8,
        force: bool,
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

        if force || meter.should_compact() {
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
                prepared,
                max_attempts,
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
    #[allow(clippy::too_many_arguments)]
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
        prepared: Option<PreparedCompaction>,
        max_attempts: u8,
    ) -> Result<Option<SummaryArtifact>> {
        if cancel.is_cancelled() {
            return Ok(None);
        }

        // A prepared summary is a frozen plan. The prune decision was made
        // when the plan was created; rows written afterwards are appended after
        // the cut and must not re-open that decision.
        let commit = if let Some(prepared) = prepared {
            // The background summary was computed against an earlier snapshot.
            // Re-anchor it on the current surface: keep the same first kept
            // row, validate the leading checkpoint, and let any rows written
            // since prepare stay verbatim after the cut.
            let snapshot = rows.clone();
            let prefix_len = persisted_prefix_len(&snapshot);
            let mut persisted = snapshot.clone();
            let tail = persisted.split_off(prefix_len);
            let prior_checkpoint_seq = persisted
                .first()
                .and_then(|row| row.checkpoint().and(row.log_seq));
            if prior_checkpoint_seq != prepared.plan.prior_checkpoint_seq {
                tracing::info!(
                    expected = ?prepared.plan.prior_checkpoint_seq,
                    actual = ?prior_checkpoint_seq,
                    "prefetched compaction rejected: leading checkpoint changed"
                );
                return Ok(None);
            }
            let Some(cut) = persisted
                .iter()
                .position(|row| row.log_seq == Some(prepared.plan.kept_from_seq))
            else {
                tracing::info!(
                    kept_from_seq = prepared.plan.kept_from_seq,
                    "prefetched compaction rejected: kept_from_seq left the surface"
                );
                return Ok(None);
            };
            let transcript = project_items(&persisted);
            let kept = transcript[cut..].to_vec();
            let tokens_before = context_meter_for_rows(
                sessions,
                session_id,
                request,
                prompt_baseline,
                &snapshot,
                data_root,
            )
            .used_tokens;
            let mut plan = prepared.plan;
            plan.prefix_len = prefix_len;
            plan.cut = cut;
            plan.prior_checkpoint_seq = prior_checkpoint_seq;
            CompactionCommit {
                snapshot,
                prefix_len,
                tail,
                kept,
                expected_revision: sessions.data().revision_blocking(session_id)?,
                plan,
                artifact: prepared.artifact,
                review: prepared.review,
                summary_attempts: prepared.summary_attempts,
                tokens_before,
                tokens_pruned: 0,
                tokens_after_prune: tokens_before,
                prune_plan: None,
                started: prepared.started,
            }
        } else {
            // Stage one of every compaction trigger: deterministic mechanical
            // pruning. It runs here, never in build_view, so ordinary turns keep
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
                let telemetry = prune_run.as_ref().map_or_else(
                    || compaction_telemetry(trigger, tokens_after, 0, tokens_after, Some(0)),
                    |run| {
                        compaction_telemetry(
                            trigger,
                            run.tokens_before,
                            run.tokens_before
                                .saturating_sub(run.meter_after.used_tokens),
                            run.meter_after.used_tokens,
                            Some(0),
                        )
                    },
                );
                emit_compact_lifecycle_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Started,
                    operation_id,
                    None,
                    None,
                    telemetry.clone(),
                    None,
                );
                emit_compact_lifecycle_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Succeeded,
                    operation_id,
                    None,
                    None,
                    telemetry,
                    None,
                );
                return Ok(None);
            }

            let tokens_before = prune_run
                .as_ref()
                .map(|run| run.tokens_before)
                .unwrap_or_else(|| {
                    context_meter_for_rows(
                        sessions,
                        session_id,
                        request,
                        prompt_baseline,
                        rows,
                        data_root,
                    )
                    .used_tokens
                });
            let tokens_pruned = prune_run
                .as_ref()
                .map(|run| {
                    run.tokens_before
                        .saturating_sub(run.meter_after.used_tokens)
                })
                .unwrap_or(0);
            let tokens_after_prune = prune_run
                .as_ref()
                .map(|run| run.meter_after.used_tokens)
                .unwrap_or(tokens_before);
            let base_telemetry = compaction_telemetry(
                trigger,
                tokens_before,
                tokens_pruned,
                tokens_after_prune,
                None,
            );
            let build = match build_compaction_plan(
                budget, sessions, session_id, request, max_tokens, rows, trigger,
            ) {
                Ok(Some(build)) => build,
                Ok(None) => return Ok(None),
                Err(error) => {
                    emit_compact_failed_with_telemetry(
                        sessions,
                        session_id,
                        trigger,
                        operation_id,
                        &error,
                        base_telemetry,
                    );
                    return Err(error);
                }
            };
            emit_compact_lifecycle_with_telemetry(
                sessions,
                session_id,
                trigger,
                CompactionStage::Started,
                operation_id,
                None,
                None,
                base_telemetry.clone(),
                Some(build.plan.at),
            );

            let (artifact, review, summary_attempts) = match Self::first_pass_compaction(
                request,
                build.plan.summary_cap,
                build.plan.cut,
                &build.snapshot,
                sessions,
                cancel,
                max_attempts,
            )
            .await
            {
                Ok(accepted) => accepted,
                Err(failure) => {
                    let telemetry = compaction_telemetry(
                        trigger,
                        tokens_before,
                        tokens_pruned,
                        tokens_after_prune,
                        Some(failure.attempts),
                    );
                    emit_compact_failed_with_telemetry(
                        sessions,
                        session_id,
                        trigger,
                        operation_id,
                        &failure.error,
                        telemetry,
                    );
                    return Err(failure.error);
                }
            };
            debug_assert!(review.accepted);

            if cancel.is_cancelled() {
                let err = LitecodeError::Canceled;
                let telemetry = compaction_telemetry(
                    trigger,
                    tokens_before,
                    tokens_pruned,
                    tokens_after_prune,
                    Some(summary_attempts),
                );
                emit_compact_failed_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    operation_id,
                    &err,
                    telemetry,
                );
                return Err(err);
            }

            CompactionCommit {
                snapshot: build.snapshot,
                prefix_len: build.plan.prefix_len,
                tail: build.tail,
                kept: build.kept,
                expected_revision: build.expected_revision,
                plan: build.plan,
                artifact,
                review,
                summary_attempts,
                tokens_before,
                tokens_pruned,
                tokens_after_prune,
                prune_plan,
                started: true,
            }
        };

        let CompactionCommit {
            snapshot,
            prefix_len,
            tail,
            kept,
            expected_revision,
            plan,
            artifact,
            review,
            summary_attempts,
            tokens_before,
            tokens_pruned,
            tokens_after_prune,
            prune_plan,
            started,
        } = commit;
        let cut = plan.cut;
        let checkpoint = CompactionCheckpoint {
            summary: artifact.text.clone(),
            from: snapshot
                .first()
                .and_then(|row| row.log_seq)
                .unwrap_or(plan.kept_from_seq),
            to: plan.kept_from_seq,
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
                request.target.context_window_max,
                request.max_output_tokens as usize,
                &envelope,
                &projection.items,
            )
        };
        let final_count = final_meter.used_tokens;
        if !started {
            let telemetry = compaction_telemetry(
                trigger,
                tokens_before,
                tokens_pruned,
                tokens_after_prune,
                None,
            );
            emit_compact_lifecycle_with_telemetry(
                sessions,
                session_id,
                trigger,
                CompactionStage::Started,
                operation_id,
                None,
                None,
                telemetry,
                Some(plan.at),
            );
        }
        if final_meter.is_over_prompt_limit() {
            *rows = snapshot;
            tracing::error!(
                final_count,
                limit = final_meter.prompt_limit,
                "token budget still exceeded after autocompact"
            );
            let err = LitecodeError::TokenBudgetExceeded;
            let telemetry = compaction_telemetry(
                trigger,
                tokens_before,
                tokens_pruned,
                final_count,
                Some(summary_attempts),
            );
            emit_compact_failed_with_telemetry(
                sessions,
                session_id,
                trigger,
                operation_id,
                &err,
                telemetry,
            );
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
                kept_from: Some(plan.kept_from_seq),
                at: Some(plan.at),
                expected_prefix: Some(prefix_len),
                // Already captured off the response stream; persisting it is what
                // stops the summarizer call from being an unrecorded cost.
                usage: artifact.usage.clone(),
            },
        ) {
            Ok(receipt) => receipt,
            Err(e) => {
                *rows = snapshot;
                let telemetry = compaction_telemetry(
                    trigger,
                    tokens_before,
                    tokens_pruned,
                    final_count,
                    Some(summary_attempts),
                );
                emit_compact_failed_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    operation_id,
                    &e,
                    telemetry,
                );
                return Err(e);
            }
        };
        let checkpoint_seq = match receipt.outcome {
            crate::session::data::command::CommitKind::Compacted { seq } => seq,
            other => {
                *rows = snapshot;
                let err = LitecodeError::ToolExecution(format!(
                    "compact mutation returned unexpected outcome {other:?}"
                ));
                let telemetry = compaction_telemetry(
                    trigger,
                    tokens_before,
                    tokens_pruned,
                    final_count,
                    Some(summary_attempts),
                );
                emit_compact_failed_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    operation_id,
                    &err,
                    telemetry,
                );
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
            summary_attempts,
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
        let telemetry = compaction_telemetry(
            trigger,
            outcome.tokens_before,
            tokens_pruned,
            outcome.tokens_after,
            Some(outcome.summary_attempts),
        );
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
                emit_compact_lifecycle_with_telemetry(
                    sessions,
                    session_id,
                    trigger,
                    CompactionStage::Succeeded,
                    operation_id,
                    None,
                    None,
                    telemetry,
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
            "compaction checkpoint committed"
        );
        let summary = outcome.summary;
        prompt_baseline.clear();
        emit_compact_lifecycle_with_telemetry(
            sessions,
            session_id,
            trigger,
            CompactionStage::Succeeded,
            operation_id,
            None,
            None,
            telemetry,
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
        max_attempts: u8,
    ) -> std::result::Result<(SummaryArtifact, SummaryReview, u8), SummaryPassFailure> {
        let max_attempts = max_attempts.max(1);
        if cancel.is_cancelled() {
            return Err(SummaryPassFailure {
                error: LitecodeError::Canceled,
                attempts: 0,
            });
        }

        // The request input is the discarded prefix in original Item order,
        // projected exactly as the main view would project it. `kept` stays in
        // the working set and never enters this request.
        let prefix = match build_compaction_input(request, sessions, rows, cut) {
            Ok(prefix) => prefix,
            Err(error) => {
                return Err(SummaryPassFailure { error, attempts: 0 });
            }
        };
        let mut last_review: Option<SummaryReview> = None;

        for attempt in 1..=max_attempts {
            let artifact = match Self::call_llm_compact_once_with_timeout(
                request,
                prefix.clone(),
                max_tokens,
                cancel,
                COMPACT_REQUEST_TIMEOUT,
            )
            .await
            {
                Ok(artifact) => artifact,
                Err(LitecodeError::Canceled) => {
                    return Err(SummaryPassFailure {
                        error: LitecodeError::Canceled,
                        attempts: attempt,
                    });
                }
                Err(error) => {
                    return Err(SummaryPassFailure {
                        error: as_transport_compaction_error(error),
                        attempts: attempt,
                    });
                }
            };

            if cancel.is_cancelled() {
                return Err(SummaryPassFailure {
                    error: LitecodeError::Canceled,
                    attempts: attempt,
                });
            }

            let review = SummaryAcceptance::review(&artifact, &prefix);
            if review.accepted {
                tracing::info!(
                    summary_len = artifact.text.len(),
                    finish = ?artifact.finish,
                    model = %artifact.model,
                    usage = ?artifact.usage,
                    attempt,
                    "keep-recent compaction accepted"
                );
                return Ok((artifact, review, attempt));
            }

            tracing::warn!(
                attempt,
                finish = ?artifact.finish,
                failures = ?review.failures,
                "compaction summary rejected by minimal acceptance"
            );
            last_review = Some(review);
        }

        let review = last_review.unwrap_or(SummaryReview {
            accepted: false,
            failures: vec![SummaryFailure::Empty],
        });
        let kind = if review.failures.iter().any(|failure| {
            matches!(
                failure,
                SummaryFailure::Finish(_) | SummaryFailure::Empty | SummaryFailure::ToolCall
            )
        }) {
            crate::types::CompactionFailureKind::SummaryFinish
        } else {
            crate::types::CompactionFailureKind::Quality
        };
        Err(SummaryPassFailure {
            error: LitecodeError::CompactionFailure {
                kind,
                detail: format!(
                    "summary rejected after {max_attempts} attempts: {:?}",
                    review.failures
                ),
            },
            attempts: max_attempts,
        })
    }

    async fn call_llm_compact_once_with_timeout(
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
                if let Some(usage) = provider_usage_from_response(response)
                    && let Ok(mut slot) = usage_cb.lock()
                {
                    *slot = Some(usage);
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
        Ok(SummaryArtifact::from_output(
            &items,
            request.model.clone(),
            usage,
            terminal_finish,
        ))
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

fn compaction_telemetry(
    trigger: CompactionTrigger,
    tokens_before: usize,
    tokens_pruned: usize,
    tokens_after: usize,
    summary_attempts: Option<u8>,
) -> CompactionTelemetry {
    CompactionTelemetry {
        tokens_before: Some(tokens_before),
        tokens_pruned: Some(tokens_pruned),
        tokens_after: Some(tokens_after),
        summary_attempts,
        overflow_retry: Some(u8::from(trigger == CompactionTrigger::Overflow)),
    }
}

fn compact_fail_kind(err: &LitecodeError) -> CompactionFailKind {
    match err {
        LitecodeError::NothingToCompact => CompactionFailKind::NothingToCompact,
        LitecodeError::Canceled => CompactionFailKind::Canceled,
        LitecodeError::CompactionFailure { kind, .. } => match kind {
            crate::types::CompactionFailureKind::SummaryFinish => CompactionFailKind::SummaryFinish,
            crate::types::CompactionFailureKind::Quality => CompactionFailKind::Quality,
            crate::types::CompactionFailureKind::Transport => CompactionFailKind::Transport,
        },
        _ => CompactionFailKind::Failed,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_compact_lifecycle_with_telemetry(
    sessions: &SessionManager,
    session_id: &str,
    trigger: CompactionTrigger,
    stage: CompactionStage,
    operation_id: Option<&str>,
    fail_kind: Option<CompactionFailKind>,
    error: Option<String>,
    telemetry: CompactionTelemetry,
    at: Option<Seq>,
) {
    // The lifecycle emitter is the single writer of the session's compaction
    // state. A snapshot built after this call therefore always agrees with the
    // stage it carries; clients project the state instead of re-deriving it.
    let operation_id = match stage {
        CompactionStage::Started => {
            let (_, id) = sessions.begin_compaction_blocking(session_id, operation_id, trigger, at);
            Some(id)
        }
        CompactionStage::Succeeded | CompactionStage::Failed | CompactionStage::Abandoned => {
            let (_, id) = sessions.end_compaction_blocking(session_id, operation_id, trigger);
            id.or_else(|| operation_id.map(str::to_string))
        }
    };
    sessions.publish_internal(
        session_id,
        InternalEvent::CompactionLifecycle {
            trigger,
            stage,
            operation_id,
            fail_kind,
            error,
            telemetry,
            at,
        },
    );
}

pub(crate) fn emit_compaction_abandoned(sessions: &SessionManager, session_id: &str) {
    emit_compact_lifecycle_with_telemetry(
        sessions,
        session_id,
        CompactionTrigger::Auto,
        CompactionStage::Abandoned,
        None,
        None,
        None,
        CompactionTelemetry::default(),
        None,
    );
}

fn emit_compact_failed_with_telemetry(
    sessions: &SessionManager,
    session_id: &str,
    trigger: CompactionTrigger,
    operation_id: Option<&str>,
    err: &LitecodeError,
    telemetry: CompactionTelemetry,
) {
    let fail_kind = compact_fail_kind(err);
    tracing::warn!(
        session_id,
        trigger = ?trigger,
        fail_kind = ?fail_kind,
        operation_id = operation_id.unwrap_or(""),
        error = %err,
        telemetry = ?telemetry,
        "compaction failed"
    );
    emit_compact_lifecycle_with_telemetry(
        sessions,
        session_id,
        trigger,
        CompactionStage::Failed,
        operation_id,
        Some(fail_kind),
        Some(err.to_string()),
        telemetry,
        None,
    );
}

/// Items as the model view will carry them, so the budget never counts media
/// that `drop_stale_media` removes from the request.
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

    /// Test mirror of the retired budget helper: project plus stale-media drop.
    fn budget_items_for_test(rows: &[WorkingRow]) -> Vec<Item> {
        let mut items = project_items(rows);
        crate::context_pipeline::media_budget::drop_stale_media(&mut items);
        items
    }

    use crate::config::TurnGuard;
    use crate::context_pipeline::{Context, ContextPipeline};
    use crate::llm::{LlmProvider, ModelRequest, RequestPurpose, ToolDef};
    use crate::platform_knobs::ThinkingSpec;
    use crate::provider_catalog::ProviderCatalog;
    use crate::types::{StreamEvents, assistant_text, user_message, user_text};
    use std::future::Future;
    use std::path::Path;
    use std::pin::Pin;
    use std::sync::Arc;

    use crate::authority::responses::{
        ResponseCompletedEvent, ResponseStreamEvent, Status as ResponseStatus,
    };

    fn completed_terminal() -> StreamEvents {
        let response = crate::types::Response {
            background: None,
            billing: None,
            conversation: None,
            created_at: 1,
            completed_at: Some(1),
            error: None,
            id: "resp_compact_test".into(),
            incomplete_details: None,
            instructions: None,
            max_output_tokens: Some(8192),
            metadata: None,
            model: "compact-model".into(),
            object: "response".into(),
            output: Vec::new(),
            parallel_tool_calls: None,
            previous_response_id: None,
            prompt: None,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            reasoning: None,
            safety_identifier: None,
            service_tier: None,
            status: ResponseStatus::Completed,
            temperature: None,
            text: None,
            tool_choice: None,
            tools: None,
            top_logprobs: None,
            top_p: None,
            truncation: None,
            usage: None,
        };
        ResponseStreamEvent::ResponseCompleted(ResponseCompletedEvent {
            sequence_number: 1,
            response,
        })
    }

    fn emit_completed(
        on_event: &mut Option<Box<dyn FnMut(crate::types::StreamEvents) + Send + '_>>,
    ) {
        if let Some(callback) = on_event.as_mut() {
            callback(completed_terminal());
        }
    }

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
            request: &'a ModelRequest,
            _api_key: &'a str,
            mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            emit_completed(&mut on_event);
            let text = summary_for_request(request);
            Box::pin(async move { Ok::<_, LitecodeError>(vec![assistant_text(&text)]) })
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
            request: &'a ModelRequest,
            _api_key: &'a str,
            mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            emit_completed(&mut on_event);
            let text = summary_for_request(request);
            Box::pin(async move { Ok::<_, LitecodeError>(vec![assistant_text(&text)]) })
        }
    }

    fn summary_for_request(_request: &ModelRequest) -> String {
        "Short checkpoint text for the main request.".into()
    }

    struct RetrySummaryProvider {
        calls: std::sync::atomic::AtomicUsize,
        requests: std::sync::Mutex<Vec<ModelRequest>>,
    }

    impl RetrySummaryProvider {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<ModelRequest> {
            self.requests.lock().expect("requests").clone()
        }
    }

    impl LlmProvider for RetrySummaryProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
                requests: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            request: &'a ModelRequest,
            _api_key: &'a str,
            mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            let attempt = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            emit_completed(&mut on_event);
            let text = if attempt == 0 {
                String::new()
            } else {
                summary_for_request(request)
            };
            Box::pin(async move { Ok::<_, LitecodeError>(vec![assistant_text(&text)]) })
        }
    }

    struct InvalidSummaryProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl InvalidSummaryProvider {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl LlmProvider for InvalidSummaryProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            mut on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            emit_completed(&mut on_event);
            Box::pin(async { Ok::<_, LitecodeError>(vec![assistant_text("")]) })
        }
    }

    struct NoTerminalSummaryProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl NoTerminalSummaryProvider {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl LlmProvider for NoTerminalSummaryProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let text = summary_for_request(request);
            Box::pin(async move { Ok::<_, LitecodeError>(vec![assistant_text(&text)]) })
        }
    }

    struct CompactionTransportFailureProvider {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl CompactionTransportFailureProvider {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl LlmProvider for CompactionTransportFailureProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            })
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async { Err(LitecodeError::Llm("network is unavailable".into())) })
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
        let max_output = target.max_output;
        CompactionRequestContext {
            provider,
            api_key: "sk-test".into(),
            model: target.id.clone(),
            context_window: target.context_window_max,
            target,
            thinking: ThinkingSpec::Tier(Default::default()),
            max_output_tokens: max_output,
            instructions: "system".into(),
            tools: vec![],
            session_id: "s1".into(),
        }
    }

    fn set_request_window(
        request: &mut CompactionRequestContext,
        window: usize,
        max_output_tokens: u32,
    ) {
        let mut target = (*request.target).clone();
        target.context_window = window;
        target.context_window_max = window;
        target.max_output = max_output_tokens;
        request.target = Arc::new(target);
        request.context_window = window;
        request.max_output_tokens = max_output_tokens;
    }

    #[tokio::test]
    async fn compact_request_times_out_on_silent_provider() {
        let cancel = CancellationToken::new();
        let request = compact_context(Arc::new(PendingProvider));
        let err = CompactPolicy::call_llm_compact_once_with_timeout(
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
        let err = CompactPolicy::call_llm_compact_once_with_timeout(
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

    fn acceptance_source(extra: &str) -> Vec<Item> {
        vec![user_text(format!(
            "Please inspect src/context_pipeline/compact.rs and preserve the exact request. {}{}",
            "word ".repeat(1_500),
            extra
        ))]
    }

    fn completed_summary(text: &str) -> SummaryArtifact {
        SummaryArtifact {
            text: text.into(),
            finish: SummaryFinish::Completed,
            has_image: false,
            usage: None,
            model: "m".into(),
        }
    }

    #[test]
    fn summary_acceptance_accepts_completed_smaller_text() {
        let source = acceptance_source("");
        let artifact = completed_summary("Short checkpoint text.");
        let review = SummaryAcceptance::review(&artifact, &source);
        assert!(review.accepted, "{review:?}");
        assert!(review.failures.is_empty());
    }

    #[test]
    fn summary_acceptance_rejects_finish_empty_tool_image_and_not_smaller() {
        let source = acceptance_source("");

        let mut missing = completed_summary("Short checkpoint text.");
        missing.finish = SummaryFinish::Missing;
        let review = SummaryAcceptance::review(&missing, &source);
        assert!(
            review
                .failures
                .contains(&SummaryFailure::Finish(SummaryFinish::Missing))
        );

        let empty = completed_summary("   ");
        let review = SummaryAcceptance::review(&empty, &source);
        assert!(review.failures.contains(&SummaryFailure::Empty));

        let tool = SummaryArtifact {
            finish: SummaryFinish::ToolCall,
            ..completed_summary("Short checkpoint text.")
        };
        let review = SummaryAcceptance::review(&tool, &source);
        assert!(review.failures.contains(&SummaryFailure::ToolCall));

        let image = SummaryArtifact {
            has_image: true,
            ..completed_summary("Short checkpoint text.")
        };
        let review = SummaryAcceptance::review(&image, &source);
        assert!(review.failures.contains(&SummaryFailure::Image));

        let too_large = completed_summary(&"word ".repeat(2_000));
        let review = SummaryAcceptance::review(&too_large, &source);
        assert!(
            review
                .failures
                .iter()
                .any(|failure| matches!(failure, SummaryFailure::NotSmaller { .. })),
            "{review:?}"
        );
    }

    #[test]
    fn summary_artifact_preserves_incomplete_provider_finish() {
        let mut truncated = assistant_text("partial summary");
        let Item::Message(MessageItem::Output(message)) = &mut truncated else {
            panic!("assistant_text must be an output message");
        };
        message.status = OutputStatus::Incomplete;
        let artifact = SummaryArtifact::from_output(&[truncated], "m".into(), None, None);
        assert_eq!(artifact.finish, SummaryFinish::Missing);
        let review = SummaryAcceptance::review(&artifact, &acceptance_source(""));
        assert!(
            review
                .failures
                .iter()
                .any(|failure| matches!(failure, SummaryFailure::Finish(SummaryFinish::Missing)))
        );

        let terminal = SummaryArtifact::from_output(
            &[assistant_text("looks complete")],
            "m".into(),
            None,
            Some(SummaryFinish::Incomplete),
        );
        assert_eq!(terminal.finish, SummaryFinish::Incomplete);
        let review = SummaryAcceptance::review(&terminal, &acceptance_source(""));
        assert!(
            review.failures.iter().any(|failure| matches!(
                failure,
                SummaryFailure::Finish(SummaryFinish::Incomplete)
            ))
        );
    }

    #[test]
    fn compaction_summary_detects_image_output() {
        let items = vec![
            assistant_text("short checkpoint"),
            user_message("", &["litecode-media:probe-image".into()]),
        ];
        let artifact =
            SummaryArtifact::from_output(&items, "m".into(), None, Some(SummaryFinish::Completed));
        assert!(artifact.has_image);
        let review = SummaryAcceptance::review(&artifact, &acceptance_source(""));
        assert!(review.failures.contains(&SummaryFailure::Image));
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
        let review = SummaryAcceptance::review(&artifact, &acceptance_source(""));
        assert!(
            review
                .failures
                .iter()
                .any(|failure| matches!(failure, SummaryFailure::ToolCall))
        );
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
        let seed = long_seed();
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
            None,
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect("persist ok + reload err must return Ok");
        assert!(outcome.is_some(), "compaction committed");

        let mem_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.payload.model_item()))
            .collect();
        assert!(
            mem_previews
                .first()
                .is_some_and(|t| t.contains("Short checkpoint text for the main request.")),
            "emergency cache must lead with the checkpoint summary, got {mem_previews:?}"
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
            db0.contains("Short checkpoint text for the main request."),
            "DB must show the compacted summary, got {db0}"
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
                telemetry,
                ..
            } = env.event
            {
                assert_eq!(operation_id.as_deref(), Some("wp039-reload-err"));
                match stage {
                    CompactionStage::Succeeded => {
                        saw_succeeded = true;
                        assert!(telemetry.tokens_before.is_some());
                        assert!(telemetry.tokens_pruned.is_some());
                        assert!(telemetry.tokens_after.is_some());
                        assert!(telemetry.summary_attempts.is_some());
                        assert_eq!(telemetry.overflow_retry, Some(0));
                    }
                    CompactionStage::Failed => saw_failed = true,
                    _ => {}
                }
            }
        }
        assert!(saw_succeeded, "must emit Succeeded after Compact persist");
        assert!(!saw_failed, "must not emit Failed after Compact persist");
    }

    fn long_seed() -> Vec<Item> {
        let mut seed: Vec<Item> = (0..29)
            .map(|i| user_text(format!("seed-{i}-{}", "word ".repeat(400))))
            .collect();
        // Keep the mechanical recent-user anchor short and deterministic while
        // the discarded prefix remains large enough for the ratio gate.
        seed.push(user_text("latest-user-anchor"));
        seed
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
        let local = crate::context_pipeline::estimate::compute_token_estimate(
            &budget_items_for_test(&rows),
        );
        assert!(
            local > crate::session::estimate::autocompact_threshold(budget.budget_limit()),
            "local estimate {local} must clear the soft window line"
        );
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        set_request_window(&mut request, budget.budget_limit(), 1_024);
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
                None,
                MAX_SUMMARY_ATTEMPTS,
                false,
            )
            .await
            .unwrap();
        assert!(!did);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn summary_retry_reuses_the_identical_request() {
        let (sessions, sid, _header) = open_seeded();
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let provider = Arc::new(RetrySummaryProvider::new());
        let request = compact_context(provider.clone());
        let outcome = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("summary-retry"),
            &sessions.data_root_path(),
            None,
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect("second summary attempt is accepted")
        .expect("compaction commits after retry");
        assert_eq!(outcome.text, "Short checkpoint text for the main request.");
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "first invalid summary plus exactly one retry"
        );
        let requests = provider.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            format!("{:?}", requests[0]),
            format!("{:?}", requests[1]),
            "retry must reuse the exact identical ModelRequest"
        );
        let first_prefix = &requests[0].input[..requests[0].input.len() - 1];
        let second_prefix = &requests[1].input[..requests[1].input.len() - 1];
        assert_eq!(
            first_prefix, second_prefix,
            "retry must reuse the exact main prefix"
        );
        assert_eq!(
            item_text_preview(requests[0].input.last().unwrap()),
            crate::llm::COMPACTION_INSTRUCTION,
            "first attempt uses the canonical instruction"
        );
        let retry_instruction = item_text_preview(requests[1].input.last().unwrap());
        assert_eq!(
            retry_instruction,
            crate::llm::COMPACTION_INSTRUCTION,
            "retry must reuse the exact same instruction"
        );
    }

    async fn run_rejecting_summary(
        provider: Arc<dyn LlmProvider>,
    ) -> (LitecodeError, Vec<crate::session::event::SessionEvent>) {
        let (sessions, sid, _header) = open_seeded();
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let request = compact_context(provider);
        let error = CompactPolicy::compact_transcript(
            &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
            &sessions,
            &sid,
            &request,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("summary-rejection"),
            &sessions.data_root_path(),
            None,
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect_err("rejected summary must fail the compaction");
        (error, sessions.data().events_blocking(&sid).unwrap())
    }

    #[tokio::test]
    async fn summary_rejection_stops_after_two_attempts_without_checkpoint() {
        let provider = Arc::new(InvalidSummaryProvider::new());
        let (error, events) = run_rejecting_summary(provider.clone()).await;
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            error.compaction_failure_kind(),
            Some(crate::types::CompactionFailureKind::SummaryFinish)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted))
        );
    }

    #[tokio::test]
    async fn summary_missing_terminal_finish_retries_once_then_fails_finish() {
        let provider = Arc::new(NoTerminalSummaryProvider::new());
        let (error, events) = run_rejecting_summary(provider.clone()).await;
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            error.compaction_failure_kind(),
            Some(crate::types::CompactionFailureKind::SummaryFinish)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted))
        );
    }

    #[tokio::test]
    async fn summary_transport_is_classified_and_not_retried() {
        let provider = Arc::new(CompactionTransportFailureProvider::new());
        let (error, events) = run_rejecting_summary(provider.clone()).await;
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            error.compaction_failure_kind(),
            Some(crate::types::CompactionFailureKind::Transport)
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted))
        );
    }

    #[tokio::test]
    async fn non_relieving_mechanical_prune_is_not_committed_before_summary_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            dir.path()
                .join("global-litecode.db")
                .to_string_lossy()
                .to_string(),
        ));
        let sid = sessions
            .open_session_sync("/p", "default", Some("test/test-primary-model"))
            .expect("open session");
        let seed = vec![
            user_text("start of the transcript"),
            prune_call("call-prune-survives"),
            prune_tool_text("call-prune-survives", "word ".repeat(250_000)),
            user_text(format!("after-tools {}", "z".repeat(40_000))),
            user_text("kept-tail"),
        ];
        sessions.insert_detail_rows(&sid, &seed).expect("seed");
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let before_revision = sessions.data().revision_blocking(&sid).unwrap();
        let provider = Arc::new(InvalidSummaryProvider::new());
        let mut request = compact_context(provider.clone());
        set_request_window(&mut request, 10_000, 1_024);
        let error = CompactPolicy::compact_transcript(
            &BudgetPolicy::new(10_000).with_keep_recent_tokens(1),
            &sessions,
            &sid,
            &request,
            1_024,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("prune-survives-summary-failure"),
            &sessions.data_root_path(),
            None,
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect_err("invalid summary must still fail when no prune can relieve");
        assert_eq!(
            error.compaction_failure_kind(),
            Some(crate::types::CompactionFailureKind::SummaryFinish)
        );
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        let durable = sessions.data().working_set_blocking(&sid).unwrap();
        let tool_outputs = durable
            .iter()
            .filter_map(|row| row.item())
            .filter(|item| matches!(item, Item::FunctionCallOutput(_)))
            .map(item_text_preview)
            .collect::<Vec<_>>();
        assert!(
            tool_outputs
                .iter()
                .all(|text| !text.contains(TOOL_RESULT_PRUNE_MARKER)),
            "a prune that cannot relieve pressure must not rewrite the surface: {tool_outputs:?}"
        );
        assert!(
            !durable.iter().any(|row| row.checkpoint().is_some()),
            "a failed summary must not write a checkpoint"
        );
        assert_eq!(
            sessions.data().revision_blocking(&sid).unwrap(),
            before_revision,
            "a skipped prune must not advance the revision"
        );
        assert!(
            !sessions
                .data()
                .events_blocking(&sid)
                .unwrap()
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted))
        );
    }

    #[tokio::test]
    async fn forced_overflow_compaction_requires_surface_advance() {
        let (sessions, sid, _header) = open_seeded();
        let pipeline = ContextPipeline::new(
            128_000,
            Context {
                cwd: Path::new("/p").to_path_buf(),
                workspace_paths: crate::config::WorkspacePaths::for_legacy_root(Path::new("/p")),
                agents_md: None,
                claude_md: None,
            },
            sessions.data_root_path(),
        )
        .with_keep_recent_tokens(1);
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        request.session_id = sid.clone();
        let before = sessions.data().revision_blocking(&sid).unwrap();
        let advanced = pipeline
            .force_compact_overflow(
                &sessions,
                &sid,
                &request,
                crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
                &CancellationToken::new(),
            )
            .await
            .expect("forced overflow compaction");
        let after = sessions.data().revision_blocking(&sid).unwrap();
        assert!(advanced, "a checkpoint must advance the surface");
        assert!(after > before, "revision must advance");
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // A transcript that keep-recent cannot split has no compactable
        // range: no provider call, no surface generation, no retry proof.
        let sessions2 = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid2 = sessions2
            .open_session_sync("/p", "default", Some("test/test-primary-model"))
            .expect("open session");
        sessions2
            .insert_detail_rows(&sid2, &[user_text("one indivisible request")])
            .expect("seed");
        let pipeline2 = ContextPipeline::new(
            128_000,
            Context {
                cwd: Path::new("/p").to_path_buf(),
                workspace_paths: crate::config::WorkspacePaths::for_legacy_root(Path::new("/p")),
                agents_md: None,
                claude_md: None,
            },
            sessions2.data_root_path(),
        );
        let provider2 = Arc::new(CountingProvider::new());
        let mut request2 = compact_context(provider2.clone());
        request2.session_id = sid2.clone();
        let before2 = sessions2.data().revision_blocking(&sid2).unwrap();
        let advanced2 = pipeline2
            .force_compact_overflow(
                &sessions2,
                &sid2,
                &request2,
                crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
                &CancellationToken::new(),
            )
            .await
            .expect("no cut is not an internal error");
        assert!(!advanced2, "no surface advance means no overflow retry");
        assert_eq!(sessions2.data().revision_blocking(&sid2).unwrap(), before2);
        assert_eq!(provider2.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn lifecycle_fail_kind_preserves_structured_compaction_subtypes() {
        let cases = [
            (
                crate::types::CompactionFailureKind::SummaryFinish,
                CompactionFailKind::SummaryFinish,
            ),
            (
                crate::types::CompactionFailureKind::Quality,
                CompactionFailKind::Quality,
            ),
            (
                crate::types::CompactionFailureKind::Transport,
                CompactionFailKind::Transport,
            ),
        ];
        for (structured, lifecycle) in cases {
            let error = LitecodeError::CompactionFailure {
                kind: structured,
                detail: "test failure".into(),
            };
            assert_eq!(compact_fail_kind(&error), lifecycle);
        }
        assert_eq!(
            compact_fail_kind(&LitecodeError::NothingToCompact),
            CompactionFailKind::NothingToCompact
        );
        assert_eq!(
            compact_fail_kind(&LitecodeError::Canceled),
            CompactionFailKind::Canceled
        );
        assert_eq!(
            compact_fail_kind(&LitecodeError::CompactionFailed),
            CompactionFailKind::Failed
        );
    }

    #[test]
    fn context_overflow_classification_is_narrow() {
        assert!(crate::types::is_context_overflow_error(
            &LitecodeError::Llm(
                "HTTP 400: {\"code\":\"context_length_exceeded\",\"message\":\"too many tokens\"}"
                    .into()
            )
        ));
        assert!(crate::types::is_context_overflow_error(
            &LitecodeError::Llm("maximum context length exceeded".into())
        ));
        assert!(crate::types::is_context_overflow_error(
            &LitecodeError::Llm("provider says context window exceeded".into())
        ));
        assert!(!crate::types::is_context_overflow_error(
            &LitecodeError::Llm(
                "HTTP 400 too many tokens requested for the max_tokens parameter".into()
            )
        ));
        assert!(!crate::types::is_context_overflow_error(
            &LitecodeError::Llm("HTTP 429 rate limit".into())
        ));
        assert!(!crate::types::is_context_overflow_error(
            &LitecodeError::Llm("HTTP 400 invalid max_tokens parameter".into())
        ));
        assert!(!crate::types::is_context_overflow_error(
            &LitecodeError::Llm("HTTP 500 internal backend failure".into())
        ));
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
        set_request_window(&mut request, budget.budget_limit(), 1_024);
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
                None,
                MAX_SUMMARY_ATTEMPTS,
                false,
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
            crate::context_pipeline::estimate::compute_token_estimate(&budget_items_for_test(
                &rows,
            )),
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
        set_request_window(&mut request, budget.budget_limit(), 1_024);
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
                None,
                MAX_SUMMARY_ATTEMPTS,
                false,
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
        assert!(!text.contains('\u{fffd}'));
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
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), 0);
        assert_eq!(
            MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), 0),
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
        let second = MechanicalPrunePlan::build(&applied, MechanicalPruneConfig::default(), 0);
        assert!(
            second.is_empty(),
            "replaying the pruned surface must not create another replacement"
        );
        assert_eq!(
            item_text_chars(applied[2].item().unwrap()),
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
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), 0);
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
            prune_media_count(rows[6].item().unwrap()),
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
            MechanicalPrunePlan::build(&applied, MechanicalPruneConfig::default(), 0).is_empty(),
            "durable media placeholders must be stable on a second pass"
        );
    }

    #[test]
    fn mechanical_prune_gate_requires_floor_and_suffix_share() {
        assert_eq!(prune_min_saved_tokens(), PRUNE_MIN_SAVED_TOKENS_FLOOR);
        assert_eq!(PRUNE_MIN_SAVED_SUFFIX_PERCENT, 60);
        assert_eq!(PRUNE_PROTECTED_WINDOW_PERCENT, 60);
        assert_eq!(protected_conversation_tokens(128_000, 1_000), 75_800);
        assert_eq!(protected_conversation_tokens(1_000, 2_000), 0);
        let rows = vec![
            WorkingRow::persisted(1, prune_call("call-small")),
            WorkingRow::persisted(2, prune_tool_text("call-small", "x".repeat(8_300))),
        ];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), 0);
        assert_eq!(plan.replacements.len(), 1);
        assert!(
            plan.estimated_saved_tokens < PRUNE_MIN_SAVED_TOKENS_FLOOR,
            "a single small text result is below the cache-bust floor: {plan:?}"
        );
        assert!(!plan.meets_min_savings());
        assert!(!plan.is_worthwhile());
        assert!(!prune_is_worthwhile(&rows, 0));
    }

    #[test]
    fn mechanical_prune_never_touches_rows_before_the_protected_line() {
        let rows = vec![
            WorkingRow::persisted(1, user_text("go")),
            WorkingRow::persisted(2, prune_call("call-early")),
            WorkingRow::persisted(3, prune_tool_text("call-early", "word ".repeat(3_600))),
            WorkingRow::persisted(4, assistant_text("middle")),
            WorkingRow::persisted(5, prune_call("call-late")),
            WorkingRow::persisted(6, prune_tool_text("call-late", "word ".repeat(3_600))),
            WorkingRow::persisted(7, assistant_text("done")),
        ];
        let starts = row_start_tokens(&rows, rows.len());
        let late_start = starts[5];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), late_start);
        assert_eq!(plan.protected_tokens, late_start);
        assert!(
            plan.replacements
                .iter()
                .all(|replacement| replacement.seq >= 5),
            "no replacement may land before the protected line: {plan:?}"
        );
        assert_eq!(plan.replacements.len(), 1);
        assert_eq!(plan.replacements[0].seq, 6);

        // A line that falls inside the early result protects that whole row,
        // even though the row ends after the line.
        let straddle = row_start_tokens(&rows, rows.len())[2] + 1;
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), straddle);
        assert!(
            plan.replacements
                .iter()
                .all(|replacement| replacement.seq >= 5),
            "a straddling row must stay protected: {plan:?}"
        );
    }

    #[test]
    fn mechanical_prune_suffix_share_uses_only_the_rewritten_tail() {
        let rows = vec![
            WorkingRow::persisted(1, user_text("go")),
            WorkingRow::persisted(2, prune_call("call-early")),
            WorkingRow::persisted(3, prune_tool_text("call-early", "word ".repeat(3_600))),
            WorkingRow::persisted(4, prune_call("call-late")),
            WorkingRow::persisted(5, prune_tool_text("call-late", "word ".repeat(3_600))),
        ];
        let starts = row_start_tokens(&rows, rows.len());
        let late_start = starts[4];
        let plan = MechanicalPrunePlan::build(&rows, MechanicalPruneConfig::default(), late_start);
        assert_eq!(plan.replacements.len(), 1);
        assert_eq!(plan.replacements[0].seq, 5);
        assert_eq!(plan.rewritten_suffix_tokens, starts[5] - late_start);
        assert!(
            plan.meets_rewritten_share(),
            "the denominator is the rewritten suffix, not the whole prefix: {plan:?}"
        );
    }
}
