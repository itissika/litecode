use super::deps::AgentDeps;
use super::outcome::TurnOutcome;
use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, OutputStatus,
};
use crate::types::{Item, LitecodeError, item_text_preview};

/// Agent loop on authority Items — no second-truth assembly.
///
/// The pipeline's working rows are the turn. Each step only holds the items
/// it just produced:
///
/// 1. `compact_if_needed` then `prepare_view` → ephemeral `PreparedView`
/// 2. `call_model` → this step's output Items
/// 3. `persist_new` those items (a streamed row is claimed by its id)
/// 4. If complete FunctionCalls are present → `execute_tools` appends outputs
/// 5. `persist_new` the outputs
///
/// Cancellation is a seal, not a discard: once `call_model` returns Items they
/// are persisted. Incomplete FunctionCalls are not executed; interrupted
/// outputs are persisted so the next turn never sees a dangling FunctionCall.
pub async fn run(deps: &mut impl AgentDeps) -> TurnOutcome {
    let mut final_text = String::new();
    let mut step = 0u64;
    let max_steps = deps.max_steps() as u64;

    loop {
        if deps.is_cancelled() {
            return TurnOutcome::Cancelled { final_text };
        }

        step += 1;
        if step > max_steps {
            tracing::warn!(step, max_steps, "max_steps reached, stopping");
            return TurnOutcome::MaxSteps { final_text };
        }

        deps.begin_step(step);
        if let Err(error) = deps.sync_request_seam(step) {
            return TurnOutcome::Error(error);
        }

        let compacted = match deps.compact_if_needed(step).await {
            Ok(compacted) => compacted,
            Err(e) => return TurnOutcome::Error(e),
        };
        if compacted && let Err(error) = deps.sync_request_seam(step) {
            return TurnOutcome::Error(error);
        }
        if let Err(error) = deps.prepare_view(step) {
            return TurnOutcome::Error(error);
        }

        let output = match deps.call_model().await {
            Ok(output) => output,
            Err(LitecodeError::Canceled) => {
                return TurnOutcome::Cancelled { final_text };
            }
            Err(LitecodeError::LlmStreamInterrupted { message, partial }) => {
                let tool_uses: Vec<FunctionToolCall> = partial
                    .iter()
                    .filter_map(|item| match item {
                        Item::FunctionCall(call) => Some(call.clone()),
                        _ => None,
                    })
                    .collect();
                let pads = interrupted_outputs(
                    &partial,
                    &tool_uses,
                    "the LLM stream was interrupted before a result arrived",
                );
                let mut step_items = partial;
                step_items.extend(pads);
                if let Some(outcome) = persist_or_stop(deps, &step_items, &final_text) {
                    return outcome;
                }
                return TurnOutcome::Error(LitecodeError::Llm(message));
            }
            Err(e) => return TurnOutcome::Error(e),
        };

        let tool_uses: Vec<FunctionToolCall> = output
            .iter()
            .filter_map(|item| match item {
                Item::FunctionCall(fc) => Some(fc.clone()),
                _ => None,
            })
            .collect();

        // Preview text for TurnOutcome / logging only — never fed back to re-synthesize Items.
        let text = output
            .iter()
            .filter_map(|item| match item {
                Item::Message(_) => {
                    let preview = item_text_preview(item);
                    if preview.is_empty() {
                        None
                    } else {
                        Some(preview)
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        tracing::info!(
            step,
            tool_count = tool_uses.len(),
            text_len = text.len(),
            "agent loop iteration"
        );

        if !text.is_empty() {
            final_text = text;
        }

        if let Some(outcome) = persist_or_stop(deps, &output, &final_text) {
            return outcome;
        }

        let skip_tools =
            deps.is_cancelled() || tool_uses.iter().any(function_call_must_not_execute);

        if !tool_uses.is_empty() && skip_tools {
            let pads = interrupted_outputs(
                &output,
                &tool_uses,
                "the user cancelled the turn before a result arrived",
            );
            if let Some(outcome) = persist_or_stop(deps, &pads, &final_text) {
                return outcome;
            }
            return TurnOutcome::Cancelled { final_text };
        }

        if !tool_uses.is_empty() {
            let mut outputs = Vec::new();
            match deps.execute_tools(&tool_uses, &mut outputs).await {
                Ok(()) => {}
                Err(LitecodeError::Canceled) => {
                    if let Some(outcome) = persist_or_stop(deps, &outputs, &final_text) {
                        return outcome;
                    }
                    return TurnOutcome::Cancelled { final_text };
                }
                Err(e) => return TurnOutcome::Error(e),
            }
            if let Some(outcome) = persist_or_stop(deps, &outputs, &final_text) {
                return outcome;
            }
            deps.emit_todo_progress();
            deps.emit_plan_changed();
            continue;
        }

        if deps.is_cancelled() {
            return TurnOutcome::Cancelled { final_text };
        }

        match deps.should_stop(&output).await {
            Ok(true) => {
                // A queued user message arrived while this step streamed. Keep
                // the turn alive: the next iteration injects it at the seam and
                // the model answers it in the same turn instead of a follow-up.
                if deps.has_pending_user_messages() {
                    continue;
                }
                break;
            }
            Ok(false) => {
                tracing::warn!(step, "should_stop returned false, continuing loop");
            }
            Err(e) => return TurnOutcome::Error(e),
        }
    }

    TurnOutcome::Completed { final_text }
}
fn persist_or_stop(deps: &impl AgentDeps, items: &[Item], final_text: &str) -> Option<TurnOutcome> {
    if items.is_empty() {
        return None;
    }
    match deps.persist_new(items) {
        Err(e) => Some(TurnOutcome::Error(e)),
        // `true` means persist skipped a write because the log shrank (回退).
        // User 取消 is `is_cancelled()` plus 封口, not projection length.
        Ok(true) => Some(TurnOutcome::Cancelled {
            final_text: final_text.to_string(),
        }),
        Ok(false) => None,
    }
}

fn function_call_must_not_execute(fc: &FunctionToolCall) -> bool {
    matches!(
        fc.status,
        Some(OutputStatus::Incomplete | OutputStatus::InProgress)
    )
}

fn interrupted_outputs(
    step_items: &[Item],
    tool_uses: &[FunctionToolCall],
    reason: &str,
) -> Vec<Item> {
    let answered: std::collections::HashSet<String> = step_items
        .iter()
        .filter_map(|item| match item {
            Item::FunctionCallOutput(out) => Some(out.call_id.clone()),
            _ => None,
        })
        .collect();
    tool_uses
        .iter()
        .filter(|fc| !answered.contains(&fc.call_id))
        .map(|fc| {
            Item::FunctionCallOutput(FunctionCallOutputItemParam {
                call_id: fc.call_id.clone(),
                output: FunctionCallOutput::Text(format!(
                    "tool '{}' was interrupted: {reason}",
                    fc.name
                )),
                id: None,
                status: None,
            })
        })
        .collect()
}

#[cfg(test)]
mod pending_continue_tests {
    use super::{AgentDeps, TurnOutcome};
    use crate::types::{FunctionToolCall, Item, Result, Transcript, assistant_text};
    use std::cell::Cell;

    /// Minimal deps: every step returns a final text answer (no tool calls), so
    /// the loop's stop check is what ends the turn.
    struct StopOnceDeps {
        steps: Cell<u64>,
        requests: Cell<u64>,
        pending: Cell<bool>,
    }

    impl AgentDeps for StopOnceDeps {
        async fn call_model(&mut self) -> Result<Vec<Item>> {
            self.requests.set(self.requests.get() + 1);
            Ok(vec![assistant_text("done")])
        }

        async fn execute_tools(
            &self,
            _tool_uses: &[FunctionToolCall],
            _transcript: &mut Transcript,
        ) -> Result<()> {
            unreachable!("no tool calls in this fixture")
        }

        async fn should_stop(&self, _output: &[Item]) -> Result<bool> {
            Ok(true)
        }

        async fn compact_if_needed(&self, _step: u64) -> Result<bool> {
            Ok(false)
        }

        fn emit_todo_progress(&mut self) {}
        fn emit_plan_changed(&mut self) {}

        fn is_cancelled(&self) -> bool {
            false
        }

        fn max_steps(&self) -> u32 {
            10
        }

        fn persist_new(&self, _items: &[Item]) -> Result<bool> {
            Ok(false)
        }

        fn begin_step(&mut self, _step: u64) {
            self.steps.set(self.steps.get() + 1);
        }

        fn has_pending_user_messages(&self) -> bool {
            // True exactly once: the first stop check keeps the turn alive, the
            // second one lets it finish.
            let pending = self.pending.get();
            self.pending.set(false);
            pending
        }
    }

    #[tokio::test]
    async fn stop_check_continues_once_for_a_queued_message() {
        let mut deps = StopOnceDeps {
            steps: Cell::new(0),
            requests: Cell::new(0),
            pending: Cell::new(true),
        };
        let outcome = crate::agent::run(&mut deps).await;
        assert!(matches!(outcome, TurnOutcome::Completed { .. }));
        assert_eq!(
            deps.requests.get(),
            2,
            "a queued message must force one more request"
        );
        assert_eq!(deps.steps.get(), 2);
    }

    #[tokio::test]
    async fn stop_check_breaks_without_pending() {
        let mut deps = StopOnceDeps {
            steps: Cell::new(0),
            requests: Cell::new(0),
            pending: Cell::new(false),
        };
        let outcome = crate::agent::run(&mut deps).await;
        assert!(matches!(outcome, TurnOutcome::Completed { .. }));
        assert_eq!(deps.requests.get(), 1);
    }
}
