use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::context_pipeline::compact::PreparedCompaction;
use crate::context_pipeline::{
    CompactPolicy, ContextEnvelope, ContextMeter, ContextMeterSource, request_token_breakdown,
};
use crate::llm::ModelRequest;
use crate::platform_knobs::CompactionPrefetchMode;
use crate::runtime::observer::{FailReason, InternalEvent, TurnError, TurnPhase, TurnTokenStats};
use crate::types::{FunctionToolCall, Item, Result, ToolResultData, Transcript};

use crate::agent::AgentDeps;
use crate::telemetry::latency;

use super::AgentRuntime;

/// Seals the session episode when the worker thread ends, including on panic.
/// Normal transitions are emitted by the branch itself; this is the last
/// chance for any path that returns without one, so a dead worker can never
/// leave the session projecting `compacting` forever.
struct BackgroundCompactionGuard {
    sessions: Arc<crate::session::manager::SessionManager>,
    session_id: String,
    operation_id: String,
}

impl Drop for BackgroundCompactionGuard {
    fn drop(&mut self) {
        if self
            .sessions
            .finish_background_compaction(&self.session_id, &self.operation_id)
        {
            crate::context_pipeline::compact::publish_compaction_abandoned(
                &self.sessions,
                &self.session_id,
                &self.operation_id,
            );
        }
    }
}

/// One independent auto-compaction branch. It runs on the worker thread's
/// own current-thread runtime, never on the turn's (which dies with the
/// turn), and owns both the summary call and its session commit.
#[allow(clippy::too_many_arguments)]
async fn run_background_compaction(
    budget: crate::context_pipeline::BudgetPolicy,
    sessions: Arc<crate::session::manager::SessionManager>,
    session_id: String,
    task_request: crate::llm::CompactionRequestContext,
    task_rows: Vec<crate::session::working::WorkingRow>,
    task_cancel: tokio_util::sync::CancellationToken,
    data_root: std::path::PathBuf,
    operation_id: String,
    task_step: u64,
    task_meter: ContextMeter,
) -> Result<super::CompactionPrefetchOutcome> {
    let started = Instant::now();
    let prepared = match CompactPolicy::prepare_compaction_with_attempts(
        &budget,
        &sessions,
        &session_id,
        &task_request,
        crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
        &task_rows,
        &task_cancel,
        &data_root,
        crate::context_pipeline::compact::MAX_SUMMARY_ATTEMPTS,
        true,
        Some(&operation_id),
    )
    .await
    {
        Ok(Some(prepared)) => prepared,
        Ok(None) => {
            tracing::info!(
                step = task_step,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "compaction branch found no cut"
            );
            return Ok(super::CompactionPrefetchOutcome::NoCut);
        }
        Err(crate::types::LitecodeError::Canceled) => {
            return Err(crate::types::LitecodeError::Canceled);
        }
        Err(error) => {
            tracing::warn!(
                step = task_step,
                error = %error,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "compaction branch summary failed"
            );
            return Ok(super::CompactionPrefetchOutcome::Failed);
        }
    };
    // The branch commits its own frozen plan. A concurrent append only
    // changes the revision; retry the same plan on a fresh surface. A
    // retry after a commit that failed during fold reload must recognize
    // its own checkpoint instead of reporting a surface-change rejection.
    let kept_from_seq = prepared.kept_from_seq();
    let checkpoint_written = || -> Result<bool> {
        let rows = sessions.data().working_set_blocking(&session_id)?;
        Ok(rows
            .iter()
            .any(|row| row.checkpoint().is_some_and(|cp| cp.to == kept_from_seq)))
    };
    let mut last_error = None;
    for _ in 0..3 {
        let mut current = match sessions.data().working_set_blocking(&session_id) {
            Ok(rows) => rows,
            Err(error) => return Err(error.into()),
        };
        let baseline = crate::context_pipeline::ProviderPromptBaseline::default();
        match CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &session_id,
            &task_request,
            crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
            &baseline,
            &mut current,
            &task_cancel,
            crate::runtime::observer::CompactionTrigger::Auto,
            Some(operation_id.as_str()),
            &data_root,
            Some(prepared.clone()),
            1,
        )
        .await
        {
            Ok(Some(_)) => {
                tracing::info!(
                    step = task_step,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    used_tokens = task_meter.used_tokens,
                    compact_at = task_meter.compact_at,
                    "compaction branch committed at the trigger boundary"
                );
                return Ok(super::CompactionPrefetchOutcome::Committed);
            }
            Ok(None) => {
                if checkpoint_written().unwrap_or(false) {
                    tracing::info!(
                        step = task_step,
                        "compaction branch found its own committed checkpoint"
                    );
                    return Ok(super::CompactionPrefetchOutcome::Committed);
                }
                tracing::info!(
                    step = task_step,
                    "compaction branch rejected: frozen plan no longer matches the surface"
                );
                crate::context_pipeline::compact::emit_compaction_abandoned(
                    &sessions,
                    &session_id,
                    Some(operation_id.as_str()),
                );
                return Ok(super::CompactionPrefetchOutcome::Rejected);
            }
            Err(crate::types::LitecodeError::Canceled) => {
                crate::context_pipeline::compact::emit_compaction_abandoned(
                    &sessions,
                    &session_id,
                    Some(operation_id.as_str()),
                );
                return Err(crate::types::LitecodeError::Canceled);
            }
            Err(crate::types::LitecodeError::TokenBudgetExceeded) => {
                if checkpoint_written().unwrap_or(false) {
                    return Ok(super::CompactionPrefetchOutcome::Committed);
                }
                return Ok(super::CompactionPrefetchOutcome::Failed);
            }
            Err(error) => {
                if checkpoint_written().unwrap_or(false) {
                    tracing::info!(
                        step = task_step,
                        "compaction branch recovered its committed checkpoint after a commit error"
                    );
                    return Ok(super::CompactionPrefetchOutcome::Committed);
                }
                last_error = Some(error);
            }
        }
    }
    tracing::warn!(
        step = task_step,
        error = ?last_error,
        "compaction branch commit exhausted retries"
    );
    Ok(super::CompactionPrefetchOutcome::Failed)
}

impl AgentDeps for AgentRuntime {
    fn begin_step(&mut self, step: u64) {
        self.set_current_step(step);
        self.emit_step_started(step);
    }

    async fn call_model(&mut self) -> Result<Vec<Item>> {
        if self.is_cancelled() {
            return Err(crate::types::LitecodeError::Canceled);
        }

        let step = self.current_step_value();
        let first = self.call_model_once(step).await;
        let Err(original_error) = first else {
            return first;
        };
        if !crate::types::is_context_overflow_error(&original_error) {
            return Err(original_error);
        }
        if self.is_cancelled() {
            return Err(original_error);
        }

        // Provider-confirmed overflow bypasses the pressure threshold. Retry
        // only when the forced compaction advanced the durable surface; the
        // one-retry budget prevents a loop on an indivisible prompt.
        tracing::warn!(
            error = %original_error,
            "provider confirmed context overflow; attempting one forced compaction recovery"
        );
        // A background worker owns the same session surface. A provider
        // overflow needs the write lock now, so seal the worker first instead
        // of racing two compactions against one revision.
        self.abort_compaction_prefetch().await;
        if self.sessions.has_background_compaction(&self.session_id) {
            tracing::info!("canceling the session background compaction for overflow recovery");
            self.sessions.cancel_background_compaction(&self.session_id);
            self.await_session_background_compaction().await;
        }
        let compaction_request = self.compaction_request_context()?;
        let advanced = match self
            .context_pipeline
            .force_compact_overflow(
                &self.sessions,
                &self.session_id,
                &compaction_request,
                crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
                &self.cancel,
            )
            .await
        {
            Ok(advanced) => advanced,
            Err(crate::types::LitecodeError::Canceled) => {
                return Err(crate::types::LitecodeError::Canceled);
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "forced overflow compaction failed without a surface advance"
                );
                false
            }
        };
        if !advanced {
            tracing::warn!(
                max_overflow_retries =
                    crate::context_pipeline::compact::MAX_CONTEXT_OVERFLOW_RETRIES,
                "overflow recovery made no durable surface progress; returning the original provider error"
            );
            return Err(original_error);
        }

