use std::collections::HashMap;
use std::sync::Arc;

use crate::config::bridge::agent_config_from_profile;
use crate::context_pipeline::build_system_prompt;
use crate::llm::ModelRequest;
use crate::llm::ToolDef;
use crate::runtime::llm_resolve::binding_for_agent;
use crate::runtime::observer::{FailReason, InternalEvent, TurnError, TurnPhase, TurnTokenStats};
use crate::runtime::provider_registry::ProviderRegistry;
use crate::session::{apply_prompt_overhead, compute_token_breakdown, count_text_tokens};
use crate::types::{FunctionToolCall, Item, LitecodeError, Result, Transcript};

use crate::agent::AgentDeps;
use crate::tool::executor::{outputs_from_tool_results, run_tool};

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

        let instructions = view
            .instructions
            .clone()
            .unwrap_or_else(|| build_system_prompt(&self.agent_config, &self.rctx().ctx));
        // Fail closed before request build when Items require unsupported modalities.
        crate::runtime::validate_llm_input_capabilities(&view.items, &self.turn_llm.model_def)?;
        let token_count = view.token_count;
        let request = self.build_model_request(&instructions, view.items, token_count);

        // Default path: Responses SSE via complete_with_stream_events → authority
        // ResponseStreamEvent; observer forwards InternalEvent::StreamEvent.
        // Chat opt-in wire may emit adapter-projected ResponseStreamEvent with
        // turn-stable ids; preferred path remains Responses SSE (R2).
        self.call_model_complete(&request, token_count).await
    }

    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()> {
        // `subagent_launch` is session delegation, not a concurrent ToolPipeline
        // job. Other tools keep the existing batch path; launches take a parent
        // capacity lease and run one at a time.
        let step = self.current_step_value();
        self.emit_phase(TurnPhase::ExecutingTools, step);

        let is_cancelled = {
            let cancel = self.cancel.clone();
            move || cancel.is_cancelled()
        };

        let mut i = 0usize;
        while i < tool_uses.len() {
            if is_cancelled() || self.cancel.is_cancelled() {
                crate::tool::executor::outputs_from_tool_results(
                    &tool_uses[i..],
                    std::collections::HashMap::new(),
                    &self.rctx().data_root,
                )
                .into_iter()
                .for_each(|item| transcript.push(item));
                return Err(crate::types::LitecodeError::Canceled);
            }
            if tool_uses[i].name == "subagent_launch" {
                self.execute_subagent_launch(&tool_uses[i], transcript)
                    .await?;
                i += 1;
            } else {
                let start = i;
                while i < tool_uses.len() && tool_uses[i].name != "subagent_launch" {
                    i += 1;
                }
                match self
                    .tool_pipeline
                    .as_ref()
                    .expect("tool_pipeline not initialized")
                    .execute_batch_cancellable(
                        &tool_uses[start..i],
                        transcript,
                        is_cancelled.clone(),
                    )
                    .await
                {
                    Ok(()) => {}
                    Err(LitecodeError::Canceled) => {
                        if i < tool_uses.len() {
                            transcript.extend(outputs_from_tool_results(
                                &tool_uses[i..],
                                HashMap::new(),
                                &self.rctx().data_root,
                            ));
                        }
                        return Err(LitecodeError::Canceled);
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }

    async fn should_stop(&self, output: &[Item]) -> Result<bool> {
        Ok(should_stop_after_output(output))
    }

    async fn compact_if_needed(&self, transcript: &mut Transcript, step: u64) -> Result<()> {
        if self.is_cancelled() {
            return Ok(());
        }

        let compaction_binding = {
            let mut registry = ProviderRegistry::new();
            binding_for_agent(&self.resolved, &mut registry, "compaction", None, 0)?
        };
        let compaction_system = if let Some(profile) = self.resolved.agents().get("compaction") {
            let compaction_agent = agent_config_from_profile(profile);
            crate::context_pipeline::build_compaction_system_prompt(&compaction_agent)
        } else {
            crate::context_pipeline::BUILTIN_COMPACTION.trim().to_string()
        };

        let task_state = self
            .sessions
            .with_entry_task_state(&self.session_id, |s| Ok(s.clone()))?;

        // Single computation: `prepare_step` reports whether a full compaction
        // actually ran; phase/compaction events are driven from that truth so
        // the wire always matches what happened (no duplicate budget math).
        self.context_pipeline
            .prepare_step(
                &self.sessions,
                &self.session_id,
                compaction_binding.provider.as_ref(),
                &compaction_binding.api_key,
                &compaction_binding.api_model_id,
                &compaction_system,
                crate::context_pipeline::keep_recent::COMPACT_MAX_OUTPUT_TOKENS,
                &self.prompt_usage_baseline,
                transcript,
                step,
                &self.cancel,
                &task_state,
                &self.turn_llm.model_def,
            )
            .await?;

        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn max_steps(&self) -> u32 {
        self.agent_config.max_steps
    }

    fn persist_items(&self, items: &mut Vec<Item>) -> Result<bool> {
        let outcome = self.sessions.with_entry_store(&self.session_id, |s| {
            Ok(self.context_pipeline.commit_step_from_items(s, items)?)
        })?;
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
        if let Some((preview, updated_at)) = outcome.preview {
            self.emit_internal(InternalEvent::SessionPreviewUpdated {
                preview,
                updated_at,
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
    async fn execute_subagent_launch(
        &self,
        tu: &FunctionToolCall,
        transcript: &mut Transcript,
    ) -> Result<()> {
        let rctx = self.rctx();
        let lease = match self.sessions.try_acquire_subagent_slot(&self.session_id) {
            Ok(lease) => Some(lease),
            Err(e) => {
                let mut results = HashMap::new();
                results.insert(
                    tu.call_id.clone(),
                    crate::types::ToolCallResult::error(e.to_string()),
                );
                transcript.extend(outputs_from_tool_results(
                    std::slice::from_ref(tu),
                    results,
                    &rctx.data_root,
                ));
                return Ok(());
            }
        };

        let result = run_tool(
            tu,
            &rctx.tools,
            &rctx.permission,
            &rctx.ctx,
            &self.session_id,
            &rctx.agent_name,
            rctx.permission_sink.as_ref(),
            self.cancel.clone(),
            &rctx.data_root,
            rctx.spill_threshold,
            rctx.turn_anchor_k(),
            Arc::clone(&rctx.write_lock),
        )
        .await;
        drop(lease);

        let mut results = HashMap::new();
        results.insert(tu.call_id.clone(), result);
        transcript.extend(outputs_from_tool_results(
            std::slice::from_ref(tu),
            results,
            &rctx.data_root,
        ));

        if self.cancel.is_cancelled() {
            return Err(LitecodeError::Canceled);
        }
        Ok(())
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

    pub(crate) fn build_model_request(
        &self,
        instructions: &str,
        input: Vec<Item>,
        token_count: usize,
    ) -> ModelRequest {
        let tool_schemas = self.rctx().tool_defs();

        let tool_names: Vec<&str> = tool_schemas.iter().map(|t| t.name.as_str()).collect();
        let model = self.turn_llm.api_model_id.clone();
        tracing::info!(
            target: "litecode.debug.llm_request",
            model = %model,
            endpoint = %self.provider().endpoint(),
            tools_count = tool_names.len(),
            tools = ?tool_names,
            token_count = token_count,
            "LLM request built"
        );

        ModelRequest {
            model,
            instructions: instructions.to_string(),
            input,
            max_output_tokens: self.turn_llm.max_tokens,
            temperature: self.agent_config.temperature,
            tools: tool_schemas,
            thinking_mode: self.resolve_thinking_mode(),
            reasoning_effort: self.resolve_reasoning_effort(),
            // Session binding only — never agent.model_ref (decoupled sticky model).
            json_output: self.turn_llm.model_def.json_output(),
            session_id: Some(self.session_id.clone()),
        }
    }

    fn resolve_thinking_mode(&self) -> Option<String> {
        crate::platform_knobs::map_thinking_to_wire(
            &self.turn_llm.model_def.adapter_id,
            self.turn_llm.thinking_tier,
        )
        .0
    }

    fn resolve_reasoning_effort(&self) -> Option<String> {
        crate::platform_knobs::map_thinking_to_wire(
            &self.turn_llm.model_def.adapter_id,
            self.turn_llm.thinking_tier,
        )
        .1
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
        let observer = std::sync::Arc::clone(&self.observer);
        let sessions = Arc::clone(&self.sessions);
        let session_id = self.session_id.clone();
        let provider = &self.turn_llm.provider;
        let api_key = &self.turn_llm.api_key;
        let stats = &mut self.turn_token_stats;
        let totals = &mut self.turn_usage_totals;
        let prompt_usage_baseline = &self.prompt_usage_baseline;
        let request_item_count = request.input.len();
        let on_event: Option<Box<dyn FnMut(crate::types::StreamEvents) + Send + '_>> =
            Some(Box::new(move |ev| {
                if let crate::types::StreamEvents::ResponseOutputItemAdded(added) = &ev {
                    let item = Item::from(added.item.clone());
                    match sessions.with_entry_store(&session_id, |s| {
                        s.persist_item(&item)?;
                        Ok(())
                    }) {
                        Ok(()) => observer.on_internal(InternalEvent::StepCommitted),
                        Err(e) => tracing::warn!(%e, "persist Item at output_item.added failed"),
                    }
                }
                if let crate::types::StreamEvents::ResponseCompleted(cev) = &ev
                    && let Some(usage) = &cev.response.usage
                {
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
                    totals.completion_tokens = totals.completion_tokens.saturating_add(completion);
                    totals.cache_hit_tokens = totals.cache_hit_tokens.saturating_add(cache_hit);
                    totals.cache_miss_tokens = totals.cache_miss_tokens.saturating_add(cache_miss);
                    // Provider truth covers this exact request prefix. The next
                    // tool-loop step adds a local estimate only for appended Items.
                    prompt_usage_baseline.record(prompt, request_item_count);
                    let stop_reason = match &cev.response.incomplete_details {
                        Some(d) => d.reason.clone(),
                        None => format!("{:?}", cev.response.status),
                    };
                    observer.on_internal(InternalEvent::LlmCompleted {
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                        cache_hit_tokens: cache_hit,
                        cache_miss_tokens: cache_miss,
                        stop_reason,
                    });
                }
                observer.on_internal(InternalEvent::StreamEvent(ev));
            }));
        match provider
            .as_ref()
            .complete_with_stream_events(request, api_key, on_event, &self.cancel)
            .await
        {
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

/// Approximate wire body size in bytes for `request` (Items + tools JSON +
/// instructions). Close to the adapter's serialized body — envelope overhead is
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
            temperature: 0.0,
            reasoning_effort: None,
            thinking_mode: None,
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
