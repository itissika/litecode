use std::collections::HashSet;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::authority::responses::{
    FunctionCallOutput, InputContent, InputRole, MessageItem, OutputStatus,
};
use crate::context_pipeline::keep_recent::find_keep_recent_cut;
use crate::context_pipeline::project_model_view;
use crate::llm::CompactionRequestContext;
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTelemetry, CompactionTrigger, InternalEvent,
};
use crate::session::data::command::WorkingSetState;
use crate::session::event::Seq;
use crate::session::manager::{
    CompactionCancelReason, CompactionCommitKind, CompactionEpisodeLease, CompactionFailureAction,
    EpisodeOutcome, SessionManager,
};
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
    /// Main binding `max_output`; the compact request and its input reserve use
    /// this exact value.
    pub output_cap: u32,
    /// Quality ceiling for the final summary text only. Never a request cap.
    pub summary_text_cap: u32,
    /// Tail seq the loop had when this cut froze. A reader draws the checkpoint
    /// here so everything the turn writes afterwards stays below it. Display
    /// only: the shadow range is still `kept_from_seq`.
    pub at: Seq,
}

/// One compaction input. Ordinary compaction receives only sealed rows;
/// overflow receives the full surface plus the live tail. The input is frozen
/// when the episode starts and never changes under the worker.
struct CompactionInput {
    /// Ordinary compaction carries the sealed prefix itself. Overflow carries
    /// the full surface plus the live tail so its keep-recent calculation sees
    /// everything the provider rejected.
    rows: Vec<WorkingRow>,
    /// Prefix of `rows` that may be rewritten by the commit. Overflow may see
    /// persisted-in-progress or pending rows in the tail; they stay outside the
    /// checkpoint even though they participate in pressure measurement.
    sealed_len: usize,
    prior_checkpoint_seq: Option<Seq>,
    first_seq: Seq,
    last_seq: Seq,
}

impl CompactionInput {
    fn from_rows(rows: &[WorkingRow], in_progress: &HashSet<Seq>) -> Option<Self> {
        let sealed_len = sealed_prefix_len(rows, in_progress);
        if sealed_len == 0 {
            return None;
        }
        Self::from_rows_and_sealed_len(rows[..sealed_len].to_vec(), sealed_len)
    }

    fn from_rows_full(rows: &[WorkingRow], in_progress: &HashSet<Seq>) -> Option<Self> {
        let sealed_len = sealed_prefix_len(rows, in_progress);
        if sealed_len == 0 {
            return None;
        }
        Self::from_rows_and_sealed_len(rows.to_vec(), sealed_len)
    }

    fn from_rows_and_sealed_len(rows: Vec<WorkingRow>, sealed_len: usize) -> Option<Self> {
        let first_seq = rows.first().and_then(|row| row.log_seq)?;
        let prior_checkpoint_seq = rows
            .first()
            .and_then(|row| row.checkpoint().and(row.log_seq));
        let last_seq = rows.get(sealed_len - 1).and_then(|row| row.log_seq)?;
        Some(Self {
            rows,
            sealed_len,
            prior_checkpoint_seq,
            first_seq,
            last_seq,
        })
    }

    fn from_state(state: &WorkingSetState) -> Option<Self> {
        Self::from_rows(&state.rows, &state.in_progress)
    }

    fn from_state_full(state: &WorkingSetState) -> Option<Self> {
        Self::from_rows_full(&state.rows, &state.in_progress)
    }
}

/// The durable result of the one commit. Rebuild reads this back and verifies
/// the surface; the commit is a fact, but success is only published after that
/// verification.
struct CommittedCompaction {
    kind: CompactionCommitKind,
    telemetry: CompactionTelemetry,
    summary: Option<SummaryArtifact>,
    checkpoint_seq: Seq,
}

fn build_summary_plan_from_input(
    input: &CompactionInput,
    budget: &BudgetPolicy,
    request: &CompactionRequestContext,
    summary_text_cap: u32,
    trigger: CompactionTrigger,
) -> Option<CompactionPlan> {
    // Overflow sees the full surface + tail, but a checkpoint can only end on a
    // sealed row. Keep at least one sealed row and leave the tail outside the
    // checkpoint; the final meter includes it and decides whether overflow
    // actually recovered.
    let max_cut = input.sealed_len.checked_sub(1)?;
    if max_cut == 0 {
        return None;
    }
    let transcript = project_items(&input.rows);
    let raw_cut = find_keep_recent_cut(&transcript, budget.keep_recent_tokens)?;
    let cut = if input.prior_checkpoint_seq.is_some() {
        raw_cut.max(1)
    } else {
        raw_cut
    }
    .min(max_cut);
    let kept_from_seq = input.rows.get(cut).and_then(|row| row.log_seq)?;
    Some(CompactionPlan {
        trigger,
        prefix_len: input.rows.len(),
        cut,
        kept_from_seq,
        prior_checkpoint_seq: input.prior_checkpoint_seq,
        output_cap: request.max_output_tokens,
        summary_text_cap,
        at: input.last_seq,
    })
}


fn is_contention_error(err: &LitecodeError) -> bool {
    if let LitecodeError::Sqlite(error) = err
        && matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        )
    {
        return true;
    }
    match err {
        LitecodeError::SessionStorage(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("locked") || lower.contains("busy")
        }
        LitecodeError::SessionBackpressure => true,
        _ => false,
    }
}

fn is_corruption_error(err: &LitecodeError) -> bool {
    if let LitecodeError::Sqlite(error) = err
        && matches!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase)
        )
    {
        return true;
    }
    match err {
        LitecodeError::SessionStorage(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("malformed")
                || lower.contains("corrupt")
                || lower.contains("not a database")
        }
        _ => false,
    }
}

fn is_retryable_write_error(err: &LitecodeError) -> bool {
    if is_contention_error(err) {
        return true;
    }
    match err {
        LitecodeError::SessionStorage(message) => {
            let lower = message.to_ascii_lowercase();
            lower.contains("commit")
                || lower.contains("before begin")
                || lower.contains("after commit")
        }
        _ => false,
    }
}