        self.prompt_usage_baseline.clear();
        self.prepare_view(step)?;
        match self.call_model_once(step).await {
            Err(error) if crate::types::is_context_overflow_error(&error) => {
                tracing::warn!(
                    "overflow retry also failed; the single recovery budget is exhausted"
                );
                Err(original_error)
            }
            other => other,
        }
    }

    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
        result_data: &mut Vec<ToolResultData>,
    ) -> Result<()> {
        let step = self.current_step_value();
        self.emit_phase(TurnPhase::ExecutingTools, step);

        // Any tool may write disk (including custom); join pre-tool snapshot first.
        self.await_pending_snapshot_track().await;
        let _tools_lap = latency::lap_step("execute_tools", step);

        let cancel = self.cancel.clone();
        let result = self
            .tool_pipeline
            .as_ref()
            .expect("tool_pipeline not initialized")
            .execute_batch_cancellable(
                tool_uses,
                transcript,
                move || cancel.is_cancelled(),
                result_data,
            )
            .await;
        self.sessions.absorb_touched_files(&self.session_id);
        result
    }

    async fn should_stop(&self, output: &[Item]) -> Result<bool> {
        Ok(should_stop_after_output(output))
    }

    async fn compact_if_needed(&self, step: u64) -> Result<bool> {
        if self.is_cancelled() {
            return Ok(false);
        }

        let compaction_request = self.compaction_request_context()?;

        // Fail-open: a stale-plan settlement error must not abort the turn.
        if let Err(error) = self.sessions.settle_stale_plan(&self.session_id) {
            tracing::warn!(
                session_id = %self.session_id,
                error = %error,
                "settle_stale_plan failed; continuing with current reminders"
            );
        }

        let _lap = latency::lap_step("compact_step", step);
        if self.compaction_prefetch_mode != CompactionPrefetchMode::On {
            self.abort_compaction_prefetch().await;
            self.reset_compaction_episode();
            return self
                .run_compaction_step(
                    step,
                    &compaction_request,
                    None,
                    crate::context_pipeline::compact::MAX_SUMMARY_ATTEMPTS,
                    false,
                )
                .await;
        }

        let (mut meter, mut rows) = self.context_pipeline.measure_working_set(
            &self.sessions,
            &self.session_id,
            &compaction_request,
            &self.prompt_usage_baseline,
        );
        let margin = crate::context_pipeline::limits::compact_prefetch_margin(meter.context_window);
        let hard_stop = meter.must_compact_before_send(margin);

        // Consume a completed branch even when the current meter has fallen
        // below 0.8: the branch already wrote its checkpoint.
        if let Some(outcome) = self.take_ready_compaction_prefetch().await {
            match outcome {
                super::CompactionPrefetchOutcome::Committed => {
                    self.reset_compaction_episode();
                    self.reload_after_branch_commit(&mut rows)?;
                    return Ok(true);
                }
                super::CompactionPrefetchOutcome::Rejected => {
                    self.prefetch_rejected.store(true, Ordering::SeqCst);
                }
                super::CompactionPrefetchOutcome::NoCut => {
                    // No summary content was produced; a later hard stop may
                    // still choose a fresh plan.
                    self.prefetch_rejected.store(true, Ordering::SeqCst);
                }
                super::CompactionPrefetchOutcome::Failed => {
                    self.prefetch_failed.store(true, Ordering::SeqCst);
                }
            }
        }

        // A branch that is still running is independent. Below the hard send
        // line the main loop keeps using the old view; at the hard line it must
        // wait for the branch because the next request cannot be sent without
        // the checkpoint.
        if self.has_compaction_prefetch() {
            if !hard_stop {
                meter.enforce_prompt_limit()?;
                return Ok(false);
            }
            if let Some(outcome) = self.await_compaction_prefetch().await {
                match outcome {
                    super::CompactionPrefetchOutcome::Committed => {
                        self.reset_compaction_episode();
                        self.reload_after_branch_commit(&mut rows)?;
                        return Ok(true);
                    }
                    super::CompactionPrefetchOutcome::Rejected => {
                        self.prefetch_rejected.store(true, Ordering::SeqCst);
                    }
                    super::CompactionPrefetchOutcome::NoCut => {
                        self.prefetch_rejected.store(true, Ordering::SeqCst);
                    }
                    super::CompactionPrefetchOutcome::Failed => {
                        self.prefetch_failed.store(true, Ordering::SeqCst);
                    }
                }
            }
            if self.is_cancelled() {
                return Ok(false);
            }
        }

        // A worker started by an earlier turn still owns the session episode.
        // At the hard line the next request cannot be sent without the
        // checkpoint, so wait for that worker and re-measure the durable
        // surface it may have just rewritten.
        if hard_stop && self.sessions.has_background_compaction(&self.session_id) {
            tracing::info!(
                step,
                "waiting for the session-owned compaction branch at the hard stop"
            );
            self.await_session_background_compaction().await;
            self.prompt_usage_baseline.clear();
            let (next_meter, next_rows) = self.context_pipeline.measure_working_set(
                &self.sessions,
                &self.session_id,
                &compaction_request,
                &self.prompt_usage_baseline,
            );
            meter = next_meter;
            rows = next_rows;
            if !meter.must_compact_before_send(margin) {
                meter.enforce_prompt_limit()?;
                return Ok(true);
            }
        }

        if hard_stop {
            // A failed frozen plan is not silently replaced by a new plan. The
            // only legal replan is a surface-change rejection.
            if self.prefetch_failed.load(Ordering::SeqCst) {
                return Err(crate::types::LitecodeError::TokenBudgetExceeded);
            }
            if self.prefetch_rejected.load(Ordering::SeqCst) {
                // A rejected/no-cut frozen plan is replanned once from the
                // current surface at the hard line. That fresh plan gets the
                // normal synchronous summary budget; the branch's two attempts
                // belonged to the frozen plan, not to this one.
                let committed = self
                    .sync_compaction(
                        step,
                        &compaction_request,
                        crate::context_pipeline::compact::MAX_SUMMARY_ATTEMPTS,
                    )
                    .await?;
                if committed {
                    self.reset_compaction_episode();
                }
                return Ok(committed);
            }
            let committed = self
                .sync_compaction(
                    step,
                    &compaction_request,
                    crate::context_pipeline::compact::MAX_SUMMARY_ATTEMPTS,
                )
                .await?;
            if committed {
                self.reset_compaction_episode();
            }
            return Ok(committed);
        }

        if !meter.should_compact() {
            // No pressure: do not cancel a branch that is still running. If
            // there is no branch, the next pressure episode starts clean.
            if !self.has_compaction_prefetch() {
                self.reset_compaction_episode();
            }
            meter.enforce_prompt_limit()?;
            return Ok(false);
        }

        // One mechanical-prune decision per pressure episode, made at the
        // trigger boundary. A prune that does not relieve pressure writes
        // nothing; the summary plan is then built on the untouched surface.
        if !self.prefetch_attempted.load(Ordering::SeqCst) {
            let pruned = CompactPolicy::prune_once(
                &self.sessions,
                &self.session_id,
                &compaction_request,
                &self.prompt_usage_baseline,
                &mut rows,
                self.context_pipeline.data_root(),
            )?;
            if pruned {
                self.reset_compaction_episode();
                return Ok(true);
            }
            self.start_compaction_prefetch(step, &compaction_request, &rows, &meter);
        }

        // The 0.8 prepare line is not the send line. The branch continues in
        // the background; the main loop keeps using the current view.
        meter.enforce_prompt_limit()?;
        Ok(false)
    }

    fn prepare_view(&mut self, step: u64) -> Result<()> {
        let _lap = latency::lap_step("build_view", step);
        self.context_pipeline.build_view(
            &self.sessions,
            &self.session_id,
            &self.prompt_usage_baseline,
            &self.turn_llm.model,
        )
    }

    fn sync_request_seam(&mut self, step: u64) -> Result<()> {
        let _lap = latency::lap_step("seam", step);
        let mut appended = self.append_seam_reminders(step)?;

        // Queued user messages are drained last: they are the freshest input
        // and must sit closest to the next request. The claim is atomic with
        // the turn's ownership, so a cancel that already marked this turn
        // stopping leaves the queue for the end-of-turn flush.
        if let Some(turn_id) = self.context_pipeline.current_turn_id()
            && let Some(claimed) = self
                .sessions
                .claim_pending_messages_for_turn(&self.session_id, &turn_id)
            && !claimed.is_empty()
        {
            let merged = crate::session::manager::merge_pending(&claimed);
            if let Err(error) = self.sessions.append_user_message_with_mentions(
                &self.session_id,
                merged,
                self.runtime_handle.workspace_root(),
            ) {
                self.sessions
                    .restore_pending_messages(&self.session_id, claimed);
                return Err(crate::types::LitecodeError::Anyhow(error));
            }
            appended = true;
        }

        if appended {
            self.emit_internal(InternalEvent::StepCommitted);
        }
        Ok(())
    }

    fn has_pending_user_messages(&self) -> bool {
        self.sessions.has_pending_messages(&self.session_id)
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn max_steps(&self) -> u32 {
        self.agent_config.max_steps
    }

    fn persist_new(&self, items: &[Item], result_data: &[ToolResultData]) -> Result<bool> {
        let _lap = latency::lap_step("commit_step.persist_new", self.current_step_value());
        let outcome = self.context_pipeline.persist_new(
            &self.sessions,
            &self.session_id,
            items,
            result_data,
        )?;
        if outcome.discarded {
            // 回退 shortened the log; do not append this turn's tail.
            return Ok(true);
        }
        if outcome.committed {
            self.emit_internal(InternalEvent::StepCommitted);
        }
        if !outcome.sealed_seqs.is_empty() {
            self.emit_internal(InternalEvent::BufferRestamp {
                seqs: outcome.sealed_seqs,
            });
        }
        if let Some(patch) = outcome.preview {
            self.emit_internal(InternalEvent::SessionPreviewUpdated {
                preview: patch.user,
                assistant_preview: patch.assistant,
                updated_at: patch.updated_at,
            });
        }
        Ok(false)
    }

    fn emit_todo_progress(&mut self) {
        use crate::session::task_state::TodoStatus;

        let (pending, in_progress, completed, items) = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| {
                let pending = state
                    .todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::Pending)
                    .count();
                let in_progress = state
                    .todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::InProgress)
                    .count();
                let completed = state
                    .todos
                    .iter()
                    .filter(|t| t.status == TodoStatus::Completed)
                    .count();
                let items = state.todos.clone();
                Ok((pending, in_progress, completed, items))
            })
            .unwrap_or((0, 0, 0, vec![]));

        self.emit_internal(InternalEvent::TodoProgress {
            pending,
            in_progress,
            completed,
            items,
        });
    }

    fn emit_plan_changed(&mut self) {
        let active_plan_path = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| {
                Ok(state
                    .active_plan
                    .as_ref()
                    .map(|plan| plan.relative_path.clone()))
            })
            .unwrap_or(None);
        self.emit_internal(InternalEvent::PlanChanged { active_plan_path });
    }
}

