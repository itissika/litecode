use std::sync::Arc;

use crate::context_pipeline::build_system_prompt;
use crate::llm::ModelRequest;
use crate::llm::ToolDef;
use crate::runtime::observer::{FailReason, InternalEvent, TurnError, TurnPhase, TurnTokenStats};
use crate::session::{apply_prompt_overhead, compute_token_breakdown, count_text_tokens};
use crate::types::{FunctionToolCall, Item, Result, Transcript};

use crate::agent::AgentDeps;

use super::AgentRuntime;

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
        self.emit_phase(TurnPhase::CallingLlm, step);

        // `compact_if_needed` already ran `prepare_step` and stored the ephemeral PreparedView.
        let view = self.context_pipeline.take_prepared_view().ok_or_else(|| {
            crate::types::LitecodeError::Llm(
                "no prepared PreparedView — prepare_step must run before call_model".into(),
            )
        })?;

        let instructions = view.instructions.clone().unwrap_or_else(|| {
            build_system_prompt(&self.agent_name, &self.agent_config, Some(&self.base_ctx))
        });
        // Fail closed before request build when Items require unsupported modalities.
        crate::runtime::validate_llm_input_capabilities(&view.items, &self.turn_llm.model)?;
        let token_count = view.token_count;
        let request =
            self.build_model_request(&instructions, view.items, view.item_seqs, token_count)?;

        // Default path: Responses SSE via complete_with_stream_events → authority
        // ResponseStreamEvent; observer forwards InternalEvent::StreamEvent.
        // The Chat Completions codec emits projected ResponseStreamEvents with
        // turn-stable ids; preferred path remains Responses SSE (R2).
        self.call_model_complete(&request, token_count).await
    }

    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()> {
        let step = self.current_step_value();
        self.emit_phase(TurnPhase::ExecutingTools, step);

        // Any tool may write disk (including custom); join pre-tool snapshot first.
        self.await_pending_snapshot_track().await;

        let cancel = self.cancel.clone();
        let result = self
            .tool_pipeline
            .as_ref()
            .expect("tool_pipeline not initialized")
            .execute_batch_cancellable(tool_uses, transcript, move || cancel.is_cancelled())
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

        let compaction_binding = self.runtime_handle.resolve_compaction_binding()?;
        let compaction_system = self.runtime_handle.compaction_system_prompt();

        // Fail-open: a stale-plan settlement error must not abort the turn.
        if let Err(error) = self.sessions.settle_stale_plan(&self.session_id) {
            tracing::warn!(
                session_id = %self.session_id,
                error = %error,
                "settle_stale_plan failed; continuing with current reminders"
            );
        }

        self.context_pipeline
            .compact_step(
                &self.sessions,
                &self.session_id,
                compaction_binding.compact_call(),
                &compaction_system,
                crate::context_pipeline::keep_recent::COMPACT_MAX_OUTPUT_TOKENS,
                &self.prompt_usage_baseline,
                step,
                &self.cancel,
            )
            .await
    }

    fn prepare_view(&mut self, _step: u64) -> Result<()> {
        self.context_pipeline.build_view(
            &self.sessions,
            &self.session_id,
            &self.prompt_usage_baseline,
            &self.turn_llm.model,
        )
    }

    fn sync_request_seam(&mut self, step: u64) -> Result<()> {
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

    fn persist_new(&self, items: &[Item]) -> Result<bool> {
        let outcome = self
            .context_pipeline
            .persist_new(&self.sessions, &self.session_id, items)?;
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
    /// Sense the seam and append one row per reminder. Subagent delivery is
    /// fail-closed: a write error puts the completions back and stops the turn.
    /// Other reminder writes are fail-open so a decoration cannot abort the turn.
    fn append_seam_reminders(&mut self, step: u64) -> Result<bool> {
        use crate::reminder::{BashExitEntry, PlanPointer, RunningBash, SettledChild, TodoSnap};
        use crate::reminder::{Facts, ReminderKind, SeamCtx, TaskFacts, sync};
        use crate::tools::bash_status::display_output_path;

        let cwd = self.base_ctx.cwd.clone();
        let turn_id = self.context_pipeline.current_turn_id().unwrap_or_default();
        let ctx = SeamCtx {
            session_id: self.session_id.clone(),
            turn_id,
            step,
            max_steps: u64::from(self.agent_config.max_steps),
            cwd: cwd.clone(),
        };

        let bash_notices = self
            .runtime_ctx
            .as_ref()
            .and_then(|runtime| runtime.tools.iter().find_map(|tool| tool.agent_terminal()))
            .map(|hub| hub.jobs.take_mailbox(&self.session_id))
            .unwrap_or_default();
        let bash_running = self
            .runtime_ctx
            .as_ref()
            .and_then(|runtime| runtime.tools.iter().find_map(|tool| tool.agent_terminal()))
            .map(|hub| hub.jobs.running(&self.session_id))
            .unwrap_or_default();
        let completions = self
            .runtime_handle
            .subagent_hub
            .take_completions(&self.session_id);
        let custom_notices = self
            .runtime_handle
            .custom_tool_hub
            .take_mailbox(&self.session_id);

        let changed = self.sessions.take_changed_files(&self.session_id);
        let changed_paths = changed
            .iter()
            .map(|path| display_output_path(path, &cwd))
            .collect::<Vec<_>>();

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
        let counts = self.sessions.child_counts(&self.session_id);
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
        let settled_detail = if completions.is_empty() {
            String::new()
        } else {
            crate::tools::subagent::status::format_batch_results(&self.sessions, &completions)
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

        let view = match self.sessions.spine_reminder_view(&self.session_id) {
            Ok(view) => view,
            Err(error) => {
                self.restore_seam_sources(&bash_notices, completions, &custom_notices);
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
        let reminders = sync(&ctx, &view, &facts);
        let mut bash_written = false;
        let mut subagent_written = false;
        let mut custom_written = false;
        let mut appended = false;
        for reminder in &reminders {
            match self.sessions.append_reminder(&self.session_id, reminder) {
                Ok(_) => {
                    appended = true;
                    match reminder.kind() {
                        ReminderKind::BashExit => bash_written = true,
                        ReminderKind::SubagentSettled => subagent_written = true,
                        ReminderKind::CustomToolSettled => custom_written = true,
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
                        self.runtime_handle
                            .subagent_hub
                            .restore_completions(&self.session_id, completions.clone());
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
        if !custom_written {
            self.restore_custom_mailbox(&custom_notices);
        }
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

    fn restore_custom_mailbox(
        &self,
        notices: &[crate::tools::custom_hub::CustomToolExitNotice],
    ) {
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
        let Some(runtime) = &self.runtime_ctx else {
            return;
        };
        let Some(hub) = runtime.tools.iter().find_map(|tool| tool.agent_terminal()) else {
            return;
        };
        hub.jobs.restore_mailbox(&self.session_id, notices.to_vec());
    }

    fn emit_llm_request_built(&self, request: &ModelRequest, token_count: usize) {
        // `token_estimate` is local budget telemetry only — never meter/ring truth.
        self.emit_internal(InternalEvent::LlmRequestBuilt {
            model: request.model.clone(),
            endpoint: self.provider().endpoint().to_string(),
            token_estimate: token_count,
            tools_count: request.tools.len(),
            context_window: self.turn_llm.context_window,
            token_breakdown: request_token_breakdown(request),
        });
    }

    /// If the agent touched its active plan this turn, record the on-disk
    /// revision as seen so the next execution turn only reminds on human edits.
    pub(crate) fn sync_active_plan_revision_after_turn(&self, items: &[Item]) {
        let Ok(Some(plan)) = self
            .sessions
            .with_entry_task_state(&self.session_id, |state| Ok(state.active_plan.clone()))
        else {
            return;
        };
        let completed: std::collections::HashSet<&str> = items
            .iter()
            .filter_map(|item| match item {
                Item::FunctionCallOutput(output) => Some(output.call_id.as_str()),
                _ => None,
            })
            .collect();
        let touched = items.iter().any(|item| {
            let Item::FunctionCall(call) = item else {
                return false;
            };
            if !completed.contains(call.call_id.as_str())
                || !matches!(call.name.as_str(), "read" | "write" | "edit")
            {
                return false;
            }
            let Ok(args) = serde_json::from_str::<serde_json::Value>(&call.arguments) else {
                return false;
            };
            args.get("file_path")
                .and_then(|value| value.as_str())
                .is_some_and(|raw| {
                    raw.replace('\\', "/")
                        .trim_start_matches("./")
                        .ends_with(&plan.relative_path)
                })
        });
        if !touched {
            return;
        }
        let path = self
            .sessions
            .plan_dir_path()
            .join(format!("{}.md", plan.slug));
        if let Some(revision) = crate::session::task_state::plan_file_revision(&path) {
            let _ =
                self.sessions
                    .update_active_plan_revision(&self.session_id, &plan.slug, &revision);
        }
    }

    pub(crate) fn build_model_request(
        &self,
        instructions: &str,
        input: Vec<Item>,
        item_seqs: Vec<Option<crate::session::event::Seq>>,
        token_count: usize,
    ) -> Result<ModelRequest> {
        let tool_schemas = self.rctx().tool_defs();

        let tool_names: Vec<&str> = tool_schemas.iter().map(|t| t.name.as_str()).collect();
        let model = self.turn_llm.api_model_id.clone();
        // Producers are read before this request's own header is appended.
        // Ciphertext replay strips foreign blobs and keeps the text. Summary
        // replay (Bailian) drops every reasoning item this provider did not mint,
        // because that input schema requires the id on the item.
        let provider_id = self.turn_llm.provider_id.clone();
        let producers = self.llm_input_producers(&item_seqs);
        let mut input = input;
        let (ciphertexts_stripped, reasoning_dropped) = if self.turn_llm.model.reasoning_replay
            == crate::provider_catalog::ReasoningReplay::Summary
        {
            (
                0,
                crate::llm::retain_own_reasoning(&mut input, &producers, &provider_id),
            )
        } else {
            (
                crate::llm::strip_foreign_ciphertext(&mut input, &producers, &provider_id),
                0,
            )
        };
        // Origin is an append-only control-plane row written before the request:
        // items this request produces inherit its provider.
        let origin_seq = self.record_request_origin()?;
        tracing::info!(
            target: "litecode.debug.llm_request",
            session_id = %self.session_id,
            step = self.current_step_value(),
            model = %model,
            endpoint = %self.provider().endpoint(),
            provider_id = %provider_id,
            origin_seq,
            ciphertexts_stripped,
            reasoning_dropped,
            tools_count = tool_names.len(),
            tools = ?tool_names,
            item_count = input.len(),
            token_count = token_count,
            instructions_len = instructions.len(),
            instructions_fp = %format!("{:016x}", instructions_fingerprint(instructions)),
            "LLM request built"
        );

        Ok(ModelRequest {
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
        })
    }

    /// The provider that produced each input item, through its durable seq and the
    /// session's `request/header` rows. Unreadable history degrades to "unknown".
    /// Ciphertext replay then drops only the ciphertext. Summary replay drops the
    /// reasoning item: that host requires an id this provider minted.
    fn llm_input_producers(
        &self,
        item_seqs: &[Option<crate::session::event::Seq>],
    ) -> Vec<Option<String>> {
        let headers = self
            .sessions
            .request_origins(&self.session_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(seq, body)| {
                let provider = body
                    .get("provider_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                (seq, provider)
            })
            .collect::<Vec<_>>();
        crate::llm::producers_for_seqs(item_seqs, &headers)
    }

    /// Append the request boundary before sending, so the next durable Item row
    /// inherits exactly this turn/step's provider.
    fn record_request_origin(&self) -> Result<crate::session::event::Seq> {
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
    ) -> Result<Vec<Item>> {
        self.emit_llm_request_built(request, token_count);
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
        let request_item_count = request.input.len();
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
                        *stats = TurnTokenStats {
                            prompt_tokens: prompt,
                            completion_tokens: completion,
                            cache_hit_tokens: cache_hit,
                            cache_miss_tokens: cache_miss,
                        };
                        // Turn-total Σ — every request in this tool loop (session cum_*).
                        totals.prompt_tokens = totals.prompt_tokens.saturating_add(prompt);
                        totals.completion_tokens =
                            totals.completion_tokens.saturating_add(completion);
                        totals.cache_hit_tokens = totals.cache_hit_tokens.saturating_add(cache_hit);
                        totals.cache_miss_tokens =
                            totals.cache_miss_tokens.saturating_add(cache_miss);
                        // Provider truth covers this exact request prefix. The next
                        // tool-loop step adds a local estimate only for appended Items.
                        prompt_usage_baseline.record(prompt, request_item_count);
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
            Ok(items) => Ok(items),
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
/// (body / CLAUDE.md splice) changed. Not a cryptographic hash.
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

fn request_token_breakdown(request: &ModelRequest) -> crate::session::estimate::ItemTokenBreakdown {
    let mut bd = compute_token_breakdown(&request.input);
    let schemas: Vec<(String, usize)> = request
        .tools
        .iter()
        .map(|t| (t.name.clone(), tool_schema_tokens(t)))
        .collect();
    apply_prompt_overhead(&mut bd, &request.instructions, &schemas);
    bd
}

fn tool_schema_tokens(tool: &ToolDef) -> usize {
    let payload = serde_json::json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    });
    count_text_tokens(&payload.to_string())
}

/// No FunctionCall → stop. Stop hooks cannot override this after persist.
fn should_stop_after_output(output: &[Item]) -> bool {
    !output.iter().any(|i| matches!(i, Item::FunctionCall(_)))
}

#[cfg(test)]
mod should_stop_tests {
    use super::should_stop_after_output;
    use crate::authority::responses::{
        FunctionToolCall, OutputMessage, OutputMessageContent, OutputTextContent,
    };
    use crate::types::Item;

    fn text_message(text: &str) -> Item {
        Item::Message(crate::authority::responses::MessageItem::Output(
            OutputMessage {
                id: "m1".into(),
                role: crate::authority::responses::AssistantRole::Assistant,
                status: crate::authority::responses::OutputStatus::Completed,
                content: vec![OutputMessageContent::OutputText(OutputTextContent {
                    text: text.into(),
                    annotations: vec![],
                    logprobs: None,
                })],
                phase: None,
            },
        ))
    }

    fn function_call() -> Item {
        Item::FunctionCall(FunctionToolCall {
            arguments: "{}".into(),
            call_id: "c1".into(),
            namespace: None,
            name: "read".into(),
            id: None,
            status: None,
        })
    }

    #[test]
    fn text_only_step_stops() {
        assert!(should_stop_after_output(&[text_message("done")]));
    }

    #[test]
    fn function_call_does_not_stop() {
        assert!(!should_stop_after_output(&[
            text_message("calling"),
            function_call()
        ]));
    }

    #[test]
    fn empty_output_stops() {
        assert!(should_stop_after_output(&[]));
    }
}

#[cfg(test)]
mod estimate_body_bytes_tests {
    use super::estimate_request_body_bytes;
    use crate::llm::ModelRequest;
    use crate::types::user_text;

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
        }
    }

    #[test]
    fn body_bytes_grows_with_input_size() {
        let small = estimate_request_body_bytes(&request(vec![user_text("hello")]));
        let large = estimate_request_body_bytes(&request(vec![user_text("x".repeat(100_000))]));
        assert!(small > 0);
        assert!(large > small);
        assert!(large > 100_000);
    }

    #[test]
    fn body_bytes_reflects_instructions() {
        let base = estimate_request_body_bytes(&request(vec![]));
        let mut with_sys = request(vec![]);
        with_sys.instructions = "system prompt ".repeat(10_000);
        assert!(estimate_request_body_bytes(&with_sys) > base);
    }
}
