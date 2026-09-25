use crate::types::{FunctionToolCall, Item, Result, Transcript};

// On a current_thread runtime, futures need not be Send; keep async fn in traits.
#[allow(async_fn_in_trait)]
pub trait AgentDeps {
    /// Call the model using the prepared LLM view (Items + instructions).
    /// Prepared view from `prepare_step` is the source of truth — not the in-memory
    /// transcript slice. Returns output Items to append verbatim to the transcript.
    async fn call_model(&mut self) -> Result<Vec<Item>>;

    /// Execute tools for the given function calls. FunctionCall Items are already
    /// in `transcript` (from model output); this only appends FunctionCallOutput Items.
    /// On cancellation it appends an "interrupted" output for every call before
    /// returning `Canceled`, so the transcript stays valid for the next turn.
    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()>;

    async fn should_stop(&self, output: &[Item]) -> Result<bool>;

    async fn compact_if_needed(&self, transcript: &mut Transcript, step: u64) -> Result<()>;

    /// Inject independently delivered harness reminders (background
    /// completions, plan-review notices) before the next request is prepared.
    /// Default is a no-op for test/runtime clients without such work.
    fn inject_background_reminders(&mut self, _transcript: &mut Transcript) -> Result<()> {
        Ok(())
    }

    /// Whether queued user messages are waiting for the next request seam.
    ///
    /// The loop must keep stepping while this is true: a message that arrived
    /// while the final response streamed would otherwise be answered by a
    /// follow-up turn instead of steering this one.
    fn has_pending_user_messages(&self) -> bool {
        false
    }

    fn emit_todo_progress(&mut self);
    fn emit_plan_changed(&mut self);

    fn is_cancelled(&self) -> bool;

    fn max_steps(&self) -> u32;

    /// Persist the uncommitted suffix. `Ok(true)` means the log was truncated
    /// under this turn: `items` was replaced with the DB prefix and the delta
    /// was not written.
    fn persist_items(&self, items: &mut Vec<Item>) -> Result<bool>;

    /// Called at the start of each agent-loop step; drives step/phase telemetry.
    fn begin_step(&mut self, step: u64);
}