impl AgentRuntime {
    pub(crate) fn reset_compaction_episode(&self) {
        self.prefetch_attempted.store(false, Ordering::SeqCst);
        self.prefetch_rejected.store(false, Ordering::SeqCst);
        self.prefetch_failed.store(false, Ordering::SeqCst);
    }

    /// Wait until the session's background worker finishes. Used only at the
    /// hard stop, where the next request cannot be sent without whatever
    /// checkpoint the worker may be about to write.
    async fn await_session_background_compaction(&self) {
        let Some(mut done) = self
            .sessions
            .subscribe_background_compaction(&self.session_id)
        else {
            return;
        };
        loop {
            if *done.borrow_and_update() {
                return;
            }
            if done.changed().await.is_err() {
                return;
            }
        }
    }

    /// The branch commits from the durable surface, so the main loop must
    /// reload that surface and reattach its own unpersisted tail.
    fn reload_after_branch_commit(
        &self,
        rows: &mut Vec<crate::session::working::WorkingRow>,
    ) -> Result<()> {
        // The old provider bill describes the pre-checkpoint prefix. It cannot
        // cover the rewritten surface; measure locally until the next response
        // records a fresh bill.
        self.prompt_usage_baseline.clear();
        let pending: Vec<_> = rows
            .iter()
            .filter(|row| row.log_seq.is_none())
            .cloned()
            .collect();
        let mut loaded = self
            .sessions
            .data()
            .working_set_blocking(&self.session_id)?;
        loaded.extend(pending);
        *rows = loaded;
        Ok(())
    }

    async fn run_compaction_step(
        &self,
        step: u64,
        request: &crate::llm::CompactionRequestContext,
        prepared: Option<PreparedCompaction>,
        max_attempts: u8,
        force: bool,
    ) -> Result<bool> {
        self.context_pipeline
            .compact_step_with(
                &self.sessions,
                &self.session_id,
                request,
                crate::context_pipeline::COMPACT_MAX_OUTPUT_TOKENS,
                &self.prompt_usage_baseline,
                step,
                &self.cancel,
                prepared,
                max_attempts,
                force,
            )
            .await
    }

    async fn sync_compaction(
        &self,
        step: u64,
        request: &crate::llm::CompactionRequestContext,
        max_attempts: u8,
    ) -> Result<bool> {
        self.run_compaction_step(step, request, None, max_attempts, true)
            .await
    }

    fn has_compaction_prefetch(&self) -> bool {
        self.prefetch
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Start the independent compaction branch at the trigger boundary. The
    /// plan is built from the same rows the old foreground compaction would
    /// have used; the branch owns both the summary call and the commit.
    fn start_compaction_prefetch(
        &self,
        step: u64,
        request: &crate::llm::CompactionRequestContext,
        rows: &[crate::session::working::WorkingRow],
        meter: &ContextMeter,
    ) {
        if self.compaction_prefetch_mode != CompactionPrefetchMode::On
            || self.is_cancelled()
            || self.has_compaction_prefetch()
        {
            return;
        }
        // A worker from an earlier turn still owns the session episode. Do not
        // start a second one; this turn will wait for it only at the hard line.
        if self.sessions.has_background_compaction(&self.session_id) {
            tracing::debug!(
                step,
                "background compaction already owns this session; not starting another"
            );
            return;
        }
        if self.prefetch_attempted.swap(true, Ordering::SeqCst) {
            return;
        }
        self.prefetch_rejected.store(false, Ordering::SeqCst);
        self.prefetch_failed.store(false, Ordering::SeqCst);
        let budget = self.context_pipeline.budget_policy();
        let sessions = Arc::clone(&self.sessions);
        let session_id = self.session_id.clone();
        let data_root = self.context_pipeline.data_root().clone();
        let child = self.cancel.child_token();
        let task_cancel = child.clone();
        let task_request = request.clone();
        let task_rows = rows.to_vec();
        let task_step = step;
        let task_meter = meter.clone();
        let operation_id = uuid::Uuid::new_v4().to_string();
        let (outcome_tx, outcome_rx) =
            tokio::sync::oneshot::channel::<Result<super::CompactionPrefetchOutcome>>();
        let guard = BackgroundCompactionGuard {
            sessions: Arc::clone(&sessions),
            session_id: session_id.clone(),
            operation_id: operation_id.clone(),
        };
        if !sessions.begin_background_compaction(&session_id, &operation_id, child.clone()) {
            self.prefetch_attempted.store(false, Ordering::SeqCst);
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(format!("compaction-{session_id}"))
            .spawn(move || {
                let _guard = guard;
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(error) => {
                        tracing::warn!(
                            step = task_step,
                            error = %error,
                            "compaction branch runtime failed"
                        );
                        let _ = outcome_tx.send(Ok(super::CompactionPrefetchOutcome::Failed));
                        return;
                    }
                };
                let outcome = rt.block_on(run_background_compaction(
                    budget,
                    sessions,
                    session_id,
                    task_request,
                    task_rows,
                    task_cancel,
                    data_root,
                    operation_id,
                    task_step,
                    task_meter,
                ));
                let _ = outcome_tx.send(outcome);
            });
        if let Err(error) = spawned {
            tracing::warn!(
                step,
                error = %error,
                "compaction branch thread failed to spawn"
            );
            // The failed spawn dropped the closure, which dropped the guard
            // and released the session registration; let a later step retry.
            self.prefetch_attempted.store(false, Ordering::SeqCst);
            return;
        }
        let mut slot = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            child.cancel();
            self.prefetch_attempted.store(false, Ordering::SeqCst);
            return;
        }
        *slot = Some(super::CompactionPrefetch {
            rx: outcome_rx,
            cancel: child,
            started: Instant::now(),
            step,
        });
        tracing::info!(
            step,
            used_tokens = meter.used_tokens,
            compact_at = meter.compact_at,
            prompt_limit = meter.prompt_limit,
            "compaction branch started at the prepare line"
        );
    }

