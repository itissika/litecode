use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

#[cfg(test)]
use std::cell::Cell;

use crate::authority::responses::MessageItem;
use crate::context_pipeline::keep_recent::{build_compaction_prompt, find_keep_recent_cut};
use crate::context_pipeline::media_budget::drop_stale_media;
use crate::llm::{CompactLlmCall, ModelRequest};
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use crate::session::manager::SessionManager;
use crate::session::working::{WorkingRow, project_items};
use crate::types::{Item, LitecodeError, Result, Transcript, item_text_preview};

use super::budget::{BudgetPolicy, ProviderPromptBaseline};
use super::summary::compact_summary_message;

/// Wall-clock cap for the compact LLM call.
///
/// A provider that accepts the request and then goes silent would otherwise pend
/// forever and wedge the session. Generous on purpose: a legitimate summary is
/// ~1-3k output tokens, while a silent peer never answers at all.
const COMPACT_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// Compaction policy and execution.
pub struct CompactPolicy;

impl CompactPolicy {
    pub fn can_compact(budget: &BudgetPolicy, transcript: &Transcript) -> bool {
        find_keep_recent_cut(transcript, budget.keep_recent_tokens).is_some()
    }

    /// One-shot user-triggered compaction.
    ///
    /// Product eligibility is enforced by the caller. This deliberately skips
    /// the automatic 80% policy and post-compact loop reminders.
    pub async fn compact_now(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        llm: CompactLlmCall<'_>,
        system_prompt: &str,
        max_tokens: u32,
        cancel: &CancellationToken,
        operation_id: Option<&str>,
    ) -> Result<bool> {
        let mut rows = sessions.data().working_set_blocking(session_id)?;
        let items = project_items(&rows);
        if !Self::can_compact(budget, &items) {
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
        }
        let prompt_baseline = ProviderPromptBaseline::default();
        let _ = sessions.settle_stale_plan(session_id);
        let did = Self::compact_transcript(
            budget,
            sessions,
            session_id,
            llm,
            system_prompt,
            max_tokens,
            &prompt_baseline,
            &mut rows,
            cancel,
            CompactionTrigger::Manual,
            operation_id,
        )
        .await?;
        if !did {
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
        }
        Ok(true)
    }

    pub async fn compact_if_needed(
        &self,
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        llm: CompactLlmCall<'_>,
        system_prompt: &str,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        step: u64,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        // Returns `true` when a full compaction actually ran — the single source
        // of truth for "did we compact" (the caller drives phase/compaction
        // events from this, not from a duplicate token-budget computation).
        if cancel.is_cancelled() {
            return Ok(false);
        }

        let items = budget_items(rows);
        let token_count = rows_occupancy_count(budget, sessions, session_id, prompt_baseline, rows);
        budget.log_iteration(step, token_count);

        if budget.should_compact(token_count) {
            if cancel.is_cancelled() {
                return Ok(false);
            }

            let prefix_len = persisted_prefix_len(rows);
            if find_keep_recent_cut(&items[..prefix_len], budget.keep_recent_tokens).is_none() {
                tracing::debug!(
                    keep_recent_tokens = budget.keep_recent_tokens,
                    transcript_len = items.len(),
                    prefix_len,
                    "keep-recent: entire persisted prefix within keep window, skipping compact"
                );
                budget.enforce_token_count(token_count)?;
                return Ok(false);
            }

            tracing::info!("token budget > 80%, triggering compaction");
            let did_compact = Self::compact_transcript(
                budget,
                sessions,
                session_id,
                llm,
                system_prompt,
                max_tokens,
                prompt_baseline,
                rows,
                cancel,
                CompactionTrigger::Auto,
                None,
            )
            .await?;

            if cancel.is_cancelled() {
                return Ok(false);
            }
            // Defensive: if compact was skipped (e.g. cut race),
            // still enforce the hard limit so over-budget tokens cannot slip through.
            if !did_compact {
                let count =
                    rows_occupancy_count(budget, sessions, session_id, prompt_baseline, rows);
                budget.enforce_token_count(count)?;
            }
            return Ok(did_compact);
        }

        budget.enforce_token_count(token_count)?;
        Ok(false)
    }

