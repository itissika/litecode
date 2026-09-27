use crate::types::{FunctionToolCall, Item, Result, Transcript};

// On a current_thread runtime, futures need not be Send; keep async fn in traits.
#[allow(async_fn_in_trait)]
pub trait AgentDeps {
    /// Call the model using the prepared LLM view (Items + instructions).
    /// Prepared view from `prepare_step` is the source of truth — not the in-memory
    /// transcript slice. Returns output Items to append verbatim to the transcript.
    async fn call_model(&mut self) -> Result<Vec<Item>>;

    /// Execute tools for the given function calls. Appends FunctionCallOutput
    /// Items only. On cancellation it appends an "interrupted" output for every
    /// call before returning `Canceled`.
    async fn execute_tools(
        &self,
        tool_uses: &[FunctionToolCall],
        transcript: &mut Transcript,
    ) -> Result<()>;

    async fn should_stop(&self, output: &[Item]) -> Result<bool>;

    /// Compact when the budget says so. `Ok(true)` means a compaction ran and
    /// the caller must sync the request seam again before building the view.
    async fn compact_if_needed(&self, step: u64) -> Result<bool>;

    /// Append harness reminders and queued user messages before the view is built.
    fn sync_request_seam(&mut self, _step: u64) -> Result<()> {
        Ok(())
    }

    /// Build the ephemeral model view from the log after the seam (and any compact).
    fn prepare_view(&mut self, _step: u64) -> Result<()> {
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

    /// Persist this step's new items. `Ok(true)` means the log was truncated
    /// under this turn and the delta was not written.
    fn persist_new(&self, items: &[Item]) -> Result<bool>;

    /// Called at the start of each agent-loop step; drives step/phase telemetry.
    fn begin_step(&mut self, step: u64);
}