    async fn take_ready_compaction_prefetch(&self) -> Option<super::CompactionPrefetchOutcome> {
        let (prefetch, ready) = {
            let mut slot = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
            let ready = {
                let Some(prefetch) = slot.as_mut() else {
                    return None;
                };
                match prefetch.rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return None,
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => None,
                }
            };
            (slot.take().expect("slot checked above"), ready)
        };
        let ready_ms = prefetch.started.elapsed().as_millis() as u64;
        match ready {
            Some(Ok(outcome)) => {
                tracing::info!(
                    step = prefetch.step,
                    ready_ms,
                    ?outcome,
                    "compaction branch result taken at the boundary"
                );
                Some(outcome)
            }
            Some(Err(crate::types::LitecodeError::Canceled)) => None,
            Some(Err(error)) => {
                tracing::warn!(
                    step = prefetch.step,
                    error = %error,
                    "compaction branch failed"
                );
                Some(super::CompactionPrefetchOutcome::Failed)
            }
            None => {
                tracing::warn!(
                    step = prefetch.step,
                    "compaction branch ended without a result"
                );
                Some(super::CompactionPrefetchOutcome::Failed)
            }
        }
    }

    async fn await_compaction_prefetch(&self) -> Option<super::CompactionPrefetchOutcome> {
        let prefetch = {
            let mut slot = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
            slot.take()
        };
        let Some(prefetch) = prefetch else {
            return None;
        };
        let ready_ms = prefetch.started.elapsed().as_millis() as u64;
        tracing::info!(
            step = prefetch.step,
            ready_ms,
            "waiting for the in-flight compaction branch at the hard stop"
        );
        match prefetch.rx.await {
            Ok(Ok(outcome)) => Some(outcome),
            Ok(Err(crate::types::LitecodeError::Canceled)) => None,
            Ok(Err(error)) => {
                tracing::warn!(
                    step = prefetch.step,
                    error = %error,
                    "in-flight compaction branch failed"
                );
                Some(super::CompactionPrefetchOutcome::Failed)
            }
            Err(error) => {
                tracing::warn!(
                    step = prefetch.step,
                    error = %error,
                    "in-flight compaction branch ended without a result"
                );
                Some(super::CompactionPrefetchOutcome::Failed)
            }
        }
    }

    pub(crate) async fn abort_compaction_prefetch(&self) {
        let prefetch = {
            let mut slot = self.prefetch.lock().unwrap_or_else(|e| e.into_inner());
            slot.take()
        };
        let Some(prefetch) = prefetch else {
            return;
        };
        prefetch.cancel.cancel();
        let _ = prefetch.rx.await;
        tracing::debug!(step = prefetch.step, "compaction prefetch joined");
    }

    async fn call_model_once(&mut self, step: u64) -> Result<Vec<Item>> {
        if self.is_cancelled() {
            return Err(crate::types::LitecodeError::Canceled);
        }

        self.emit_phase(TurnPhase::CallingLlm, step);
        let build_lap = latency::lap_step("call_model.build", step);

        // `compact_if_needed` already ran `prepare_step` and stored the ephemeral PreparedView.
        let view = self.context_pipeline.take_prepared_view().ok_or_else(|| {
            crate::types::LitecodeError::Llm(
                "no prepared PreparedView 鈥?prepare_step must run before call_model".into(),
            )
        })?;

        let instructions = view
            .instructions
            .clone()
            .unwrap_or_else(|| self.instructions.clone());
        // Fail closed before request build when Items require unsupported modalities.
        crate::runtime::validate_llm_input_capabilities(&view.items, &self.turn_llm.model)?;
        let token_count = view.token_count;
        let context_meter = view.context_meter;
        let (request, origin_seq) = self.build_model_request(
            &instructions,
            view.items,
            view.item_seqs,
            &view.rebased,
            token_count,
            view.projected_parts,
            context_meter,
        )?;
        build_lap.done();
        // Turn entry -> first request leaves this process (once per turn).
        latency::report_since(
            "turn_to_first_request",
            self.latency_turn_start.take(),
            None,
        );
        // Whole provider round trip incl. streaming; not part of our overhead.
        let _llm_lap = latency::lap_step("call_model.llm", step);

        // Default path: Responses SSE via complete_with_stream_events 鈫?authority
        // ResponseStreamEvent; observer forwards InternalEvent::StreamEvent.
        // The Chat Completions codec emits projected ResponseStreamEvents with
        // turn-stable ids; preferred path remains Responses SSE (R2).
        self.call_model_complete(&request, token_count, origin_seq)
            .await
    }

    /// Sense the seam and append one row per reminder. Subagent delivery is
    /// fail-closed: a write error puts the completions back and stops the turn.
    /// Other reminder writes are fail-open so a decoration cannot abort the turn.
    fn append_seam_reminders(&mut self, step: u64) -> Result<bool> {
        use crate::reminder::{BashExitEntry, PlanPointer, RunningBash, SettledChild, TodoSnap};
        use crate::reminder::{
            Facts, MemoryFacts, MemorySnap, ReminderKind, SeamCtx, TaskFacts, sync,
        };
        use crate::terminal::display_output_path;

        let mut stages = latency::Stages::new("seam.sources", Some(step));
        let cwd = self.base_ctx.cwd.clone();
        let ctx = SeamCtx { cwd: cwd.clone() };

        let jobs = &self.runtime_handle.ide.terminal.jobs;
        let bash_notices = jobs.take_mailbox(&self.session_id);
        let bash_running = jobs.running(&self.session_id);
        stages.mark("bash");
        let completions = self
            .runtime_handle
            .subagent_hub
            .take_completions(&self.session_id);
        // WP-090 K5: non-waking ChildStopped/ChildDeleted notices ride the
        // same subagent_settled reminder and restore on the same terms.
        let parent_notices = self
            .runtime_handle
            .subagent_hub
            .take_notices(&self.session_id);
        let custom_notices = self
            .runtime_handle
            .custom_tool_hub
            .take_mailbox(&self.session_id);
        stages.mark("subagent_and_custom_mailbox");

        let changed = self.sessions.take_changed_files(&self.session_id);
        let changed_paths = changed
            .iter()
            .map(|path| display_output_path(path, &cwd))
            .collect::<Vec<_>>();
        stages.mark("changed_files");

        let task_state = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| Ok(state.clone()))
            .unwrap_or_default();
        let plan_disk_revision = task_state.active_plan.as_ref().and_then(|plan| {
            let path = self
                .sessions
                .plan_dir_path()
                .join(format!("{}.md", plan.slug));
            crate::session::task_state::plan_file_revision(&path)
        });
        stages.mark("task_state_and_plan");
        let counts = self.sessions.child_counts(&self.session_id);
        stages.mark("child_counts");
        let running_bash = bash_running
            .iter()
            .map(|job| RunningBash {
                job_id: job.id.clone(),
                command: job.command_preview.clone(),
                output_file: display_output_path(&job.output_path, &cwd),
            })
            .collect::<Vec<_>>();
        let bash_exits = bash_notices
            .iter()
            .map(|notice| BashExitEntry {
                job_id: notice.bash_id.clone(),
                command: notice.command_preview.clone(),
                exit_code: notice.exit_code.map(|code| code as i32).unwrap_or(-1),
                killed: notice.user_killed,
                output_file: display_output_path(&notice.output_path, &cwd),
            })
            .collect::<Vec<_>>();
        let settled = completions
            .iter()
            .map(|completion| {
                let (agent, _) = crate::tools::subagent::status::session_labels(
                    &self.sessions,
                    &completion.child_session_id,
                );
                let reason = self
                    .sessions
                    .data()
                    .turn_result_blocking(&completion.child_session_id, &completion.turn_id)
                    .map(|result| result.reason)
                    .unwrap_or_else(|_| "unknown".into());
                SettledChild {
                    child_session_id: completion.child_session_id.clone(),
                    turn_id: completion.turn_id.clone(),
                    agent: agent.unwrap_or_default(),
                    reason,
                }
            })
            .collect::<Vec<_>>();
        let mut settled = settled;
        for notice in &parent_notices {
            let (agent, _) = crate::tools::subagent::status::session_labels(
                &self.sessions,
                notice.child_session_id(),
            );
            settled.push(SettledChild {
                child_session_id: notice.child_session_id().to_string(),
                turn_id: String::new(),
                agent: agent.unwrap_or_default(),
                reason: parent_notice_reason(notice).to_string(),
            });
        }
        let settled_detail = if completions.is_empty() {
            String::new()
        } else {
            let mut detail =
                crate::tools::subagent::status::format_batch_results(&self.sessions, &completions);
            for notice in &parent_notices {
                detail.push_str(&format!(
                    "\nchild_session_id: {} · {}\n",
                    notice.child_session_id(),
                    parent_notice_reason(notice)
                ));
            }
            detail
        };
        let custom_tool_settled = custom_notices
            .iter()
            .map(|notice| {
                let (status, detail) = match &notice.outcome {
                    crate::tools::custom_hub::CustomToolOutcome::Ok { output } => {
                        ("ok".to_string(), output.clone())
                    }
                    crate::tools::custom_hub::CustomToolOutcome::Error { message } => {
                        ("error".to_string(), message.clone())
                    }
                    crate::tools::custom_hub::CustomToolOutcome::Cancelled => {
                        ("cancelled".to_string(), String::new())
                    }
                };
                crate::reminder::CustomToolSettledEntry {
                    job_id: notice.job_id.clone(),
                    call_id: notice.call_id.clone(),
                    tool_name: notice.tool_name.clone(),
                    status,
                    detail,
                }
            })
            .collect::<Vec<_>>();
        stages.mark("settled_and_custom_format");

        let memory = match self.sessions.load_memory(&self.session_id) {
            Ok(memory) => memory,
            Err(error) => {
                tracing::warn!(
                    session_id = %self.session_id,
                    error = %error,
                    "failed to load session memory for reminder"
                );
                Default::default()
            }
        };
        stages.mark("session_memory");

        let view = match self.sessions.spine_reminder_view(&self.session_id) {
            Ok(view) => view,
            Err(error) => {
                self.restore_seam_sources(&bash_notices, completions, &custom_notices);
                self.restore_parent_notices(&parent_notices);
                // ISSUE-032: take_changed_files already advanced snaps; undo so
                // the next seam can still report these paths.
                if !changed.is_empty() {
                    self.sessions
                        .restore_changed_files(&self.session_id, &changed);
                }
                return Err(error);
            }
        };
        let facts = Facts {
            tasks: TaskFacts {
                todos: task_state
                    .todos
                    .iter()
                    .map(|todo| TodoSnap {
                        id: todo.id.clone(),
                        content: todo.content.clone(),
                        status: match todo.status {
                            crate::session::task_state::TodoStatus::Pending => "pending",
                            crate::session::task_state::TodoStatus::InProgress => "in_progress",
                            crate::session::task_state::TodoStatus::Completed => "completed",
                        }
                        .to_string(),
                        priority: todo.priority.clone(),
                    })
                    .collect(),
                active_plan: task_state.active_plan.as_ref().map(|plan| PlanPointer {
                    relative_path: plan.relative_path.clone(),
                    slug: plan.slug.clone(),
                }),
                plan_disk_revision,
                plan_seen_revision: task_state
                    .active_plan
                    .as_ref()
                    .and_then(|plan| plan.revision.clone()),
            },
            memory: MemoryFacts {
                entries: memory
                    .entries
                    .iter()
                    .map(|entry| MemorySnap {
                        id: entry.id.clone(),
                        text: entry.text.clone(),
                    })
                    .collect(),
            },
            background: crate::reminder::BackgroundFacts {
                running_bash: running_bash.clone(),
                children_running: counts.running,
                children_idle: counts.total.saturating_sub(counts.running),
            },
            bash_exits,
            bash_running: running_bash,
            settled,
            settled_detail,
            custom_tool_settled,
            changed_paths,
        };
        stages.mark("spine_reminder_view");
        let reminders = sync(&ctx, &view, &facts);
        stages.mark("sync");
        stages.note("reminders", reminders.len());
        let mut bash_written = false;
        let mut subagent_written = false;
        let mut custom_written = false;
        let mut files_changed_written = false;
        let mut appended = false;
        for reminder in &reminders {
            match self.sessions.append_reminder(&self.session_id, reminder) {
                Ok(_) => {
                    appended = true;
                    match reminder.kind() {
                        ReminderKind::BashExit => bash_written = true,
                        ReminderKind::SubagentSettled => subagent_written = true,
                        ReminderKind::CustomToolSettled => custom_written = true,
                        ReminderKind::FilesChanged => files_changed_written = true,
                        _ => {}
                    }
                }
                Err(error) => {
                    if reminder.kind() == ReminderKind::SubagentSettled {
                        if !bash_written {
                            self.restore_bash_mailbox(&bash_notices);
                        }
                        if !custom_written {
                            self.restore_custom_mailbox(&custom_notices);
                        }
                        if !changed.is_empty() && !files_changed_written {
                            self.sessions
                                .restore_changed_files(&self.session_id, &changed);
                        }
                        self.runtime_handle
                            .subagent_hub
                            .restore_completions(&self.session_id, completions.clone());
                        self.restore_parent_notices(&parent_notices);
                        return Err(crate::types::LitecodeError::Anyhow(error));
                    }
                    tracing::warn!(
                        session_id = %self.session_id,
                        kind = reminder.kind().wire(),
                        error = %error,
                        "failed to persist reminder"
                    );
                }
            }
        }
        if !bash_written {
            self.restore_bash_mailbox(&bash_notices);
        }
        if !subagent_written && !completions.is_empty() {
            self.runtime_handle
                .subagent_hub
                .restore_completions(&self.session_id, completions);
        }
        if !subagent_written {
            self.restore_parent_notices(&parent_notices);
        }
        if !custom_written {
            self.restore_custom_mailbox(&custom_notices);
        }
        // ISSUE-032: soft restore like bash/custom when FilesChanged was not
        // persisted (append warn path or sync omitted the kind).
        if !changed.is_empty() && !files_changed_written {
            self.sessions
                .restore_changed_files(&self.session_id, &changed);
        }
        stages.mark("append_reminders");
        Ok(appended)
    }

    fn restore_seam_sources(
        &self,
        notices: &[crate::terminal::ExitNotice],
        completions: Vec<crate::tools::subagent::CompletionRef>,
        custom_notices: &[crate::tools::custom_hub::CustomToolExitNotice],
    ) {
        self.restore_bash_mailbox(notices);
        if !completions.is_empty() {
            self.runtime_handle
                .subagent_hub
                .restore_completions(&self.session_id, completions);
        }
        self.restore_custom_mailbox(custom_notices);
    }

    fn restore_parent_notices(&self, notices: &[crate::tools::subagent::ParentNotice]) {
        if notices.is_empty() {
            return;
        }
        self.runtime_handle
            .subagent_hub
            .restore_notices(&self.session_id, notices.to_vec());
    }

    fn restore_custom_mailbox(&self, notices: &[crate::tools::custom_hub::CustomToolExitNotice]) {
        if notices.is_empty() {
            return;
        }
        self.runtime_handle
            .custom_tool_hub
            .restore_mailbox(&self.session_id, notices.to_vec());
    }

    fn restore_bash_mailbox(&self, notices: &[crate::terminal::ExitNotice]) {
        if notices.is_empty() {
            return;
        }
        self.runtime_handle
            .ide
            .terminal
            .jobs
            .restore_mailbox(&self.session_id, notices.to_vec());
    }

    fn emit_llm_request_built(
        &self,
        request: &ModelRequest,
        token_count: usize,
        health: &LlmRequestHealth,
        context_meter: &ContextMeter,
        context_envelope: &ContextEnvelope,
    ) {
        // `token_estimate` is local budget telemetry only — never meter/ring truth.
        self.emit_internal(InternalEvent::LlmRequestBuilt {
            model: request.model.clone(),
            endpoint: self.provider().endpoint().to_string(),
            token_estimate: token_count,
            tools_count: request.tools.len(),
            context_window: self.turn_llm.context_window,
            token_breakdown: request_token_breakdown(request),
            context_envelope: context_envelope.clone(),
            context_meter: context_meter.clone(),
            sticky_model_id: health.sticky_model_id.clone(),
            turn_model_ref: health.turn_model_ref.clone(),
            turn_provider_id: health.turn_provider_id.clone(),
            turn_endpoint_type: health.turn_endpoint_type.clone(),
            bind_matches_sticky: health.bind_matches_sticky,
            origin_provider_id: health.origin_provider_id.clone(),
            origin_endpoint_type: health.origin_endpoint_type.clone(),
            origin_model_ref: health.origin_model_ref.clone(),
            ciphertexts_stripped: health.ciphertexts_stripped,
            rebased_stripped: health.rebased_stripped,
            reasoning_dropped: health.reasoning_dropped,
            reasoning_wholly_omitted: health.reasoning_wholly_omitted,
            rebased_true_count: health.rebased_true_count,
            projection_rewrote: health.projection_rewrote,
            severity: health.severity.clone(),
            code: health.code.clone(),
            message: health.message.clone(),
        });
        // Every current code is debug-only: a switch or a foreign strip is
        // evidence for the log, not a reason to interrupt the user. If a code
        // ever becomes user-visible, route it through the durable
        // `model/changed` report instead of reviving a toast here.
    }

    pub(crate) fn build_model_request(
        &self,
        instructions: &str,
        input: Vec<Item>,
        item_seqs: Vec<Option<crate::session::event::Seq>>,
        rebased: &[bool],
        token_count: usize,
        projected_parts: u32,
        context_meter: ContextMeter,
    ) -> Result<(ModelRequest, crate::session::event::Seq)> {
        let tool_schemas = self.rctx().tool_defs();

        let tool_names: Vec<&str> = tool_schemas.iter().map(|t| t.name.as_str()).collect();
        let model = self.turn_llm.api_model_id.clone();
        // Producers are read before this request's own header is appended.
        // Ciphertext replay strips foreign blobs and keeps the text. Summary
        // replay (Bailian) drops every reasoning item this provider did not mint,
        // because that input schema requires the id on the item.
        let provider_id = self.turn_llm.provider_id.clone();
        let endpoint_type = self.turn_llm.model.endpoint_type.as_str().to_string();
        let model_ref = self.turn_llm.model_ref.clone();
        let context_envelope = ContextEnvelope::from_request_parts(
            instructions,
            &tool_schemas,
            self.turn_llm.max_tokens,
        );
        let producers = self.llm_input_producers(&item_seqs);
        let mut input = input;
        let rebased_true_count = rebased.iter().filter(|flag| **flag).count() as u32;
        // One semantic gate. The port decides what this target may receive and
        // counts every removal; codecs below only spell what survives.
        let mut report = crate::llm::port::port(
            &mut input,
            &producers,
            rebased,
            &self.turn_llm.model,
            crate::platform_knobs::ThinkingSpec::Tier(self.turn_llm.thinking_tier),
            !tool_schemas.is_empty(),
        );
        if let Some(block) = report.blocked {
            return Err(crate::types::LitecodeError::Config(format!(
                "model '{}' cannot serve this turn: {}",
                model_ref,
                block.message()
            )));
        }
        report.projected_parts = projected_parts;
        let ciphertexts_stripped = report.ciphertext_stripped;
        let rebased_stripped = report.signatures_stripped;
        let reasoning_dropped = report.reasoning_dropped;
        let reasoning_wholly_omitted = 0u32;
        // Origin is an append-only control-plane row written before the request:
        // items this request produces inherit its provider.
        let origin_seq = self.record_request_origin(&context_envelope)?;
        tracing::info!(
            target: "litecode.debug.llm_request",
            session_id = %self.session_id,
            step = self.current_step_value(),
            model = %model,
            endpoint = %self.provider().endpoint(),
            provider_id = %provider_id,
            origin_seq,
            ciphertexts_stripped,
            rebased_stripped,
            reasoning_dropped,
            reasoning_wholly_omitted,
            tools_count = tool_names.len(),
            tools = ?tool_names,
            item_count = input.len(),
            token_count = token_count,
            instructions_len = instructions.len(),
            instructions_fp = %format!("{:016x}", instructions_fingerprint(instructions)),
            "LLM request built"
        );

        let sticky_model_id = self
            .sessions
            .session_model_id(&self.session_id)
            .unwrap_or_default();
        let bind_matches_sticky = sticky_model_id == model_ref;
        // The pipeline counts every part it rewrote; a switch to a text-only
        // model is visible instead of silent.
        let projection_rewrote = report.projected_parts > 0;
        let health = classify_llm_request_health(LlmRequestHealthInput {
            sticky_model_id,
            turn_model_ref: model_ref.clone(),
            turn_provider_id: provider_id.clone(),
            turn_endpoint_type: endpoint_type.clone(),
            bind_matches_sticky,
            origin_provider_id: provider_id.clone(),
            origin_endpoint_type: endpoint_type.clone(),
            origin_model_ref: model_ref.clone(),
            ciphertexts_stripped,
            rebased_stripped,
            // The port now counts a wholly-unusable reasoning item as a
            // drop, so the old separate counter has no producer.
            reasoning_dropped,
            reasoning_wholly_omitted,
            rebased_true_count,
            projection_rewrote,
        });

        let request = ModelRequest {
            model,
            instructions: instructions.to_string(),
            input,
            tools: tool_schemas,
            max_output_tokens: self.turn_llm.max_tokens,
            thinking: crate::platform_knobs::ThinkingSpec::Tier(self.turn_llm.thinking_tier),
            // Session binding only — never agent.model_ref (decoupled sticky model).
            // No turn-level JSON intent: `model.json_output` is a capability the
            // codec gates on, not a per-turn instruction.
            json_output: false,
            session_id: Some(self.session_id.clone()),
            purpose: crate::llm::RequestPurpose::Main,
        };
        self.emit_llm_request_built(
            &request,
            token_count,
            &health,
            &context_meter,
            &context_envelope,
        );
        Ok((request, origin_seq))
    }

    /// The provider that produced each input item, through its durable seq and the
    /// session's `request/header` rows. Unreadable history degrades to "unknown".
    /// Ciphertext replay then drops only the ciphertext. Summary replay drops the
    /// reasoning item: that host requires an id this provider minted.
    fn llm_input_producers(
        &self,
        item_seqs: &[Option<crate::session::event::Seq>],
    ) -> Vec<Option<crate::llm::ItemProducer>> {
        crate::runtime::llm_resolve::producers_from_origins(
            self.sessions
                .request_origins(&self.session_id)
                .unwrap_or_default(),
            item_seqs,
        )
    }

    /// Append the request boundary before sending, so the next durable Item row
    /// inherits exactly this turn/step's provider.
    fn record_request_origin(
        &self,
        context_envelope: &ContextEnvelope,
    ) -> Result<crate::session::event::Seq> {
        let turn_id = self.context_pipeline.current_turn_id().ok_or_else(|| {
            crate::types::LitecodeError::Llm(
                "request origin cannot be recorded without a turn id".into(),
            )
        })?;
        let step = self.current_step_value();
        let record = serde_json::json!({
            "schema": 1,
            "turn": turn_id,
            "step": step,
            "endpoint_type": self.turn_llm.model.endpoint_type.as_str(),
            "provider_id": self.turn_llm.provider_id,
            "model_ref": self.turn_llm.model_ref,
            "context_envelope": context_envelope,
        });
        self.sessions
            .append_request_origin(&self.session_id, &record)
    }

    /// Items in/out via `complete_with_stream_events` (Responses SSE by default).
    ///
    /// Usage from `response.completed` **replaces** `turn_token_stats` (last request
    /// only — never sum across tool-loop steps). The provider count is tied to the
    /// exact request Item count; the next step locally estimates only appended Items.
    pub(crate) async fn call_model_complete(
        &mut self,
        request: &ModelRequest,
        token_count: usize,
        origin_seq: crate::session::event::Seq,
    ) -> Result<Vec<Item>> {
        // LlmRequestBuilt (+ toast SnapshotNotice) already emitted in build_model_request.
        // Split borrows so the stream closure can mutate token meters while the
        // provider call borrows the (disjoint) binding fields.
        let step = self.current_step_value();
        let observer = std::sync::Arc::clone(&self.observer);
        let sessions = Arc::clone(&self.sessions);
        let session_id = self.session_id.clone();
        let provider = &self.turn_llm.provider;
        let api_key = &self.turn_llm.api_key;
        let stats = &mut self.turn_token_stats;
        let totals = &mut self.turn_usage_totals;
        let prompt_usage_baseline = &self.prompt_usage_baseline;
        let covered_seq = origin_seq;
        let meter_context_window = self.turn_llm.context_window;
        let meter_model_context_window = self.turn_llm.model.context_window_max;
        let meter_output_reserve = request.max_output_tokens as usize;
        let usage_slot = std::sync::Arc::new(std::sync::Mutex::new(None::<TurnTokenStats>));
        let usage_slot_cb = std::sync::Arc::clone(&usage_slot);
        // The stream and the terminal payload describe the same items. This is
        // the only thing that decides which `seq` they share, so an item that
        // streams and then completes stays one row in the log.
        let turn_id = self.context_pipeline.current_turn_id().ok_or_else(|| {
            crate::types::LitecodeError::Llm(
                "call_model_complete without a turn id: a streamed row must belong to a turn"
                    .into(),
            )
        })?;
        let projection = super::stream_projection::StreamProjection::new(
            Arc::clone(&sessions),
            Arc::clone(&observer),
            session_id.clone(),
            turn_id,
        );
        let stream_projection = Arc::clone(&projection);
        let on_event: Option<Box<dyn FnMut(crate::types::StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                // The raw event stays on the runtime bus for turn progress and
                // diagnosis. It is not a second body: what the client shows is the
                // row this fold writes, so the fold is what has to be durable.
                observer.on_internal(InternalEvent::StreamEvent(ev.clone()));
                stream_projection.observe(&ev);
                if let crate::types::StreamEvents::ResponseCompleted(cev) = &ev {
                    if let Some(usage) = &cev.response.usage {
                        let prompt = usage.input_tokens as u64;
                        let completion = usage.output_tokens as u64;
                        let cache_hit = usage.input_tokens_details.cached_tokens as u64;
                        let cache_miss = usage
                            .input_tokens
                            .saturating_sub(usage.input_tokens_details.cached_tokens)
                            as u64;
                        // Last request only — each LLM call sends the full context.
                        let recorded = TurnTokenStats {
                            prompt_tokens: prompt,
                            completion_tokens: completion,
                            cache_hit_tokens: cache_hit,
                            cache_miss_tokens: cache_miss,
                        };
                        *stats = recorded.clone();
                        if let Ok(mut slot) = usage_slot_cb.lock() {
                            *slot = Some(recorded);
                        }
                        // Turn-total Σ — every request in this tool loop (session cum_*).
                        totals.prompt_tokens = totals.prompt_tokens.saturating_add(prompt);
                        totals.completion_tokens =
                            totals.completion_tokens.saturating_add(completion);
                        totals.cache_hit_tokens = totals.cache_hit_tokens.saturating_add(cache_hit);
                        totals.cache_miss_tokens =
                            totals.cache_miss_tokens.saturating_add(cache_miss);
                        // Provider truth covers items at or before this request header.
                        // The next step estimates only rows persisted after it.
                        prompt_usage_baseline.record(prompt, covered_seq);
                        let stop_reason = match &cev.response.incomplete_details {
                            Some(d) => d.reason.clone(),
                            None => format!("{:?}", cev.response.status),
                        };
                        tracing::info!(
                            target: "litecode.debug.llm_usage",
                            session_id = %session_id,
                            step,
                            prompt_tokens = prompt,
                            completion_tokens = completion,
                            cache_hit_tokens = cache_hit,
                            cache_miss_tokens = cache_miss,
                            "LLM request completed"
                        );
                        observer.on_internal(InternalEvent::LlmCompleted {
                            prompt_tokens: prompt,
                            completion_tokens: completion,
                            cache_hit_tokens: cache_hit,
                            cache_miss_tokens: cache_miss,
                            stop_reason,
                            context_meter: ContextMeter::new(
                                meter_context_window,
                                meter_model_context_window,
                                meter_output_reserve,
                                prompt as usize,
                                ContextMeterSource::Provider,
                            ),
                        });
                    } else {
                        tracing::info!(
                            target: "litecode.debug.llm_usage",
                            session_id = %session_id,
                            step,
                            "LLM request completed without usage"
                        );
                    }
                }
            }));
        let outcome = provider
            .as_ref()
            .complete_with_stream_events(request, api_key, on_event, &self.cancel)
            .await;
        // Settle the rows the stream opened from the call's own copy of its
        // items — the same bytes the turn is about to commit. A failure to settle
        // is the call's failure: the log and the model's output would otherwise
        // disagree about what this call produced.
        match &outcome {
            Ok(items) => projection.settle(Some(items))?,
            Err(error) => {
                // Its partial items are still what the agent will persist, so they
                // are what the rows settle with. A lifecycle failure is combined
                // with the provider failure rather than downgraded to a warning:
                // both are part of the failed call's boundary contract.
                let fallback = match error {
                    crate::types::LitecodeError::LlmStreamInterrupted { partial, .. } => {
                        Some(partial.as_slice())
                    }
                    _ => None,
                };
                if let Err(settle_error) = projection.settle(fallback) {
                    return Err(crate::types::LitecodeError::Llm(format!(
                        "{error}; streamed rows could not be settled: {settle_error}"
                    )));
                }
            }
        }
        match outcome {
            Ok(items) => {
                if let Some(usage) = usage_slot.lock().ok().and_then(|mut slot| slot.take()) {
                    let bill = crate::session::store::ProviderUsage {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        cache_hit_tokens: usage.cache_hit_tokens,
                        cache_miss_tokens: usage.cache_miss_tokens,
                    };
                    if let Err(error) =
                        self.sessions
                            .record_provider_usage(&self.session_id, origin_seq, &bill)
                    {
                        tracing::warn!(
                            session_id = %self.session_id,
                            origin_seq,
                            %error,
                            "provider usage was not stored"
                        );
                    }
                }
                Ok(items)
            }
            Err(crate::types::LitecodeError::Canceled) => {
                Err(crate::types::LitecodeError::Canceled)
            }
            Err(e) => {
                // Attach request metadata to the surfaced message (not the
                // returned error — callers see the original). This is what
                // tells "request dropped mid-send because the body was huge"
                // apart from provider/network faults (see transport_error).
                let message = format!(
                    "{e}; model={}, tokens={token_count}, stream=true, body_bytes_est={}",
                    request.model,
                    estimate_request_body_bytes(request),
                );
                // WP-188 observe-only: associate LlmHttp TurnError with wire capture path.
                let turn_id = crate::runtime::diagnostics::DiagContext::snapshot()
                    .map(|s| s.turn_id)
                    .unwrap_or_default();
                crate::runtime::diagnostics::log_turn_error_wire(
                    &self.session_id,
                    &turn_id,
                    &message,
                );
                self.emit_internal(InternalEvent::Error(TurnError {
                    reason: FailReason::LlmHttp,
                    message,
                }));
                Err(e)
            }
        }
    }
}

