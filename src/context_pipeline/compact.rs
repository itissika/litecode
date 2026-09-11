use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::authority::responses::MessageItem;
use crate::context_pipeline::keep_recent::{build_compaction_prompt, find_keep_recent_cut};
use crate::llm::{LlmProvider, ModelRequest};
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use crate::session::event::Seq;
use crate::session::manager::SessionManager;
use crate::types::{Item, LitecodeError, Result, Transcript, item_text_preview, user_text};

use super::budget::{BudgetPolicy, ProviderPromptBaseline};
use super::summary::compact_summary_message_with_reminder;

/// Wall-clock cap for the non-stream compact call.
///
/// [`LlmProvider::complete`] takes no cancel token and the shared HTTP client
/// only sets `connect_timeout`, so a provider that accepts the request and then
/// goes silent would otherwise pend forever and wedge the session. Generous on
/// purpose: a legitimate summary is ~1-3k output tokens, while a silent peer
/// never answers at all.
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
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        max_tokens: u32,
        transcript: &mut Transcript,
        cancel: &CancellationToken,
        operation_id: Option<&str>,
    ) -> Result<bool> {
        crate::session::store::Session::snip_stale_results(transcript);
        if !Self::can_compact(budget, transcript) {
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
        let prefix_len = transcript.len();
        let persisted_seqs: Vec<Seq> = sessions
            .data()
            .working_set_blocking(session_id)?
            .into_iter()
            .filter_map(|row| row.log_seq)
            .collect();
        let reminder = sessions
            .with_entry_task_state(session_id, |state| {
                Ok(crate::context_pipeline::tail_reminders::build_compaction_content(state))
            })
            .ok()
            .flatten();
        let did = Self::compact_transcript(
            budget,
            sessions,
            session_id,
            provider,
            api_key,
            model,
            system_prompt,
            max_tokens,
            &prompt_baseline,
            transcript,
            prefix_len,
            &persisted_seqs,
            reminder.as_deref(),
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
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        transcript: &mut Transcript,
        persisted_prefix_len: usize,
        persisted_seqs: &[Seq],
        reminder: Option<&str>,
        step: u64,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        // Returns `true` when a full compaction actually ran — the single source
        // of truth for "did we compact" (the caller drives phase/compaction
        // events from this, not from a duplicate token-budget computation).
        if cancel.is_cancelled() {
            return Ok(false);
        }

        crate::session::store::Session::snip_stale_results(transcript);

        let token_count = budget.token_count_with_baseline(transcript, prompt_baseline);
        budget.log_iteration(step, token_count);

        if budget.should_compact(token_count) {
            if cancel.is_cancelled() {
                return Ok(false);
            }

            // Know whether keep-recent has anything to discard in the persisted
            // prefix. Length mismatch is Error, never skip.
            let prefix_len = require_persisted_prefix(transcript.len(), persisted_prefix_len)?;
            if find_keep_recent_cut(&transcript[..prefix_len], budget.keep_recent_tokens).is_none()
            {
                tracing::debug!(
                    keep_recent_tokens = budget.keep_recent_tokens,
                    transcript_len = transcript.len(),
                    persisted_prefix_len = prefix_len,
                    "keep-recent: entire persisted prefix within keep window, skipping compact"
                );
                budget.enforce_hard_limit_with_baseline(transcript, prompt_baseline)?;
                return Ok(false);
            }

            tracing::info!("token budget > 80%, triggering compaction");
            let did_compact = Self::compact_transcript(
                budget,
                sessions,
                session_id,
                provider,
                api_key,
                model,
                system_prompt,
                max_tokens,
                prompt_baseline,
                transcript,
                persisted_prefix_len,
                persisted_seqs,
                reminder,
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
                budget.enforce_hard_limit_with_baseline(transcript, prompt_baseline)?;
            }
            return Ok(did_compact);
        }

        budget.enforce_hard_limit_with_baseline(transcript, prompt_baseline)?;
        Ok(false)
    }

    /// Returns `Ok(true)` when history was rewritten; `Ok(false)` when keep-recent
    /// found nothing to discard (no checkpoint written).
    pub async fn compact_transcript(
        budget: &BudgetPolicy,
        sessions: &SessionManager,
        session_id: &str,
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        transcript: &mut Transcript,
        persisted_prefix_len: usize,
        persisted_seqs: &[Seq],
        reminder: Option<&str>,
        cancel: &CancellationToken,
        trigger: CompactionTrigger,
        operation_id: Option<&str>,
    ) -> Result<bool> {
        if cancel.is_cancelled() {
            return Ok(false);
        }

        let prefix_len = match require_persisted_prefix(transcript.len(), persisted_prefix_len) {
            Ok(n) => n,
            Err(e) => {
                emit_compact_failed(sessions, session_id, trigger, operation_id, &e);
                return Err(e);
            }
        };
        let snapshot = transcript.clone();
        let tail = transcript.split_off(prefix_len);

        let Some(cut) = find_keep_recent_cut(transcript, budget.keep_recent_tokens) else {
            tracing::debug!(
                keep_recent_tokens = budget.keep_recent_tokens,
                prefix_len,
                "keep-recent: entire persisted prefix within keep window, skipping compact"
            );
            *transcript = snapshot;
            return Ok(false);
        };

        // Map in-memory cut → original DB seq from the persist working set.
        if persisted_seqs.len() != persisted_prefix_len {
            let err = LitecodeError::ToolExecution(format!(
                "compact cut map: persisted prefix len {persisted_prefix_len} != working seqs {}",
                persisted_seqs.len()
            ));
            *transcript = snapshot;
            emit_compact_lifecycle(
                sessions,
                session_id,
                trigger,
                CompactionStage::Failed,
                operation_id,
                Some(CompactionFailKind::Failed),
                Some(err.to_string()),
            );
            return Err(err.into());
        }
        let kept_from_seq = match persisted_seqs.get(cut).copied() {
            Some(seq) => seq as i64,
            None => {
                *transcript = snapshot;
                let err = LitecodeError::ToolExecution(format!(
                    "compact cut {cut} out of range (persisted prefix len={})",
                    persisted_seqs.len()
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
                return Err(err.into());
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
            provider,
            api_key,
            model,
            system_prompt,
            summary_max_tokens,
            cut,
            transcript,
            reminder,
            session_id,
            cancel,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                *transcript = snapshot;
                emit_compact_failed(sessions, session_id, trigger, operation_id, &e);
                return Err(e);
            }
        };

        if cancel.is_cancelled() {
            *transcript = snapshot;
            let err = LitecodeError::Canceled;
            emit_compact_failed(sessions, session_id, trigger, operation_id, &err);
            return Err(err);
        }

        let final_count = {
            let mut view = transcript.clone();
            view.extend(tail.iter().cloned());
            budget.token_count_with_baseline(&view, prompt_baseline)
        };
        if final_count > limit {
            *transcript = snapshot;
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
            .unwrap_or_else(|| compact_summary_message_with_reminder(&summary, false, reminder));

        if let Err(e) =
            sessions.mutate_blocking(crate::session::data::command::SessionMutation::Compact {
                session_id: session_id.to_string(),
                expected_revision: sessions.data().revision_blocking(session_id).unwrap_or(0),
                operation_id: crate::session::data::command::MutationId::new(),
                summary: summary_item,
                token_estimate: final_count as i64,
                kept_from: Some(kept_from_seq as crate::session::event::Seq),
                expected_prefix: Some(persisted_prefix_len),
            })
        {
            *transcript = snapshot;
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

        // Align in-memory working set with the folded log, plus unpersisted tail.
        let mut model: Transcript = match sessions.data().working_set_blocking(session_id) {
            Ok(rows) => rows.into_iter().map(|row| row.item).collect(),
            Err(e) => {
                *transcript = snapshot;
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
        model.extend(tail);
        *transcript = model;

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
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        max_tokens: u32,
        cut: usize,
        transcript: &mut Transcript,
        reminder: Option<&str>,
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
        let prompt = build_compaction_prompt(discarded);
        let summary = Self::call_llm_compact(
            provider,
            api_key,
            model,
            system_prompt,
            &prompt,
            max_tokens,
            session_id,
            cancel,
        )
        .await?;

        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        if summary.is_empty() {
            return Err(LitecodeError::CompactionFailed);
        }

        transcript.clear();
        transcript.push(compact_summary_message_with_reminder(
            &summary, false, reminder,
        ));
        transcript.extend(kept);

        tracing::info!(
            summary_len = summary.len(),
            kept = transcript.len().saturating_sub(1),
            "keep-recent compaction succeeded"
        );
        Ok(summary)
    }

    pub(crate) async fn call_llm_compact(
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system: &str,
        prompt: &str,
        max_tokens: u32,
        session_id: &str,
        cancel: &CancellationToken,
    ) -> Result<String> {
        Self::call_llm_compact_with_timeout(
            provider,
            api_key,
            model,
            system,
            prompt,
            max_tokens,
            session_id,
            cancel,
            COMPACT_REQUEST_TIMEOUT,
        )
        .await
    }

    /// Cancellable, wall-clock-capped non-stream compact call.
    ///
    /// Cancellation is observed *while awaiting* the provider (not only before
    /// and after), and a silent peer fails the request instead of pending
    /// forever. Dropping the `complete` future aborts the HTTP request.
    async fn call_llm_compact_with_timeout(
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
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

        let request = compact_model_request(model, system, prompt, max_tokens, session_id);
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
            result = tokio::time::timeout(timeout, provider.complete(&request, api_key)) => {
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
        Ok(summary_text_from_compact_output(&items))
    }
}

/// Compact is a one-shot summarizer: no thinking, fixed output cap from the caller.
fn compact_model_request(
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    session_id: &str,
) -> ModelRequest {
    ModelRequest {
        model: model.to_string(),
        instructions: system.to_string(),
        input: vec![user_text(prompt)],
        max_output_tokens: max_tokens,
        temperature: 0.3,
        tools: vec![],
        thinking_mode: Some("disabled".into()),
        reasoning_effort: Some("none".into()),
        json_output: false,
        session_id: Some(session_id.to_string()),
    }
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

/// Compact only the claimed persist prefix. A stale-high cursor is Error, not `min()`.
fn require_persisted_prefix(transcript_len: usize, persisted_prefix_len: usize) -> Result<usize> {
    if persisted_prefix_len > transcript_len {
        return Err(LitecodeError::ToolExecution(format!(
            "compact cut map: persisted prefix len {persisted_prefix_len} > in-memory working set {transcript_len}"
        )));
    }
    Ok(persisted_prefix_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority::responses::{
        OutputStatus, ReasoningItem, ReasoningItemContent, ReasoningTextContent,
    };
    use crate::types::{StreamEvents, assistant_text};
    use std::future::Future;
    use std::pin::Pin;

    /// Non-stream fake whose `complete` never resolves — the silent-provider
    /// failure mode that used to pend the turn forever.
    struct PendingProvider;

    impl LlmProvider for PendingProvider {
        fn endpoint(&self) -> &str {
            "https://compact.invalid/v1"
        }

        fn box_clone(&self) -> Box<dyn LlmProvider> {
            Box::new(PendingProvider)
        }

        fn complete<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            Box::pin(std::future::pending::<Result<Vec<Item>>>())
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            unimplemented!("compact only uses the non-stream path")
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

        fn complete<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            Box::pin(async move { Ok::<_, LitecodeError>(vec![assistant_text("## summary\nkept")]) })
        }

        fn complete_with_stream_events<'a>(
            &'a self,
            _request: &'a ModelRequest,
            _api_key: &'a str,
            _on_event: Option<Box<dyn FnMut(StreamEvents) + Send + 'a>>,
            _cancel: &'a CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Item>>> + Send + 'a>> {
            unimplemented!("compact only uses the non-stream path")
        }
    }

    fn reasoning(text: &str) -> Item {
        Item::Reasoning(ReasoningItem {
            id: Some("rs_compact".into()),
            summary: vec![],
            content: Some(vec![ReasoningItemContent::ReasoningText(
                ReasoningTextContent { text: text.into() },
            )]),
            encrypted_content: None,
            status: Some(OutputStatus::Completed),
        })
    }

    #[tokio::test]
    async fn compact_request_times_out_on_silent_provider() {
        let cancel = CancellationToken::new();
        let err = CompactPolicy::call_llm_compact_with_timeout(
            &PendingProvider,
            "sk-test",
            "compact-model",
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
            &PendingProvider,
            "sk-test",
            "compact-model",
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

    #[tokio::test]
    async fn compact_request_returns_summary_on_provider_success() {
        let cancel = CancellationToken::new();
        let summary = CompactPolicy::call_llm_compact_with_timeout(
            &TextProvider,
            "sk-test",
            "compact-model",
            "system",
            "prompt",
            128,
            "s1",
            &cancel,
            Duration::from_secs(30),
        )
        .await
        .expect("provider answered");
        assert_eq!(summary, "## summary\nkept");
    }

    #[test]
    fn persisted_prefix_gate_errors_when_cursor_exceeds_memory() {
        let err = require_persisted_prefix(8, 10).expect_err("stale-high cursor must fail-closed");
        let msg = err.to_string();
        assert!(
            msg.contains("persisted prefix len 10") && msg.contains("working set 8"),
            "got {msg}"
        );
    }

    #[test]
    fn persisted_prefix_gate_keeps_exact_cursor_when_tail_exists() {
        assert_eq!(require_persisted_prefix(12, 10).unwrap(), 10);
    }

    #[test]
    fn compact_request_disables_thinking_and_uses_caller_cap() {
        let req = compact_model_request("m", "sys", "prompt", 20_480, "s1");
        assert_eq!(req.thinking_mode.as_deref(), Some("disabled"));
        assert_eq!(req.reasoning_effort.as_deref(), Some("none"));
        assert_eq!(req.max_output_tokens, 20_480);
        assert!(req.tools.is_empty());
    }

    #[test]
    fn compact_summary_drops_reasoning_items() {
        let items = vec![
            reasoning("chain of thought leak"),
            assistant_text("1. Primary Request\nFix the compact preview"),
        ];
        let summary = summary_text_from_compact_output(&items);
        assert_eq!(summary, "1. Primary Request\nFix the compact preview");
        assert!(!summary.contains("chain of thought"));
    }

    #[test]
    fn compact_summary_empty_when_only_reasoning() {
        let items = vec![reasoning("only thinking")];
        assert!(summary_text_from_compact_output(&items).is_empty());
    }
}
