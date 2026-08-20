use tokio_util::sync::CancellationToken;

use crate::context_pipeline::Context;
use crate::context_pipeline::keep_recent::{build_compaction_prompt, find_keep_recent_cut};
use crate::hook::{HookAction, HookDispatcher, HookPayload};
use crate::llm::{LlmProvider, ModelRequest};
use crate::runtime::observer::{
    CompactionFailKind, CompactionStage, CompactionTrigger, InternalEvent,
};
use crate::session::manager::SessionManager;
use crate::types::{LitecodeError, Result, Transcript, item_text_preview, user_text};

use super::budget::{BudgetPolicy, ProviderPromptBaseline};
use super::summary::{compact_summary_message_with_reminder, format_compact_summary_with_reminder};

/// Compaction policy and execution.
pub struct CompactPolicy;

impl CompactPolicy {
    pub fn can_compact(budget: &BudgetPolicy, transcript: &Transcript) -> bool {
        find_keep_recent_cut(transcript, budget.keep_recent_tokens).is_some()
    }

    /// One-shot user-triggered compaction.
    ///
    /// Product eligibility is enforced by the caller. This deliberately skips
    /// the automatic 80% policy, hooks, and post-compact loop reminders.
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
        hooks: &HookDispatcher,
        sessions: &SessionManager,
        session_id: &str,
        ctx: &Context,
        provider: &dyn LlmProvider,
        api_key: &str,
        model: &str,
        system_prompt: &str,
        max_tokens: u32,
        prompt_baseline: &ProviderPromptBaseline,
        transcript: &mut Transcript,
        persisted_prefix_len: usize,
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
            // prefix before PreCompact, so we do not fire a false-positive
            // PreCompact when cut is None. Length mismatch is Error, never skip.
            let prefix_len = require_persisted_prefix(transcript.len(), persisted_prefix_len)?;
            if find_keep_recent_cut(&transcript[..prefix_len], budget.keep_recent_tokens).is_none()
            {
                tracing::info!(
                    keep_recent_tokens = budget.keep_recent_tokens,
                    transcript_len = transcript.len(),
                    persisted_prefix_len = prefix_len,
                    "keep-recent: entire persisted prefix within keep window, skipping compact"
                );
                budget.enforce_hard_limit_with_baseline(transcript, prompt_baseline)?;
                return Ok(false);
            }

            let pre_payload = HookPayload::new(
                "PreCompact",
                session_id,
                &ctx.cwd.display().to_string(),
                serde_json::json!({
                    "message_count": transcript.len(),
                    "token_estimate": token_count,
                    "step": step,
                }),
            );
            let pre_output = hooks.fire("PreCompact", &pre_payload, ctx).await;

            if pre_output.action == HookAction::Block {
                tracing::warn!("compaction blocked by PreCompact hook");
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
                reminder,
                cancel,
                CompactionTrigger::Auto,
                None,
            )
            .await?;

            if cancel.is_cancelled() {
                return Ok(false);
            }
            // Defensive: if compact was skipped after PreCompact (e.g. cut race),
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
            tracing::info!(
                keep_recent_tokens = budget.keep_recent_tokens,
                prefix_len,
                "keep-recent: entire persisted prefix within keep window, skipping compact"
            );
            *transcript = snapshot;
            return Ok(false);
        };

        // Pi firstKept: map in-memory cut → original DB detail seq. Compact
        // only the persisted prefix; the uncommitted tail is restored after.
        let kept_from_seq = sessions.with_entry_store(session_id, |s| {
            let seqs = s.model_surface_seqs()?;
            if seqs.len() != persisted_prefix_len {
                return Err(LitecodeError::ToolExecution(format!(
                    "compact cut map: persisted prefix len {persisted_prefix_len} != DB working set {}",
                    seqs.len()
                ))
                .into());
            }
            let seq = seqs.get(cut).ok_or_else(|| {
                LitecodeError::ToolExecution(format!(
                    "compact cut {cut} out of range (persisted prefix len={})",
                    seqs.len()
                ))
            })?;
            Ok(*seq as i64)
        });
        let kept_from_seq = match kept_from_seq {
            Ok(seq) => seq,
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
        let summary_item = transcript.first().cloned().unwrap_or_else(|| {
            crate::types::user_text(format_compact_summary_with_reminder(
                &summary, false, reminder,
            ))
        });

        if let Err(e) = sessions.with_entry_store(session_id, |s| {
            if cancel.is_cancelled() {
                return Err(LitecodeError::Canceled.into());
            }
            Ok(s.apply_compact_checkpoint_checked(
                &summary_item,
                Some(kept_from_seq),
                final_count as i64,
                Some(persisted_prefix_len),
            )?)
        }) {
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
        let mut model = match sessions.with_entry_store(session_id, |s| {
            s.reload_persisted_max_seq()?;
            Ok(s.load_transcript()?)
        }) {
            Ok(items) => items,
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
        if cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }

        let request = ModelRequest {
            model: model.to_string(),
            instructions: system.to_string(),
            input: vec![user_text(prompt)],
            max_output_tokens: max_tokens,
            temperature: 0.3,
            tools: vec![],
            thinking_mode: None,
            reasoning_effort: None,
            json_output: false,
            session_id: Some(session_id.to_string()),
        };

        let items = provider.complete(&request, api_key).await?;
        Ok(items
            .iter()
            .map(item_text_preview)
            .collect::<Vec<_>>()
            .join(""))
    }
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
    emit_compact_lifecycle(
        sessions,
        session_id,
        trigger,
        CompactionStage::Failed,
        operation_id,
        Some(compact_fail_kind(err)),
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
    use super::require_persisted_prefix;

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
}