/// Process-local fingerprint so consecutive steps can tell whether `instructions`
/// changed. The string is assembled once per runtime from the process snapshot
/// of CLAUDE.md / AGENTS.md. Not a cryptographic hash.
fn instructions_fingerprint(instructions: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    instructions.hash(&mut hasher);
    hasher.finish()
}

/// Approximate wire body size in bytes for `request` (Items + tools JSON +
/// instructions). Close to the codec's serialized body — envelope overhead is
/// a few hundred bytes — so it is enough to tell "request dropped mid-send
/// because the body was huge" apart from provider/network faults.
fn estimate_request_body_bytes(request: &ModelRequest) -> usize {
    let items = serde_json::to_string(&request.input)
        .unwrap_or_default()
        .len();
    let tools: usize = request
        .tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "name": t.name,
                "description": t.description,
                "parameters": t.input_schema,
            })
            .to_string()
            .len()
        })
        .sum();
    items + tools + request.instructions.len() + 512
}

/// No FunctionCall → stop. Stop hooks cannot override this after persist.
fn should_stop_after_output(output: &[Item]) -> bool {
    !output.iter().any(|i| matches!(i, Item::FunctionCall(_)))
}

/// Inputs for WP-191 request-health classification (pure; unit-tested).
#[derive(Debug, Clone)]
pub(crate) struct LlmRequestHealthInput {
    pub sticky_model_id: String,
    pub turn_model_ref: String,
    pub turn_provider_id: String,
    pub turn_endpoint_type: String,
    pub bind_matches_sticky: bool,
    pub origin_provider_id: String,
    pub origin_endpoint_type: String,
    pub origin_model_ref: String,
    pub ciphertexts_stripped: u32,
    pub rebased_stripped: u32,
    pub reasoning_dropped: u32,
    pub reasoning_wholly_omitted: u32,
    pub rebased_true_count: u32,
    pub projection_rewrote: bool,
}