/// Read the durable working state. Contention is retried a bounded number of
/// times; the data layer already falls back from an anchored fold to the full
/// log when the anchor cannot be proven. A terminal read failure is a compaction
/// failure, not a panic and not a fake success.
fn read_compaction_state_result(
    sessions: &SessionManager,
    session_id: &str,
) -> Result<WorkingSetState> {
    for attempt in 1..=COMPACTION_STORE_RETRIES {
        match sessions.data().working_set_state_blocking(session_id) {
            Ok(state) => return Ok(state),
            Err(error) if is_contention_error(&error) && attempt < COMPACTION_STORE_RETRIES => {
                tracing::warn!(
                    session_id,
                    attempt,
                    error = %error,
                    "compaction surface read is blocked; retrying"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Err(LitecodeError::SessionStorage(
        "compaction surface read kept contending".into(),
    ))
}

fn read_compaction_surface(sessions: &SessionManager, session_id: &str) -> Result<Vec<WorkingRow>> {
    read_compaction_state_result(sessions, session_id).map(|state| state.rows)
}

fn measure_final_surface(
    request: &CompactionRequestContext,
    checkpoint: &CompactionCheckpoint,
    rows_after_cut: &[WorkingRow],
    data_root: &std::path::Path,
) -> ContextMeter {
    let envelope = ContextEnvelope::from_compaction_request(request);
    let mut view = vec![checkpoint.model_item()];
    view.extend(rows_after_cut.iter().map(|row| row.payload.model_item()));
    let view_seqs = vec![None; view.len()];
    let projection =
        super::project_model_view(&view, &view_seqs, None, None, &request.target, data_root);
    ContextMeter::measure_local(
        request.context_window,
        request.target.context_window_max,
        request.max_output_tokens as usize,
        &envelope,
        &projection.items,
    )
}

struct SummaryCommitPlan {
    plan: CompactionPlan,
    checkpoint: CompactionCheckpoint,
    summary: SummaryArtifact,
    first_seq: Seq,
    tokens_after: usize,
    telemetry: CompactionTelemetry,
}

async fn commit_summary_plan(
    sessions: &SessionManager,
    session_id: &str,
    operation_id: &str,
    request: &CompactionRequestContext,
    commit: &SummaryCommitPlan,
) -> Result<Seq> {
    for attempt in 1..=COMPACTION_STORE_RETRIES {
        let mutation = crate::session::data::command::SessionMutation::Compact {
            session_id: session_id.to_string(),
            operation_id: crate::session::data::command::MutationId(operation_id.to_string()),
            summary: commit.checkpoint.summary.clone(),
            token_estimate: commit.tokens_after as i64,
            kept_from: Some(commit.plan.kept_from_seq),
            at: Some(commit.plan.at),
            expected_prior_checkpoint: commit.plan.prior_checkpoint_seq,
            expected_from: commit.first_seq,
            usage: commit.summary.usage.clone(),
            provider_id: Some(request.target.provider_id.clone()),
        };
        match sessions.mutate(mutation).await {
            Ok(receipt) => match receipt.outcome {
                crate::session::data::command::CommitKind::Compacted { seq } => {
                    return Ok(seq);
                }
                other => {
                    return Err(LitecodeError::ToolExecution(format!(
                        "summary compact mutation returned unexpected outcome {other:?}"
                    )));
                }
            },
            Err(error) if is_corruption_error(&error) => {
                panic!("summary commit hit a corrupt session store: {error}");
            }
            Err(error)
                if is_retryable_write_error(&error) && attempt < COMPACTION_STORE_RETRIES =>
            {
                tracing::warn!(
                    attempt,
                    error = %error,
                    "summary commit blocked; retrying"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(LitecodeError::SessionStorage(
        "summary commit kept contending".into(),
    ))
}

fn rebuild_summary_surface(
    sessions: &SessionManager,
    session_id: &str,
    checkpoint_seq: Seq,
) -> Result<Vec<WorkingRow>> {
    let rows = read_compaction_surface(sessions, session_id)?;
    let first = rows.first();
    if first
        .and_then(|row| row.log_seq)
        .is_none_or(|seq| seq != checkpoint_seq)
        || first.and_then(|row| row.checkpoint()).is_none()
    {
        return Err(LitecodeError::CompactionSurfaceChanged(format!(
            "summary commit {checkpoint_seq} is not the first durable checkpoint row for {session_id}"
        )));
    }
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
async fn prepare_summary_commit(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    input: &CompactionInput,
    plan: CompactionPlan,
    tokens_before: usize,
    cancel: &CancellationToken,
    trigger: CompactionTrigger,
    operation_id: &str,
    data_root: &std::path::Path,
    max_attempts: u8,
) -> Result<SummaryCommitPlan> {
    let base_telemetry = compaction_telemetry(trigger, tokens_before, 0, tokens_before, None);
    emit_compact_started(
        sessions,
        session_id,
        trigger,
        Some(operation_id),
        base_telemetry,
        Some(plan.at),
    );
    let (summary, _review, summary_attempts) = match CompactPolicy::first_pass_compaction(
        request,
        plan.output_cap,
        plan.summary_text_cap,
        plan.cut,
        &input.rows,
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
                0,
                tokens_before,
                Some(failure.attempts),
            );
            record_compact_failure(
                sessions,
                session_id,
                trigger,
                Some(operation_id),
                &failure.error,
                telemetry,
            );
            return Err(failure.error);
        }
    };
    if cancel.is_cancelled() {
        let error = LitecodeError::Canceled;
        let telemetry = compaction_telemetry(
            trigger,
            tokens_before,
            0,
            tokens_before,
            Some(summary_attempts),
        );
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            telemetry,
        );
        return Err(error);
    }

    let current = read_compaction_state_result(sessions, session_id)?.rows;
    if current.first().and_then(|row| row.log_seq) != Some(input.first_seq)
        || current
            .first()
            .and_then(|row| row.checkpoint().and(row.log_seq))
            != input.prior_checkpoint_seq
    {
        let error = LitecodeError::CompactionSurfaceChanged(
            "sealed prefix leading identity changed".into(),
        );
        let telemetry = compaction_telemetry(
            trigger,
            tokens_before,
            0,
            tokens_before,
            Some(summary_attempts),
        );
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            telemetry,
        );
        return Err(error);
    }
    let Some(cut) = current
        .iter()
        .position(|row| row.log_seq == Some(plan.kept_from_seq))
    else {
        let error = LitecodeError::CompactionSurfaceChanged(format!(
            "kept_from seq {} left the surface",
            plan.kept_from_seq
        ));
        let telemetry = compaction_telemetry(
            trigger,
            tokens_before,
            0,
            tokens_before,
            Some(summary_attempts),
        );
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            telemetry,
        );
        return Err(error);
    };
    let checkpoint = CompactionCheckpoint {
        summary: summary.text.clone(),
        from: input.first_seq,
        to: plan.kept_from_seq,
    };
    // Measure the same frozen input that selected the method. Ordinary/manual
    // sees only its sealed prefix; overflow sees surface + tail. Rows appended
    // after this plan are not allowed to decide whether this attempt succeeded.
    let rows_after_cut = &input.rows[cut..];
    let final_meter = measure_final_surface(request, &checkpoint, rows_after_cut, data_root);
    let final_count = final_meter.used_tokens;
    if final_meter.is_over_hard_line() {
        let error = LitecodeError::ContextStillOverLimit {
            used: final_count,
            limit: final_meter.hard_limit(),
        };
        let telemetry = compaction_telemetry(
            trigger,
            tokens_before,
            0,
            final_count,
            Some(summary_attempts),
        );
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            telemetry,
        );
        return Err(error);
    }
    if cancel.is_cancelled() {
        let error = LitecodeError::Canceled;
        let telemetry = compaction_telemetry(
            trigger,
            tokens_before,
            0,
            final_count,
            Some(summary_attempts),
        );
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            telemetry,
        );
        return Err(error);
    }
    Ok(SummaryCommitPlan {
        plan,
        checkpoint,
        summary,
        first_seq: input.first_seq,
        tokens_after: final_count,
        telemetry: compaction_telemetry(
            trigger,
            tokens_before,
            0,
            final_count,
            Some(summary_attempts),
        ),
    })
}

async fn commit_plan(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    operation_id: &str,
    trigger: CompactionTrigger,
    cancel: &CancellationToken,
    commit: SummaryCommitPlan,
) -> Result<CommittedCompaction> {
    if cancel.is_cancelled() {
        let error = LitecodeError::Canceled;
        record_compact_failure(
            sessions,
            session_id,
            trigger,
            Some(operation_id),
            &error,
            commit.telemetry.clone(),
        );
        return Err(error);
    }
    let checkpoint_seq =
        match commit_summary_plan(sessions, session_id, operation_id, request, &commit).await {
            Ok(seq) => seq,
            Err(error) => {
                record_compact_failure(
                    sessions,
                    session_id,
                    trigger,
                    Some(operation_id),
                    &error,
                    commit.telemetry.clone(),
                );
                return Err(error);
            }
        };
    Ok(CommittedCompaction {
        kind: CompactionCommitKind::Checkpoint {
            seq: checkpoint_seq,
        },
        telemetry: commit.telemetry,
        summary: Some(commit.summary),
        checkpoint_seq,
    })
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_REBUILD_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn fail_next_rebuild_reads(count: usize) {
    FAIL_NEXT_REBUILD_READS.with(|slot| slot.set(count));
}

#[cfg(test)]
fn injected_rebuild_read_failure() -> Option<LitecodeError> {
    FAIL_NEXT_REBUILD_READS.with(|slot| {
        let remaining = slot.get();
        if remaining == 0 {
            return None;
        }
        slot.set(remaining - 1);
        Some(LitecodeError::SessionStorage(
            "injected committed-surface read failure".into(),
        ))
    })
}

/// Read the committed surface back. This is the delivery gate: a committed
/// compaction is retried on transient reads and only becomes Succeeded after
/// the durable surface is verified.
fn rebuild_committed_surface(
    sessions: &SessionManager,
    session_id: &str,
    committed: &CommittedCompaction,
) -> Result<Vec<WorkingRow>> {
    // Retry read failures a few times before declaring the committed surface
    // undeliverable. The reader itself has already tried the anchored fold and
    // the full durable log; this loop only rides out a transient read-side
    // failure. A validation mismatch is deterministic and is not retried.
    let mut last_error = None;
    for attempt in 1..=3 {
        #[cfg(test)]
        if let Some(error) = injected_rebuild_read_failure() {
            last_error = Some(error);
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let read = rebuild_summary_surface(sessions, session_id, committed.checkpoint_seq);
        match read {
            Ok(rows) => return Ok(rows),
            Err(error)
                if attempt < 3
                    && !matches!(
                        error,
                        LitecodeError::CompactionSurfaceChanged(_)
                            | LitecodeError::InvalidSessionEvent(_)
                    ) =>
            {
                tracing::warn!(
                    session_id,
                    attempt,
                    error = %error,
                    "committed compaction surface read failed; retrying"
                );
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| LitecodeError::SessionStorage("committed surface read failed".into())))
}

#[allow(clippy::too_many_arguments)]
async fn compact_input(
    sessions: &SessionManager,
    session_id: &str,
    request: &CompactionRequestContext,
    budget: &BudgetPolicy,
    input: CompactionInput,
    cancel: &CancellationToken,
    trigger: CompactionTrigger,
    operation_id: &str,
    summary_text_cap: u32,
    data_root: &std::path::Path,
    max_attempts: u8,
) -> Result<Option<CommittedCompaction>> {
    if cancel.is_cancelled() {
        return Err(LitecodeError::Canceled);
    }
    let tokens_before = context_meter_for_rows_local(request, &input.rows, data_root).used_tokens;
    let Some(plan) =
        build_summary_plan_from_input(&input, budget, request, summary_text_cap, trigger)
    else {
        return Ok(None);
    };

    let commit = prepare_summary_commit(
        sessions,
        session_id,
        request,
        &input,
        plan,
        tokens_before,
        cancel,
        trigger,
        operation_id,
        data_root,
        max_attempts,
    )
    .await?;

    let run = commit_plan(
        sessions,
        session_id,
        request,
        operation_id,
        trigger,
        cancel,
        commit,
    )
    .await?;
    Ok(Some(run))
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
    TooLarge {
        summary_tokens: usize,
        cap_tokens: usize,
    },
}

/// One total-attempt budget: first try plus one identical retry.
pub const MAX_SUMMARY_ATTEMPTS: u8 = 2;

/// Bounded retries for a blocked read/write. After this the failure goes back
/// to the session; the pipeline never spins on a busy store.
const COMPACTION_STORE_RETRIES: u8 = 3;
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
        Self::review_with_text_cap(artifact, source_items, u32::MAX)
    }

    /// `text_cap` bounds the final summary text only; it is never a model
    /// request `max_output_tokens`.
    pub fn review_with_text_cap(
        artifact: &SummaryArtifact,
        source_items: &[Item],
        text_cap: u32,
    ) -> SummaryReview {
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
        if summary_tokens > text_cap as usize {
            failures.push(SummaryFailure::TooLarge {
                summary_tokens,
                cap_tokens: text_cap as usize,
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

/// Compaction policy and execution.
pub struct CompactPolicy;

impl CompactPolicy {
    pub fn can_compact(budget: &BudgetPolicy, transcript: &Transcript) -> bool {
        find_keep_recent_cut(transcript, budget.keep_recent_tokens).is_some()
    }

    /// Manual fast-fail against the sealed prefix only. In-progress or pending
    /// tail rows cannot make a manual summary look eligible.
    pub fn can_compact_sealed(
        rows: &[WorkingRow],
        in_progress: &HashSet<Seq>,
        budget: &BudgetPolicy,
    ) -> bool {
        let sealed_len = sealed_prefix_len(rows, in_progress);
        if sealed_len == 0 {
            return false;
        }
        Self::can_compact(budget, &project_items(&rows[..sealed_len]))
    }

    /// Meter for the ordinary/manual auto trigger: only sealed rows count.
    /// The full working-set meter remains the hard send gate; overflow uses
    /// its own full input. `None` means there is no sealed row to compact.
    pub fn measure_sealed_meter(
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        prompt_baseline: &ProviderPromptBaseline,
        data_root: &std::path::Path,
    ) -> Option<ContextMeter> {
        let state = sessions
            .data()
            .working_set_state_blocking(session_id)
            .ok()?;
        let sealed_len = sealed_prefix_len(&state.rows, &state.in_progress);
        if sealed_len == 0 {
            return None;
        }
        Some(Self::measure_meter(
            sessions,
            session_id,
            request,
            prompt_baseline,
            &state.rows[..sealed_len],
            data_root,
        ))
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
        summary_text_cap: u32,
        rows: &[WorkingRow],
    ) -> Result<Option<CompactionPlan>> {
        let in_progress = read_compaction_state_result(sessions, session_id)?.in_progress;
        let Some(input) = CompactionInput::from_rows(rows, &in_progress) else {
            return Ok(None);
        };
        Ok(build_summary_plan_from_input(
            &input,
            budget,
            request,
            summary_text_cap,
            CompactionTrigger::Auto,
        ))
    }

    /// Compatibility/test entry for one sealed-prefix compaction.
    ///
    /// `summary_text_cap` bounds accepted summary text only; the model request
    /// cap always comes from `request.max_output_tokens`. Production episodes
    /// call the same `compact_input` core directly; this wrapper only reattaches
    /// unpersisted test rows after the common rebuild.
    #[allow(clippy::too_many_arguments)]
    pub async fn compact_transcript(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        summary_text_cap: u32,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        cancel: &CancellationToken,
        trigger: CompactionTrigger,
        operation_id: Option<&str>,
        data_root: &std::path::Path,
        max_attempts: u8,
    ) -> Result<Option<SummaryArtifact>> {
        let in_progress = read_compaction_state_result(sessions, session_id)?.in_progress;
        let Some(input) = CompactionInput::from_rows(rows, &in_progress) else {
            return Ok(None);
        };
        let pending_tail: Vec<WorkingRow> = rows
            .iter()
            .filter(|row| row.log_seq.is_none())
            .cloned()
            .collect();
        let operation_id = operation_id
            .map(str::to_string)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let Some(run) = compact_input(
            sessions,
            session_id,
            request,
            budget,
            input,
            cancel,
            trigger,
            &operation_id,
            summary_text_cap,
            data_root,
            max_attempts,
        )
        .await?
        else {
            return Ok(None);
        };
        // The old bill describes a prefix that no longer exists. The runtime
        // clears its own baseline after the episode; this keeps the test-facing
        // wrapper's baseline in step too.
        prompt_baseline.invalidate();
        sessions.set_compaction_terminal_telemetry(
            session_id,
            Some(&operation_id),
            run.telemetry.clone(),
        );
        let mut surface = rebuild_committed_surface(sessions, session_id, &run)?;
        surface.extend(pending_tail);
        *rows = surface;
        Ok(run.summary)
    }

    /// Run the one session-owned episode. Ordinary/manual snapshots the sealed
    /// prefix; overflow snapshots the full surface + live tail. Exactly one
    /// `Compacted` checkpoint may land. The commit is the fact; read-back
    /// is a separate delivery stage. A committed but unverified surface is not
    /// Succeeded; it is reported so the session can try the read. The terminal is
    /// published by `CompactionEpisodeLease::finish` (manager).
    #[allow(clippy::too_many_arguments)]
    pub async fn run_episode(
        lease: &mut CompactionEpisodeLease,
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        request: &CompactionRequestContext,
        trigger: CompactionTrigger,
        data_root: &std::path::Path,
        overflow_rows: Option<Vec<WorkingRow>>,
    ) -> Result<()> {
        let state = match read_compaction_state_result(sessions, session_id) {
            Ok(state) => state,
            Err(error) => {
                lease.finish(EpisodeOutcome::Failed {
                    kind: compact_fail_kind(&error),
                    action: failure_action(trigger, &error),
                    message: format!("compaction input read failed: {error}"),
                });
                return Ok(());
            }
        };
        let operation_id = lease.id().to_string();
        let cancel = lease.cancel_token();
        let input = match trigger {
            CompactionTrigger::Overflow => overflow_rows
                .as_deref()
                .and_then(|rows| CompactionInput::from_rows_full(rows, &state.in_progress))
                .or_else(|| CompactionInput::from_state_full(&state)),
            _ => CompactionInput::from_state(&state),
        };
        let Some(input) = input else {
            let terminal = EpisodeOutcome::Failed {
                kind: CompactionFailKind::NothingToCompact,
                message: "session has no sealed rows to compact".into(),
                action: failure_action(trigger, &LitecodeError::NothingToCompact),
            };
            lease.finish(terminal);
            return Ok(());
        };

        let result = compact_input(
            sessions,
            session_id,
            request,
            budget,
            input,
            &cancel,
            trigger,
            &operation_id,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            data_root,
            MAX_SUMMARY_ATTEMPTS,
        )
        .await;

        let committed = match result {
            Ok(Some(run)) => run,
            Ok(None) => {
                lease.finish(EpisodeOutcome::Failed {
                    kind: CompactionFailKind::NothingToCompact,
                    message: "no sealed-prefix prune or summary cut".into(),
                    action: failure_action(trigger, &LitecodeError::NothingToCompact),
                });
                return Ok(());
            }
            Err(LitecodeError::Canceled) => {
                lease.finish(EpisodeOutcome::Cancelled {
                    reason: CompactionCancelReason::External,
                });
                return Ok(());
            }
            Err(error) => {
                lease.finish(EpisodeOutcome::Failed {
                    kind: compact_fail_kind(&error),
                    action: failure_action(trigger, &error),
                    message: error.to_string(),
                });
                return Ok(());
            }
        };

        // The mutation is the commit fact. Mark it before read-back so an
        // abort still leaves the durable commit visible; delivery is recorded
        // only after the read surface is verified.
        let commit = committed.kind.clone();
        lease.mark_committed(commit.clone());
        sessions.set_compaction_terminal_telemetry(
            session_id,
            Some(&operation_id),
            committed.telemetry.clone(),
        );
        let verified_surface = match rebuild_committed_surface(sessions, session_id, &committed) {
            Ok(rows) => rows,
            Err(error) => {
                // The commit is durable, but its product is not verified yet.
                // Do not report success: let the session try the read. A
                // validation mismatch is a bad surface and stops the turn.
                lease.finish(EpisodeOutcome::Failed {
                    kind: compact_fail_kind(&error),
                    action: readback_failure_action(trigger, &error),
                    message: format!("compaction committed but surface read-back failed: {error}"),
                });
                return Ok(());
            }
        };
        tracing::debug!(
            session_id,
            rows = verified_surface.len(),
            "committed compaction surface delivered"
        );
        lease.mark_delivered();
        lease.finish(EpisodeOutcome::Succeeded { commit });
        Ok(())
    }

    async fn first_pass_compaction(
        request: &CompactionRequestContext,
        output_cap: u32,
        summary_text_cap: u32,
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
        let (prefix, omitted_history) =
            match build_compaction_input(request, sessions, rows, cut, output_cap) {
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
                output_cap,
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

            let review =
                SummaryAcceptance::review_with_text_cap(&artifact, &prefix, summary_text_cap);
            if review.accepted {
                tracing::info!(
                    summary_len = artifact.text.len(),
                    finish = ?artifact.finish,
                    model = %artifact.model,
                    usage = ?artifact.usage,
                    attempt,
                    omitted_history,
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
        output_cap: u32,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<SummaryArtifact> {
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        let session_id = request.session_id.as_str();
        let model = request.model.as_str();
        let provider_request = request.build_model_request(input, output_cap);
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
fn is_user_message(item: &Item) -> bool {
    matches!(
        item,
        Item::Message(MessageItem::Input(message)) if matches!(message.role, InputRole::User)
    )
}

fn build_compaction_input(
    request: &CompactionRequestContext,
    sessions: &SessionManager,
    rows: &[WorkingRow],
    cut: usize,
    output_cap: u32,
) -> Result<(Vec<Item>, bool)> {
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

    // The summary call is a real request with its own physical prompt limit.
    // Keep its input under 80% of that limit, dropping the oldest whole user
    // turns first. Tool calls/outputs and assistant segments are never split
    // because the cut only lands on user-message boundaries.
    let output_cap = output_cap as usize;
    let fixed = ContextEnvelope::from_compaction_request(request)
        .with_output_reserve(output_cap)
        .fixed_prompt_tokens()
        .saturating_add(crate::session::estimate::count_text_tokens(
            crate::config::global_db::COMPACTION_INSTRUCTION,
        ));
    let physical = request
        .target
        .context_window_max
        .saturating_sub(output_cap)
        .max(1);
    let target = crate::session::estimate::compaction_budget(physical).max(1);
    let mut omitted = false;
    if fixed.saturating_add(crate::session::estimate::compute_token_estimate(&input)) > target {
        let starts = std::iter::once(0).chain(
            input
                .iter()
                .enumerate()
                .filter_map(|(index, item)| is_user_message(item).then_some(index)),
        );
        let Some(start) = starts
            .filter(|&start| {
                fixed.saturating_add(crate::session::estimate::compute_token_estimate(
                    &input[start..],
                )) <= target
            })
            .next()
        else {
            return Err(LitecodeError::CompactionInputTooLarge(format!(
                "summary input still exceeds {target} tokens after dropping oldest history"
            )));
        };
        if start > 0 {
            input = input[start..].to_vec();
            input_seqs = input_seqs[start..].to_vec();
            input_rebased = input_rebased[start..].to_vec();
            omitted = true;
        }
    }

    // The summarizer attributes only the segment it is about to summarize, so
    // the window is that segment, not the whole log.
    let origins = match crate::runtime::llm_resolve::item_seq_window(&input_seqs) {
        Some((from_seq, through_seq)) => sessions
            .request_origins(&request.session_id, from_seq, through_seq)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let producers = crate::runtime::llm_resolve::producers_from_origins(origins, &input_seqs);
    // The summarizer sends its own instructions and tool list, so it mints
    // under its own fixed prefix.
    let (instructions_digest, tools_digest) =
        crate::llm::fixed_prefix_digests(&request.instructions, &request.tools);
    let report = crate::llm::port::port(
        &mut input,
        &producers,
        &input_rebased,
        request.target.as_ref(),
        request.thinking,
        !request.tools.is_empty(),
        Some(crate::llm::FixedPrefix {
            instructions_digest: &instructions_digest,
            tools_digest: &tools_digest,
        }),
    );
    if let Some(block) = report.blocked {
        return Err(LitecodeError::Config(format!(
            "model '{}' cannot serve compaction: {}",
            request.target.reference,
            block.message()
        )));
    }
    Ok((input, omitted))
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
thread_local! {}

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

fn failure_action(trigger: CompactionTrigger, err: &LitecodeError) -> CompactionFailureAction {
    // Manual has no turn to stop. Its failure is reported through the
    // lifecycle; an auto turn that happens to share the session must not be
    // stopped by a manual episode's outcome.
    if trigger == CompactionTrigger::Manual {
        return CompactionFailureAction::Continue;
    }
    if trigger == CompactionTrigger::Overflow
        || matches!(
            err,
            LitecodeError::ContextStillOverLimit { .. }
                | LitecodeError::CompactionInputTooLarge(_)
                | LitecodeError::SessionStorage(_)
                | LitecodeError::Sqlite(_)
                | LitecodeError::InvalidSessionEvent(_)
                | LitecodeError::SessionDataClosed
        )
    {
        CompactionFailureAction::StopTurn
    } else {
        CompactionFailureAction::Continue
    }
}

/// A committed surface that cannot be read back is unverified, not success:
/// the session still gets to try the read. A validation mismatch is a bad
/// surface and stops the turn. Manual never stops a turn.
fn readback_failure_action(
    trigger: CompactionTrigger,
    err: &LitecodeError,
) -> CompactionFailureAction {
    if trigger == CompactionTrigger::Manual {
        return CompactionFailureAction::Continue;
    }
    if matches!(
        err,
        LitecodeError::CompactionSurfaceChanged(_) | LitecodeError::InvalidSessionEvent(_)
    ) {
        CompactionFailureAction::StopTurn
    } else {
        CompactionFailureAction::Continue
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
        LitecodeError::CompactionSurfaceChanged(_) => CompactionFailKind::Failed,
        LitecodeError::CompactionInputTooLarge(_) => CompactionFailKind::Failed,
        _ => CompactionFailKind::Failed,
    }
}

/// Publish the one Started event. Terminal events belong to the episode owner
/// (`run_episode`) and are published only after the lease has finished.
fn emit_compact_started(
    sessions: &SessionManager,
    session_id: &str,
    trigger: CompactionTrigger,
    operation_id: Option<&str>,
    telemetry: CompactionTelemetry,
    at: Option<Seq>,
) {
    // The episode was admitted by `try_begin_compaction_episode`; this only
    // updates the display anchor and never creates a second record.
    let (_, id) = sessions.begin_compaction_blocking(session_id, operation_id, trigger, at);
    sessions.publish_internal(
        session_id,
        InternalEvent::CompactionLifecycle {
            trigger,
            stage: CompactionStage::Started,
            operation_id: Some(id),
            fail_kind: None,
            error: None,
            telemetry,
            at,
        },
    );
}

/// Record the terminal telemetry without publishing a terminal. The episode
/// owner publishes exactly one terminal after `finish`; publishing here would
/// make clients see a pre-finish terminal and, on cancellation, a Failed
/// followed by Cancelled.
fn record_compact_failure(
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
    sessions.set_compaction_terminal_telemetry(session_id, operation_id, telemetry);
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

/// Leading persisted rows that are already sealed. The first in-progress row
/// ends the summarisable/rewritable prefix: a seal rewrites that same seq, so a
/// summary that read it would be stale before it commits.
fn sealed_prefix_len(rows: &[WorkingRow], in_progress: &HashSet<Seq>) -> usize {
    let persisted = persisted_prefix_len(rows);
    rows[..persisted]
        .iter()
        .position(|row| row.log_seq.is_some_and(|seq| in_progress.contains(&seq)))
        .unwrap_or(persisted)
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
            context_window: target.context_window,
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

        let output_cap = request.max_output_tokens;
        let req = request.build_model_request(prefix.clone(), output_cap);
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
        assert_eq!(req.max_output_tokens, output_cap);
        assert_eq!(req.thinking, request.thinking);
        assert_eq!(req.purpose, RequestPurpose::Compaction);
        assert_eq!(req.session_id.as_deref(), Some("s1"));
    }

    /// A commit that durable-succeeds but returns AfterCommit is retried with
    /// the same operation id; the journal must return the one receipt instead
    /// of dispatching a second checkpoint.
    #[tokio::test]
    async fn commit_retry_reuses_operation_id_after_after_commit_fault() {
        let (sessions, sid, _header) = open_seeded().await;
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let request = compact_context(Arc::new(TextProvider));
        sessions
            .data()
            .hooks()
            .inject(crate::session::data::FaultKind::AfterCommit);

        let outcome = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            1024,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("retry-after-commit"),
            &sessions.data_root_path(),
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect("AfterCommit must be retried, not surfaced as failure")
        .expect("summary committed");
        assert!(outcome.text.contains("Short checkpoint text"));

        let events = sessions.data().events_blocking(&sid).expect("events");
        let compacted: Vec<_> = events
            .iter()
            .filter(|event| matches!(event.event_type, crate::session::EventType::Compacted))
            .collect();
        assert_eq!(
            compacted.len(),
            1,
            "same operation id must not create two checkpoints"
        );
    }

    /// A committed Compact is rebuilt from DB; pending test rows are reattached
    /// after the common rebuild, never synthesized into the durable surface.
    #[tokio::test]
    async fn compact_persist_rebuilds_from_db_and_keeps_pending_tail() {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session("/p", "default", Some("test/test-primary-model"))
            .await
            .expect("open session");
        let seed = long_seed();
        sessions
            .insert_detail_rows(&sid, &seed)
            .await
            .expect("seed");

        let lease = sessions
            .try_begin_compaction_episode(&sid, CompactionTrigger::Manual)
            .expect("manual episode");
        let op_id = lease.id().to_string();

        let mut rows = sessions
            .data()
            .working_set_blocking(&sid)
            .expect("pre-compact working set");
        let before_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.payload.model_item()))
            .collect();
        rows.push(WorkingRow::pending(user_text("pending-tail-after-prefix")));

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
            Some(&op_id),
            &sessions.data_root_path(),
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect("persist + common rebuild must return Ok");
        assert!(outcome.is_some(), "compaction committed");

        let mem_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.payload.model_item()))
            .collect();
        assert!(
            mem_previews
                .first()
                .is_some_and(|t| t.contains("Short checkpoint text for the main request.")),
            "rebuilt surface must lead with the checkpoint summary, got {mem_previews:?}"
        );
        assert!(
            mem_previews
                .iter()
                .any(|t| t.contains("pending-tail-after-prefix")),
            "rebuilt surface must keep pending tail, got {mem_previews:?}"
        );
        assert_ne!(
            &mem_previews[..before_previews.len().min(mem_previews.len())],
            before_previews.as_slice(),
            "memory must not equal pre-compact snapshot content"
        );
        let checkpoint_row = rows.first().expect("rebuilt checkpoint");
        assert!(checkpoint_row.checkpoint().is_some());
        assert!(
            checkpoint_row.log_seq.is_some(),
            "rebuilt checkpoint must retain the committed seq"
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
            "rebuilt checkpoint seq must match DB truth"
        );
        assert!(matches!(&db_rows[0].payload, WorkingPayload::Checkpoint(_)));
        assert!(
            db_rows.iter().all(|row| row.log_seq.is_some()),
            "DB fold rows must be persisted"
        );

        // The episode owner publishes the terminal after `finish`; the
        // pipeline only records telemetry. No terminal event may be emitted
        // by compact_transcript itself.
        let telemetry = sessions
            .take_compaction_terminal_telemetry(&sid, &op_id)
            .expect("Compact persist must record terminal telemetry");
        assert!(telemetry.tokens_before.is_some());
        assert!(telemetry.tokens_pruned.is_some());
        assert!(telemetry.tokens_after.is_some());
        assert!(telemetry.summary_attempts.is_some());
        assert_eq!(telemetry.overflow_retry, Some(0));

        drop(lease);
        assert!(!sessions.is_compacting_blocking(&sid));
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

    async fn open_seeded() -> (Arc<SessionManager>, String, u64) {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session("/p", "default", Some("test/test-primary-model"))
            .await
            .expect("open session");
        sessions
            .insert_detail_rows(&sid, &long_seed())
            .await
            .expect("seed");
        let header = sessions
            .append_request_origin(
                &sid,
                &serde_json::json!({"schema": 1, "turn": "t", "step": 1}),
            )
            .await
            .expect("header");
        (sessions, sid, header)
    }

    #[tokio::test]
    async fn precommit_summary_failure_keeps_the_old_surface() {
        let (sessions, sid, _) = open_seeded().await;
        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let request = compact_context(Arc::new(InvalidSummaryProvider::new()));
        let mut lease = sessions
            .try_begin_compaction_episode(&sid, CompactionTrigger::Auto)
            .expect("auto episode");
        let id = lease.id().to_string();

        CompactPolicy::run_episode(
            &mut lease,
            &budget,
            &sessions,
            &sid,
            &request,
            CompactionTrigger::Auto,
            &sessions.data_root_path(),
            None,
        )
        .await
        .expect("precommit failure must settle");

        let outcome = sessions
            .last_episode_outcome(&sid, &id)
            .expect("terminal outcome");
        match outcome {
            EpisodeOutcome::Failed { action, .. } => assert_eq!(
                action,
                crate::session::manager::CompactionFailureAction::Continue
            ),
            other => panic!("expected soft precommit failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn committed_surface_readback_retries_a_transient_failure() {
        let (sessions, sid, _) = open_seeded().await;
        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let request = compact_context(Arc::new(TextProvider));
        let mut lease = sessions
            .try_begin_compaction_episode(&sid, CompactionTrigger::Manual)
            .expect("manual episode");
        let id = lease.id().to_string();

        super::fail_next_rebuild_reads(1);
        CompactPolicy::run_episode(
            &mut lease,
            &budget,
            &sessions,
            &sid,
            &request,
            CompactionTrigger::Manual,
            &sessions.data_root_path(),
            None,
        )
        .await
        .expect("transient readback failure must settle");

        let outcome = sessions
            .last_episode_outcome(&sid, &id)
            .expect("terminal outcome");
        assert!(
            matches!(outcome, EpisodeOutcome::Succeeded { .. }),
            "a retried readback must be Succeeded, got {outcome:?}"
        );
    }

    #[test]
    fn readback_failure_action_separates_unverified_from_bad_and_manual() {
        let unreadable = LitecodeError::SessionStorage("read failed".into());
        let bad = LitecodeError::CompactionSurfaceChanged("wrong checkpoint".into());
        assert_eq!(
            readback_failure_action(CompactionTrigger::Auto, &unreadable),
            CompactionFailureAction::Continue,
            "an unreadable commit is unverified, not a turn stop"
        );
        assert_eq!(
            readback_failure_action(CompactionTrigger::Auto, &bad),
            CompactionFailureAction::StopTurn,
            "a readable but wrong surface is bad"
        );
        assert_eq!(
            readback_failure_action(CompactionTrigger::Manual, &bad),
            CompactionFailureAction::Continue,
            "manual has no turn to stop"
        );
    }

    #[tokio::test]
    async fn committed_surface_readback_failure_is_unverified() {
        let (sessions, sid, _) = open_seeded().await;
        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let request = compact_context(Arc::new(TextProvider));
        let mut lease = sessions
            .try_begin_compaction_episode(&sid, CompactionTrigger::Manual)
            .expect("manual episode");
        let id = lease.id().to_string();
        let mut lifecycle = sessions.subscribe(&sid).expect("subscribe");

        super::fail_next_rebuild_reads(3);
        CompactPolicy::run_episode(
            &mut lease,
            &budget,
            &sessions,
            &sid,
            &request,
            CompactionTrigger::Manual,
            &sessions.data_root_path(),
            None,
        )
        .await
        .expect("committed readback failure must settle");

        let outcome = sessions
            .last_episode_outcome(&sid, &id)
            .expect("terminal outcome");
        match outcome {
            EpisodeOutcome::Failed {
                action, message, ..
            } => {
                assert_eq!(
                    action,
                    crate::session::manager::CompactionFailureAction::Continue,
                    "unreadable read-back is unverified, not a turn stop"
                );
                assert!(message.contains("read-back failed"), "got {message}");
            }
            other => panic!("expected unverified readback failure, got {other:?}"),
        }
        assert!(
            sessions
                .data()
                .events_blocking(&sid)
                .unwrap()
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted)),
            "the failure happened after the durable checkpoint landed"
        );

        let mut succeeded = 0;
        let mut failed = 0;
        while let Ok(envelope) = lifecycle.try_recv() {
            if let InternalEvent::CompactionLifecycle { stage, .. } = envelope.event {
                match stage {
                    CompactionStage::Succeeded => succeeded += 1,
                    CompactionStage::Failed => failed += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(succeeded, 0, "unverified commit must not publish success");
        assert_eq!(failed, 1, "exactly one failure terminal");
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
        let (sessions, sid, header) = open_seeded().await;
        sessions
            .record_provider_usage(&sid, header, &bill(1_000))
            .await
            .unwrap();
        let rows = sessions.data().working_set_blocking(&sid).unwrap();
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
        assert!(
            !meter.should_compact(),
            "a provider bill covering the prefix must win over the hot local estimate"
        );
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn summary_retry_reuses_the_identical_request() {
        let (sessions, sid, _header) = open_seeded().await;
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

    #[tokio::test]
    async fn compact_request_uses_main_binding_max_output_and_reserves_it() {
        let (sessions, sid, _) = open_seeded().await;
        let huge: Vec<Item> = (0..3)
            .map(|i| user_text(format!("huge-{i}-{}", "x".repeat(40_000))))
            .collect();
        sessions
            .insert_detail_rows(&sid, &huge)
            .await
            .expect("append huge history");
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();

        let provider = Arc::new(RetrySummaryProvider::new());
        let mut request = compact_context(provider.clone());
        // Main binding: 128k output on a 150k physical window. The old fixed
        // 20,480 cap would leave a much larger prompt and break the sum below.
        set_request_window(&mut request, 150_000, 128_000);
        CompactPolicy::compact_transcript(
            &BudgetPolicy::new(150_000).with_keep_recent_tokens(1),
            &sessions,
            &sid,
            &request,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("main-output-cap"),
            &sessions.data_root_path(),
            2,
        )
        .await
        .expect("summary retry commits")
        .expect("compaction commits");

        let requests = provider.requests();
        assert_eq!(requests.len(), 2, "one empty summary plus one retry");
        for req in requests {
            assert_eq!(
                req.max_output_tokens, 128_000,
                "compact must send the main binding max_output, not a fixed cap"
            );
            let input_tokens = crate::session::estimate::compute_token_estimate(&req.input);
            assert!(
                input_tokens + 128_000 <= 150_000,
                "input={input_tokens} + output=128000 must fit the physical window"
            );
        }
    }

    async fn run_rejecting_summary(
        provider: Arc<dyn LlmProvider>,
    ) -> (LitecodeError, Vec<crate::session::event::SessionEvent>) {
        let (sessions, sid, _header) = open_seeded().await;
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

    #[test]
    fn sealed_prefix_stops_at_the_first_in_progress_row() {
        let rows = vec![
            WorkingRow::persisted(0, user_text("sealed-0")),
            WorkingRow::persisted(1, user_text("in-flight")),
            WorkingRow::persisted(2, user_text("later-sealed")),
        ];
        let in_progress = HashSet::from([1u64]);
        assert_eq!(sealed_prefix_len(&rows, &in_progress), 1);
        assert_eq!(sealed_prefix_len(&rows, &HashSet::new()), 3);
    }

    #[test]
    fn overflow_input_retains_full_rows_but_checkpoint_stays_sealed() {
        let rows = vec![
            WorkingRow::persisted(0, user_text("oldest")),
            WorkingRow::persisted(1, user_text("newest sealed")),
            WorkingRow::pending(user_text("live tail")),
        ];
        let input =
            CompactionInput::from_rows_full(&rows, &HashSet::new()).expect("overflow input");
        assert_eq!(input.rows.len(), 3, "overflow sees surface + tail");
        assert_eq!(input.sealed_len, 2, "commit may end only on sealed rows");
        assert_eq!(input.last_seq, 1, "checkpoint anchor is the sealed tail");

        let plan = build_summary_plan_from_input(
            &input,
            &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
            &compact_context(Arc::new(TextProvider)),
            1_024,
            CompactionTrigger::Overflow,
        )
        .expect("summary plan");
        assert!(
            plan.cut < input.sealed_len,
            "kept_from must be a sealed row, got cut {} of {}",
            plan.cut,
            input.sealed_len
        );
        assert_eq!(plan.kept_from_seq, input.rows[plan.cut].log_seq.unwrap());

        let ordinary = CompactionInput::from_rows(&rows, &HashSet::new()).expect("ordinary input");
        assert_eq!(ordinary.rows.len(), 2);
        assert_eq!(ordinary.sealed_len, 2);
    }

    #[tokio::test]
    async fn overflow_final_meter_includes_the_live_tail() {
        let (sessions, sid, _) = open_seeded().await;
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        rows.push(WorkingRow::pending(user_text(format!(
            "overflow-tail {}",
            "x".repeat(2_000_000)
        ))));
        let input =
            CompactionInput::from_rows_full(&rows, &HashSet::new()).expect("full overflow input");
        let request = compact_context(Arc::new(TextProvider));
        let error = match compact_input(
            &sessions,
            &sid,
            &request,
            &BudgetPolicy::new(128_000).with_keep_recent_tokens(1),
            input,
            &CancellationToken::new(),
            CompactionTrigger::Overflow,
            "overflow-tail-meter",
            1_024,
            &sessions.data_root_path(),
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        {
            Err(error) => error,
            Ok(_) => panic!("a tail that still exceeds the hard line must fail before commit"),
        };
        assert!(
            matches!(error, LitecodeError::ContextStillOverLimit { .. }),
            "expected hard-line failure, got {error:?}"
        );
        assert!(
            !sessions
                .data()
                .events_blocking(&sid)
                .unwrap()
                .iter()
                .any(|event| matches!(event.event_type, crate::session::EventType::Compacted)),
            "a rejected overflow must not write a checkpoint"
        );
    }

    #[test]
    fn summary_input_drops_oldest_units_to_fit_the_physical_cap() {
        let sessions = SessionManager::ephemeral_registry();
        let mut request = compact_context(Arc::new(TextProvider));
        let mut target = (*request.target).clone();
        target.context_window_max = 2_000;
        request.target = Arc::new(target);
        let rows = vec![
            WorkingRow::persisted(0, user_text(format!("old {}", "word ".repeat(5_000)))),
            WorkingRow::persisted(1, user_text("newest user anchor")),
        ];
        let (input, omitted) =
            build_compaction_input(&request, &sessions, &rows, 2, 512).expect("summary input");
        assert!(omitted, "oldest history must be dropped");
        assert!(
            input
                .first()
                .is_some_and(|item| item_text_preview(item).contains("newest user anchor")),
            "the newest unit must survive: {:?}",
            input.iter().map(item_text_preview).collect::<Vec<_>>()
        );
        assert!(
            input
                .iter()
                .all(|item| !item_text_preview(item).contains("old word word")),
            "the oldest unit must not remain"
        );
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
        let (sessions, sid, header) = open_seeded().await;
        sessions
            .record_provider_usage(&sid, header, &bill(9_000))
            .await
            .unwrap();
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();
        let budget = BudgetPolicy::new(10_000);
        let baseline = ProviderPromptBaseline::default();
        let provider = Arc::new(CountingProvider::new());
        let mut request = compact_context(provider.clone());
        set_request_window(&mut request, budget.budget_limit(), 1_024);
        let did = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            1024,
            &baseline,
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Auto,
            Some("provider-bill-compact"),
            &sessions.data_root_path(),
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .unwrap();
        assert!(did.is_some(), "a provider prompt over 80% must compact");
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let compacted = sessions
            .data()
            .events_blocking(&sid)
            .expect("compaction events")
            .into_iter()
            .find(|event| event.event_type == crate::session::event::EventType::Compacted)
            .expect("compacted row");
        let body: crate::session::model::CompactedBody =
            serde_json::from_value(compacted.data).expect("compacted body");
        assert_eq!(
            body.provider_id.as_deref(),
            Some("p"),
            "the summarizer bill must name the provider that paid for it"
        );

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
        assert!(
            !meter.should_compact(),
            "the retired bill must not trigger a second compact"
        );
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dual_window_compaction_uses_product_window_for_budget_and_physical_max_for_input() {
        let sessions = Arc::new(SessionManager::new_for_test(
            Arc::new(TurnGuard::new()),
            String::new(),
        ));
        let sid = sessions
            .open_session("/p", "default", Some("test/test-primary-model"))
            .await
            .expect("open session");

        let mut seed = Vec::with_capacity(17);
        for i in 0..15 {
            seed.push(user_text(format!("old-{i} {}", "word ".repeat(16_000))));
        }
        for j in 0..2 {
            seed.push(user_text(format!("keep-{j} {}", "word ".repeat(20_000))));
        }
        sessions
            .insert_detail_rows(&sid, &seed)
            .await
            .expect("seed");
        let mut rows = sessions.data().working_set_blocking(&sid).unwrap();

        let provider = Arc::new(RetrySummaryProvider::new());
        let mut request = compact_context(provider.clone());
        let mut target = (*request.target).clone();
        target.context_window = 256_000;
        target.context_window_max = 1_000_000;
        target.max_output = 128_000;
        request.target = Arc::new(target);
        request.context_window = 256_000;
        request.max_output_tokens = 128_000;

        let budget = BudgetPolicy::new(request.context_window);
        assert_eq!(budget.keep_recent_tokens, 38_338);

        let compacted = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            &request,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &ProviderPromptBaseline::default(),
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Manual,
            Some("dual-window"),
            &sessions.data_root_path(),
            MAX_SUMMARY_ATTEMPTS,
        )
        .await
        .expect("dual-window compaction must succeed")
        .expect("manual compaction over 276K history must commit");

        assert!(compacted.text.contains("Short checkpoint text"));
        let requests = provider.requests();
        assert!(!requests.is_empty());
        assert!(
            requests[0]
                .input
                .iter()
                .any(|item| item_text_preview(item).contains("old-0 ")),
            "1M physical ceiling must retain old-0 instead of truncating to 256K during summary"
        );

        let checkpoint_text = item_text_preview(&rows[0].payload.model_item());
        assert!(checkpoint_text.contains("<system-reminder>"));
        assert!(checkpoint_text.contains(crate::config::global_db::CHECKPOINT_PREAMBLE.trim()));
        assert!(checkpoint_text.contains("</system-reminder>"));

        let retained_previews = rows[1..]
            .iter()
            .filter_map(|row| row.item())
            .map(|item| {
                let text = item_text_preview(item);
                let head: String = text.chars().take(24).collect();
                (head, crate::session::estimate::item_token_estimate(item))
            })
            .collect::<Vec<_>>();
        assert!(
            retained_previews.iter().any(|(head, _)| head.contains("keep-0 ")),
            "keep-0 missing from retained tail: {retained_previews:?}"
        );
        assert!(
            retained_previews.iter().any(|(head, _)| head.contains("keep-1 ")),
            "keep-1 missing from retained tail: {retained_previews:?}"
        );
        assert!(
            !retained_previews.iter().any(|(head, _)| head.contains("old-")),
            "unexpected old row in retained tail (budget keep_recent={}): {retained_previews:?}",
            budget.keep_recent_tokens
        );
    }
}