    /// Returns `Ok(true)` when history was rewritten; `Ok(false)` when keep-recent
    /// found nothing to discard (no checkpoint written).
    pub async fn compact_transcript(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        llm: CompactLlmCall<'_>,
        system_prompt: &str,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        rows: &mut Vec<WorkingRow>,
        cancel: &CancellationToken,
        trigger: CompactionTrigger,
        operation_id: Option<&str>,
    ) -> Result<bool> {
        if cancel.is_cancelled() {
            return Ok(false);
        }

        let snapshot = rows.clone();
        // Capture revision with the snapshot so the optimistic lock covers
        // snapshot→persist (ISSUE-035). Fail closed — never unwrap_or(0).
        let expected_revision = sessions.data().revision_blocking(session_id)?;
        let prefix_len = persisted_prefix_len(rows);
        let tail = rows.split_off(prefix_len);
        let mut transcript = project_items(rows);

        let Some(cut) = find_keep_recent_cut(&transcript, budget.keep_recent_tokens) else {
            tracing::debug!(
                keep_recent_tokens = budget.keep_recent_tokens,
                prefix_len,
                "keep-recent: entire persisted prefix within keep window, skipping compact"
            );
            *rows = snapshot;
            return Ok(false);
        };

        let kept_from_seq = match rows.get(cut).and_then(|row| row.log_seq) {
            Some(seq) => seq as i64,
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

        emit_compact_lifecycle(
            sessions,
            session_id,
            trigger,
            CompactionStage::Started,
            operation_id,
            None,
            None,
        );

        let limit = budget.budget_limit();
        let summary_max_tokens = budget.compact_output_tokens(max_tokens);

        let summary = match Self::first_pass_compaction(
            llm,
            system_prompt,
            summary_max_tokens,
            cut,
            &mut transcript,
            session_id,
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

        let final_count = {
            let mut view = transcript.clone();
            view.extend(tail.iter().map(|row| row.item.clone()));
            drop_stale_media(&mut view);
            // The rewritten transcript is not a suffix of the billed request.
            budget.occupancy_token_count(&view, &[], None)
        };
        if final_count > limit {
            *rows = snapshot;
            tracing::error!(
                final_count,
                limit,
                "token budget still exceeded after autocompact"
            );
            let err = LitecodeError::TokenBudgetExceeded;
            emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
            return Err(err);
        }

        // Persist replace; memory working set is reloaded from fold, not rebuilt as [summary]+kept.
        let summary_item = transcript
            .first()
            .cloned()
            .unwrap_or_else(|| compact_summary_message(&summary, false));

        if let Err(e) =
            sessions.mutate_blocking(crate::session::data::command::SessionMutation::Compact {
                session_id: session_id.to_string(),
                expected_revision,
                operation_id: crate::session::data::command::MutationId::new(),
                summary: summary_item,
                token_estimate: final_count as i64,
                kept_from: Some(kept_from_seq as crate::session::event::Seq),
                expected_prefix: Some(prefix_len),
            })
        {
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
                let mut emergency: Vec<WorkingRow> =
                    transcript.into_iter().map(WorkingRow::pending).collect();
                emergency.extend(tail);
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
                return Ok(true);
            }
        };
        model.extend(tail);
        *rows = model;

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
        Ok(true)
    }

    async fn first_pass_compaction(
        llm: CompactLlmCall<'_>,
        system_prompt: &str,
        max_tokens: u32,
        cut: usize,
        transcript: &mut Transcript,
        session_id: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        // LLM input is discarded-only: `transcript` here is already the persist
        // prefix (in-flight tail split off by the caller). Keep-recent stays in
        // `kept` and is never serialized into the compact prompt.
        let discarded = &transcript[..cut];
        let kept = transcript[cut..].to_vec();
        let prompt = build_compaction_prompt(
            discarded,
            summarizer_input_budget(llm.context_window, system_prompt, max_tokens),
        );
        let summary =
            Self::call_llm_compact(llm, system_prompt, &prompt, max_tokens, session_id, cancel)
                .await?;

        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        if summary.is_empty() {
            return Err(LitecodeError::CompactionFailed);
        }

        transcript.clear();
        transcript.push(compact_summary_message(&summary, false));
        transcript.extend(kept);

        tracing::info!(
            summary_len = summary.len(),
            kept = transcript.len().saturating_sub(1),
            "keep-recent compaction succeeded"
        );
        Ok(summary)
    }

    pub(crate) async fn call_llm_compact(
        llm: CompactLlmCall<'_>,
        system: &str,
        prompt: &str,
        max_tokens: u32,
        session_id: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        Self::call_llm_compact_with_timeout(
            llm,
            system,
            prompt,
            max_tokens,
            session_id,
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
        llm: CompactLlmCall<'_>,
        system: &str,
        prompt: &str,
        max_tokens: u32,
        session_id: &str,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<String> {
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        let request = ModelRequest::compact(llm.model, system, prompt, max_tokens, session_id);
        let started = Instant::now();
        let items = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                tracing::info!(
                    session_id,
                    model = llm.model,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "compaction request canceled while waiting for the provider"
                );
                return Err(LitecodeError::Canceled);
            }
            result = tokio::time::timeout(
                timeout,
                crate::llm::reconnect::scope_compaction(llm.provider.complete_with_stream_events(
                    &request,
                    llm.api_key,
                    None,
                    cancel,
                )),
            ) => {
                match result {
                    Ok(Ok(items)) => items,
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {
                        tracing::error!(
                            session_id,
                            model = llm.model,
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
        Ok(summary_text_from_compact_output(&items))
    }
}

/// Room for the compact user message in the summarizer's window. The local
/// count is cl100k and a provider's tokenizer can count higher, so only 80% of
/// what is left after the system prompt and the output reserve is used.
fn summarizer_input_budget(context_window: usize, system: &str, max_output: u32) -> Option<usize> {
    if context_window == 0 {
        return None;
    }
    let reserved = crate::session::estimate::count_text_tokens(system) + max_output as usize;
    Some(context_window.saturating_sub(reserved) * 4 / 5)
}

/// Keep only assistant output text. Reasoning items must not enter the checkpoint.
fn summary_text_from_compact_output(items: &[Item]) -> String {
    items
        .iter()
        .filter(|item| matches!(item, Item::Message(MessageItem::Output(_))))
        .map(item_text_preview)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("")
}

/// Test-only: next post-persist `working_set_blocking` in compact returns Err once.
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
/// that [`drop_stale_media`] removes from the request.
fn budget_items(rows: &[WorkingRow]) -> Vec<Item> {
    budget_items_with_seqs(rows).0
}

fn budget_items_with_seqs(rows: &[WorkingRow]) -> (Vec<Item>, Vec<Option<u64>>) {
    let seqs = rows.iter().map(|row| row.log_seq).collect();
    let mut items = project_items(rows);
    drop_stale_media(&mut items);
    (items, seqs)
}

/// Provider prompt for the latest unretired bill, plus a local estimate of rows
/// written after that request. No bill → the whole working set is local.
fn rows_occupancy_count(
    budget: &BudgetPolicy,
    sessions: &SessionManager,
    session_id: &str,
    baseline: &ProviderPromptBaseline,
    rows: &[WorkingRow],
) -> usize {
    let (items, seqs) = budget_items_with_seqs(rows);
    let bill = super::resolve_occupancy_bill(sessions, session_id, baseline);
    budget.occupancy_token_count(&items, &seqs, bill)
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
    use crate::llm::LlmProvider;
    use crate::types::{StreamEvents, assistant_text, user_text};
    use std::future::Future;
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

    fn compact_llm(provider: &dyn LlmProvider) -> CompactLlmCall<'_> {
        CompactLlmCall {
            provider,
            api_key: "sk-test",
            model: "compact-model",
            context_window: 0,
        }
    }

    #[tokio::test]
    async fn compact_request_times_out_on_silent_provider() {
        let cancel = CancellationToken::new();
        let err = CompactPolicy::call_llm_compact_with_timeout(
            compact_llm(&PendingProvider),
            "system",
            "prompt",
            128,
            "s1",
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
        let err = CompactPolicy::call_llm_compact_with_timeout(
            compact_llm(&PendingProvider),
            "system",
            "prompt",
            128,
            "s1",
            &cancel,
            Duration::from_secs(30),
        )
        .await
        .expect_err("cancel must abort the pending request");
        assert!(matches!(err, LitecodeError::Canceled), "got {err}");
    }

    #[test]
    fn compact_request_disables_thinking_and_uses_caller_cap() {
        let req = ModelRequest::compact("m", "sys", "prompt", 20_480, "s1");
        assert_eq!(req.thinking, crate::platform_knobs::ThinkingSpec::Off);
        assert_eq!(req.max_output_tokens, 20_480);
        assert!(req.tools.is_empty());
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
            .map(|row| item_text_preview(&row.item))
            .collect();
        rows.push(WorkingRow::pending(user_text("pending-tail-after-prefix")));

        inject_reload_err_after_compact_once();

        let budget = BudgetPolicy::new(128_000).with_keep_recent_tokens(1);
        let baseline = ProviderPromptBaseline::default();
        let compacted = CompactPolicy::compact_transcript(
            &budget,
            &sessions,
            &sid,
            compact_llm(&TextProvider),
            "system",
            1024,
            &baseline,
            &mut rows,
            &CancellationToken::new(),
            CompactionTrigger::Manual,
            Some("wp039-reload-err"),
        )
        .await
        .expect("persist ok + reload err must return Ok");
        assert!(compacted, "compaction committed");

        let mem_previews: Vec<String> = rows
            .iter()
            .map(|row| item_text_preview(&row.item))
            .collect();
        assert!(
            mem_previews.first().is_some_and(|t| {
                t.starts_with("[Conversation summary]") && t.contains("## summary")
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
        assert!(
            rows.iter().all(|row| row.log_seq.is_none()),
            "emergency cache rows are pending"
        );

        let db_rows = sessions
            .data()
            .working_set_blocking(&sid)
            .expect("DB working set");
        let db0 = item_text_preview(&db_rows[0].item);
        assert!(
            db0.starts_with("[Conversation summary]") && db0.contains("## summary"),
            "DB must show compacted summary, got {db0}"
        );
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
            "local estimate {local} must clear 80%"
        );
        assert_eq!(
            rows_occupancy_count(&budget, &sessions, &sid, &baseline, &rows),
            1_000
        );
        let provider = CountingProvider::new();
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                compact_llm(&provider),
                "system",
                1024,
                &baseline,
                &mut rows,
                1,
                &CancellationToken::new(),
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
        let provider = CountingProvider::new();
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                compact_llm(&provider),
                "system",
                1024,
                &baseline,
                &mut rows,
                1,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(did, "a provider prompt over 80% must compact");
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let stale = ProviderPromptBaseline::default();
        stale.record(9_000, header);
        let rows = sessions.data().working_set_blocking(&sid).unwrap();
        let count = rows_occupancy_count(&budget, &sessions, &sid, &stale, &rows);
        let local = crate::context_pipeline::estimate::compute_token_estimate(&budget_items(&rows));
        assert_eq!(
            count, local,
            "a compact must retire the old provider prompt"
        );
        assert!(count < budget.autocompact_threshold(), "count {count}");
        let provider = CountingProvider::new();
        let mut rows = rows;
        let did = CompactPolicy
            .compact_if_needed(
                &budget,
                &sessions,
                &sid,
                compact_llm(&provider),
                "system",
                1024,
                &stale,
                &mut rows,
                2,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(!did);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