/// Classified request-health snapshot carried on `LlmRequestBuilt`.
#[derive(Debug, Clone)]
pub(crate) struct LlmRequestHealth {
    pub sticky_model_id: String,
    pub turn_model_ref: String,
    pub turn_provider_id: String,
    pub turn_endpoint_type: String,
    pub bind_matches_sticky: bool,
    pub origin_provider_id: String,
    pub origin_endpoint_type: String,
    pub origin_model_ref: String,
    pub ciphertexts_stripped: u32,
    pub rebased_stripped: u32,
    pub reasoning_dropped: u32,
    pub reasoning_wholly_omitted: u32,
    pub rebased_true_count: u32,
    pub projection_rewrote: bool,
    pub severity: String,
    pub code: String,
    pub message: String,
}

/// Rank debug > silent. Cross-model thinking loss and a sticky-binding
/// mismatch stay on the request log. They are not this model's own turn
/// dropping its thinking, so they do not toast. Stable `code` values match
/// WP-191 plan §3.2 / §4.
pub(crate) fn classify_llm_request_health(input: LlmRequestHealthInput) -> LlmRequestHealth {
    let (severity, code, message) = if input.reasoning_dropped > 0
        || input.reasoning_wholly_omitted > 0
    {
        (
            "debug",
            "thinking_not_refilled",
            "Prior model thinking was not carried into this request; the model may reason from less context.",
        )
    } else if !input.bind_matches_sticky {
        (
            "debug",
            "bind_sticky_mismatch",
            "Turn model binding does not match the session sticky model.",
        )
    } else if input.ciphertexts_stripped > 0 {
        (
            "debug",
            "foreign_strip",
            "Foreign reasoning ciphertext was stripped; text was retained where applicable.",
        )
    } else if input.rebased_stripped > 0 {
        (
            "debug",
            "rebased_sig_strip",
            "Rebased Messages signatures were stripped (Rule 5).",
        )
    } else if input.projection_rewrote {
        (
            "debug",
            "projection_rewrite",
            "Modality projection rewrote items in this request view.",
        )
    } else {
        ("silent", "", "")
    };

    LlmRequestHealth {
        sticky_model_id: input.sticky_model_id,
        turn_model_ref: input.turn_model_ref,
        turn_provider_id: input.turn_provider_id,
        turn_endpoint_type: input.turn_endpoint_type,
        bind_matches_sticky: input.bind_matches_sticky,
        origin_provider_id: input.origin_provider_id,
        origin_endpoint_type: input.origin_endpoint_type,
        origin_model_ref: input.origin_model_ref,
        ciphertexts_stripped: input.ciphertexts_stripped,
        rebased_stripped: input.rebased_stripped,
        reasoning_dropped: input.reasoning_dropped,
        reasoning_wholly_omitted: input.reasoning_wholly_omitted,
        rebased_true_count: input.rebased_true_count,
        projection_rewrote: input.projection_rewrote,
        severity: severity.into(),
        code: code.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod should_stop_tests {}

#[cfg(test)]
mod estimate_body_bytes_tests {
    use super::estimate_request_body_bytes;
    use crate::llm::ModelRequest;

    fn request(input: Vec<crate::types::Item>) -> ModelRequest {
        ModelRequest {
            model: "test-model".into(),
            instructions: "sys".into(),
            input,
            tools: vec![],
            max_output_tokens: 64,
            thinking: ModelRequest::sample_thinking(),
            json_output: false,
            session_id: None,
            purpose: crate::llm::RequestPurpose::Main,
        }
    }

    #[test]
    fn body_bytes_reflects_instructions() {
        let base = estimate_request_body_bytes(&request(vec![]));
        let mut with_sys = request(vec![]);
        with_sys.instructions = "system prompt ".repeat(10_000);
        assert!(estimate_request_body_bytes(&with_sys) > base);
    }
}

/// WP-090 K5: reason text for a non-waking parent notice in subagent_settled.
fn parent_notice_reason(notice: &crate::tools::subagent::ParentNotice) -> String {
    notice.reason_text()
}

#[cfg(test)]
mod request_health_tests {}

#[cfg(test)]
mod background_compaction_guard_tests {
    use super::BackgroundCompactionGuard;
    use crate::runtime::observer::CompactionTrigger;
    use crate::session::manager::SessionManager;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn guard_seals_an_episode_without_a_terminal_event() {
        let sessions = Arc::new(SessionManager::ephemeral_registry());
        let session_id = sessions.open_session_sync("", "default", None).unwrap();
        assert!(sessions.begin_background_compaction(
            &session_id,
            "bg-guard",
            CancellationToken::new()
        ));
        sessions.begin_compaction_blocking(
            &session_id,
            Some("bg-guard"),
            CompactionTrigger::Auto,
            None,
        );
        assert!(
            sessions
                .compaction_state_blocking(&session_id)
                .active
                .is_some()
        );

        drop(BackgroundCompactionGuard {
            sessions: Arc::clone(&sessions),
            session_id: session_id.clone(),
            operation_id: "bg-guard".into(),
        });

        assert!(!sessions.has_background_compaction(&session_id));
        assert!(
            sessions
                .compaction_state_blocking(&session_id)
                .active
                .is_none()
        );
    }
}
