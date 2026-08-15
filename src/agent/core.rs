use super::deps::AgentDeps;
use super::outcome::TurnOutcome;
use crate::authority::responses::{
    FunctionCallOutput, FunctionCallOutputItemParam, FunctionToolCall, OutputStatus,
};
use crate::types::{Item, LitecodeError, Transcript, item_text_preview};

/// Agent loop on authority Items — no second-truth assembly.
///
/// Data flow per step:
/// 1. `prepare_step` (via `compact_if_needed`) → ephemeral `PreparedView` Items
/// 2. `call_model` → `output: Vec<Item>` (completed or incomplete seal)
/// 3. Append **all** output Items verbatim to the transcript
/// 4. Persist Item delta (FunctionCall visible before tools run)
/// 5. If complete FunctionCalls present and not cancelled → `execute_tools`
///    appends FunctionCallOutput only
/// 6. Persist again when tools ran; otherwise step already persisted at 4
///
/// Cancellation is a seal, not a discard: once `call_model` returns Items they
/// are extended and persisted. User abort mid-stream is sealed as incomplete
/// Items by the adapter. Incomplete FunctionCalls are not executed; interrupted
/// outputs are appended so the next turn never sees a dangling FunctionCall.
pub async fn run(deps: &mut impl AgentDeps, transcript: &mut Transcript) -> TurnOutcome {
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

        if let Err(e) = deps.compact_if_needed(transcript, step).await {
            return TurnOutcome::Error(e);
        }

        let output = match deps.call_model().await {
            Ok(output) => output,
            Err(LitecodeError::Canceled) => {
                return TurnOutcome::Cancelled { final_text };
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

        let persist_at = transcript.len();
        if !output.is_empty() {
            transcript.extend(output.iter().cloned());
        }

        if let Err(e) = deps.persist_items(transcript) {
            transcript.truncate(persist_at);
            return TurnOutcome::Error(e);
        }

        let skip_tools =
            deps.is_cancelled() || tool_uses.iter().any(function_call_must_not_execute);

        if !tool_uses.is_empty() && skip_tools {
            let before_pad = transcript.len();
            append_interrupted_outputs(transcript, &tool_uses);
            if let Err(e) = deps.persist_items(transcript) {
                transcript.truncate(before_pad);
                return TurnOutcome::Error(e);
            }
            return TurnOutcome::Cancelled { final_text };
        }

        if !tool_uses.is_empty() {
            let before_tools = transcript.len();
            match deps.execute_tools(&tool_uses, transcript).await {
                Ok(()) => {}
                Err(LitecodeError::Canceled) => {
                    if let Err(e) = deps.persist_items(transcript) {
                        transcript.truncate(before_tools);
                        return TurnOutcome::Error(e);
                    }
                    return TurnOutcome::Cancelled { final_text };
                }
                Err(e) => return TurnOutcome::Error(e),
            }
            if let Err(e) = deps.persist_items(transcript) {
                transcript.truncate(before_tools);
                return TurnOutcome::Error(e);
            }
            deps.emit_todo_progress();
            continue;
        }

        if deps.is_cancelled() {
            return TurnOutcome::Cancelled { final_text };
        }

        match deps.should_stop(&output).await {
            Ok(true) => break,
            Ok(false) => {
                tracing::warn!(step, "should_stop returned false, continuing loop");
            }
            Err(e) => return TurnOutcome::Error(e),
        }
    }

    TurnOutcome::Completed { final_text }
}

fn function_call_must_not_execute(fc: &FunctionToolCall) -> bool {
    matches!(
        fc.status,
        Some(OutputStatus::Incomplete | OutputStatus::InProgress)
    )
}

fn append_interrupted_outputs(transcript: &mut Transcript, tool_uses: &[FunctionToolCall]) {
    let answered: std::collections::HashSet<String> = transcript
        .iter()
        .filter_map(|item| match item {
            Item::FunctionCallOutput(out) => Some(out.call_id.clone()),
            _ => None,
        })
        .collect();
    for fc in tool_uses {
        if answered.contains(&fc.call_id) {
            continue;
        }
        transcript.push(Item::FunctionCallOutput(FunctionCallOutputItemParam {
            call_id: fc.call_id.clone(),
            output: FunctionCallOutput::Text(format!(
                "tool '{}' was interrupted: the user cancelled the turn before a result arrived",
                fc.name
            )),
            id: None,
            status: None,
        }));
    }
}
